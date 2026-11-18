//! Build an in-memory indexed tree to measure aggregation and paging without filesystem I/O.
use disk_scan::{aggregate, Identity, Node, NodeKind, Snapshot};
use std::{path::PathBuf, time::Instant};
fn main() {
    let count = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1_000_000);
    let start = Instant::now();
    let mut s = Snapshot::empty(PathBuf::from("/fixture"));
    for id in 0..count {
        let directory = id == 0;
        s.append_legacy_node(Node {
            clone_metadata: None,
            id,
            parent: if directory { None } else { Some(0) },
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
            logical_size: 0,
            allocated_size: 0,
            own_logical_size: if directory { 0 } else { 1024 },
            own_allocated_size: if directory { 0 } else { 4096 },
            is_directory: directory,
            hard_link: false,
            excluded: false,
        })
        .unwrap();
    }
    let construct_ms = start.elapsed().as_millis();
    let a = Instant::now();
    aggregate(&mut s);
    let aggregate_ms = a.elapsed().as_millis();
    let p = Instant::now();
    let mut page: Vec<_> = s
        .children(0)
        .unwrap()
        .iter()
        .map(|id| s.node(*id).unwrap())
        .collect();
    page.sort_unstable_by_key(|n| std::cmp::Reverse(n.allocated_size));
    let sample: Vec<_> = page.into_iter().take(100).map(|n| n.id).collect();
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
        serde_json::json!({"entries":count,"construct_ms":construct_ms,"aggregate_ms":aggregate_ms,"sort_page_ms":p.elapsed().as_millis(),"page_entries":sample.len(),"peak_rss_native_units":rss})
    );
}
