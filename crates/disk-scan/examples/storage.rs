//! Separate untimed storage inspection. Never compare its RSS/timing to scan benchmark runs.
use disk_scan::{aggregate, scan, storage_stats, Identity, Node, NodeKind, ScanOptions, Snapshot};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let snapshot = if args.get(1).map(String::as_str) == Some("--synthetic") {
        let count = args
            .get(2)
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(1_000_000);
        let mut s = Snapshot::empty(PathBuf::from("/fixture"));
        for id in 0..count {
            let directory = id == 0;
            s.append_legacy_node(Node {
                clone_metadata: None,
                id,
                parent: (!directory).then_some(0),
                children: if directory {
                    (1..count).collect()
                } else {
                    vec![]
                },
                name: if directory {
                    "fixture".into()
                } else {
                    format!("entry-{id}")
                },
                path: if directory {
                    PathBuf::from("/fixture")
                } else {
                    PathBuf::from(format!("/fixture/entry-{id}"))
                },
                identity: Identity {
                    device: 1,
                    inode: id as u64,
                    kind: if directory {
                        NodeKind::Directory
                    } else {
                        NodeKind::File
                    },
                },
                allocated_size: 0,
                length: if directory { 0 } else { 1024 },
                own_allocated_size: if directory { 0 } else { 4096 },
                is_directory: directory,
                hard_link: false,
                excluded: false,
            })
            .unwrap();
        }
        aggregate(&mut s);
        s
    } else {
        let root = PathBuf::from(
            args.get(1)
                .expect("usage: storage PATH | --synthetic [count]"),
        );
        scan(
            root,
            ScanOptions::default(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .expect("scan failed")
    };
    println!(
        "{}",
        serde_json::to_string(&storage_stats(&snapshot)).unwrap()
    );
}
