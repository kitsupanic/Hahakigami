use disk_scan::{scan_live, Backend, ScanOptions, ScanQos, Snapshot};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc, RwLock},
};
fn main() {
    let raw_args: Vec<_> = std::env::args().collect();
    let (qos, args) = match qos_arguments(&raw_args) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    let root = PathBuf::from(
        args.get(1)
            .expect("usage: scan PATH [workers] [auto|portable|bulk] [--qos automatic|userInitiated|background]"),
    );
    // Report the effective bound used by the scanner.
    let workers = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8)
        .clamp(1, disk_scan::MAX_WORKERS);
    let backend = match args.get(3).map(String::as_str) {
        Some("portable") => Backend::Portable,
        Some("bulk") => Backend::Bulk,
        _ => Backend::Auto,
    };
    let store = Arc::new(RwLock::new(Snapshot::empty(root.clone())));
    match scan_live(
        root,
        ScanOptions {
            workers,
            backend,
            qos,
            descriptor_limit: None,
        },
        Arc::new(AtomicBool::new(false)),
        store.clone(),
        |_| {},
    ) {
        Ok(()) => {
            let s = store.read().unwrap();
            #[cfg(unix)]
            let rss = unsafe {
                let mut r: libc::rusage = std::mem::zeroed();
                libc::getrusage(libc::RUSAGE_SELF, &mut r);
                r.ru_maxrss
            };
            #[cfg(not(unix))]
            let rss = 0;
            println!(
                "{}",
                serde_json::json!({"accounting":s.accounting,"scan_qos":qos,"descriptor_limit":s.descriptor_limit,"descriptor_peak":s.descriptor_peak,"worker_stats":s.worker_stats,"finalization_us":s.finalization_us,"enumeration_tail":s.enumeration_tail,"stats":s.stats,"state":s.state,"backend":backend,"workers":workers,"peak_rss_native_units":rss})
            );
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

fn qos_arguments(args: &[String]) -> Result<(ScanQos, &[String]), &'static str> {
    let Some(index) = args.iter().position(|arg| arg == "--qos") else {
        return Ok((ScanQos::Automatic, args));
    };
    if index == 0 || index + 2 != args.len() {
        return Err(
            "--qos must be a trailing flag followed by automatic, userInitiated, or background",
        );
    }
    let qos = match args[index + 1].as_str() {
        "automatic" => ScanQos::Automatic,
        "userInitiated" => ScanQos::UserInitiated,
        "background" => ScanQos::Background,
        _ => return Err("unknown QoS: expected automatic, userInitiated, or background"),
    };
    Ok((qos, &args[..index]))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn positional_compatibility_and_optional_qos() {
        for (text, expected, count) in [
            ("scan /path 8 bulk", ScanQos::Automatic, 4),
            ("scan /path --qos background", ScanQos::Background, 2),
            (
                "scan /path 4 portable --qos userInitiated",
                ScanQos::UserInitiated,
                4,
            ),
        ] {
            let a: Vec<_> = text.split_whitespace().map(str::to_string).collect();
            let (qos, remaining) = qos_arguments(&a).unwrap();
            assert_eq!(qos, expected);
            assert_eq!(remaining.len(), count);
        }
    }
    #[test]
    fn rejects_malformed_qos() {
        for text in [
            "scan / --qos",
            "scan / --qos realtime",
            "scan / --qos background extra",
        ] {
            let a: Vec<_> = text.split_whitespace().map(str::to_string).collect();
            assert!(qos_arguments(&a).is_err());
        }
    }
}
