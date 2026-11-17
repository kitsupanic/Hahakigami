//! Headless entrypoint; launching without command arguments retains the desktop flow.
use crate::automation::Service;
use clap::{Args, Parser, Subcommand};
use serde_json::Value;
use std::{io::Write, path::PathBuf, sync::Arc, time::Duration};

#[derive(Parser)]
// Drop the "turned off" sentence from `about` when SAVED_SCANS_ENABLED flips back on.
#[command(
    name = "hahakigami",
    version,
    about = "Metadata-only disk scans and queries. Saved scans are turned off in this version, so list, summary, node, children, search and issues are unavailable."
)]
struct Cli {
    /// Dedicated automation storage directory, separate from desktop saved scans.
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct Page {
    #[arg(long, default_value_t = 0)]
    offset: usize,
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u16).range(1..=1000))]
    limit: u16,
}

#[derive(Subcommand)]
enum Command {
    /// Scan a directory and print the result. Ctrl-C requests cancellation.
    Scan { path: PathBuf },
    /// List retained automation scans, newest first.
    List(Page),
    /// Show recorded sizes, state, coverage, and accounting warnings.
    Summary { scan_id: String },
    /// Show one recorded node by its scan-local ID.
    Node { scan_id: String, node_id: u32 },
    /// List a directory's recorded children, largest allocated size first, folders first.
    Children {
        scan_id: String,
        node_id: u32,
        #[command(flatten)]
        page: Page,
    },
    /// Search recorded filenames without reading file contents.
    Search {
        scan_id: String,
        query: String,
        #[command(flatten)]
        page: Page,
    },
    /// List recorded coverage issues and intentional exclusions.
    Issues {
        scan_id: String,
        #[command(flatten)]
        page: Page,
    },
    /// Serve the local MCP protocol on stdin/stdout until the client disconnects.
    Mcp,
}

fn default_directory() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is unavailable; specify --data-dir")?;
    let home = PathBuf::from(home);
    if !home.is_absolute() {
        return Err("HOME must be absolute; specify --data-dir".into());
    }
    Ok(home.join("Library/Application Support/app.hahakigami.automation"))
}

async fn execute(command: Command, service: Arc<Service>) -> Result<(i32, Option<Value>), String> {
    let result = match command {
        Command::Scan { path } => {
            let scanner = service.clone();
            let task = tokio::task::spawn_blocking(move || scanner.scan(path));
            tokio::select! {
                result = task => result.map_err(|error| error.to_string())??,
                signal = tokio::signal::ctrl_c() => {
                    signal.map_err(|error| error.to_string())?;
                    service.cancel()?;
                    return Ok((130, Some(service.status()?)));
                }
            }
        }
        Command::List(page) => service.list(page.offset, usize::from(page.limit))?,
        Command::Summary { scan_id } => service.summary(&scan_id)?,
        Command::Node { scan_id, node_id } => service.node(&scan_id, node_id)?,
        Command::Children {
            scan_id,
            node_id,
            page,
        } => service.children(&scan_id, node_id, page.offset, usize::from(page.limit))?,
        Command::Search {
            scan_id,
            query,
            page,
        } => service.search(&scan_id, &query, page.offset, usize::from(page.limit))?,
        Command::Issues { scan_id, page } => {
            service.issues(&scan_id, page.offset, usize::from(page.limit))?
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
        let directory = cli.data_dir.map_or_else(default_directory, Ok)?;
        let service = Arc::new(Service::new(directory));
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
