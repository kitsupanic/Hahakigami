//! Headless entrypoint; launching without command arguments retains the desktop flow.
use crate::automation::Service;
use clap::{Parser, Subcommand};
use serde_json::Value;
use std::{io::Write, path::PathBuf, sync::Arc, time::Duration};

#[derive(Parser)]
#[command(
    name = "hahakigami",
    version,
    about = "Metadata-only disk scans and a local MCP server."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan a directory and print the result. Ctrl-C requests cancellation.
    Scan { path: PathBuf },
    /// Serve the local MCP protocol on stdin/stdout until the client disconnects.
    Mcp,
}

async fn execute(command: Command, service: Arc<Service>) -> Result<(i32, Option<Value>), String> {
    let result = match command {
        Command::Scan { path } => {
            let scanner = service.clone();
            let mut task = tokio::task::spawn_blocking(move || scanner.scan(path));
            tokio::select! {
                result = &mut task => result.map_err(|error| error.to_string())??,
                signal = tokio::signal::ctrl_c() => {
                    signal.map_err(|error| error.to_string())?;
                    service.cancel()?;
                    // Let the scanner see the flag so the status shows the cancelled partial result, not "running".
                    let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
                    return Ok((130, Some(service.status()?)));
                }
            }
        }
        Command::Mcp => {
            tokio::select! {
                result = crate::mcp::serve(service.clone()) => result?,
                signal = tokio::signal::ctrl_c() => {
                    signal.map_err(|error| error.to_string())?;
                    let _ = service.cancel();
                    return Ok((130, None));
                }
            }
            return Ok((0, None));
        }
    };
    Ok((0, Some(result)))
}

/// Return an exit code for command mode, or None for an ordinary desktop launch.
pub fn run() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() <= 1
        || args[1..]
            .iter()
            .all(|arg| arg.as_encoded_bytes().starts_with(b"-psn_"))
    {
        return None;
    }
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            let _ = error.print();
            return Some(error.exit_code());
        }
    };
    let result = (|| {
        let service = Arc::new(Service::new());
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        let result = runtime.block_on(execute(cli.command, service.clone()));
        let _ = service.cancel();
        // A filesystem read may outlive cancellation; command shutdown stays bounded.
        runtime.shutdown_timeout(Duration::from_millis(200));
        let (code, value) = result?;
        if let Some(value) = value {
            let mut stdout = std::io::stdout().lock();
            serde_json::to_writer(&mut stdout, &value).map_err(|error| error.to_string())?;
            writeln!(stdout).map_err(|error| error.to_string())?;
        }
        Ok::<_, String>(code)
    })();
    Some(match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    })
}
