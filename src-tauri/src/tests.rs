use super::*;
use cleanup::{TrashOutcome, TrashOutcomeStatus};
use disk_scan::Node;
use std::{
    sync::{atomic::AtomicBool, RwLock},
    time::Duration,
};
use tauri::Manager;

fn app(state: AppState) -> tauri::App<tauri::test::MockRuntime> {
    // Fixtures that scan model Full Disk Access as on; the real probe depends on the machine.
    let config = tempfile::tempdir().unwrap();
    *state.cleanup_queue.lock().unwrap() =
        cleanup_queue::Queue::load(config.path().join("cleanup-queue"));
    force_access(&state, access_setup::Status::On);
    tauri::test::mock_builder()
        .manage(state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap()
}

fn force_access(state: &AppState, status: access_setup::Status) {
    *state.access_setup.lock().unwrap() = access_setup::AccessSetup::forcing(status);
}

const FDA_OFF: &str = "FULL_DISK_ACCESS_OFF";

#[test]
fn full_disk_access_status_reports_the_contract_shape() {
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    force_access(&state, access_setup::Status::Off);
    assert_eq!(
        serde_json::to_value(
            tauri::async_runtime::block_on(full_disk_access_status(app.state())).unwrap()
        )
        .unwrap(),
        serde_json::json!({ "status": "off", "scanAnyway": false, "appPath": "" })
    );
    let info = serde_json::to_value(
        tauri::async_runtime::block_on(scan_without_full_disk_access(app.state())).unwrap(),
    )
    .unwrap();
    assert_eq!(
        info,
        serde_json::json!({ "status": "off", "scanAnyway": true, "appPath": "" })
    );
    force_access(&state, access_setup::Status::Unknown);
    assert_eq!(
        serde_json::to_value(
            tauri::async_runtime::block_on(full_disk_access_status(app.state())).unwrap()
        )
        .unwrap()["status"],
        "unknown"
    );
}

#[test]
fn scan_starts_are_refused_only_while_off_unless_scanning_anyway() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        for (status, refused) in [
            (access_setup::Status::Off, true),
            (access_setup::Status::On, false),
            (access_setup::Status::Unknown, false),
        ] {
            force_access(&state, status);
            let start = start_scan(
                app.state(),
                "missing".into(),
                Channel::new(|_| Ok(())),
                app.handle().clone(),
            )
            .await
            .unwrap_err();
            assert_eq!(start == FDA_OFF, refused, "{status:?}: {start}");
        }
        force_access(&state, access_setup::Status::Off);
        scan_without_full_disk_access(app.state()).await.unwrap();
        let start = start_scan(
            app.state(),
            "missing".into(),
            Channel::new(|_| Ok(())),
            app.handle().clone(),
        )
        .await
        .unwrap_err();
        assert_ne!(start, FDA_OFF);
        assert!(state.scans.sessions().is_empty());
    });
}

#[test]
fn only_scan_starts_are_gated_by_full_disk_access() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let s = install(&state, synthetic(2));
        force_access(&state, access_setup::Status::Off);
        // choose_folder takes the production AppHandle, so it cannot run here;
        // its gate was removed with the others and UI tests cover the picker.
        let results = [
            preview_item(&s, 1).err(),
            quick_look_node(app.state(), s.id.clone(), 1, app.handle().clone())
                .await
                .err(),
            prepare_trash(app.state(), s.id.clone(), vec![1])
                .await
                .err(),
            commit_trash(
                app.state(),
                s.id.clone(),
                "ticket".into(),
                app.handle().clone(),
            )
            .await
            .err(),
            add_cleanup_queue(app.state(), s.id.clone(), vec![1])
                .await
                .err(),
            cleanup_queue_target(app.state(), "missing".into())
                .await
                .err(),
            prepare_cleanup_queue(app.state(), s.id.clone()).await.err(),
        ];
        for error in results {
            assert_ne!(error.as_deref(), Some(FDA_OFF));
        }
    });
}

#[test]
fn file_and_cleanup_commands_refuse_a_scan_that_is_not_usable_in_the_same_words() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let s = install(&state, synthetic(2));
        let id = || s.id.clone();
        // A scan still walking, or ended without completing, is not ready for cleanup.
        for (scan_state, finishing) in [
            (ScanState::Running, true),
            (ScanState::Cancelled, false),
            (ScanState::Failed, false),
        ] {
            s.store.write().unwrap().state = scan_state;
            s.finishing.store(finishing, Ordering::Release);
            let errors = [
                prepare_trash(app.state(), id(), vec![1]).await.err(),
                add_cleanup_queue(app.state(), id(), vec![1]).await.err(),
            ];
            for error in errors {
                assert_eq!(error.as_deref(), Some(slots::NOT_COMPLETED));
            }
            assert_eq!(
                preview_item(&s, 1).err().as_deref(),
                Some(slots::NOT_COMPLETED)
            );
        }
        s.store.write().unwrap().state = ScanState::Running;
        s.finishing.store(true, Ordering::Release);
        let errors = [
            commit_trash(app.state(), id(), "ticket".into(), app.handle().clone())
                .await
                .err(),
            prepare_cleanup_queue(app.state(), id()).await.err(),
        ];
        for error in errors {
            assert_eq!(error.as_deref(), Some(slots::NOT_COMPLETED));
        }
        // Showing a file in Finder needs a live scan, not a finished one: it gets as far as the lookup.
        assert_eq!(
            reveal_node(app.state(), id(), 9999).await.err().as_deref(),
            Some("Unknown node")
        );
        // Once a cleanup or a rescan invalidated it, every one of them says so.
        s.invalidate();
        let errors = [
            prepare_trash(app.state(), id(), vec![1]).await.err(),
            add_cleanup_queue(app.state(), id(), vec![1]).await.err(),
            prepare_cleanup_queue(app.state(), id()).await.err(),
            commit_trash(app.state(), id(), "ticket".into(), app.handle().clone())
                .await
                .err(),
            reveal_node(app.state(), id(), 1).await.err(),
            quick_look_node(app.state(), id(), 1, app.handle().clone())
                .await
                .err(),
        ];
        for error in errors {
            assert_eq!(error.as_deref(), Some(slots::RESULTS_CHANGED));
        }
        assert!(!state.activity.busy());
    });
}

#[test]
fn restart_waits_for_a_running_scan_whatever_the_access_status() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let mut snapshot = synthetic(1);
        snapshot.state = ScanState::Running;
        install(&state, snapshot);
        for status in [
            access_setup::Status::On,
            access_setup::Status::Off,
            access_setup::Status::Unknown,
        ] {
            force_access(&state, status);
            let error = restart_app(app.handle().clone(), app.state())
                .await
                .unwrap_err();
            assert!(error.contains("scan"), "{status:?}: {error}");
        }
    });
}

#[test]
fn retired_worker_preferences_cannot_override_new_scans_or_change_existing_sessions() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let mut old_snapshot = synthetic(1);
        old_snapshot.state = ScanState::Running;
        let old = install(&state, old_snapshot);
        assert_eq!(old.summary().qos, disk_scan::ScanQos::Automatic);
        assert_eq!(old.summary().workers, 8);
        assert_eq!(old.summary().state, "running");
        old.store.write().unwrap().state = ScanState::Completed;
        let fixture = fixture();
        let root = std::fs::canonicalize(fixture.path()).unwrap();
        std::fs::write(root.join("file"), b"abc").unwrap();
        let target = register(&state, platform::target(&root, "fixture".into()), root).unwrap();
        let (id, events) = start_recorded(&state, &target, Recorder::default()).unwrap();
        let workers = state.scans.find(&id).unwrap().workers;
        assert!((1..=6).contains(&workers));
        let end = events.wait_end();
        for summary in events.all() {
            assert_eq!(summary.workers, workers);
            assert_eq!(summary.worker_mode, settings::WorkerMode::Auto);
            assert_eq!(summary.worker_source, "hardware");
            assert_eq!(summary.qos, disk_scan::ScanQos::Automatic);
        }
        assert_eq!(end.state, "completed");
        assert_eq!(end.entries, 2);
        #[cfg(target_os = "macos")]
        assert!(end.worker_stats.iter().all(|row| row.qos_error.is_none()));
    });
}
/// The test `Progress` adapter: keeps every summary a scan reports, so a test can wait for the final one.
#[derive(Clone, Default)]
struct Recorder {
    events: Arc<(Mutex<Vec<Summary>>, std::sync::Condvar)>,
    // Panics on the first report instead, as a panic anywhere in the scan would.
    panic_first: Arc<AtomicBool>,
}
impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Recorder")
    }
}
impl Progress for Recorder {
    fn report(&self, summary: Summary) -> bool {
        if self.panic_first.swap(false, Ordering::SeqCst) {
            panic!("scan thread failed");
        }
        let (events, reported) = &*self.events;
        events.lock().unwrap().push(summary);
        reported.notify_all();
        true
    }
}
impl Recorder {
    fn panicking() -> Self {
        let recorder = Self::default();
        recorder.panic_first.store(true, Ordering::SeqCst);
        recorder
    }
    /// Waits for the scan's final report (phase "done", sent once it no longer reads as running) and returns it.
    fn wait_end(&self) -> Summary {
        let (events, reported) = &*self.events;
        let (events, wait) = reported
            .wait_timeout_while(events.lock().unwrap(), Duration::from_secs(15), |events| {
                events.last().is_none_or(|s| s.scan_phase != "done")
            })
            .unwrap();
        assert!(!wait.timed_out(), "the scan did not end");
        events.last().unwrap().clone()
    }
    fn all(&self) -> Vec<Summary> {
        self.events.0.lock().unwrap().clone()
    }
}
/// Starts a scan of `target` as `start_scan` does past its gate and Quick Look close.
fn start_recorded(
    state: &AppState,
    target: &platform::Target,
    events: Recorder,
) -> Result<(String, Recorder), String> {
    start_scan_impl(state, target.id.clone(), events.clone()).map(|id| (id, events))
}
pub(crate) fn install(state: &AppState, snapshot: Snapshot) -> Arc<Session> {
    install_in(state, Slot::Folder, "fixture", snapshot)
}
fn install_in(state: &AppState, slot: Slot, id: &str, snapshot: Snapshot) -> Arc<Session> {
    install_on(state, slot, id, snapshot, None)
}
/// `install_in`, but with every device of the scan recorded on `volume` when given, as if the disk had been
/// reformatted between scans.
fn install_on(
    state: &AppState,
    slot: Slot,
    id: &str,
    snapshot: Snapshot,
    volume: Option<safety::Volume>,
) -> Arc<Session> {
    let root_ancestors = safety::ancestors(&snapshot.root).unwrap_or_default();
    let mut volumes = scan_volumes(&snapshot);
    if let Some(volume) = volume {
        volumes.values_mut().for_each(|v| *v = volume.clone());
    }
    let mut s = Session::live(
        Request {
            root: snapshot.root.clone(),
            root_ancestors,
            workers: 8,
            cached_validation_workers: 8,
            worker_mode: settings::WorkerMode::Auto,
            worker_source: "manual".into(),
            qos: disk_scan::ScanQos::Automatic,
        },
        None,
    );
    // A finished scan rather than one just started.
    s.id = id.into();
    s.completed_at_ms = AtomicU64::new(now_ms());
    s.finishing = AtomicBool::new(false);
    s.scan_details = Mutex::new(ScanDetails::default());
    s.store = Arc::new(RwLock::new(snapshot));
    s.volumes = volumes.into();
    let s = Arc::new(s);
    state.scans.install(slot, s.clone());
    s
}
fn fixture() -> tempfile::TempDir {
    tempfile::tempdir_in(std::env::var_os("HOME").unwrap()).unwrap()
}
pub(crate) fn snapshot(root: PathBuf) -> Snapshot {
    disk_scan::scan(
        root,
        disk_scan::ScanOptions::default(),
        Arc::new(AtomicBool::new(false)),
        |_| {},
    )
    .unwrap()
}
fn synthetic(count: u32) -> Snapshot {
    let root = PathBuf::from("/fixture");
    let mut s = Snapshot::empty(root.clone());
    for id in 0..=count {
        let directory = id == 0;
        s.append_legacy_node(Node {
            id,
            parent: if directory { None } else { Some(0) },
            children: if directory {
                (1..=count).collect()
            } else {
                vec![]
            },
            name: format!("item-{id:08}"),
            path: root.join(format!("item-{id:08}")),
            identity: disk_scan::Identity {
                device: 1,
                inode: id as u64,
                kind: if directory {
                    NodeKind::Directory
                } else {
                    NodeKind::File
                },
            },
            allocated_size: if directory { count as u64 * 4096 } else { 4096 },
            length: if directory { 0 } else { 4096 },
            own_allocated_size: if directory { 0 } else { 4096 },
            is_directory: directory,
            hard_link: false,
            clone_metadata: None,
            excluded: false,
        })
        .unwrap();
    }
    s.state = ScanState::Completed;
    s.stats.entries = count as u64 + 1;
    s
}

#[test]
fn bounded_map_preserves_sizes_and_pagination() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        install(&state, synthetic(6000));
        let map = get_map(app.state(), "fixture".into(), 0, usize::MAX, None)
            .await
            .unwrap();
        assert_eq!(map.nodes.len(), 5000);
        assert_eq!(
            map.nodes
                .iter()
                .skip(1)
                .map(|n| n.allocated_bytes)
                .sum::<u64>(),
            6000 * 4096
        );
        let aggregate = map.nodes.last().unwrap();
        assert_eq!(aggregate.kind, "aggregate");
        assert!(aggregate.id >= 0x80000000);
        let page = get_children(app.state(), "fixture".into(), 0, 500, 1000, None)
            .await
            .unwrap();
        assert_eq!(page.items.len(), 500);
        assert_eq!(page.total, 6000);
        assert_eq!(page.items[0].name, "item-00000501");
        assert!(get_node(app.state(), "stale".into(), 0).await.is_err());
    });
}
// Oracle consumes the original owned DTO and preserves the previous display
// conversion/native issue rules without any compact path/name accessors.
fn legacy_node_view_json(node: &Node, issues: &[PathBuf]) -> serde_json::Value {
    serde_json::json!({
        "id":node.id,"parentId":node.parent,"name":node.name,
        "path":node.path.to_string_lossy(),
        "pathKey":path_key(&node.path),
        "kind":match node.identity.kind {NodeKind::Directory=>"directory",NodeKind::File=>"file",NodeKind::Symlink=>"symlink",NodeKind::Other=>"other"},
        "allocatedBytes":node.allocated_size,
        "rawAllocatedBytes":node.own_allocated_size,"verifiedClone":false,
        "cloneDeductionBytes":0,"cloneOwner":null,
        "childrenCount":node.children.len(),
        "incomplete":issues.get(issues.partition_point(|path|path<&node.path)).is_some_and(|path|path.starts_with(&node.path)),
        "excluded":node.excluded
    })
}
fn view_dto(id: u32, path: PathBuf, name: &str) -> Node {
    let root = id == 0;
    Node {
        id,
        parent: if root { None } else { Some(0) },
        children: if root { vec![1, 2] } else { vec![] },
        name: name.into(),
        path,
        identity: disk_scan::Identity {
            device: 7,
            inode: u64::from(id) + 40,
            kind: if root {
                NodeKind::Directory
            } else {
                NodeKind::File
            },
        },
        allocated_size: 4096,
        length: if root { 0 } else { 11 },
        own_allocated_size: if root { 0 } else { 4096 },
        is_directory: root,
        hard_link: false,
        clone_metadata: None,
        excluded: false,
    }
}
#[test]
fn clone_view_distinguishes_raw_allocation_from_attributed_data() {
    let mut snapshot = Snapshot::empty(PathBuf::from("/scope"));
    for id in 0..=2 {
        let name = match id {
            0 => "scope",
            1 => "a",
            _ => "z",
        };
        let path = if id == 0 {
            PathBuf::from("/scope")
        } else {
            PathBuf::from("/scope").join(name)
        };
        let mut node = view_dto(id, path, name);
        if id > 0 {
            node.own_allocated_size = 100;
            node.clone_metadata = Some(disk_scan::CloneMetadata {
                clone_id: 42,
                data_allocated_size: 64,
                data_length: 11,
                reference_count: 2,
                may_share_blocks: true,
                verified: true,
            });
        }
        snapshot.append_legacy_node(node).unwrap();
    }
    snapshot.accounting.model = disk_scan::AccountingModel::FullClonesV1;
    disk_scan::aggregate(&mut snapshot);
    let owner = view(&snapshot.node(1).unwrap(), &[]).unwrap();
    let duplicate = view(&snapshot.node(2).unwrap(), &[]).unwrap();
    assert_eq!(
        (
            owner.raw_allocated_bytes,
            owner.allocated_bytes,
            owner.clone_owner
        ),
        (100, 100, Some(true))
    );
    assert_eq!(
        (
            duplicate.raw_allocated_bytes,
            duplicate.allocated_bytes,
            duplicate.clone_deduction_bytes
        ),
        (100, 36, 64)
    );
    assert!(duplicate.verified_clone);
}
#[test]
fn node_view_consumed_path_matches_legacy_display_and_historical_overrides() {
    use std::os::unix::ffi::OsStringExt;
    for bytes in [
        b"/scope/file".as_slice(),
        "/scope/Юнікод 😀".as_bytes(),
        b"/scope/nul\0file".as_slice(),
        b"/scope/native-\xff".as_slice(),
        b"/scope/native-\xfe".as_slice(),
        b"/scope/truncated-\xe2\x82".as_slice(),
        b"/scope//file".as_slice(),
        b"/scope/./file".as_slice(),
    ] {
        for root_spelling in ["/scope", "/scope//", "/scope/./"] {
            let root = view_dto(0, root_spelling.into(), "historical root label");
            let leaf = view_dto(
                1,
                std::ffi::OsString::from_vec(bytes.to_vec()).into(),
                "historical leaf label 🧪",
            );
            let mut snapshot = Snapshot::empty("/scope".into());
            snapshot.append_legacy_node(root.clone()).unwrap();
            snapshot.append_legacy_node(leaf.clone()).unwrap();
            let mut issues = vec![leaf.path.join("unreadable child")];
            issues.sort();
            for node in [&root, &leaf] {
                assert_eq!(
                    serde_json::to_value(view(&snapshot.node(node.id).unwrap(), &issues).unwrap())
                        .unwrap(),
                    legacy_node_view_json(node, &issues)
                );
            }
        }
    }
}
#[test]
fn node_view_lossy_collisions_keep_native_issue_matching() {
    use std::os::unix::ffi::OsStringExt;
    let first = view_dto(
        1,
        std::ffi::OsString::from_vec(b"/scope/native-\xff".to_vec()).into(),
        "same display name",
    );
    let second = view_dto(
        2,
        std::ffi::OsString::from_vec(b"/scope/native-\xfe".to_vec()).into(),
        "same display name",
    );
    assert_eq!(first.path.to_string_lossy(), second.path.to_string_lossy());
    let root = view_dto(0, "/scope".into(), "scope");
    let mut snapshot = Snapshot::empty("/scope".into());
    for node in [&root, &first, &second] {
        snapshot.append_legacy_node(node.clone()).unwrap();
    }
    let issues = vec![first.path.join("issue")];
    let first_view = view(&snapshot.node(1).unwrap(), &issues).unwrap();
    let second_view = view(&snapshot.node(2).unwrap(), &issues).unwrap();
    assert_eq!(first_view.path, second_view.path);
    assert_eq!(first_view.path_key, "/scope/native-%FF");
    assert_eq!(second_view.path_key, "/scope/native-%FE");
    assert!(first_view.incomplete);
    assert!(!second_view.incomplete);
    assert_eq!(
        serde_json::to_value(first_view).unwrap(),
        legacy_node_view_json(&first, &issues)
    );
    assert_eq!(
        serde_json::to_value(second_view).unwrap(),
        legacy_node_view_json(&second, &issues)
    );
}

#[test]
fn native_path_keys_preserve_components_without_display_collisions() {
    use std::os::unix::ffi::OsStringExt;
    for (native, expected) in [
        (
            "/scope/é %/'()!~*_.-".as_bytes(),
            "/scope/%C3%A9%20%25/'()!~*_.-",
        ),
        (b"/scope/native-\xff".as_slice(), "/scope/native-%FF"),
        (b"/scope/native-%FF".as_slice(), "/scope/native-%25FF"),
        ("/scope/native-�".as_bytes(), "/scope/native-%EF%BF%BD"),
    ] {
        let native = PathBuf::from(std::ffi::OsString::from_vec(native.to_vec()));
        assert_eq!(path_key(&native), expected);
    }
}

// Frozen map ordering oracle uses the original owned DTO fields, including
// sorting u32 IDs directly with no directory or ID tie-breaker.
fn legacy_map_order(nodes: &[Node], children: &[u32]) -> Vec<u32> {
    let mut ids = children.to_vec();
    ids.sort_unstable_by(|&a, &b| {
        nodes[b as usize]
            .allocated_size
            .cmp(&nodes[a as usize].allocated_size)
            .then_with(|| nodes[a as usize].name.cmp(&nodes[b as usize].name))
    });
    ids
}
fn map_key_fixture(count: u32, all_identical: bool) -> (Snapshot, Vec<Node>, Vec<u32>) {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let root = PathBuf::from("/map");
    let mut children: Vec<u32> = (1..=count).collect();
    children.rotate_left((count / 3) as usize);
    children.reverse();
    children.extend_from_slice(&[7, 7, 19, 19]);
    let mut root_node = view_dto(0, root.clone(), "map");
    root_node.children = children.clone();
    let mut nodes = vec![root_node];
    for id in 1..=count {
        let native = match id % 8 {
            0 => b"native-\xff".to_vec(),
            1 => b"native-\xfe".to_vec(),
            2 | 3 => b"same-key".to_vec(),
            4 => b"Item10".to_vec(),
            5 => "Zulu 🧪".as_bytes().to_vec(),
            6 => b"small".to_vec(),
            _ => format!("leaf-{id}").into_bytes(),
        };
        let path = root.join(std::ffi::OsString::from_vec(native));
        let name = if all_identical {
            "same".into()
        } else if id % 8 == 7 {
            "historical sparse label".into()
        } else {
            path.file_name().unwrap().to_string_lossy().into_owned()
        };
        let mut node = view_dto(id, path, &name);
        if id % 5 == 0 {
            node.identity.kind = NodeKind::Directory;
            node.is_directory = true;
        }
        node.allocated_size = if all_identical {
            20
        } else {
            u64::from(id % 4) * 4096
        };
        // Raw historical path overrides retain independent display labels.
        if !all_identical && id % 17 == 0 {
            let mut bytes = root.as_os_str().as_bytes().to_vec();
            bytes.extend_from_slice(b"//");
            bytes.extend_from_slice(node.path.file_name().unwrap().as_bytes());
            node.path = std::ffi::OsString::from_vec(bytes).into();
        }
        nodes.push(node);
    }
    let mut snapshot = Snapshot::empty(root);
    for node in &nodes {
        snapshot.append_legacy_node(node.clone()).unwrap();
    }
    snapshot.state = ScanState::Completed;
    (snapshot, nodes, children)
}
#[test]
fn map_display_keys_match_legacy_unstable_order_for_large_collisions() {
    for all_identical in [false, true] {
        let (snapshot, nodes, children) = map_key_fixture(2048, all_identical);
        let actual = sort_map_children(&snapshot, &children).unwrap();
        assert_eq!(
            actual.ids,
            legacy_map_order(&nodes, &children)
                .into_iter()
                .filter(|&id| nodes[id as usize].allocated_size > 0)
                .collect::<Vec<_>>()
        );
    }
}
#[test]
fn positive_map_filter_preserves_full_legacy_output_and_aggregate_sizes() {
    tauri::async_runtime::block_on(async {
        let (snapshot, nodes, _) = map_key_fixture(2048, false);
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let session = install(&state, snapshot);
        for limit in [2, 17, 5000] {
            session.sorts.lock().unwrap().clear();
            let actual = get_map(app.state(), "fixture".into(), 0, limit, None)
                .await
                .unwrap();
            {
                let mut cache = session.sorts.lock().unwrap();
                cache.clear();
                for node in &nodes {
                    let ids = legacy_map_order(&nodes, &node.children)
                        .into_iter()
                        .filter(|&id| nodes[id as usize].allocated_size > 0)
                        .collect();
                    cache.insert(node.id, ids);
                }
            }
            let expected = get_map(app.state(), "fixture".into(), 0, limit, None)
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
    });
}
#[test]
fn file_type_map_commands_preserve_folder_cache_and_reject_invalid_requests() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let current = install(&state, synthetic(100));
        let typed = get_map(
            app.state(),
            current.id.clone(),
            0,
            3,
            Some("fileType".into()),
        )
        .await
        .unwrap();
        assert!(current.sorts.lock().unwrap().is_empty());
        let group = typed
            .nodes
            .iter()
            .find(|n| n.file_type.as_deref() == Some(""))
            .unwrap();
        assert_eq!(group.file_count, Some(100));
        assert_eq!(group.allocated_bytes, 409600);
        assert!(get_node(app.state(), current.id.clone(), group.id)
            .await
            .is_err());
        let default = get_map(app.state(), current.id.clone(), 0, 20, None)
            .await
            .unwrap();
        let explicit = get_map(
            app.state(),
            current.id.clone(),
            0,
            20,
            Some("folder".into()),
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::to_value(default).unwrap(),
            serde_json::to_value(explicit).unwrap()
        );
        let original_cache = current.sorts.lock().unwrap().clone();
        get_map(
            app.state(),
            current.id.clone(),
            0,
            5000,
            Some("fileType".into()),
        )
        .await
        .unwrap();
        assert_eq!(*current.sorts.lock().unwrap(), original_cache);
        for (scope, mode) in [(999, "fileType"), (0, "invalid")] {
            assert!(get_map(
                app.state(),
                current.id.clone(),
                scope,
                5000,
                Some(mode.into())
            )
            .await
            .is_err());
        }
        assert!(get_map(
            app.state(),
            "stale".into(),
            0,
            5000,
            Some("fileType".into())
        )
        .await
        .is_err());
        {
            let mut snapshot = current.store.write().unwrap();
            snapshot.state = ScanState::Running;
            snapshot
                .update_node(1, |node| {
                    node.allocated_size += 4096;
                })
                .unwrap();
            snapshot
                .update_node(0, |node| {
                    node.allocated_size += 4096;
                })
                .unwrap();
        }
        let running = get_map(
            app.state(),
            current.id.clone(),
            0,
            3,
            Some("fileType".into()),
        )
        .await
        .unwrap();
        let group = running
            .nodes
            .iter()
            .find(|n| n.file_type.is_some())
            .unwrap();
        assert_eq!(group.allocated_bytes, 413696);
        assert_eq!(*current.sorts.lock().unwrap(), original_cache);
        current.invalidate();
        assert!(get_map(
            app.state(),
            current.id.clone(),
            0,
            5000,
            Some("fileType".into())
        )
        .await
        .err()
        .unwrap()
        .contains(slots::RESULTS_CHANGED));
    });
}
#[test]
fn sparse_positive_map_keys_only_allocate_for_visible_children() {
    let (_, mut nodes, children) = map_key_fixture(512, false);
    for node in nodes.iter_mut().skip(1) {
        node.allocated_size = if node.id % 7 == 0 {
            u64::from(node.id) * 4096
        } else {
            0
        };
    }
    // Repeated child IDs must be absent to exercise the collision-free path.
    let children: Vec<_> = children.into_iter().take(512).collect();
    let mut snapshot = Snapshot::empty(PathBuf::from("/map"));
    for node in &nodes {
        snapshot.append_legacy_node(node.clone()).unwrap();
    }
    let actual = sort_map_children(&snapshot, &children).unwrap();
    let expected: Vec<_> = legacy_map_order(&nodes, &children)
        .into_iter()
        .filter(|&id| nodes[id as usize].allocated_size > 0)
        .collect();
    assert_eq!(actual.ids, expected);
    assert_eq!(actual.ids.capacity(), expected.len());
    assert_eq!(
        actual.scratch.key_vector.length_bytes,
        expected.len() * std::mem::size_of::<MapSortKey<'_>>()
    );
    assert_eq!(
        actual.scratch.temporary_ids.capacity_bytes,
        expected.len() * std::mem::size_of::<u32>()
    );
}
#[test]
fn zero_sized_map_children_do_not_allocate_keys_or_cache_ids() {
    let (_, mut nodes, children) = map_key_fixture(32768, false);
    for node in nodes.iter_mut().skip(1) {
        node.allocated_size = 0;
    }
    let mut snapshot = Snapshot::empty(PathBuf::from("/map"));
    for node in &nodes {
        snapshot.append_legacy_node(node.clone()).unwrap();
    }
    snapshot.state = ScanState::Completed;
    let order = sort_map_children(&snapshot, &children).unwrap();
    assert!(order.ids.is_empty());
    assert_eq!(order.ids.capacity(), 0);
    assert_eq!(order.scratch.key_vector.capacity_bytes, 0);
    assert_eq!(order.scratch.owned_lossy_names.capacity_bytes, 0);
    assert_eq!(order.scratch.temporary_ids.capacity_bytes, 0);
}
#[test]
fn map_display_key_scratch_counts_only_vector_and_owned_lossy_buffers() {
    let (snapshot, _nodes, children) = map_key_fixture(512, false);
    let actual = sort_map_children(&snapshot, &children).unwrap();
    assert_eq!(
        actual.scratch.key_vector.length_bytes,
        children.len() * std::mem::size_of::<MapSortKey<'_>>()
    );
    assert_eq!(
        actual.scratch.key_vector.capacity_bytes,
        actual.scratch.key_vector.length_bytes
    );
    let lossy_bytes: usize = children
        .iter()
        .filter_map(|&id| match snapshot.display_name(id).unwrap() {
            std::borrow::Cow::Owned(name) => Some(name.len()),
            std::borrow::Cow::Borrowed(_) => None,
        })
        .sum();
    assert!(lossy_bytes > 0);
    assert_eq!(actual.scratch.owned_lossy_names.length_bytes, lossy_bytes);
    assert!(actual.scratch.owned_lossy_names.capacity_bytes >= lossy_bytes);
    let keys = vec![MapSortKey {
        size: 1,
        display_name: std::borrow::Cow::Borrowed("large borrowed snapshot label"),
    }];
    assert_eq!(
        MapSortScratch::from_keys(&keys)
            .owned_lossy_names
            .capacity_bytes,
        0
    );
    assert!(sort_map_children(&snapshot, &[u32::MAX]).is_err());
    #[cfg(feature = "performance-benchmark")]
    {
        let state = AppState::default();
        let session = install(&state, snapshot.clone());
        assert!(session
            .scan_details
            .lock()
            .unwrap()
            .map_sort_scratch
            .is_none());
        let ids = sorted_children(&session, &snapshot, 0).unwrap();
        let sample = session
            .scan_details
            .lock()
            .unwrap()
            .map_sort_scratch
            .unwrap();
        assert_eq!(
            sample.key_vector.capacity_bytes,
            actual.scratch.key_vector.capacity_bytes
        );
        assert_eq!(sorted_children(&session, &snapshot, 0).unwrap(), ids);
        let cached = session.sorts.lock().unwrap();
        assert_eq!(cached[&0], ids); // Persistent cache remains ID buffers only.
        drop(cached);
        assert!(sorted_children(&session, &snapshot, 5).unwrap().is_empty());
        assert_eq!(
            serde_json::to_value(
                session
                    .scan_details
                    .lock()
                    .unwrap()
                    .map_sort_scratch
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(sample).unwrap()
        ); // Smaller later sorts cannot rewrite the largest actual sample.
    }
}

#[test]
fn errors_are_indexed_by_native_path_components() {
    let mut s = synthetic(2);
    s.errors.push(disk_scan::ScanIssue {
        error_kind: None,
        raw_os_error: None,
        path: s.path(1).unwrap().join("child"),
        message: "permission denied".into(),
    });
    s.stats.errors = 1;
    let state = AppState::default();
    let session = install(&state, s);
    let snapshot = session.store.read().unwrap();
    let issues = issue_index(&session, &snapshot);
    assert!(
        view(&snapshot.node(1).unwrap(), &issues.1)
            .unwrap()
            .incomplete
    );
    assert!(
        !view(&snapshot.node(2).unwrap(), &issues.1)
            .unwrap()
            .incomplete
    );
}
#[test]
fn registration_rejects_changed_folder() {
    let d = fixture();
    let parent = std::fs::canonicalize(d.path()).unwrap();
    let path = parent.join("selected");
    std::fs::create_dir(&path).unwrap();
    let state = AppState::default();
    let target = register(
        &state,
        platform::target(&path, "selected".into()),
        path.clone(),
    )
    .unwrap();
    std::fs::rename(&path, parent.join("old")).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(validate_target(&state.target_guards.lock().unwrap()[&target.id]).is_err());
}
#[test]
fn discovery_skips_a_location_that_fails_to_register() {
    let d = fixture();
    let good = std::fs::canonicalize(d.path()).unwrap();
    let gone = good.join("unplugged");
    let state = AppState::default();
    let found = [&gone, &good]
        .map(|p| (platform::target(p, "disk".into()), p.clone()))
        .to_vec();
    let listed = register_all(&state, found);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].path, good.to_string_lossy());
}
#[test]
fn registering_a_location_again_keeps_one_entry() {
    let d = fixture();
    let parent = std::fs::canonicalize(d.path()).unwrap();
    let path = parent.join("selected");
    std::fs::create_dir(&path).unwrap();
    let state = AppState::default();
    let pick = || {
        register(
            &state,
            platform::target(&path, "selected".into()),
            path.clone(),
        )
        .unwrap()
    };
    let first = pick();
    assert_eq!(pick().id, first.id, "an unchanged folder keeps its ID");
    std::fs::rename(&path, parent.join("old")).unwrap();
    std::fs::create_dir(&path).unwrap();
    let replaced = pick();
    assert_ne!(replaced.id, first.id, "a replaced folder gets a new ID");
    assert_eq!(state.targets.lock().unwrap().len(), 1);
    assert_eq!(state.target_guards.lock().unwrap().len(), 1);
}
#[test]
fn scan_lifecycle_rejects_unregistered_targets_and_invalidates_old_results() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let d = fixture();
        let root = std::fs::canonicalize(d.path()).unwrap();
        std::fs::write(root.join("file"), b"abc").unwrap();
        assert!(start_scan(
            app.state(),
            root.display().to_string(),
            Channel::new(|_| Ok(())),
            app.handle().clone()
        )
        .await
        .is_err());
        let target = register(
            &app.state(),
            platform::target(&root, "fixture".into()),
            root.clone(),
        )
        .unwrap();
        let old = install(&app.state(), snapshot(root));
        old.store.write().unwrap().state = ScanState::Running;
        assert!(start_scan(
            app.state(),
            target.id.clone(),
            Channel::new(|_| Ok(())),
            app.handle().clone()
        )
        .await
        .is_err());
        assert!(old.is_valid());
        old.store.write().unwrap().state = ScanState::Completed;
        let (_, events) = start_recorded(&app.state(), &target, Recorder::default()).unwrap();
        assert!(!old.is_valid());
        assert_eq!(events.wait_end().entries, 2);
    });
}
#[test]
fn summaries_expose_worker_diagnostics_with_exact_accounting() {
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let fixture = fixture();
    let path = std::fs::canonicalize(fixture.path()).unwrap();
    std::fs::write(path.join("file"), b"abc").unwrap();
    let session = install(&state, snapshot(path));
    let summary = session.summary();
    assert_eq!(
        summary.worker_stats.iter().map(|w| w.entries).sum::<u64>(),
        summary.entries - 1
    );
    assert!(!summary.worker_stats.is_empty());
    let json = serde_json::to_value(summary).unwrap();
    assert!(json.get("workerStats").is_some());
    assert!(json.get("finalizationUs").is_some());
    assert!(json["workerStats"][0].get("queueWaitUs").is_some());
}
#[test]
fn finished_walk_reports_verifying_until_the_scan_completes() {
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let fixture = fixture();
    let path = std::fs::canonicalize(fixture.path()).unwrap();
    let session = install(&state, snapshot(path));
    let set = |state: ScanState, walk_ms: u64, phase: &str| {
        let mut s = session.store.write().unwrap();
        s.state = state;
        s.stats.walk_ms = walk_ms;
        drop(s);
        session.scan_details.lock().unwrap().phase = phase.into();
    };
    set(ScanState::Running, 0, "scanning");
    assert_eq!(session.summary().scan_phase, "scanning");
    set(ScanState::Running, 1_200, "scanning");
    let summary = session.summary();
    assert_eq!(
        (summary.scan_phase.as_str(), summary.walk_ms),
        ("verifying", 1_200)
    );
    assert_eq!(serde_json::to_value(&summary).unwrap()["walkMs"], 1_200);
    // Later app phases and terminal states keep their own labels.
    set(ScanState::Running, 1_200, "caching");
    assert_eq!(session.summary().scan_phase, "caching");
    set(ScanState::Completed, 1_200, "done");
    assert_eq!(session.summary().scan_phase, "done");
    set(ScanState::Cancelled, 1_200, "scanning");
    assert_eq!(session.summary().scan_phase, "scanning");
}
#[test]
fn cleanup_reviews_collapse_selection_and_reject_stale_or_expired_tickets() {
    tauri::async_runtime::block_on(async {
        let d = fixture();
        let root = std::fs::canonicalize(d.path()).unwrap();
        let dir = root.join("folder");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("file"), b"safe").unwrap();
        let app = app(AppState::default());
        let s = install(&app.state(), snapshot(root));
        let (dir_id, file_id) = {
            let snapshot = s.store.read().unwrap();
            let ids = (
                snapshot
                    .iter_nodes()
                    .find(|n| n.path().unwrap() == dir)
                    .unwrap()
                    .id,
                snapshot
                    .iter_nodes()
                    .find(|n| n.path().unwrap() == dir.join("file"))
                    .unwrap()
                    .id,
            );
            ids
        };
        assert!(prepare_trash(app.state(), "fixture".into(), vec![0])
            .await
            .is_err());
        let review = prepare_trash(app.state(), "fixture".into(), vec![dir_id, file_id, dir_id])
            .await
            .unwrap();
        assert_eq!(review.items.len(), 1);
        assert_eq!(review.items[0].id, dir_id);
        s.ticket.lock().unwrap().as_mut().unwrap().created =
            Instant::now() - Duration::from_secs(121);
        assert!(commit_trash(
            app.state(),
            "fixture".into(),
            review.token,
            app.handle().clone()
        )
        .await
        .is_err());
        assert!(dir.join("file").exists());
        let review = prepare_trash(app.state(), "fixture".into(), vec![file_id])
            .await
            .unwrap();
        assert!(commit_trash(
            app.state(),
            "fixture".into(),
            "wrong-token".into(),
            app.handle().clone()
        )
        .await
        .is_err());
        // The wrong token did not spend the ticket the right token still owns.
        assert_eq!(
            s.ticket.lock().unwrap().as_ref().map(|t| &t.token),
            Some(&review.token)
        );
        assert!(dir.join("file").exists());
        s.store.write().unwrap().state = ScanState::Cancelled;
        assert!(prepare_trash(app.state(), "fixture".into(), vec![file_id])
            .await
            .is_err());
    });
}

#[test]
fn reselecting_replaced_folder_uses_exact_new_target_guard() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let fixture = fixture();
        let parent = std::fs::canonicalize(fixture.path()).unwrap();
        let root = parent.join("selected");
        std::fs::create_dir(&root).unwrap();
        let old = register(
            &app.state(),
            platform::target(&root, "old".into()),
            root.clone(),
        )
        .unwrap();
        std::fs::rename(&root, parent.join("previous")).unwrap();
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("new"), b"new").unwrap();
        let new = register(
            &app.state(),
            platform::target(&root, "new".into()),
            root.clone(),
        )
        .unwrap();
        assert!(start_scan(
            app.state(),
            old.id,
            Channel::new(|_| Ok(())),
            app.handle().clone()
        )
        .await
        .is_err());
        assert!(start_scan(
            app.state(),
            root.display().to_string(),
            Channel::new(|_| Ok(())),
            app.handle().clone()
        )
        .await
        .is_err());
        let (id, events) = start_recorded(&app.state(), &new, Recorder::default()).unwrap();
        events.wait_end();
        let page = get_children(app.state(), id, 0, 0, 150, None)
            .await
            .unwrap();
        assert_eq!(page.items[0].name, "new");
    });
}

#[test]
#[ignore = "Measures million-node query latency and memory; run explicitly"]
fn million_node_query_latency() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let started = Instant::now();
        install(&app.state(), synthetic(1_000_000));
        let construct_ms = started.elapsed().as_millis();
        let started = Instant::now();
        let page = get_children(app.state(), "fixture".into(), 0, 0, 150, None)
            .await
            .unwrap();
        let first_page_ms = started.elapsed().as_millis();
        assert_eq!(page.total, 1_000_000);
        let started = Instant::now();
        get_children(app.state(), "fixture".into(), 0, 999850, 150, None)
            .await
            .unwrap();
        let cached_page_ms = started.elapsed().as_millis();
        let started = Instant::now();
        let map = get_map(app.state(), "fixture".into(), 0, 5000, None)
            .await
            .unwrap();
        let map_ms = started.elapsed().as_millis();
        assert_eq!(map.nodes.len(), 5000);
        let started = Instant::now();
        let search = search_nodes(app.state(), "fixture".into(), "999999".into(), 0, 150, None)
            .await
            .unwrap();
        let search_ms = started.elapsed().as_millis();
        assert_eq!(search.total, 1);
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        println!(
            "{}",
            serde_json::json!({"entries":1_000_001,"construct_ms":construct_ms,"first_page_ms":first_page_ms,"cached_page_ms":cached_page_ms,"map_ms":map_ms,"search_ms":search_ms,"peak_rss_native_units":usage.ru_maxrss})
        );
    });
}

fn automatic_info(state: &AppState, target_id: &str) -> settings::SettingsInfo {
    let guard = state
        .target_guards
        .lock()
        .unwrap()
        .get(target_id)
        .cloned()
        .unwrap();
    let root = disk_scan::ScanRoot::open(
        guard.path.clone(),
        disk_scan::Identity {
            device: guard.identity.device,
            inode: guard.identity.inode,
            kind: NodeKind::Directory,
        },
    )
    .unwrap();
    let scope = if platform::is_volume_root(&root) {
        settings::AutoScope::Volume
    } else {
        settings::AutoScope::Folder
    };
    state
        .scan_settings
        .lock()
        .unwrap()
        .automatic_info_for_scope(scope)
}
#[test]
fn hardware_auto_ignores_legacy_preferences_and_is_captured_by_new_sessions() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let fixture = fixture();
        let root = std::fs::canonicalize(fixture.path()).unwrap();
        std::fs::write(root.join("file"), b"x").unwrap();
        let target = register(&state, platform::target(&root, "fixture".into()), root).unwrap();
        let info = automatic_info(&state, &target.id);
        let (id, events) = start_recorded(&state, &target, Recorder::default()).unwrap();
        let s = state.scans.find(&id).unwrap();
        assert_eq!(s.workers, info.resolved_workers);
        let captured_validation_workers = s.cached_validation_workers;
        assert_eq!(s.worker_mode, settings::WorkerMode::Auto);
        #[cfg(target_os = "macos")]
        {
            assert_eq!(s.worker_source, "hardware");
            assert_eq!(info.auto_scope, settings::AutoScope::Folder);
        }
        assert_eq!(s.workers, info.resolved_workers);
        assert_eq!(s.cached_validation_workers, captured_validation_workers);
        events.wait_end();
    });
}
#[test]
#[ignore = "Read-only release hardware Auto probe; disposable preferences only"]
fn automatic_workers_probe() {
    tauri::async_runtime::block_on(async {
        let path = std::fs::canonicalize(
            std::env::var_os("HAHAKIGAMI_WORKERS_PATH").expect("set HAHAKIGAMI_WORKERS_PATH"),
        )
        .unwrap();
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let target = register(
            &state,
            platform::target(&path, "probe".into()),
            path.clone(),
        )
        .unwrap();
        let before = automatic_info(&state, &target.id);
        let id = start_scan(
            app.state(),
            target.id.clone(),
            Channel::new(|_| Ok(())),
            app.handle().clone(),
        )
        .await
        .unwrap();
        let held = state.scans.find(&id).unwrap();
        assert_eq!(held.worker_source, "hardware");
        assert_eq!(held.worker_mode, settings::WorkerMode::Auto);
        let captured = held.workers;
        let until = Instant::now() + Duration::from_secs(180);
        loop {
            let summary = scan_summary(app.state(), id.clone()).await.unwrap();
            assert_eq!(summary.workers, captured);
            if summary.state != "running" {
                assert_eq!(summary.state, "completed");
                break;
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(50));
        }
        let scanned = scan_summary(app.state(), id.clone()).await.unwrap();
        assert_eq!(held.workers, captured);
        let automatic = automatic_info(&state, &target.id);
        assert_eq!(automatic.worker_mode, settings::WorkerMode::Auto);
        assert_eq!(automatic.qos, disk_scan::ScanQos::Automatic);
        assert_eq!(automatic.resolved_workers, automatic.default_workers);
        let report = serde_json::json!({"settings":before,"scan":scanned,"automaticAfterLegacyReload":automatic,"capturedWorkers":captured});
        if let Some(output) = std::env::var_os("HAHAKIGAMI_WORKERS_REPORT") {
            std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
        println!("{}", report);
    });
}

#[test]
fn exclusion_summary_and_node_flags_do_not_claim_read_failures() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let mut snapshot = synthetic(2);
        let path = snapshot.path(1).unwrap();
        snapshot.exclusions.push(disk_scan::ScanIssue {
            path,
            message: "Outside scan scope".into(),
            error_kind: None,
            raw_os_error: None,
        });
        snapshot.stats.excluded = 1;
        snapshot.update_node(1, |n| n.excluded = true).unwrap();
        let s = install(&state, snapshot);
        let summary = s.summary();
        assert_eq!(summary.errors, 0);
        assert_eq!(summary.excluded, 1);
        assert_eq!(summary.errors_stored, 0);
        assert_eq!(summary.exclusions_stored, 1);
        assert!(!summary.incomplete);
        assert!(summary.message.is_none());
        let excluded = get_node(app.state(), "fixture".into(), 1).await.unwrap();
        assert!(excluded.excluded);
        assert!(!excluded.incomplete);
        assert!(
            !get_node(app.state(), "fixture".into(), 0)
                .await
                .unwrap()
                .excluded
        );
        let page = get_children(app.state(), "fixture".into(), 0, 0, 10, None)
            .await
            .unwrap();
        assert!(page.items.iter().find(|n| n.id == 1).unwrap().excluded);
        assert!(!page.items.iter().find(|n| n.id == 2).unwrap().excluded);
    });
}

#[test]
fn issue_pages_are_bounded_separate_and_preserve_native_reasons() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let mut snapshot = synthetic(1);
        let unusual = PathBuf::from("/fixture/quote\" newline\n 日本語");
        for _ in 0..103 {
            snapshot.errors.push(disk_scan::ScanIssue {
                path: unusual.clone(),
                message: "permission denied".into(),
                error_kind: Some("PermissionDenied".into()),
                raw_os_error: Some(libc::EACCES),
            });
        }
        snapshot.stats.errors = 20000;
        snapshot.stats.incomplete = true;
        snapshot.exclusions.push(disk_scan::ScanIssue {
            path: "/fixture/mount".into(),
            message: "Outside scan scope".into(),
            error_kind: None,
            raw_os_error: None,
        });
        snapshot.stats.excluded = 1;
        let s = install(&state, snapshot);
        assert!(s.summary().incomplete);
        assert_eq!(s.summary().errors_stored, 103);
        let first = get_scan_issues(app.state(), "fixture".into(), IssueKind::Errors, 0, 100)
            .await
            .unwrap();
        assert_eq!(first.scan_id, "fixture");
        assert_eq!(first.node_id, 0);
        assert_eq!(first.total, 20000);
        assert_eq!(first.retained_total, 103);
        assert_eq!(first.items.len(), 100);
        assert_eq!(first.items[0].path, unusual.to_string_lossy());
        assert_eq!(
            first.items[0].error_kind.as_deref(),
            Some("PermissionDenied")
        );
        assert_eq!(first.items[0].raw_os_error, Some(libc::EACCES));
        let json = serde_json::to_value(&first).unwrap();
        assert_eq!(json["scanId"], "fixture");
        assert_eq!(json["kind"], "errors");
        assert_eq!(json["items"][0]["path"], unusual.to_string_lossy().as_ref());
        let last = get_scan_issues(app.state(), "fixture".into(), IssueKind::Errors, 100, 100)
            .await
            .unwrap();
        assert_eq!(last.offset, 100);
        assert_eq!(last.items.len(), 3);
        let excluded = get_scan_issues(app.state(), "fixture".into(), IssueKind::Excluded, 0, 100)
            .await
            .unwrap();
        assert_eq!(excluded.kind, IssueKind::Excluded);
        assert_eq!(excluded.total, 1);
        assert_eq!(excluded.items[0].path, "/fixture/mount");
        assert!(excluded.items[0].raw_os_error.is_none());
        let empty = get_scan_issues(
            app.state(),
            "fixture".into(),
            IssueKind::Errors,
            usize::MAX,
            100,
        )
        .await
        .unwrap();
        assert_eq!(empty.offset, usize::MAX);
        assert!(empty.items.is_empty());
        for limit in [0, 101, usize::MAX] {
            assert!(
                get_scan_issues(app.state(), "fixture".into(), IssueKind::Errors, 0, limit)
                    .await
                    .is_err()
            );
        }
    });
}

#[test]
fn issue_pages_reject_stale_or_invalidated_sessions() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let s = install(&state, synthetic(1));
        assert!(
            get_scan_issues(app.state(), "old-scan".into(), IssueKind::Errors, 0, 100)
                .await
                .is_err()
        );
        s.invalidate();
        assert!(
            get_scan_issues(app.state(), "fixture".into(), IssueKind::Excluded, 0, 100)
                .await
                .is_err()
        );
        assert!(serde_json::from_str::<IssueKind>("\"filesystemPath\"").is_err());
    });
}

#[test]
fn exclusions_beyond_retained_details_keep_node_flags_and_cleanup_protection() {
    tauri::async_runtime::block_on(async {
        let d = fixture();
        let root = std::fs::canonicalize(d.path()).unwrap();
        let dir = root.join("excluded");
        std::fs::create_dir(&dir).unwrap();
        let app = app(AppState::default());
        let mut snapshot = snapshot(root);
        let id = snapshot
            .iter_nodes()
            .find(|n| n.path().unwrap() == dir)
            .unwrap()
            .id;
        // The node's exclusion occurred after the 10,000 retained details.
        snapshot.exclusions = (0..10000)
            .map(|i| disk_scan::ScanIssue {
                path: PathBuf::from(format!("/fixture/earlier-{i}")),
                message: "Outside scan scope".into(),
                error_kind: None,
                raw_os_error: None,
            })
            .collect();
        snapshot.stats.excluded = 10001;
        snapshot.update_node(id, |n| n.excluded = true).unwrap();
        let s = install(&app.state(), snapshot);
        let node = get_node(app.state(), "fixture".into(), id).await.unwrap();
        assert!(node.excluded);
        assert!(!node.incomplete);
        assert_eq!(s.summary().exclusions_stored, 10000);
        let error = prepare_trash(app.state(), "fixture".into(), vec![id])
            .await
            .err()
            .unwrap();
        assert!(error.contains("Excluded locations"));
        assert!(s.ticket.lock().unwrap().is_none());
        assert!(dir.is_dir());
    });
}

#[cfg(target_os = "macos")]
#[test]
fn restricted_and_other_issue_pages_keep_full_totals_and_all_errors_copy_contract() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let mut snapshot = synthetic(2);
        for i in 0..101 {
            snapshot.errors.push(disk_scan::ScanIssue {
                path: PathBuf::from(format!("/private/var/db/service-{i}")),
                message: "Permission denied".into(),
                error_kind: Some("PermissionDenied".into()),
                raw_os_error: Some(libc::EACCES),
            });
        }
        snapshot.errors.push(disk_scan::ScanIssue {
            path: "/Users/fixture/project".into(),
            message: "Permission denied in project".into(),
            error_kind: Some("PermissionDenied".into()),
            raw_os_error: Some(libc::EACCES),
        });
        snapshot.errors.push(disk_scan::ScanIssue {
            path: "/System/Library/bad-metadata".into(),
            message: "Input/output error".into(),
            error_kind: Some("Other".into()),
            raw_os_error: Some(libc::EIO),
        });
        // Full counters can exceed retained detail counts without concealing
        // either restrictions or unexpected failures in the summary.
        snapshot.stats.errors = 20005;
        snapshot.stats.restricted_system_issues = 20000;
        snapshot.stats.incomplete = true;
        let s = install(&state, snapshot);
        let summary = s.summary();
        assert_eq!(summary.restricted_system_issues, 20000);
        assert_eq!(summary.errors - summary.restricted_system_issues, 5);
        assert!(summary.incomplete);
        assert!(summary.message.unwrap().contains("project"));
        let first = get_scan_issues(app.state(), "fixture".into(), IssueKind::Restricted, 0, 100)
            .await
            .unwrap();
        assert_eq!(first.total, 20000);
        assert_eq!(first.retained_total, 101);
        assert_eq!(first.items.len(), 100);
        assert!(first
            .items
            .iter()
            .all(|issue| issue.category == "restrictedSystem"));
        let last = get_scan_issues(
            app.state(),
            "fixture".into(),
            IssueKind::Restricted,
            100,
            100,
        )
        .await
        .unwrap();
        assert_eq!(last.items.len(), 1);
        assert!(last.items[0].path.ends_with("service-100"));
        let other = get_scan_issues(app.state(), "fixture".into(), IssueKind::Other, 0, 100)
            .await
            .unwrap();
        assert_eq!(other.total, 5);
        assert_eq!(other.retained_total, 2);
        assert!(other.items.iter().all(|issue| issue.category == "other"));
        assert!(other
            .items
            .iter()
            .any(|issue| issue.raw_os_error == Some(libc::EIO)));
        let all = get_scan_issues(app.state(), "fixture".into(), IssueKind::Errors, 100, 100)
            .await
            .unwrap();
        assert_eq!(all.total, 20005);
        assert_eq!(all.retained_total, 103);
        assert_eq!(all.items.len(), 3);
        assert_eq!(all.items[0].category, "restrictedSystem");
        assert_eq!(all.items[1].category, "other");
    });
}

#[cfg(target_os = "macos")]
#[test]
fn restrictions_only_keep_incomplete_results_without_prominent_native_message() {
    let state = AppState::default();
    let mut snapshot = synthetic(1);
    snapshot.stats.errors = 1;
    snapshot.stats.restricted_system_issues = 1;
    snapshot.stats.incomplete = true;
    snapshot.errors.push(disk_scan::ScanIssue {
        path: "/Library/Caches/com.apple.service".into(),
        message: "Operation not permitted".into(),
        error_kind: Some("PermissionDenied".into()),
        raw_os_error: Some(libc::EPERM),
    });
    let s = install(&state, snapshot);
    let summary = s.summary();
    assert!(summary.incomplete);
    assert!(summary.message.is_none());
    assert_eq!(summary.errors, 1);
    let json = serde_json::to_value(summary).unwrap();
    assert_eq!(json["restrictedSystemIssues"], 1);
    // Existing node incompleteness remains based on recorded native paths.
    let snapshot = s.store.read().unwrap();
    assert_eq!(snapshot.errors.len(), 1);
}

#[test]
#[ignore = "native journal/cache parity and timing on disposable local fixture"]
fn native_rescan_cache_parity() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("source");
        for i in 0..24 {
            let folder = root.join(format!("d{i}"));
            std::fs::create_dir_all(&folder).unwrap();
            for j in 0..32 {
                std::fs::write(folder.join(format!("f{j}")), b"data").unwrap();
            }
        }
        // A write outside the scan root can change a cached entry through a hard link.
        let external_alias = dir.path().join("external-alias");
        std::fs::hard_link(root.join("d6/f0"), root.join("d7/internal-alias")).unwrap();
        let existing_external_alias = dir.path().join("existing-external-alias");
        std::fs::hard_link(root.join("d8/f0"), &existing_external_alias).unwrap();
        // Let the creation batch reach the journal before capturing the seed.
        // Otherwise a delayed root creation event legitimately dirties every subtree.
        std::thread::sleep(Duration::from_secs(2));
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        state.baselines.configure(dir.path().join("cache"));
        let root = std::fs::canonicalize(root).unwrap();
        let target = register(
            &state,
            platform::target(&root, "fixture".into()),
            root.clone(),
        )
        .unwrap();
        let until_done = |id: &str| {
            let until = Instant::now() + Duration::from_secs(15);
            loop {
                let s = state.scans.find(id).unwrap();
                if !session_running(&s) {
                    return s;
                }
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let first = start_scan(
            app.state(),
            target.id.clone(),
            Channel::new(|_| Ok(())),
            app.handle().clone(),
        )
        .await
        .unwrap();
        let first = until_done(&first);
        assert_eq!(first.summary().state, "completed");
        assert!(first.cache_token.lock().unwrap().is_some());
        let first_summary = first.summary();
        assert!(first
            .store
            .read()
            .unwrap()
            .iter_nodes()
            .any(|n| n.hard_link));
        std::fs::write(root.join("d7/internal-alias"), vec![3; 32768]).unwrap();
        std::fs::write(&existing_external_alias, vec![4; 65536]).unwrap();
        std::fs::write(root.join("d0/f0"), b"larger content").unwrap();
        std::fs::remove_file(root.join("d1/f1")).unwrap();
        std::fs::rename(root.join("d2/f2"), root.join("d2/renamed")).unwrap();
        std::fs::write(root.join("d3/new"), b"new").unwrap();
        std::fs::hard_link(root.join("d4/f0"), &external_alias).unwrap();
        std::fs::write(
            &external_alias,
            b"changed through an alias outside the scan root",
        )
        .unwrap();
        std::fs::remove_file(&external_alias).unwrap();
        // APFS clone notifications outside the root must not invalidate unrelated
        // cached directories. The fresh scan below also checks allocated-size parity.
        use std::os::unix::ffi::OsStrExt;
        let clone_source =
            std::ffi::CString::new(root.join("d5/f0").as_os_str().as_bytes()).unwrap();
        let clone_destination =
            std::ffi::CString::new(dir.path().join("external-clone").as_os_str().as_bytes())
                .unwrap();
        assert_eq!(
            unsafe { libc::clonefile(clone_source.as_ptr(), clone_destination.as_ptr(), 0) },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
        std::fs::write(dir.path().join("external-clone"), b"overwritten clone").unwrap();
        std::fs::remove_file(dir.path().join("external-clone")).unwrap();
        let second = start_scan(
            app.state(),
            target.id.clone(),
            Channel::new(|_| Ok(())),
            app.handle().clone(),
        )
        .await
        .unwrap();
        let second = until_done(&second);
        assert_eq!(second.summary().state, "completed");
        #[cfg(feature = "performance-benchmark")]
        assert!(!second
            .scan_details
            .lock()
            .unwrap()
            .phase_timings_us
            .contains_key("speculative_reconcile"));
        assert_eq!(
            second.summary().scan_mode,
            "incremental",
            "{:?}",
            second.summary().fallback_reason
        );
        // FullHistory may still include the entire fixture creation batch.
        // Positive reuse is required separately on an existing read-only folder.
        let full = snapshot(root.clone());
        let signature = |s: &Snapshot| -> std::collections::BTreeMap<_, _> {
            s.iter_nodes()
                .map(|n| {
                    (
                        n.path().unwrap(),
                        (
                            n.identity.clone(),
                            n.length,
                            n.own_allocated_size,
                            n.allocated_size,
                            n.excluded,
                            n.hard_link,
                        ),
                    )
                })
                .collect()
        };
        {
            let candidate = second.store.read().unwrap();
            assert_eq!(signature(&candidate), signature(&full));
            assert_eq!(candidate.stats.entries, full.stats.entries);
            assert_eq!(candidate.stats.errors, 0);
        }
        eprintln!(
            "native cache fixture: first_ms={},second_ms={},mode={},reused={},rescanned={},fallback={:?}",
            first_summary.elapsed_ms,
            second.summary().elapsed_ms,
            second.summary().scan_mode,
            second.summary().reused_entries,
            second.summary().rescanned_directories,
            second.summary().fallback_reason
        );
        // A fresh backend process model reloads the persisted cache rather than transferring memory.
        let app2 = self::app(AppState::default());
        let state2 = app2.state::<AppState>();
        state2.baselines.configure(dir.path().join("cache"));
        let target2 = register(
            &state2,
            platform::target(&root, "fixture".into()),
            root.clone(),
        )
        .unwrap();
        let id = start_scan(
            app2.state(),
            target2.id,
            Channel::new(|_| Ok(())),
            app2.handle().clone(),
        )
        .await
        .unwrap();
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            let s = state2.scans.find(&id).unwrap();
            if !session_running(&s) {
                assert_eq!(s.summary().state, "completed");
                #[cfg(feature = "performance-benchmark")]
                assert!(!s
                    .scan_details
                    .lock()
                    .unwrap()
                    .phase_timings_us
                    .contains_key("speculative_reconcile"));
                assert_eq!(
                    s.summary().scan_mode,
                    "incremental",
                    "{:?}",
                    s.summary().fallback_reason
                );

                assert_eq!(
                    signature(&s.store.read().unwrap()),
                    signature(&snapshot(root.clone()))
                );
                eprintln!(
                    "persisted reload: elapsed_ms={},mode={},reused={},fallback={:?}",
                    s.summary().elapsed_ms,
                    s.summary().scan_mode,
                    s.summary().reused_entries,
                    s.summary().fallback_reason
                );
                break;
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(5));
        }
    });
}

#[test]
#[ignore = "native read-only existing-folder reuse; set HAH_CACHE_REUSE_ROOT"]
fn native_existing_folder_cache_reuse() {
    tauri::async_runtime::block_on(async {
        let requested = PathBuf::from(
            std::env::var_os("HAH_CACHE_REUSE_ROOT")
                .expect("set HAH_CACHE_REUSE_ROOT to an existing absolute folder"),
        );
        assert!(requested.is_absolute());
        let root = std::fs::canonicalize(requested).unwrap();
        assert!(root.is_dir());
        let dir = tempfile::tempdir().unwrap();
        let signature = |s: &Snapshot| -> std::collections::BTreeMap<_, _> {
            s.iter_nodes()
                .map(|n| {
                    (
                        n.path().unwrap(),
                        (
                            n.identity.clone(),
                            n.length,
                            n.own_allocated_size,
                            n.allocated_size,
                            n.excluded,
                        ),
                    )
                })
                .collect()
        };
        // All writes are confined to this temporary cache. The chosen root is read-only.
        let mut original_archive = None;
        for process in 0..2 {
            let app = app(AppState::default());
            let state = app.state::<AppState>();
            state.baselines.configure(dir.path().join("cache"));
            let target = register(
                &state,
                platform::target(&root, "existing fixture".into()),
                root.clone(),
            )
            .unwrap();
            let scans = if process == 0 { 2 } else { 1 };
            for scan in 0..scans {
                let id = start_scan(
                    app.state(),
                    target.id.clone(),
                    Channel::new(|_| Ok(())),
                    app.handle().clone(),
                )
                .await
                .unwrap();
                let until = Instant::now() + Duration::from_secs(60);
                let completed = loop {
                    let s = state.scans.find(&id).unwrap();
                    if !session_running(&s) {
                        break s;
                    }
                    assert!(Instant::now() < until, "existing-folder scan timed out");
                    std::thread::sleep(Duration::from_millis(5));
                };
                let summary = completed.summary();
                assert_eq!(summary.state, "completed");
                assert!(completed.cache_token.lock().unwrap().is_some());
                use std::os::unix::fs::MetadataExt;
                let archive = std::fs::read_dir(dir.path().join("cache"))
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .find(|p| p.extension().is_some_and(|e| e == "zst"))
                    .unwrap();
                let archive_signature = (
                    archive.clone(),
                    std::fs::metadata(&archive).unwrap().ino(),
                    std::fs::metadata(&archive).unwrap().len(),
                );
                if process == 0 && scan == 0 {
                    original_archive = Some(archive_signature.clone());
                }
                if std::env::var_os("HAH_EXPECT_ARCHIVE_RETAINED").is_some() {
                    assert_eq!(
                        original_archive.as_ref().unwrap(),
                        &archive_signature,
                        "unchanged rescan must retain the original archive"
                    );
                }
                if process > 0 || scan > 0 {
                    assert_eq!(
                        summary.scan_mode, "incremental",
                        "{:?}",
                        summary.fallback_reason
                    );
                    assert!(
                        summary.reused_entries > 0,
                        "existing-folder scan must reuse cached entries"
                    );
                    let candidate = completed.store.read().unwrap();
                    let fresh = snapshot(root.clone());
                    assert_eq!(signature(&candidate), signature(&fresh));
                    assert_eq!(candidate.stats.entries, fresh.stats.entries);
                    assert_eq!(candidate.stats.errors, fresh.stats.errors);
                    eprintln!(
                        "existing-folder cache: process={process}, reused={}, rescanned={}, elapsed_ms={}",
                        summary.reused_entries, summary.rescanned_directories, summary.elapsed_ms
                    );
                }
            }
        }
    });
}

#[test]
fn published_candidate_reindexes_issues_with_unchanged_error_count() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let mut published = synthetic(2);
        published.stats.errors = 1;
        published.stats.incomplete = true;
        published.errors.push(disk_scan::ScanIssue {
            path: published.path(1).unwrap(),
            message: "fixture failure".into(),
            error_kind: None,
            raw_os_error: None,
        });
        let mut placeholder = Snapshot::empty(published.root.clone());
        placeholder.stats.entries = published.stats.entries;
        placeholder.stats.errors = 1;
        let s = install(&state, placeholder);
        assert!(get_node(app.state(), s.id.clone(), 1).await.is_err());
        *s.store.write().unwrap() = published;
        assert!(
            get_node(app.state(), s.id.clone(), 1)
                .await
                .unwrap()
                .incomplete
        );
    });
}

#[test]
fn filtered_native_pages_validate_requests_and_reject_stale_sessions() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("photo.JPG"), vec![0; 2048]).unwrap();
        std::fs::write(source.join("notes.txt"), b"notes").unwrap();
        std::fs::write(source.join("zero"), b"").unwrap();
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let live = install(&state, snapshot(source));
        force_access(&state, access_setup::Status::Off);
        let page = filter_nodes(
            app.state(),
            live.id.clone(),
            "PHOTO".into(),
            "images".into(),
            None,
            None,
            0,
            150,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].name, "photo.JPG");
        for (category, min, max) in [("invalid", None, None), ("all", Some(2), Some(1))] {
            assert!(filter_nodes(
                app.state(),
                live.id.clone(),
                "".into(),
                category.into(),
                min,
                max,
                0,
                150,
                None,
                None,
                None,
            )
            .await
            .is_err());
        }
        let zero = filter_nodes(
            app.state(),
            live.id.clone(),
            "".into(),
            "all".into(),
            Some(0),
            Some(0),
            0,
            150,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(zero.items.iter().any(|n| n.name == "zero"));
        live.invalidate();
        assert!(filter_nodes(
            app.state(),
            live.id.clone(),
            "".into(),
            "all".into(),
            None,
            None,
            0,
            150,
            None,
            None,
            None,
        )
        .await
        .err()
        .unwrap()
        .contains(slots::RESULTS_CHANGED));
    });
}

#[test]
fn filtered_native_page_limits_and_running_refresh() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let s = install(&state, synthetic(600));
        let first = filter_nodes(
            app.state(),
            s.id.clone(),
            "".into(),
            "all".into(),
            None,
            None,
            0,
            usize::MAX,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!((first.items.len(), first.total), (500, 600));
        let next = filter_nodes(
            app.state(),
            s.id.clone(),
            "".into(),
            "all".into(),
            None,
            None,
            500,
            500,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(next.items.len(), 100);
        assert!(!first
            .items
            .iter()
            .any(|n| next.items.iter().any(|m| n.id == m.id)));
        let outside = filter_nodes(
            app.state(),
            s.id.clone(),
            "".into(),
            "all".into(),
            None,
            None,
            usize::MAX,
            500,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!((outside.items.len(), outside.total), (0, 600));
        // A previously completed query must not be reused when a running snapshot changes.
        {
            let mut snapshot = s.store.write().unwrap();
            snapshot.state = ScanState::Running;
            let mut node = snapshot.legacy_node(1).unwrap();
            node.name = "unique-refresh".into();
            snapshot.replace_node(1, node).unwrap();
        }
        let refreshed = filter_nodes(
            app.state(),
            s.id.clone(),
            "unique-refresh".into(),
            "all".into(),
            None,
            None,
            0,
            150,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(refreshed.total, 1);
        {
            let mut snapshot = s.store.write().unwrap();
            let mut node = snapshot.legacy_node(1).unwrap();
            node.name = "renamed-refresh".into();
            snapshot.replace_node(1, node).unwrap();
        }
        let refreshed = filter_nodes(
            app.state(),
            s.id.clone(),
            "unique-refresh".into(),
            "all".into(),
            None,
            None,
            0,
            150,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(refreshed.total, 0);
    });
}

#[test]
fn list_sort_orders_cover_browsing_search_filters_without_changing_maps() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        for (name, count) in [("item-folder2", 8192), ("item-folder10", 65536)] {
            let folder = dir.path().join(name);
            std::fs::create_dir(&folder).unwrap();
            std::fs::write(folder.join("payload"), vec![0; count]).unwrap();
        }
        std::fs::write(dir.path().join("item2"), vec![0; 131072]).unwrap();
        std::fs::write(dir.path().join("item10"), vec![0; 16384]).unwrap();
        let app = app(AppState::default());
        let s = install(&app.state(), snapshot(dir.path().to_path_buf()));
        let before_map = serde_json::to_value(
            get_map(app.state(), s.id.clone(), 0, 5000, None)
                .await
                .unwrap(),
        )
        .unwrap();
        for (order, names) in [
            (
                "sizeDescending",
                vec!["item-folder10", "item-folder2", "item2", "item10"],
            ),
            (
                "sizeAscending",
                vec!["item-folder2", "item-folder10", "item10", "item2"],
            ),
            (
                "nameAscending",
                vec!["item-folder2", "item-folder10", "item2", "item10"],
            ),
            (
                "nameDescending",
                vec!["item-folder10", "item-folder2", "item10", "item2"],
            ),
        ] {
            let children = get_children(app.state(), s.id.clone(), 0, 0, 150, Some(order.into()))
                .await
                .unwrap();
            assert_eq!(
                children
                    .items
                    .iter()
                    .map(|n| n.name.as_str())
                    .collect::<Vec<_>>(),
                names
            );
            let search = search_nodes(
                app.state(),
                s.id.clone(),
                "item".into(),
                0,
                150,
                Some(order.into()),
            )
            .await
            .unwrap();
            assert_eq!(
                search
                    .items
                    .iter()
                    .map(|n| n.name.as_str())
                    .collect::<Vec<_>>(),
                names
            );
            let filtered = filter_nodes(
                app.state(),
                s.id.clone(),
                "item".into(),
                "all".into(),
                None,
                None,
                0,
                150,
                Some(order.into()),
                None,
                None,
            )
            .await
            .unwrap();
            assert_eq!(
                filtered
                    .items
                    .iter()
                    .map(|n| n.name.as_str())
                    .collect::<Vec<_>>(),
                names
            );
            let next = get_children(app.state(), s.id.clone(), 0, 2, 2, Some(order.into()))
                .await
                .unwrap();
            assert_eq!(
                next.items
                    .iter()
                    .map(|n| n.name.as_str())
                    .collect::<Vec<_>>(),
                names[2..]
            );
        }
        let after_map = serde_json::to_value(
            get_map(app.state(), s.id.clone(), 0, 5000, None)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(before_map, after_map);
        assert!(
            get_children(app.state(), s.id.clone(), 0, 0, 150, Some("invalid".into()))
                .await
                .err()
                .unwrap()
                .contains("sort")
        );
        assert!(search_nodes(
            app.state(),
            s.id.clone(),
            "item".into(),
            0,
            150,
            Some("invalid".into())
        )
        .await
        .is_err());
        assert!(filter_nodes(
            app.state(),
            s.id.clone(),
            "".into(),
            "all".into(),
            None,
            None,
            0,
            150,
            Some("invalid".into()),
            None,
            None,
        )
        .await
        .is_err());
    });
}

#[test]
fn explorer_sort_commands_restore_and_preserve_preferences() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        *state.explorer_preferences.lock().unwrap() =
            explorer_preferences::Preferences::load(dir.path());
        let source = dir.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("file2"), b"two").unwrap();
        std::fs::write(source.join("file10"), b"ten").unwrap();
        let live = install(&state, snapshot(source));
        let chosen = set_explorer_sort(app.state(), "nameDescending".into())
            .await
            .unwrap();
        assert_eq!(chosen.sort, sorting::Order::NameDescending);
        assert_eq!(
            get_explorer_preferences(app.state()).await.unwrap().sort,
            chosen.sort
        );
        assert_eq!(
            explorer_preferences::Preferences::load(dir.path())
                .view()
                .sort,
            chosen.sort
        );
        let page = get_children(
            app.state(),
            live.id.clone(),
            0,
            0,
            150,
            Some("nameDescending".into()),
        )
        .await
        .unwrap();
        assert_eq!(page.total, 2);
        assert!(set_explorer_sort(app.state(), "bad".into()).await.is_err());
        assert_eq!(
            get_explorer_preferences(app.state()).await.unwrap().sort,
            chosen.sort
        );
        live.invalidate();
        assert!(search_nodes(
            app.state(),
            live.id.clone(),
            "file".into(),
            0,
            150,
            Some("nameAscending".into())
        )
        .await
        .err()
        .unwrap()
        .contains(slots::RESULTS_CHANGED));
    });
}

#[test]
fn newer_sort_request_cannot_be_overwritten_by_an_older_blocking_job() {
    let dir = tempfile::tempdir().unwrap();
    let preferences = Mutex::new(explorer_preferences::Preferences::load(dir.path()));
    let requests = AtomicU64::new(2); // Accepted A=1, then B=2; run B first.
    save_sort_request(&preferences, &requests, 2, sorting::Order::NameAscending).unwrap();
    assert!(
        save_sort_request(&preferences, &requests, 1, sorting::Order::SizeAscending)
            .err()
            .unwrap()
            .contains("superseded")
    );
    assert_eq!(
        preferences.lock().unwrap().view().sort,
        sorting::Order::NameAscending
    );
    assert_eq!(
        explorer_preferences::Preferences::load(dir.path())
            .view()
            .sort,
        sorting::Order::NameAscending
    );
}

fn registered_preview_fixture(state: &AppState, snapshot: Snapshot) -> Arc<Session> {
    // The ancestors of the root must resolve, as they do for a scanned root.
    safety::ancestors(&snapshot.root).unwrap();
    install(state, snapshot)
}
#[test]
fn quick_look_authority_rejects_partial_stale_and_unsupported_items() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("preview.txt"), b"preview").unwrap();
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let s = registered_preview_fixture(&state, snapshot(root.clone()));
        assert!(preview_item(&s, 1).unwrap().open_checked().is_ok());
        assert!(preview_item(&s, 0).is_err());
        for status in [ScanState::Running, ScanState::Cancelled, ScanState::Failed] {
            s.store.write().unwrap().state = status;
            assert!(
                quick_look_node(app.state(), s.id.clone(), 1, app.handle().clone())
                    .await
                    .is_err()
            );
            assert!(!state.activity.busy());
        }
        s.store.write().unwrap().state = ScanState::Completed;
        s.finishing.store(true, Ordering::Release);
        assert!(preview_item(&s, 1).is_err());
        s.finishing.store(false, Ordering::Release);
        s.store.write().unwrap().stats.incomplete = true;
        assert!(preview_item(&s, 1).unwrap().open_checked().is_ok());
        for kind in [NodeKind::Directory, NodeKind::Symlink, NodeKind::Other] {
            s.store
                .write()
                .unwrap()
                .update_node(1, |n| n.identity.kind = kind)
                .unwrap();
            assert!(
                quick_look_node(app.state(), s.id.clone(), 1, app.handle().clone())
                    .await
                    .is_err()
            );
        }
        s.store
            .write()
            .unwrap()
            .update_node(1, |n| n.identity.kind = NodeKind::File)
            .unwrap();
        s.store
            .write()
            .unwrap()
            .update_node(1, |n| n.excluded = true)
            .unwrap();
        assert!(preview_item(&s, 1).is_err());
        s.store
            .write()
            .unwrap()
            .update_node(1, |n| n.excluded = false)
            .unwrap();
        assert!(
            quick_look_node(app.state(), "stale".into(), 1, app.handle().clone())
                .await
                .err()
                .unwrap()
                .contains("active")
        );
        s.invalidate();
        assert!(
            quick_look_node(app.state(), s.id.clone(), 1, app.handle().clone())
                .await
                .err()
                .unwrap()
                .contains(slots::RESULTS_CHANGED)
        );
    });
}
#[test]
fn preview_close_and_session_mutations_revoke_pending_authority() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let s = install(&state, synthetic(1));
        let pending = state.preview.begin().unwrap();
        let closed = close_quick_look(app.state(), app.handle().clone())
            .await
            .unwrap();
        assert!(!closed.open);
        assert!(closed.revision > pending);
        assert!(state
            .preview
            .publish(pending, s.id.clone(), 1, true)
            .is_err());
        let held = Activity::begin(&state.activity).unwrap();
        assert!(
            quick_look_node(app.state(), s.id.clone(), 1, app.handle().clone())
                .await
                .err()
                .unwrap()
                .contains("progress")
        );
        // Dismissal is always allowed, including while an opener owns activity.
        assert!(
            !close_quick_look(app.state(), app.handle().clone())
                .await
                .unwrap()
                .open
        );
        drop(held);
        let active = state.preview.begin().unwrap();
        state
            .preview
            .publish(active, s.id.clone(), 1, true)
            .unwrap();
        assert!(start_scan(
            app.state(),
            "missing".into(),
            Channel::new(|_| Ok(())),
            app.handle().clone()
        )
        .await
        .is_err());
        assert!(!state.preview.status().open);
        assert!(state
            .preview
            .publish(active, s.id.clone(), 1, true)
            .is_err());
        let active = state.preview.begin().unwrap();
        state
            .preview
            .publish(active, s.id.clone(), 1, true)
            .unwrap();
        assert!(commit_trash(
            app.state(),
            s.id.clone(),
            "missing-ticket".into(),
            app.handle().clone()
        )
        .await
        .is_err());
        assert!(!state.preview.status().open);
        assert!(state
            .preview
            .publish(active, "fixture".into(), 1, true)
            .is_err());
    });
}

/// A core reconciliation probe, not an end-to-end app/cache benchmark. Never mutates the root.
#[test]
#[ignore = "opt-in read-only cached directory validation worker sweep"]
#[cfg(target_os = "macos")]
fn native_validation_worker_sweep() {
    use baseline::Journal;
    use std::os::unix::fs::OpenOptionsExt;
    fn clean(s: &Snapshot) -> bool {
        s.state == ScanState::Completed
            && !s.stats.incomplete
            && s.stats.errors == 0
            && s.stats.excluded == 0
    }
    fn totals(s: &Snapshot) -> serde_json::Value {
        serde_json::json!({"entries": s.stats.entries, "directories": s.stats.directories,
            "allocated_bytes": s.stats.allocated_bytes})
    }
    let input =
        PathBuf::from(std::env::var_os("HAH_VALIDATION_SWEEP_ROOT").expect("root required"));
    assert!(input.is_absolute());
    let root = input.canonicalize().unwrap();
    let report_path =
        PathBuf::from(std::env::var_os("HAH_VALIDATION_SWEEP_REPORT").expect("report required"));
    assert!(report_path.is_absolute());
    let parent = report_path.parent().unwrap().canonicalize().unwrap();
    assert!(
        !parent.starts_with(&root),
        "report must be outside scanned root"
    );
    let report_path = parent.join(report_path.file_name().unwrap());
    assert!(!report_path.exists(), "use a fresh report path");
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&report_path)
        .unwrap();
    let checkpoint = |output: &mut std::fs::File, report: &serde_json::Value| {
        use std::io::{Seek, Write};
        output.rewind().unwrap();
        output.set_len(0).unwrap();
        serde_json::to_writer_pretty(&mut *output, report).unwrap();
        output.flush().unwrap();
    };
    let authority =
        disk_scan::ScanRoot::open(root.clone(), disk_scan::platform_identity(&root).unwrap())
            .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let cursor = events::current_id().expect("public FSEvents cursor required");
    let full = |workers| {
        let store = Arc::new(RwLock::new(Snapshot::empty(root.clone())));
        disk_scan::scan_live_authorized(
            authority.reopen().unwrap(),
            disk_scan::ScanOptions {
                workers,
                ..Default::default()
            },
            cancel.clone(),
            store.clone(),
            |_| {},
        )
        .unwrap();
        Arc::try_unwrap(store).unwrap().into_inner().unwrap()
    };
    let first_full = full(3);
    assert!(clean(&first_full), "baseline must be complete and clean");
    // Public replay flushes pending history; resolving it preserves ambiguous hard-link refresh.
    let volume = baseline::Fsevents
        .history(&authority)
        .expect("local volume required")
        .volume;
    let (_, changes) =
        events::replay_with_load_on_volume(&root, &volume, cursor, &cancel, || Ok(())).unwrap();
    let changes = changes.resolve(&first_full).unwrap();
    let mut report = serde_json::json!({"schema_version":1,"complete":false,
        "metric":"core reconciliation through completed publication; excludes seed clone, reopen, replay, cache decode/save and app launch",
        "root":root,"baseline":totals(&first_full),"cursor":cursor,
        "dirty_paths":changes.paths.len(),"dirty_directories":changes.directories.len(),"runs":[]});
    checkpoint(&mut output, &report);
    let counts = [1, 2, 3, 4, 6, 8];
    let mut qualified = true;
    for round in 0..4 {
        for position in 0..counts.len() {
            let workers = counts[(position + round) % counts.len()];
            let seed = disk_scan::IncrementalSeed {
                snapshot: first_full.clone(),
                dirty_paths: changes.paths.clone(),
                dirty_directories: changes.directories.clone(),
            };
            let reopened = authority.reopen().unwrap();
            let store = Arc::new(RwLock::new(Snapshot::empty(root.clone())));
            let start = std::time::Instant::now();
            let result = disk_scan::scan_live_incremental_authorized(
                reopened,
                disk_scan::ScanOptions {
                    workers,
                    ..Default::default()
                },
                cancel.clone(),
                store.clone(),
                |_| {},
                seed,
            );
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            let candidate = store.read().unwrap();
            let opens: u64 = candidate
                .worker_stats
                .iter()
                .map(|w| w.relative_opens + w.absolute_reopens)
                .sum();
            let success = result
                .as_ref()
                .is_ok_and(|reuse| reuse.fallback_reason.is_none() && reuse.reused_entries > 0)
                && clean(&candidate)
                && candidate.worker_stats.len() == workers
                && opens == candidate.stats.directories.saturating_sub(1);
            let matches_baseline = totals(&candidate) == totals(&first_full);
            qualified &= success && matches_baseline;
            report["runs"].as_array_mut().unwrap().push(serde_json::json!({
                "round":round,"warm":round>0,"requested_workers":workers,"actual_workers":candidate.worker_stats.len(),
                "elapsed_ms":elapsed_ms,"reuse":result.as_ref().ok(),"error":result.as_ref().err().map(ToString::to_string),
                "totals":totals(&candidate),"errors":candidate.stats.errors,"excluded":candidate.stats.excluded,
                "fresh_opens":opens,"worker_stats":candidate.worker_stats,"finalization_us":candidate.finalization_us,
                "qualified":success,"matches_baseline_aggregates":matches_baseline}));
            checkpoint(&mut output, &report);
            eprintln!(
                "validation sweep: round={round}, workers={workers}, elapsed_ms={elapsed_ms:.3}, qualified={success}, baseline_parity={matches_baseline}"
            );
        }
    }
    let final_full = full(3);
    let stable = clean(&final_full) && totals(&final_full) == totals(&first_full);
    report["final_full"] = totals(&final_full);
    report["aggregate_stable"] = stable.into();
    report["qualified"] = (qualified && stable).into();
    report["coverage_limitation"] = "Aggregate probe only; final app benchmark must check exact path coverage. Live changes are retained as drift, never filtered.".into();
    report["complete"] = true.into();
    checkpoint(&mut output, &report);
    assert!(
        qualified && stable,
        "sweep contains failed invariants or live-data drift; inspect report"
    );
}

#[test]
fn actual_worker_selection_is_reported_with_matching_worker_rows() {
    tauri::async_runtime::block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("source");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("file"), b"fixture").unwrap();
        let mut source = disk_scan::scan(
            root,
            disk_scan::ScanOptions {
                workers: 4,
                ..Default::default()
            },
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .unwrap();
        assert_eq!(source.worker_stats.len(), 4);
        source.state = ScanState::Completed;
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let live = install(&state, source);
        assert_eq!(live.workers, 8); // Captured base remains immutable.
        live.scan_details.lock().unwrap().actual_workers = Some(4);
        assert_eq!(live.actual_workers(), 4);
        assert_eq!(live.summary().workers, live.summary().worker_stats.len());
        // A core retry callback can reset reported selection without changing captured base.
        live.scan_details.lock().unwrap().actual_workers = Some(live.workers);
        assert_eq!(live.actual_workers(), live.workers);
        assert_eq!(live.summary().workers, live.workers);
    });
}

#[test]
fn cleanup_queue_restart_requires_live_matching_item_root_and_ancestry() {
    tauri::async_runtime::block_on(async {
        let dir = fixture();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let folder = root.join("folder");
        std::fs::create_dir(&folder).unwrap();
        let file = folder.join("item");
        std::fs::write(&file, b"recorded").unwrap();
        let storage = root.join("queue-storage");
        let app = app(AppState::default());
        *app.state::<AppState>().cleanup_queue.lock().unwrap() =
            cleanup_queue::Queue::load(storage.clone());
        let scan = snapshot(root.clone());
        let id = scan
            .iter_nodes()
            .find(|n| n.path().unwrap() == file)
            .unwrap()
            .id;
        install(&app.state::<AppState>(), scan);
        let added = add_cleanup_queue(app.state(), "fixture".into(), vec![id])
            .await
            .unwrap();
        assert_eq!(added.entries[0].status, "ready");
        let entry_id = added.entries[0].id.clone();
        // As after a restart: no scan is held any more.
        app.state::<AppState>().scans.discard("fixture");
        *app.state::<AppState>().cleanup_queue.lock().unwrap() =
            cleanup_queue::Queue::load(storage.clone());
        assert_eq!(
            list_cleanup_queue(app.state()).await.unwrap().entries[0].status,
            "needsScan"
        );
        let target = cleanup_queue_target(app.state(), entry_id.clone())
            .await
            .unwrap();
        assert_eq!(
            app.state::<AppState>().targets.lock().unwrap()[&target.id].1,
            root
        );
        // A plain folder stays out of the Disks list and uses the shared folder slot.
        assert!(!target.volume);
        install(&app.state::<AppState>(), snapshot(root.clone()));
        assert_eq!(
            list_cleanup_queue(app.state()).await.unwrap().entries[0].status,
            "ready"
        );
        std::fs::rename(&folder, root.join("old-folder")).unwrap();
        std::fs::create_dir(&folder).unwrap();
        std::fs::rename(root.join("old-folder/item"), &file).unwrap();
        install(&app.state::<AppState>(), snapshot(root.clone()));
        assert_eq!(
            list_cleanup_queue(app.state()).await.unwrap().entries[0].status,
            "changed"
        );
        assert!(prepare_cleanup_queue(app.state(), "fixture".into())
            .await
            .is_err());
        std::fs::rename(&file, folder.join("old-item")).unwrap();
        std::fs::write(&file, b"replacement").unwrap();
        install(&app.state::<AppState>(), snapshot(root.clone()));
        assert_eq!(
            list_cleanup_queue(app.state()).await.unwrap().entries[0].status,
            "changed"
        );
        remove_cleanup_queue(app.state(), vec![entry_id])
            .await
            .unwrap();
        let scan = snapshot(root.clone());
        let id = scan
            .iter_nodes()
            .find(|n| n.path().unwrap() == file)
            .unwrap()
            .id;
        install(&app.state::<AppState>(), scan);
        add_cleanup_queue(app.state(), "fixture".into(), vec![id])
            .await
            .unwrap();
        std::fs::rename(&root, root.with_extension("moved")).unwrap();
        assert!(cleanup_queue_target(
            app.state(),
            list_cleanup_queue(app.state()).await.unwrap().entries[0]
                .id
                .clone()
        )
        .await
        .is_err());
        std::fs::rename(root.with_extension("moved"), root).unwrap();
    });
}

#[test]
fn cleanup_queue_batch_removal_publishes_once_and_does_not_remove_covering_parent() {
    tauri::async_runtime::block_on(async {
        let dir = fixture();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let parent = root.join("parent");
        std::fs::create_dir(&parent).unwrap();
        for path in [parent.join("child"), root.join("a"), root.join("b")] {
            std::fs::write(path, []).unwrap();
        }
        let app = app(AppState::default());
        let scan = snapshot(root.clone());
        let id = |path: &std::path::Path| {
            scan.iter_nodes()
                .find(|n| n.path().unwrap() == path)
                .unwrap()
                .id
        };
        let child = id(&parent.join("child"));
        let top_level = vec![id(&parent), id(&root.join("a")), id(&root.join("b"))];
        install(&app.state::<AppState>(), scan);
        let child_queue = add_cleanup_queue(app.state(), "fixture".into(), vec![child])
            .await
            .unwrap();
        let child_id = child_queue.entries[0].id.clone();
        let added = add_cleanup_queue(app.state(), "fixture".into(), top_level)
            .await
            .unwrap();
        let ids: Vec<_> = added
            .entries
            .iter()
            .filter(|entry| entry.path != parent.to_string_lossy())
            .map(|entry| entry.id.clone())
            .collect();
        let mut with_covered_child = ids.clone();
        with_covered_child.push(child_id);
        assert!(remove_cleanup_queue(app.state(), with_covered_child)
            .await
            .err()
            .unwrap()
            .contains("Unknown cleanup queue entry"));
        let unchanged = list_cleanup_queue(app.state()).await.unwrap();
        assert_eq!(unchanged.revision, added.revision);
        assert_eq!(unchanged.entries.len(), 3);
        let removed = remove_cleanup_queue(app.state(), ids).await.unwrap();
        assert_eq!(removed.revision, added.revision + 1);
        assert_eq!(removed.entries.len(), 1);
        assert_eq!(removed.entries[0].path, parent.to_string_lossy());
        assert_eq!(removed.entries[0].status, "ready");
    });
}

#[test]
fn cleanup_queue_revision_replay_and_scan_switch_guard_review_without_side_effects() {
    tauri::async_runtime::block_on(async {
        let dir = fixture();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("a"), b"a").unwrap();
        std::fs::write(root.join("b"), b"b").unwrap();
        let app = app(AppState::default());
        let scan = snapshot(root.clone());
        let ids: Vec<_> = scan
            .iter_nodes()
            .filter(|n| n.parent == Some(0))
            .map(|n| n.id)
            .collect();
        install(&app.state::<AppState>(), scan);
        let added = add_cleanup_queue(app.state(), "fixture".into(), ids)
            .await
            .unwrap();
        let review = prepare_cleanup_queue(app.state(), "fixture".into())
            .await
            .unwrap();
        {
            let state = app.state::<AppState>();
            let session = state.scans.find("fixture").unwrap();
            let ticket = session.ticket.lock().unwrap();
            let ticket = ticket.as_ref().unwrap();
            assert_eq!(review.items.len(), ticket.items.len());
            for (view, item) in review.items.iter().zip(&ticket.items) {
                assert_eq!(view.path, item.path.to_string_lossy());
            }
        }
        remove_cleanup_queue(app.state(), vec![added.entries[0].id.clone()])
            .await
            .unwrap();
        assert!(commit_cleanup_queue(
            app.state(),
            "fixture".into(),
            review.token.clone(),
            app.handle().clone()
        )
        .await
        .unwrap_err()
        .contains("queue changed"));
        assert!(commit_cleanup_queue(
            app.state(),
            "fixture".into(),
            review.token,
            app.handle().clone()
        )
        .await
        .is_err());
        let review = prepare_cleanup_queue(app.state(), "fixture".into())
            .await
            .unwrap();
        app.state::<AppState>()
            .scans
            .find("fixture")
            .unwrap()
            .ticket
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .created = Instant::now() - Duration::from_secs(121);
        assert!(commit_cleanup_queue(
            app.state(),
            "fixture".into(),
            review.token,
            app.handle().clone()
        )
        .await
        .unwrap_err()
        .contains("expired"));
        let review = prepare_cleanup_queue(app.state(), "fixture".into())
            .await
            .unwrap();
        install(&app.state::<AppState>(), snapshot(root.clone()));
        assert!(commit_cleanup_queue(
            app.state(),
            "fixture".into(),
            review.token,
            app.handle().clone()
        )
        .await
        .is_err());
        assert!(root.join("a").exists() && root.join("b").exists());
        let s = install(&app.state::<AppState>(), snapshot(root));
        s.store.write().unwrap().state = ScanState::Cancelled;
        assert_eq!(
            list_cleanup_queue(app.state()).await.unwrap().entries[0].status,
            "needsScan"
        );
        assert!(prepare_cleanup_queue(app.state(), "fixture".into())
            .await
            .is_err());
    });
}

/// The test Trash adapter: records every item it is asked to move, moves nothing, and fails the paths in `fails`.
struct Recording {
    calls: Vec<PathBuf>,
    fails: Vec<PathBuf>,
    error: &'static str,
}
impl Recording {
    fn ok() -> Self {
        Self::failing(&[], "")
    }
    fn failing(paths: &[&Path], error: &'static str) -> Self {
        Recording {
            calls: vec![],
            fails: paths.iter().map(|p| p.to_path_buf()).collect(),
            error,
        }
    }
    fn trash(&mut self, item: &safety::CheckedItem) -> Result<(), String> {
        self.calls.push(item.path.clone());
        if self.fails.contains(&item.path) {
            Err(self.error.into())
        } else {
            Ok(())
        }
    }
}

#[test]
fn cleanup_queue_executor_partial_failure_whole_batch_validation_and_pruning() {
    let dir = fixture();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let a = root.join("a");
    let b = root.join("b");
    std::fs::write(&a, b"a").unwrap();
    std::fs::write(&b, b"b").unwrap();
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    // A fresh scan with a review of both files: a commit that trashes anything invalidates its scan.
    let reviewed = || {
        let scan = snapshot(root.clone());
        let ids: Vec<_> = scan
            .iter_nodes()
            .filter(|n| n.parent == Some(0))
            .map(|n| n.id)
            .collect();
        let s = install(&state, scan);
        let token = cleanup::review(&s, cleanup::Pick::Nodes(&ids))
            .unwrap()
            .token;
        (s, token)
    };
    let (s, token) = reviewed();
    let mut trash = Recording::failing(&[&b], "injected native Trash failure");
    let result =
        cleanup::commit(&s, &state.cleanup_queue, &token, false, |i| trash.trash(i)).unwrap();
    assert_eq!(result.trashed, 1);
    assert_eq!(result.failures, vec!["injected native Trash failure"]);
    // Every item is offered to the Trash in review order.
    let items = trash.calls;
    assert_eq!(items.len(), 2);
    assert_eq!(result.outcomes.len(), items.len());
    for (index, path) in items.iter().enumerate() {
        let moved = *path == a;
        assert_eq!(
            result.outcomes[index],
            TrashOutcome {
                review_index: index,
                status: if moved {
                    TrashOutcomeStatus::Trashed
                } else {
                    TrashOutcomeStatus::Failed
                },
                message: (!moved).then(|| "injected native Trash failure".into()),
            }
        );
        let serialized = serde_json::to_value(&result.outcomes[index]).unwrap();
        assert_eq!(
            serialized,
            if moved {
                serde_json::json!({"reviewIndex": index, "status": "trashed"})
            } else {
                serde_json::json!({"reviewIndex": index, "status": "failed", "message": "injected native Trash failure"})
            }
        );
    }
    let serialized = serde_json::to_value(&result).unwrap();
    assert_eq!(serialized["trashed"], 1);
    assert_eq!(
        serialized["failures"],
        serde_json::json!(["injected native Trash failure"])
    );
    assert!(serialized["persistenceWarning"].is_null());
    assert_eq!(serialized["outcomes"].as_array().unwrap().len(), 2);

    // The adapter only simulates native outcomes; fixture paths remain untouched.
    for succeeds in [true, false] {
        let (s, token) = reviewed();
        let all: Vec<&Path> = items.iter().map(|p| p.as_path()).collect();
        let mut trash = if succeeds {
            Recording::ok()
        } else {
            Recording::failing(&all, "all failed")
        };
        let result =
            cleanup::commit(&s, &state.cleanup_queue, &token, false, |i| trash.trash(i)).unwrap();
        assert_eq!(trash.calls.len(), items.len());
        assert_eq!(result.trashed, if succeeds { items.len() } else { 0 });
        assert_eq!(result.failures.len(), items.len() - result.trashed);
        assert_eq!(s.is_valid(), !succeeds);
        assert_eq!(result.outcomes.len(), items.len());
        for (index, outcome) in result.outcomes.iter().enumerate() {
            assert_eq!(outcome.review_index, index);
            assert_eq!(
                outcome.status,
                if succeeds {
                    TrashOutcomeStatus::Trashed
                } else {
                    TrashOutcomeStatus::Failed
                }
            );
            assert_eq!(
                outcome.message.as_deref(),
                if succeeds { None } else { Some("all failed") }
            );
        }
    }
    // A commit with the wrong token leaves the valid ticket usable.
    let (s, token) = reviewed();
    let mut trash = Recording::ok();
    assert!(cleanup::commit(&s, &state.cleanup_queue, "wrong", false, |i| trash.trash(i)).is_err());
    assert!(trash.calls.is_empty());
    let result = cleanup::commit(&s, &state.cleanup_queue, &token, false, |i| trash.trash(i));
    assert_eq!(result.err(), None);
    assert_eq!(trash.calls.len(), items.len());
    // One replaced item fails the whole batch before anything moves.
    let (s, token) = reviewed();
    std::fs::rename(&b, root.join("old-b")).unwrap();
    std::fs::write(&b, b"replacement").unwrap();
    let mut trash = Recording::ok();
    assert_eq!(
        cleanup::commit(&s, &state.cleanup_queue, &token, false, |i| trash.trash(i)).err(),
        Some("The selected item changed. Rescan before cleanup.".into())
    );
    assert!(trash.calls.is_empty());
}

#[test]
fn cleanup_queue_direct_success_partial_failure_and_persistence_warning() {
    tauri::async_runtime::block_on(async {
        let dir = fixture();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let storage = root.join("queue-storage");
        let a = root.join("a");
        let b = root.join("b");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        let app = app(AppState::default());
        *app.state::<AppState>().cleanup_queue.lock().unwrap() =
            cleanup_queue::Queue::load(storage.clone());
        let scan = snapshot(root.clone());
        let ids: Vec<_> = scan
            .iter_nodes()
            .filter(|n| n.parent == Some(0))
            .map(|n| n.id)
            .collect();
        let s = install(&app.state::<AppState>(), scan);
        add_cleanup_queue(app.state(), "fixture".into(), ids.clone())
            .await
            .unwrap();
        // Direct and queued commits call this exact executor/finalizer; native I/O is injected.
        let review = prepare_trash(app.state(), "fixture".into(), ids)
            .await
            .unwrap();
        {
            let ticket = s.ticket.lock().unwrap();
            let ticket = ticket.as_ref().unwrap();
            assert_eq!(review.token, ticket.token);
            assert!(ticket.queue_revision.is_none());
            assert_eq!(review.items.len(), ticket.items.len());
            for (view, item) in review.items.iter().zip(&ticket.items) {
                assert_eq!(view.path, item.path.to_string_lossy());
            }
        }
        let mut trash = Recording::failing(&[&b], "injected failure");
        let result = cleanup::commit(
            &s,
            &app.state::<AppState>().cleanup_queue,
            &review.token,
            false,
            |i| trash.trash(i),
        )
        .unwrap();
        assert_eq!(result.trashed, 1);
        assert!(result.persistence_warning.is_none());
        assert!(!s.is_valid());
        let restored = cleanup_queue::Queue::load(storage.clone());
        assert_eq!(restored.entries.len(), 1);
        assert_eq!(restored.entries[0].item.path, b);
        assert_eq!(
            restored.entries[0].last_error.as_deref(),
            Some("injected failure")
        );
        assert_eq!(
            list_cleanup_queue(app.state()).await.unwrap().entries[0].status,
            "needsScan"
        );
        let queued = list_cleanup_queue(app.state()).await.unwrap();
        assert_eq!(
            queued.entries[0].last_error.as_deref(),
            Some("injected failure")
        );
        assert_ne!(
            queued.entries[0].reason.as_deref(),
            Some("injected failure")
        );
        let scan = snapshot(root.clone());
        let id = scan
            .iter_nodes()
            .find(|n| n.path().unwrap() == b)
            .unwrap()
            .id;
        let s = install(&app.state::<AppState>(), scan);
        let queued = list_cleanup_queue(app.state()).await.unwrap();
        assert_eq!(queued.entries[0].status, "ready");
        assert_eq!(
            queued.entries[0].last_error.as_deref(),
            Some("injected failure")
        );
        assert!(queued.entries[0].reason.is_none());
        let review = prepare_trash(app.state(), "fixture".into(), vec![id])
            .await
            .unwrap();
        let moved = root.join("old-storage");
        std::fs::rename(&storage, &moved).unwrap();
        std::fs::write(&storage, b"blocks persistence").unwrap();
        let mut trash = Recording::ok();
        let result = cleanup::commit(
            &s,
            &app.state::<AppState>().cleanup_queue,
            &review.token,
            false,
            |i| trash.trash(i),
        )
        .unwrap();
        assert_eq!(result.trashed, 1);
        assert!(result.persistence_warning.is_some());
        assert_eq!(
            result.outcomes,
            vec![TrashOutcome {
                review_index: 0,
                status: TrashOutcomeStatus::Trashed,
                message: None,
            }]
        );
        assert!(!s.is_valid());
        assert_eq!(
            app.state::<AppState>()
                .cleanup_queue
                .lock()
                .unwrap()
                .entries
                .len(),
            1
        );
        assert_eq!(cleanup_queue::Queue::load(moved).entries.len(), 1);
        assert_eq!(
            list_cleanup_queue(app.state()).await.unwrap().entries[0].status,
            "needsScan"
        );
    });
}

/// Runs `work` on another thread while the scan's store is write-locked, which parks its status checks partway,
/// and gives `edit` the queue if it is free by then. Returns whether it was, with the result of `work`.
fn while_parked<T: Send>(
    s: &Session,
    queue: &Mutex<cleanup_queue::Queue>,
    work: impl FnOnce() -> T + Send,
    edit: impl FnOnce(&mut cleanup_queue::Queue),
) -> (bool, T) {
    let store = s.store.write().unwrap();
    std::thread::scope(|scope| {
        let work = scope.spawn(work);
        std::thread::sleep(Duration::from_millis(200));
        let free = match queue.try_lock() {
            Ok(mut queue) => {
                edit(&mut queue);
                true
            }
            Err(_) => false,
        };
        drop(store);
        (free, work.join().unwrap())
    })
}

#[test]
fn queue_view_and_review_check_the_disk_without_the_queue_lock() {
    let dir = fixture();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("a"), b"a").unwrap();
    std::fs::write(root.join("b"), b"b").unwrap();
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let _storage = queue_in_tempdir(&state);
    let scan = snapshot(root);
    let ids: Vec<_> = scan
        .iter_nodes()
        .filter(|n| n.parent == Some(0))
        .map(|n| n.id)
        .collect();
    let s = install(&state, scan);
    cleanup::add(&s, &state.cleanup_queue, &ids).unwrap();
    let queue = &state.cleanup_queue;
    let scans = state.scans.sessions();
    // An edit that lands while the entries are checked is in the view that comes back.
    let (free, view) = while_parked(
        &s,
        queue,
        || cleanup::queue_view(queue, &scans),
        |q| {
            let id = q.entries[0].id.clone();
            q.remove(&[id]).unwrap();
        },
    );
    assert!(
        free,
        "queue_view held the queue lock while checking the disk"
    );
    assert_eq!(view.revision, queue.lock().unwrap().revision);
    assert_eq!(view.entries.len(), 1);
    // The ticket records the revision of the queue the review read.
    let (free, review) = while_parked(
        &s,
        queue,
        || cleanup::review(&s, cleanup::Pick::Queue(queue, &scans)),
        |_| {},
    );
    assert!(free, "review held the queue lock while checking the disk");
    review.unwrap();
    assert_eq!(
        s.ticket.lock().unwrap().as_ref().unwrap().queue_revision,
        Some(queue.lock().unwrap().revision)
    );
}

#[test]
fn a_selection_that_cannot_fit_the_queue_is_refused_before_any_disk_check() {
    const FULL: &str = "Cleanup queue supports at most 1,000 entries. Remove items first.";
    let dir = fixture();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    for i in 0..=cleanup_queue::MAX_ENTRIES {
        std::fs::write(root.join(i.to_string()), []).unwrap();
    }
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let _storage = queue_in_tempdir(&state);
    let scan = snapshot(root.clone());
    let all: Vec<_> = scan
        .iter_nodes()
        .filter(|n| n.parent == Some(0))
        .map(|n| n.id)
        .collect();
    let last = node_of(&scan, &root.join(cleanup_queue::MAX_ENTRIES.to_string()));
    let s = install(&state, scan);
    let queue = &state.cleanup_queue;
    // A file gone from disk would fail its own check with another message: the count is judged first.
    std::fs::remove_file(root.join(cleanup_queue::MAX_ENTRIES.to_string())).unwrap();
    assert_eq!(cleanup::add(&s, queue, &all), Err(FULL.into()));
    let fits: Vec<_> = all.iter().copied().filter(|&id| id != last).collect();
    cleanup::add(&s, queue, &fits).unwrap();
    assert_eq!(
        queue.lock().unwrap().entries.len(),
        cleanup_queue::MAX_ENTRIES
    );
    // A full queue still takes what it already holds, and refuses one more.
    cleanup::add(&s, queue, &fits[..1]).unwrap();
    assert_eq!(cleanup::add(&s, queue, &[last]), Err(FULL.into()));
}

// Per-disk scan slots: each disk keeps its own live scan, folders share one.
fn register_disk(state: &AppState, root: &Path) -> platform::Target {
    let mut target = platform::target(root, "disk".into());
    target.volume = true;
    register(state, target, root.to_path_buf()).unwrap()
}
fn scan_to_end(app: &tauri::App<tauri::test::MockRuntime>, target: &platform::Target) -> String {
    let (id, events) = start(app, target).unwrap();
    events.wait_end();
    id
}
fn disk_root() -> (tempfile::TempDir, PathBuf) {
    let dir = fixture();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::write(root.join("file"), b"abc").unwrap();
    (dir, root)
}

#[test]
fn scanning_another_disk_keeps_the_first_disks_scan_and_rescans_replace_only_their_own() {
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let (_a_dir, a_root) = disk_root();
    let (_b_dir, b_root) = disk_root();
    let a = register_disk(&state, &a_root);
    let b = register_disk(&state, &b_root);
    let first_a = scan_to_end(&app, &a);
    let first_b = scan_to_end(&app, &b);
    for id in [&first_a, &first_b] {
        let s = state.scans.find(id).unwrap();
        assert_eq!(s.summary().state, "completed");
        assert_eq!(s.summary().entries, 2);
    }
    assert_eq!(state.scans.sessions().len(), 2);
    // Rescanning A replaces A only.
    let second_a = scan_to_end(&app, &a);
    assert_ne!(second_a, first_a);
    assert!(state
        .scans
        .find(&first_a)
        .err()
        .unwrap()
        .contains("no longer active"));
    assert!(state.scans.find(&first_b).is_ok());
    assert!(state.scans.find(&second_a).is_ok());
    assert_eq!(state.scans.sessions().len(), 2);
}

#[test]
fn a_rescan_takes_only_its_own_disks_baseline_and_leaves_other_disks_intact() {
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let (_a_dir, a_root) = disk_root();
    let (_b_dir, b_root) = disk_root();
    let a = register_disk(&state, &a_root);
    register_disk(&state, &b_root);
    let token = || baseline::Token {
        cursor: 1,
        epoch: "epoch".into(),
        full_scan_at_ms: now_ms(),
    };
    let old_a = install_in(&state, Slot::Disk(a_root.clone()), "a", snapshot(a_root));
    let old_b = install_in(&state, Slot::Disk(b_root.clone()), "b", snapshot(b_root));
    *old_a.cache_token.lock().unwrap() = Some(token());
    *old_b.cache_token.lock().unwrap() = Some(token());
    let new_a = scan_to_end(&app, &a);
    // A's snapshot moved out as the baseline; B's was never touched.
    assert_eq!(old_a.store.read().unwrap().node_count(), 0);
    assert!(!old_a.is_valid());
    assert_eq!(old_b.store.read().unwrap().node_count(), 2);
    assert!(old_b.is_valid());
    assert!(old_b.cache_token.lock().unwrap().is_some());
    assert!(state.scans.find("b").is_ok());
    assert!(state.scans.find(&new_a).is_ok());
}

#[test]
fn a_start_does_not_wait_for_a_read_of_the_old_results_and_scans_without_a_baseline() {
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let (_a_dir, a_root) = disk_root();
    let (_b_dir, b_root) = disk_root();
    let a = register_disk(&state, &a_root);
    let b = register_disk(&state, &b_root);
    let old = install_in(&state, Slot::Disk(a_root.clone()), "a", snapshot(a_root));
    *old.cache_token.lock().unwrap() = Some(baseline::Token {
        cursor: 1,
        epoch: "epoch".into(),
        full_scan_at_ms: now_ms(),
    });
    std::thread::scope(|scope| {
        // A read of the old results that never ends, like the queue view's across a hung disk check. Dropped on
        // a failure too.
        let reader = old.store.read().unwrap();
        let starting = scope.spawn(|| start_recorded(&state, &a, Recorder::default()).unwrap());
        // Fails instead of hanging when the start waits for the reader.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !starting.is_finished() {
            assert!(
                std::time::Instant::now() < deadline,
                "the start waited for a read of the old results"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let (id, events) = starting.join().unwrap();
        // The old scan is gone and keeps its results (no baseline was taken); the new one runs on its own.
        assert!(!old.is_valid());
        assert_eq!(reader.node_count(), 2);
        assert!(state.access_setup.try_lock().is_ok());
        let (_, other) = start_recorded(&state, &b, Recorder::default()).unwrap();
        other.wait_end();
        events.wait_end();
        assert!(state.scans.find(&id).is_ok());
        assert!(setup_idle(&state).is_ok());
    });
}

/// Starts a scan of `target` as `start_scan` does past its gate, reporting to a fresh `Recorder`.
fn start(
    app: &tauri::App<tauri::test::MockRuntime>,
    target: &platform::Target,
) -> Result<(String, Recorder), String> {
    start_recorded(&app.state::<AppState>(), target, Recorder::default())
}
fn appstate_with_budget(workers: usize, descriptors: usize) -> AppState {
    AppState {
        scans: Arc::new(slots::Slots::with_budget(budget::Budget::new(
            budget::Share {
                workers,
                descriptors,
            },
        ))),
        ..Default::default()
    }
}
/// Whether a scan runs anywhere, a discarded one still winding down included.
fn scan_running(state: &AppState) -> bool {
    state.scans.require_idle("running").is_err()
}
fn in_slot(state: &AppState, slot: Slot) -> Arc<Session> {
    state
        .scans
        .sessions()
        .into_iter()
        .find(|(key, _)| *key == slot)
        .unwrap()
        .1
}

#[test]
fn a_scan_start_is_refused_only_for_the_slot_that_is_already_scanning() {
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let (_a_dir, a_root) = disk_root();
    let (_b_dir, b_root) = disk_root();
    let (_f_dir, f_root) = disk_root();
    let running = install_in(
        &state,
        Slot::Disk(a_root.clone()),
        "a",
        snapshot(a_root.clone()),
    );
    running.store.write().unwrap().state = ScanState::Running;
    let a = register_disk(&state, &a_root);
    let b = register_disk(&state, &b_root);
    let folder = register(
        &state,
        platform::target(&f_root, "folder".into()),
        f_root.clone(),
    )
    .unwrap();
    // The same disk is refused, with nothing replaced.
    assert_eq!(
        start(&app, &a).unwrap_err(),
        "This disk is already being scanned. Cancel that scan or wait for it to finish first."
    );
    assert!(running.is_valid());
    assert!(state.scans.find("a").is_ok());
    // Another disk and the folder slot start beside it.
    let (_, b_events) = start(&app, &b).unwrap();
    let (_, folder_events) = start(&app, &folder).unwrap();
    b_events.wait_end();
    folder_events.wait_end();
    assert_eq!(state.scans.sessions().len(), 3);
    // A scan that is still running in its slot refuses a folder start the same way.
    let folder_running = in_slot(&state, Slot::Folder);
    folder_running.store.write().unwrap().state = ScanState::Running;
    assert_eq!(
        start(&app, &folder).unwrap_err(),
        "A folder is already being scanned. Cancel that scan or wait for it to finish first."
    );
    assert!(folder_running.is_valid());
    assert!(running.is_valid());
}

#[test]
fn scans_on_different_disks_run_side_by_side_with_independent_results_and_a_shared_budget() {
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let total = state.scans.budget().total();
    let roots: Vec<_> = (0..3).map(|_| disk_root()).collect();
    for (i, (_, root)) in roots.iter().enumerate() {
        std::fs::write(root.join("more"), vec![0u8; 100 * (i + 1)]).unwrap();
    }
    let targets: Vec<_> = roots
        .iter()
        .map(|(_, root)| register_disk(&state, root))
        .collect();
    let (ids, events): (Vec<_>, Vec<_>) = targets
        .iter()
        .map(|target| start(&app, target).unwrap())
        .unzip();
    for events in &events {
        events.wait_end();
    }
    for (i, id) in ids.iter().enumerate() {
        let s = state.scans.find(id).unwrap();
        let summary = s.summary();
        // Each scan finished on its own tree, whichever order they ran in.
        assert_eq!(summary.state, "completed");
        assert_eq!(summary.entries, 3);
        assert_eq!(s.store.read().unwrap().root, roots[i].1);
        assert!(s.workers >= 1 && s.workers <= total.workers);
        assert!(summary.descriptor_limit.unwrap() <= total.descriptors);
        assert!(summary.descriptor_peak.unwrap() <= summary.descriptor_limit.unwrap());
    }
    assert_eq!(state.scans.sessions().len(), 3);
    // Every share is back once the scans have ended.
    assert_eq!(state.scans.budget().free(), total);
}

#[test]
fn concurrent_scans_split_the_workers_and_descriptors_and_return_them_when_they_end() {
    let app = app(appstate_with_budget(4, 64));
    let state = app.state::<AppState>();
    let total = state.scans.budget().total();
    // Two scans already running hold 2 + 1 workers and 32 + 16 descriptors.
    let first = state.scans.budget().grant(2).unwrap();
    let second = state.scans.budget().grant(2).unwrap();
    let held = (first.share(), second.share());
    assert_eq!((held.0.workers, held.1.workers), (2, 1));
    assert_eq!((held.0.descriptors, held.1.descriptors), (32, 16));
    let (_dir, root) = disk_root();
    let (id, events) = start(&app, &register_disk(&state, &root)).unwrap();
    let s = state.scans.find(&id).unwrap();
    // The third scan got what was left, not its configured worker count.
    assert_eq!(s.workers, 1);
    assert_eq!(s.cached_validation_workers, 1);
    assert_eq!(s.descriptor_limit, Some(8));
    events.wait_end();
    let summary = s.summary();
    assert_eq!(summary.state, "completed");
    assert_eq!(summary.workers, 1);
    assert_eq!(summary.descriptor_limit, Some(8));
    // Only the two running scans still hold a share.
    let free = state.scans.budget().free();
    assert_eq!(
        free.workers,
        total.workers - held.0.workers - held.1.workers
    );
    assert_eq!(
        free.descriptors,
        total.descriptors - held.0.descriptors - held.1.descriptors
    );
    drop((first, second));
    assert_eq!(state.scans.budget().free(), total);
}

#[test]
fn a_second_disk_starts_beside_a_scan_that_holds_every_worker_of_a_four_thread_mac() {
    let app = app(appstate_with_budget(4, 512));
    let state = app.state::<AppState>();
    let total = state.scans.budget().total();
    // The first scan, still running, took all four workers.
    let first = state.scans.budget().grant(4).unwrap();
    assert_eq!(state.scans.budget().free().workers, 0);
    let (_dir, root) = disk_root();
    let (id, events) = start(&app, &register_disk(&state, &root)).unwrap();
    let s = state.scans.find(&id).unwrap();
    assert_eq!(s.workers, 1);
    events.wait_end();
    assert_eq!(s.summary().state, "completed");
    assert_eq!(s.summary().entries, 2);
    drop(first);
    assert_eq!(state.scans.budget().free(), total);
}

#[test]
fn a_start_with_no_descriptors_left_is_refused_and_replaces_nothing() {
    let app = app(appstate_with_budget(3, 16));
    let state = app.state::<AppState>();
    let (_dir, root) = disk_root();
    let target = register_disk(&state, &root);
    let old = install_in(&state, Slot::Disk(root.clone()), "old", snapshot(root));
    let holders = [
        state.scans.budget().grant(2).unwrap(),
        state.scans.budget().grant(2).unwrap(),
    ];
    assert_eq!(state.scans.budget().free().descriptors, 0);
    let error = start(&app, &target).unwrap_err();
    assert!(error.contains("every file"), "{error}");
    assert!(old.is_valid());
    assert!(state.scans.find("old").is_ok());
    // The refused start left no share behind; ending a scan makes room again.
    drop(holders);
    assert_eq!(state.scans.budget().free(), state.scans.budget().total());
    start(&app, &target).unwrap().1.wait_end();
    assert_eq!(state.scans.budget().free(), state.scans.budget().total());
}

#[test]
fn a_panicking_scan_thread_ends_its_session_as_failed_and_opens_the_gates_again() {
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let total = state.scans.budget().total();
    let (_a_dir, a_root) = disk_root();
    let (_b_dir, b_root) = disk_root();
    let a = register_disk(&state, &a_root);
    let other = install_in(&state, Slot::Disk(b_root.clone()), "b", snapshot(b_root));
    // The port panics on the scan thread's first event, as a panic anywhere in the scan would.
    let (id, events) = start_recorded(&state, &a, Recorder::panicking()).unwrap();
    // A final event was sent, and it says the scan failed.
    let end = events.wait_end();
    assert_eq!(end.state, "failed");
    let s = state.scans.find(&id).unwrap();
    let summary = s.summary();
    assert_eq!(summary.state, "failed");
    assert!(summary.incomplete);
    assert!(summary.message.unwrap().contains("stopped unexpectedly"));
    assert_eq!(summary.scan_phase, "done");
    // Its share is back, no gate stays closed, and nothing else was touched.
    assert_eq!(state.scans.budget().free(), total);
    assert!(!scan_running(&state));
    assert!(other.is_valid());
    // The slot starts again.
    let (again, events) = start(&app, &a).unwrap();
    assert_eq!(events.wait_end().state, "completed");
    assert_eq!(
        state.scans.find(&again).unwrap().summary().state,
        "completed"
    );
}

#[test]
fn a_poisoned_rescan_cache_does_not_stop_the_scans_that_follow() {
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let dir = tempfile::tempdir().unwrap();
    state.baselines.configure(dir.path().join("cache"));
    // A scan thread that panics while it holds the cache lock poisons it.
    state.baselines.poison();
    let (_a_dir, a_root) = disk_root();
    let (_b_dir, b_root) = disk_root();
    for root in [a_root, b_root] {
        let (id, events) = start(&app, &register_disk(&state, &root)).unwrap();
        events.wait_end();
        assert_eq!(state.scans.find(&id).unwrap().summary().state, "completed");
    }
}

#[test]
fn a_cancelled_scan_gives_its_share_back_and_leaves_other_scans_running() {
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let total = state.scans.budget().total();
    let (_a_dir, a_root) = disk_root();
    let (_b_dir, b_root) = disk_root();
    let other = install_in(&state, Slot::Disk(b_root.clone()), "b", snapshot(b_root));
    other.store.write().unwrap().state = ScanState::Running;
    let (id, events) = start(&app, &register_disk(&state, &a_root)).unwrap();
    cancel_scan(app.state(), id.clone()).unwrap();
    events.wait_end();
    assert!(matches!(
        state.scans.find(&id).unwrap().summary().state.as_str(),
        "cancelled" | "completed"
    ));
    assert_eq!(state.scans.budget().free(), total);
    // The other scan was not touched.
    assert!(!other.cancel.load(Ordering::Acquire));
    assert!(other.is_valid());
    assert!(session_running(&other));
}

#[test]
fn a_cancel_and_the_start_of_the_cache_write_are_ordered() {
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let (_a_dir, a_root) = disk_root();
    let (_b_dir, b_root) = disk_root();
    let writing = install_in(&state, Slot::Disk(a_root.clone()), "a", snapshot(a_root));
    let cancelled = install_in(&state, Slot::Disk(b_root.clone()), "b", snapshot(b_root));
    // Once the write began, a cancel is refused and does not flag the scan.
    assert!(writing.begin_caching());
    assert!(cancel_scan(app.state(), "a".into()).is_err());
    assert!(!writing.cancel.load(Ordering::Acquire));
    // A cancel that was accepted first stops the write before it starts.
    cancel_scan(app.state(), "b".into()).unwrap();
    assert!(!cancelled.begin_caching());
    assert_ne!(cancelled.scan_details.lock().unwrap().phase, "caching");
}

#[test]
fn folder_scans_share_one_slot_and_leave_disk_scans_alone() {
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let (_d_dir, d_root) = disk_root();
    let (_f_dir, f_root) = disk_root();
    let (_g_dir, g_root) = disk_root();
    let disk = install_in(&state, Slot::Disk(d_root.clone()), "disk", snapshot(d_root));
    let old_folder = install_in(&state, Slot::Folder, "folder", snapshot(f_root.clone()));
    let new_target = register(
        &state,
        platform::target(&g_root, "other folder".into()),
        g_root,
    )
    .unwrap();
    let new_folder = scan_to_end(&app, &new_target);
    assert!(!old_folder.is_valid());
    assert!(state.scans.find("folder").is_err());
    assert!(disk.is_valid());
    assert!(state.scans.find("disk").is_ok());
    assert!(state.scans.find(&new_folder).is_ok());
    assert_eq!(state.scans.sessions().len(), 2);
}

#[test]
fn discarding_a_scan_cancels_it_frees_its_slot_and_keeps_the_others() {
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let (_a_dir, a_root) = disk_root();
    let (_b_dir, b_root) = disk_root();
    let a = install_in(&state, Slot::Disk(a_root.clone()), "a", snapshot(a_root));
    let b = install_in(&state, Slot::Disk(b_root.clone()), "b", snapshot(b_root));
    b.store.write().unwrap().state = ScanState::Running;
    assert!(scan_running(&state));
    discard_scan(app.state(), "b".into());
    assert!(b.cancel.load(Ordering::Acquire));
    assert!(!b.is_valid());
    assert!(state
        .scans
        .find("b")
        .err()
        .unwrap()
        .contains("no longer active"));
    assert_eq!(state.scans.sessions().len(), 1);
    // Still winding down: it keeps counting as running until it ends.
    assert!(scan_running(&state));
    b.store.write().unwrap().state = ScanState::Cancelled;
    assert!(!scan_running(&state));
    assert!(a.is_valid());
    assert!(!a.cancel.load(Ordering::Acquire));
    // Discarding an unknown or already discarded scan is a no-op.
    discard_scan(app.state(), "b".into());
    discard_scan(app.state(), "missing".into());
    assert!(state.scans.find("a").is_ok());
    // A scan that already ended leaves nothing to drain.
    discard_scan(app.state(), "a".into());
    assert!(!scan_running(&state));
}

#[test]
fn a_discarded_scan_still_winding_down_keeps_its_budget_but_only_stops_the_global_operations() {
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let (_a_dir, a_root) = disk_root();
    let (_b_dir, b_root) = disk_root();
    let ejected = install_in(&state, Slot::Disk(a_root.clone()), "a", snapshot(a_root));
    ejected.store.write().unwrap().state = ScanState::Running;
    // The walk still holds its share of the budget.
    let held = state.scans.budget().grant(2).unwrap();
    let b = register_disk(&state, &b_root);
    discard_scan(app.state(), "a".into());
    assert!(state.scans.sessions().is_empty());
    // It does not stop a scan on another slot.
    let (_, events) = start(&app, &b).unwrap();
    assert_eq!(state.scans.sessions().len(), 1);
    events.wait_end();
    assert!(state.scans.budget().free().workers < state.scans.budget().total().workers);
    // Setup still waits for it.
    assert!(setup_idle(&state).is_err());
    assert!(scan_running(&state));
    // Once the walk ends, setup goes ahead and its share is returned.
    ejected.store.write().unwrap().state = ScanState::Cancelled;
    drop(held);
    assert!(!scan_running(&state));
    assert!(setup_idle(&state).is_ok());
    assert_eq!(state.scans.budget().free(), state.scans.budget().total());
}

#[test]
fn setup_operations_refuse_while_any_scan_runs_on_any_disk() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        force_access(&state, access_setup::Status::On);
        let (_a_dir, a_root) = disk_root();
        let (_b_dir, b_root) = disk_root();
        install_in(&state, Slot::Disk(a_root.clone()), "a", snapshot(a_root));
        let running = install_in(&state, Slot::Disk(b_root.clone()), "b", snapshot(b_root));
        running.store.write().unwrap().state = ScanState::Running;
        // A finished scan on one disk does not make the others idle.
        assert!(setup_idle(&state).is_err());
        assert!(restart_app(app.handle().clone(), app.state())
            .await
            .unwrap_err()
            .contains("scan"));
        running.store.write().unwrap().state = ScanState::Completed;
        assert!(setup_idle(&state).is_ok());
    });
}

#[test]
fn scan_starts_share_the_activity_gate_and_exclusive_operations_exclude_them() {
    let gate = Arc::new(ActivityGate::default());
    let first = Activity::begin_shared(&gate).unwrap();
    let second = Activity::begin_shared(&gate).unwrap();
    assert!(gate.busy());
    assert!(Activity::begin(&gate).is_err());
    drop(first);
    assert!(Activity::begin(&gate).is_err());
    drop(second);
    assert!(!gate.busy());
    let exclusive = Activity::begin(&gate).unwrap();
    assert!(Activity::begin(&gate).is_err());
    assert!(Activity::begin_shared(&gate).is_err());
    drop(exclusive);
    assert!(Activity::begin_shared(&gate).is_ok());
    assert!(!gate.busy());
}

#[test]
fn a_start_stalled_reading_its_target_does_not_hold_the_activity_gate() {
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let (_dir, root) = disk_root();
    let target = register_disk(&state, &root);
    let handle = app.handle().clone();
    std::thread::scope(|scope| {
        // Stalls the start where it looks the target up, as an unresponsive mount stalls validation. Dropped on
        // a failure too.
        let stall = state.targets.lock().unwrap();
        let starting = scope.spawn(|| {
            tauri::async_runtime::block_on(start_scan(
                handle.state(),
                target.id.clone(),
                Channel::new(|_| Ok(())),
                handle.clone(),
            ))
        });
        std::thread::sleep(Duration::from_millis(200));
        assert!(!starting.is_finished(), "the start did not stall");
        // Cleanup, queue edits and restart are not refused for as long as the disk stalls.
        drop(Activity::begin(&state.activity).expect("a stalled start holds the activity gate"));
        drop(stall);
        let id = starting.join().unwrap().unwrap();
        while scan_running(&state) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(state.scans.find(&id).is_ok());
        assert!(!state.activity.busy());
    });
}

#[test]
fn a_queue_scan_of_a_disk_root_gets_a_disk_target_and_a_folder_does_not() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let guard = |path: &str| cleanup_queue::Guard {
            path: path.into(),
            identity: safety::Identity::read(Path::new(path)).unwrap(),
        };
        // An entry recorded under the startup disk: its rescan must use that disk's slot.
        state
            .cleanup_queue
            .lock()
            .unwrap()
            .add(vec![cleanup_queue::Candidate {
                id: uuid::Uuid::new_v4().to_string(),
                item: guard("/usr"),
                ancestors: vec![guard("/")],
                root: guard("/"),
                root_ancestors: vec![],
                allocated_bytes: 1,
                last_error: None,
                volume: None,
            }])
            .unwrap();
        let entry = state.cleanup_queue.lock().unwrap().entries[0].id.clone();
        let target = cleanup_queue_target(app.state(), entry).await.unwrap();
        assert_eq!(target.path, "/");
        assert!(target.volume);
        assert!(!target.home);
        assert!(state.targets.lock().unwrap()[&target.id].0.volume);
    });
}

#[test]
fn a_queue_scan_target_reads_the_disk_off_the_activity_gate() {
    let app = app(AppState::default());
    let state = app.state::<AppState>();
    let guard = |path: &str| cleanup_queue::Guard {
        path: path.into(),
        identity: safety::Identity::read(Path::new(path)).unwrap(),
    };
    state
        .cleanup_queue
        .lock()
        .unwrap()
        .add(vec![cleanup_queue::Candidate {
            id: uuid::Uuid::new_v4().to_string(),
            item: guard("/usr"),
            ancestors: vec![guard("/")],
            root: guard("/"),
            root_ancestors: vec![],
            allocated_bytes: 1,
            last_error: None,
            volume: None,
        }])
        .unwrap();
    let entry = state.cleanup_queue.lock().unwrap().entries[0].id.clone();
    let handle = app.handle().clone();
    std::thread::scope(|scope| {
        // Stalls the command where it looks the entry up, as an unresponsive mount stalls what follows. Dropped
        // on a failure too.
        let stall = state.cleanup_queue.lock().unwrap();
        let targeting = scope
            .spawn(|| tauri::async_runtime::block_on(cleanup_queue_target(handle.state(), entry)));
        std::thread::sleep(Duration::from_millis(200));
        assert!(!targeting.is_finished(), "the command did not stall");
        drop(Activity::begin(&state.activity).expect("a stalled command holds the activity gate"));
        drop(stall);
        assert_eq!(targeting.join().unwrap().unwrap().path, "/");
        assert!(!state.activity.busy());
    });
}

#[test]
fn a_cleanup_invalidates_only_the_session_it_cleaned() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let (_a_dir, a_root) = disk_root();
        let (_b_dir, b_root) = disk_root();
        let scan = snapshot(a_root.clone());
        let ids: Vec<_> = scan
            .iter_nodes()
            .filter(|n| n.parent == Some(0))
            .map(|n| n.id)
            .collect();
        let a = install_in(&state, Slot::Disk(a_root.clone()), "a", scan);
        let b = install_in(&state, Slot::Disk(b_root.clone()), "b", snapshot(b_root));
        let review = prepare_trash(app.state(), "a".into(), ids).await.unwrap();
        let mut trash = Recording::ok();
        let result = cleanup::commit(&a, &state.cleanup_queue, &review.token, false, |i| {
            trash.trash(i)
        })
        .unwrap();
        assert_eq!(result.trashed, 1);
        assert!(!a.is_valid());
        assert!(b.is_valid());
        assert!(state
            .scans
            .find("a")
            .err()
            .unwrap()
            .contains(slots::RESULTS_CHANGED));
        assert!(state.scans.find("b").is_ok());
    });
}

// A folder `D/sub` with one file `D/sub/file`, inside a "disk" `D`, plus an unrelated folder to replace the folder slot with.
struct Nested {
    _dirs: Vec<tempfile::TempDir>,
    disk: PathBuf,
    folder: PathBuf,
    item: PathBuf,
    other: PathBuf,
}
fn nested() -> Nested {
    let (d, disk) = disk_root();
    let (o, other) = disk_root();
    let folder = disk.join("sub");
    std::fs::create_dir(&folder).unwrap();
    let item = folder.join("file");
    std::fs::write(&item, b"abc").unwrap();
    Nested {
        _dirs: vec![d, o],
        disk,
        folder,
        item,
        other,
    }
}
fn node_of(scan: &Snapshot, path: &std::path::Path) -> u32 {
    scan.iter_nodes()
        .find(|n| n.path().unwrap() == path)
        .unwrap()
        .id
}
fn queue_in_tempdir(state: &AppState) -> tempfile::TempDir {
    let storage = fixture();
    *state.cleanup_queue.lock().unwrap() = cleanup_queue::Queue::load(storage.path().join("queue"));
    storage
}
async fn first_entry(app: &tauri::App<tauri::test::MockRuntime>) -> cleanup_queue::EntryView {
    list_cleanup_queue(app.state())
        .await
        .unwrap()
        .entries
        .remove(0)
}

// The queue rules take scans as plain data, so these tests need no app, no `Session` and no live slots: every
// scan listed is one the caller found usable. The commands' own state filter is covered further down.
struct Held {
    id: &'static str,
    disk: bool,
    snapshot: Snapshot,
    root_ancestors: Vec<(PathBuf, safety::Identity)>,
    volumes: HashMap<u32, safety::Volume>,
}
fn held(id: &'static str, disk: bool, snapshot: Snapshot) -> Held {
    Held {
        id,
        disk,
        root_ancestors: safety::ancestors(&snapshot.root).unwrap_or_default(),
        volumes: scan_volumes(&snapshot),
        snapshot,
    }
}
fn completed_empty(root: &str) -> Snapshot {
    let mut scan = Snapshot::empty(root.into());
    scan.state = ScanState::Completed;
    scan
}
impl Held {
    fn scan(&self) -> safety::Scan<'_> {
        safety::Scan {
            id: self.id,
            disk: self.disk,
            snapshot: &self.snapshot,
            root_ancestors: &self.root_ancestors,
            volumes: Some(&self.volumes),
        }
    }
}
fn scans<'a>(held: &[&'a Held]) -> Vec<safety::Scan<'a>> {
    held.iter().map(|h| h.scan()).collect()
}
/// Queues nodes of `from` as `add_cleanup_queue` does, minus the live-session checks.
fn queue_nodes(queue: &mut cleanup_queue::Queue, from: &Held, paths: &[&std::path::Path]) {
    let scan = from.scan();
    let ids: Vec<_> = paths.iter().map(|p| node_of(&from.snapshot, p)).collect();
    let items: Vec<_> = ids
        .iter()
        .map(|&id| safety::checked_item(&scan, id).unwrap())
        .collect();
    queue
        .add(cleanup_queue::candidates(&scan, items, &ids).unwrap())
        .unwrap();
}
fn entries(queue: &cleanup_queue::Queue, against: &[&Held]) -> Vec<cleanup_queue::EntryView> {
    let statuses = cleanup_queue::statuses(queue, &scans(against), None, &[]);
    cleanup_queue::view(queue, &statuses).entries
}
/// The first entry's status and the scan it is matched to.
fn standing(queue: &cleanup_queue::Queue, against: &[&Held]) -> (String, Option<String>) {
    let entry = entries(queue, against).remove(0);
    (entry.status, entry.matched_scan_id)
}
fn is(status: &str, scan: Option<&str>) -> (String, Option<String>) {
    (status.into(), scan.map(Into::into))
}
/// The nodes a review of scan `id` holds.
fn review_of(queue: &cleanup_queue::Queue, against: &[&Held], id: &str) -> Vec<u32> {
    let statuses = cleanup_queue::statuses(queue, &scans(against), None, &[]);
    cleanup_queue::ready_nodes(queue, &statuses, id)
}
fn queue_of(storage: &tempfile::TempDir) -> cleanup_queue::Queue {
    cleanup_queue::Queue::load(storage.path().join("queue"))
}
/// A queue entry recorded from a scan of `/Volumes/X/dir`, with made-up identities.
fn recorded_entry(item: &str, item_kind: u8) -> cleanup_queue::Candidate {
    let guard = |path: &str, kind: u8| cleanup_queue::Guard {
        path: path.into(),
        identity: safety::Identity {
            device: 7,
            inode: 7,
            kind,
            birth: None,
        },
    };
    cleanup_queue::Candidate {
        id: uuid::Uuid::new_v4().to_string(),
        item: guard(item, item_kind),
        ancestors: vec![
            guard("/Volumes/X/dir", 1),
            guard("/Volumes/X", 1),
            guard("/Volumes", 1),
            guard("/", 1),
        ],
        root: guard("/Volumes/X/dir", 1),
        root_ancestors: vec![guard("/Volumes/X", 1), guard("/Volumes", 1), guard("/", 1)],
        allocated_bytes: 0,
        last_error: None,
        volume: None,
    }
}

#[test]
fn queue_entries_follow_the_scan_of_their_own_disk() {
    let storage = fixture();
    let mut queue = queue_of(&storage);
    let (_a_dir, a_root) = disk_root();
    let (_b_dir, b_root) = disk_root();
    let a = held("a", true, snapshot(a_root.clone()));
    let b = held("b", true, snapshot(b_root.clone()));
    queue_nodes(&mut queue, &a, &[&a_root.join("file")]);
    queue_nodes(&mut queue, &b, &[&b_root.join("file")]);
    let status = |queue: &cleanup_queue::Queue, against: &[&Held], root: &PathBuf| {
        entries(queue, against)
            .into_iter()
            .find(|e| e.root_path == root.to_string_lossy())
            .map(|e| e.status)
    };
    // Both disks are scanned, whichever started last.
    for both in [[&a, &b], [&b, &a]] {
        assert_eq!(status(&queue, &both, &a_root), Some("ready".into()));
        assert_eq!(status(&queue, &both, &b_root), Some("ready".into()));
    }
    // A review covers the scan it is addressed to; the other disk's entries wait.
    let ready = review_of(&queue, &[&a, &b], "a");
    assert_eq!(ready.len(), 1);
    assert_eq!(a.snapshot.path(ready[0]).unwrap(), a_root.join("file"));
    assert_eq!(review_of(&queue, &[&a, &b], "b").len(), 1);
    // A disk without a usable scan leaves only its own entries needing a scan.
    assert_eq!(status(&queue, &[&a], &a_root), Some("ready".into()));
    assert_eq!(status(&queue, &[&a], &b_root), Some("needsScan".into()));
    assert!(review_of(&queue, &[&a], "b").is_empty());
    // A rescan of A is another scan: the entry follows it, and a review is addressed to it.
    let a2 = held("a2", true, snapshot(a_root.clone()));
    assert_eq!(standing(&queue, &[&a2]).1.as_deref(), Some("a2"));
    assert_eq!(status(&queue, &[&a2, &b], &a_root), Some("ready".into()));
    assert!(review_of(&queue, &[&a2, &b], "a").is_empty());
    assert_eq!(review_of(&queue, &[&a2, &b], "a2").len(), 1);
    // Once A's entry is gone (cleaned), B's stays ready.
    queue.prune(&[a_root.join("file")]).unwrap();
    assert_eq!(status(&queue, &[&a2, &b], &a_root), None);
    assert_eq!(status(&queue, &[&a2, &b], &b_root), Some("ready".into()));
}

#[test]
fn a_disk_scan_wins_over_a_folder_scan_of_the_same_root() {
    let storage = fixture();
    let mut queue = queue_of(&storage);
    let dir = fixture();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    // The folder scan ran before the file existed, so it cannot see it.
    let stale = held("folder", false, snapshot(root.clone()));
    std::fs::write(root.join("file"), b"abc").unwrap();
    let fresh = held("disk", true, snapshot(root.clone()));
    queue_nodes(&mut queue, &fresh, &[&root.join("file")]);
    // The folder scan also sees the root, but the disk's scan decides, whichever is listed first.
    assert_eq!(
        standing(&queue, &[&stale, &fresh]),
        is("ready", Some("disk"))
    );
    assert_eq!(
        standing(&queue, &[&fresh, &stale]),
        is("ready", Some("disk"))
    );
    // Without the disk scan, the folder scan is the match and does not hold the item.
    assert_eq!(standing(&queue, &[&stale]), is("missing", Some("folder")));
}

#[test]
fn the_fallback_needs_a_usable_disk_scan_that_contains_the_item() {
    let storage = fixture();
    let mut queue = queue_of(&storage);
    let n = nested();
    let folder = held("folder", false, snapshot(n.folder.clone()));
    queue_nodes(&mut queue, &folder, &[&n.item]);
    assert_eq!(standing(&queue, &[&folder]), is("ready", Some("folder")));
    // A folder scan of the disk's root is not a disk scan: it never vouches for another root.
    let wide = held("wide", false, snapshot(n.disk.clone()));
    assert_eq!(standing(&queue, &[&wide]), is("needsScan", None));
    // A disk scan elsewhere does not contain the item.
    let elsewhere = held("elsewhere", true, snapshot(n.other.clone()));
    assert_eq!(
        standing(&queue, &[&wide, &elsewhere]),
        is("needsScan", None)
    );
    // A completed disk scan that does not hold the item leaves the entry needing a scan.
    let bare = held("bare", true, completed_empty(n.disk.to_str().unwrap()));
    assert_eq!(
        standing(&queue, &[&wide, &elsewhere, &bare]),
        is("needsScan", None)
    );
    // A disk scan that holds it does.
    let disk = held("disk", true, snapshot(n.disk.clone()));
    assert_eq!(
        standing(&queue, &[&wide, &elsewhere, &disk]),
        is("ready", Some("disk"))
    );
}

#[test]
fn the_disk_with_the_longest_root_containing_the_item_decides() {
    let storage = fixture();
    let mut queue = queue_of(&storage);
    let n = nested();
    let deep = n.folder.join("deep");
    std::fs::create_dir(&deep).unwrap();
    std::fs::write(deep.join("file"), b"abc").unwrap();
    let folder = held("folder", false, snapshot(deep.clone()));
    queue_nodes(&mut queue, &folder, &[&deep.join("file")]);
    let other = held("other", false, snapshot(n.other.clone()));
    let outer = held("outer", true, snapshot(n.disk.clone()));
    let inner = held("inner", true, snapshot(n.folder.clone()));
    for all in [[&other, &outer, &inner], [&inner, &outer, &other]] {
        assert_eq!(standing(&queue, &all), is("ready", Some("inner")));
        // Only the matched scan reviews the entry.
        assert!(review_of(&queue, &all, "outer").is_empty());
        assert_eq!(review_of(&queue, &all, "inner").len(), 1);
    }
    // A scan that cannot vouch is not offered, and the next wider disk takes over.
    assert_eq!(
        standing(&queue, &[&other, &outer]),
        is("ready", Some("outer"))
    );
}

#[test]
fn an_item_on_another_mount_is_never_matched_by_the_startup_disk() {
    let storage = fixture();
    let mut queue = queue_of(&storage);
    // A queued item under /Volumes/X, recorded from a scan of /Volumes/X/dir.
    queue
        .add(vec![recorded_entry("/Volumes/X/dir/file", 2)])
        .unwrap();
    // A finished scan of "/" never descends into /Volumes (disk-scan `traversal_allowed`), so it holds
    // no node for the item and cannot vouch for it.
    let startup = held("startup", true, completed_empty("/"));
    assert_eq!(standing(&queue, &[&startup]), is("needsScan", None));
    // Same for a finished scan of the volume that lacks the item.
    let volume = held("volume", true, completed_empty("/Volumes/X"));
    assert_eq!(
        standing(&queue, &[&startup, &volume]),
        is("needsScan", None)
    );
}

#[test]
fn a_disk_scan_that_disagrees_with_the_record_is_stale_not_a_verdict_but_an_own_root_scan_is() {
    let storage = fixture();
    let mut queue = queue_of(&storage);
    let n = nested();
    let folder = held("folder", false, snapshot(n.folder.clone()));
    queue_nodes(&mut queue, &folder, &[&n.item]);
    let other = held("other", false, snapshot(n.other.clone()));
    let disk = held("disk", true, snapshot(n.disk.clone()));
    assert_eq!(
        standing(&queue, &[&other, &disk]),
        is("ready", Some("disk"))
    );
    let original = queue.entries[0].clone();
    // The record holds a different file than the disk scan has at that path.
    queue.entries[0].item.identity.inode += 1;
    assert_eq!(standing(&queue, &[&other, &disk]), is("needsScan", None));
    assert!(review_of(&queue, &[&other, &disk], "disk").is_empty());
    // The queued folder was a different directory when recorded: it is part of the recorded ancestors.
    queue.entries[0] = original;
    queue.entries[0].ancestors[0].identity.inode += 1;
    assert_eq!(standing(&queue, &[&other, &disk]), is("needsScan", None));
    // The queued folder's own scan reports the same disagreement as "changed".
    let folder2 = held("folder2", false, snapshot(n.folder.clone()));
    assert_eq!(
        standing(&queue, &[&folder2, &disk]),
        is("changed", Some("folder2"))
    );
}

#[test]
fn a_covering_disk_scan_cannot_review_an_item_replaced_on_disk_since_that_scan() {
    let storage = fixture();
    let mut queue = queue_of(&storage);
    let n = nested();
    let folder = held("folder", false, snapshot(n.folder.clone()));
    queue_nodes(&mut queue, &folder, &[&n.item]);
    let other = held("other", false, snapshot(n.other.clone()));
    let disk = held("disk", true, snapshot(n.disk.clone()));
    assert_eq!(
        standing(&queue, &[&other, &disk]),
        is("ready", Some("disk"))
    );
    // Replace the file on disk without rescanning: the scan and the record still agree, the disk does not.
    std::fs::rename(&n.item, n.folder.join("moved-away")).unwrap();
    std::fs::write(&n.item, b"new file").unwrap();
    assert_eq!(
        standing(&queue, &[&other, &disk]),
        is("blocked", Some("disk"))
    );
    assert!(review_of(&queue, &[&other, &disk], "disk").is_empty());
}

#[test]
fn an_excluded_mount_in_a_disk_scan_does_not_vouch_for_a_queued_item() {
    let storage = fixture();
    let mut queue = queue_of(&storage);
    // Queued from a scan of /Volumes/X/dir: /Volumes/X/dir/mount is a directory inside it.
    queue
        .add(vec![recorded_entry("/Volumes/X/dir/mount", 1)])
        .unwrap();
    // The volume's scan holds that path only as an excluded node (a mount point it did not enter).
    let mut volume = Snapshot::empty("/Volumes/X".into());
    let node = |id: u32,
                parent: Option<u32>,
                children: Vec<u32>,
                name: &str,
                path: &str,
                excluded: bool| Node {
        id,
        parent,
        children,
        name: name.into(),
        path: path.into(),
        identity: disk_scan::Identity {
            device: 7,
            inode: 7,
            kind: NodeKind::Directory,
        },
        allocated_size: 0,
        length: 0,
        own_allocated_size: 0,
        is_directory: true,
        hard_link: false,
        clone_metadata: None,
        excluded,
    };
    volume
        .append_legacy_node(node(0, None, vec![1], "X", "/Volumes/X", false))
        .unwrap();
    volume
        .append_legacy_node(node(1, Some(0), vec![2], "dir", "/Volumes/X/dir", false))
        .unwrap();
    volume
        .append_legacy_node(node(
            2,
            Some(1),
            vec![],
            "mount",
            "/Volumes/X/dir/mount",
            true,
        ))
        .unwrap();
    volume.state = ScanState::Completed;
    let volume = held("volume", true, volume);
    // Not "blocked": the scan simply never covered it.
    assert_eq!(standing(&queue, &[&volume]), is("needsScan", None));
}

// What the commands add to the rules above: only a Live, valid, Completed, not running scan is offered to them.
#[test]
fn only_a_live_valid_completed_scan_can_vouch_for_a_queue_entry() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let _storage = queue_in_tempdir(&state);
        let n = nested();
        let folder_scan = snapshot(n.folder.clone());
        let id = node_of(&folder_scan, &n.item);
        install_in(&state, Slot::Folder, "folder", folder_scan);
        add_cleanup_queue(app.state(), "folder".into(), vec![id])
            .await
            .unwrap();
        // The folder's own scan is replaced, so only a covering disk scan can vouch.
        install_in(&state, Slot::Folder, "other", snapshot(n.other.clone()));
        let disk = install_in(
            &state,
            Slot::Disk(n.disk.clone()),
            "disk",
            snapshot(n.disk.clone()),
        );
        assert_eq!(first_entry(&app).await.status, "ready");
        // A disk scan that holds it, but is running, cancelled, failed, finishing or invalidated, does not match.
        for scan_state in [ScanState::Running, ScanState::Cancelled, ScanState::Failed] {
            disk.store.write().unwrap().state = scan_state;
            assert_eq!(first_entry(&app).await.status, "needsScan");
        }
        disk.store.write().unwrap().state = ScanState::Completed;
        assert_eq!(first_entry(&app).await.status, "ready");
        disk.finishing.store(true, Ordering::Release);
        assert_eq!(first_entry(&app).await.status, "needsScan");
        disk.finishing.store(false, Ordering::Release);
        assert_eq!(first_entry(&app).await.status, "ready");
        disk.invalidate();
        assert_eq!(first_entry(&app).await.status, "needsScan");
        // A rescan puts a fresh scan in the slot; a discarded scan is gone from the live slots.
        install_in(
            &state,
            Slot::Disk(n.disk.clone()),
            "disk",
            snapshot(n.disk.clone()),
        );
        assert_eq!(first_entry(&app).await.status, "ready");
        discard_scan(app.state(), "disk".into());
        assert_eq!(first_entry(&app).await.status, "needsScan");
    });
}

#[test]
fn a_cleanup_of_one_disk_and_the_state_of_its_scan_leave_the_other_disks_entries_alone() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let storage = fixture();
        *state.cleanup_queue.lock().unwrap() =
            cleanup_queue::Queue::load(storage.path().join("queue"));
        let (_a_dir, a_root) = disk_root();
        let (_b_dir, b_root) = disk_root();
        let file_id = |scan: &Snapshot| scan.iter_nodes().find(|n| n.parent == Some(0)).unwrap().id;
        let scan_a = snapshot(a_root.clone());
        let scan_b = snapshot(b_root.clone());
        let (id_a, id_b) = (file_id(&scan_a), file_id(&scan_b));
        let a = install_in(&state, Slot::Disk(a_root.clone()), "a", scan_a);
        let b = install_in(&state, Slot::Disk(b_root.clone()), "b", scan_b);
        add_cleanup_queue(app.state(), "a".into(), vec![id_a])
            .await
            .unwrap();
        add_cleanup_queue(app.state(), "b".into(), vec![id_b])
            .await
            .unwrap();
        let statuses = || async {
            let view = list_cleanup_queue(app.state()).await.unwrap();
            let status = |root: &PathBuf| {
                view.entries
                    .iter()
                    .find(|e| e.root_path == root.to_string_lossy())
                    .map(|e| e.status.clone())
            };
            (status(&a_root), status(&b_root))
        };
        // Both disks are scanned, whichever started last.
        assert_eq!(
            statuses().await,
            (Some("ready".into()), Some("ready".into()))
        );
        // A review covers the scan it is addressed to; the other disk's entries wait.
        let review = prepare_cleanup_queue(app.state(), "a".into())
            .await
            .unwrap();
        assert_eq!(review.items.len(), 1);
        assert_eq!(review.items[0].path, a_root.join("file").to_string_lossy());
        assert!(b.ticket.lock().unwrap().is_none());
        // A real cleanup of disk A (native trash injected) invalidates A's scan, drops A's entry
        // and leaves B's entry ready.
        let mut trash = Recording::ok();
        let result = cleanup::commit(&a, &state.cleanup_queue, &review.token, true, |i| {
            trash.trash(i)
        })
        .unwrap();
        assert_eq!(result.trashed, 1);
        assert!(!a.is_valid());
        assert!(b.is_valid());
        assert_eq!(statuses().await, (None, Some("ready".into())));
        // The fixture file is still on disk, so A can be rescanned and queued again.
        let rescan = snapshot(a_root.clone());
        let id_a = file_id(&rescan);
        install_in(&state, Slot::Disk(a_root.clone()), "a2", rescan);
        add_cleanup_queue(app.state(), "a2".into(), vec![id_a])
            .await
            .unwrap();
        assert_eq!(
            statuses().await,
            (Some("ready".into()), Some("ready".into()))
        );
        // A scan that is running, cancelled, failed or discarded leaves only its own entries needing a scan.
        for scan_state in [ScanState::Running, ScanState::Cancelled, ScanState::Failed] {
            b.store.write().unwrap().state = scan_state;
            assert_eq!(
                statuses().await,
                (Some("ready".into()), Some("needsScan".into()))
            );
        }
        b.store.write().unwrap().state = ScanState::Completed;
        discard_scan(app.state(), "b".into());
        assert_eq!(
            statuses().await,
            (Some("ready".into()), Some("needsScan".into()))
        );
        // A review for A never includes B's entry.
        let review = prepare_cleanup_queue(app.state(), "a2".into())
            .await
            .unwrap();
        assert_eq!(review.items.len(), 1);
    });
}

#[test]
fn a_folder_queued_item_is_ready_through_a_scan_of_its_disk_and_cleaned_through_that_scan_only() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let _storage = queue_in_tempdir(&state);
        let n = nested();
        let folder_scan = snapshot(n.folder.clone());
        let id = node_of(&folder_scan, &n.item);
        install_in(&state, Slot::Folder, "folder", folder_scan);
        add_cleanup_queue(app.state(), "folder".into(), vec![id])
            .await
            .unwrap();
        // Another disk with its own entry, which must stay untouched.
        let (_e_dir, e_root) = disk_root();
        let e_scan = snapshot(e_root.clone());
        let e_id = node_of(&e_scan, &e_root.join("file"));
        let e = install_in(&state, Slot::Disk(e_root.clone()), "e", e_scan);
        add_cleanup_queue(app.state(), "e".into(), vec![e_id])
            .await
            .unwrap();
        // The disk is scanned too, but the folder's own scan is the exact match.
        let disk = install_in(
            &state,
            Slot::Disk(n.disk.clone()),
            "disk",
            snapshot(n.disk.clone()),
        );
        let entry = first_entry(&app).await;
        assert_eq!(
            (entry.status.as_str(), entry.matched_scan_id.as_deref()),
            ("ready", Some("folder"))
        );
        // The folder slot is replaced: the disk scan now covers the item.
        install_in(&state, Slot::Folder, "other", snapshot(n.other.clone()));
        let entry = first_entry(&app).await;
        assert_eq!(
            (entry.status.as_str(), entry.matched_scan_id.as_deref()),
            ("ready", Some("disk"))
        );
        // Review from the unrelated folder scan has nothing; review from the disk scan holds the item.
        assert!(prepare_cleanup_queue(app.state(), "other".into())
            .await
            .is_err());
        let review = prepare_cleanup_queue(app.state(), "disk".into())
            .await
            .unwrap();
        assert_eq!(review.items.len(), 1);
        assert_eq!(review.items[0].path, n.item.to_string_lossy());
        // Commit through the disk scan (native trash injected): the entry goes, only that scan is invalidated.
        let mut trash = Recording::ok();
        let result = cleanup::commit(&disk, &state.cleanup_queue, &review.token, true, |i| {
            trash.trash(i)
        })
        .unwrap();
        assert_eq!(result.trashed, 1);
        assert_eq!(trash.calls, vec![n.item.clone()]);
        assert!(!disk.is_valid());
        assert!(e.is_valid());
        assert!(state.scans.find("other").is_ok());
        let view = list_cleanup_queue(app.state()).await.unwrap();
        assert_eq!(view.entries.len(), 1);
        assert_eq!(view.entries[0].root_path, e_root.to_string_lossy());
        assert_eq!(view.entries[0].status, "ready");
    });
}

#[test]
fn a_reformatted_volume_with_the_same_identities_is_never_ready_or_trashed() {
    tauri::async_runtime::block_on(async {
        let app = app(AppState::default());
        let state = app.state::<AppState>();
        let _storage = queue_in_tempdir(&state);
        let n = nested();
        let folder_scan = snapshot(n.folder.clone());
        let id = node_of(&folder_scan, &n.item);
        install_in(&state, Slot::Folder, "folder", folder_scan);
        add_cleanup_queue(app.state(), "folder".into(), vec![id])
            .await
            .unwrap();
        assert_eq!(first_entry(&app).await.status, "ready");
        // Same paths, devices and inodes (FAT IDs come from layout), but a new volume UUID after a reformat.
        let other = || Some(safety::Volume::Uuid(uuid::Uuid::new_v4().to_string()));
        install_on(
            &state,
            Slot::Folder,
            "own",
            snapshot(n.folder.clone()),
            other(),
        );
        assert_eq!(first_entry(&app).await.status, "changed");
        // The covering-disk fallback does not vouch for it either.
        install_in(&state, Slot::Folder, "away", snapshot(n.other.clone()));
        install_on(
            &state,
            Slot::Disk(n.disk.clone()),
            "disk",
            snapshot(n.disk.clone()),
            other(),
        );
        assert_eq!(first_entry(&app).await.status, "needsScan");
        // An entry queued by an older version (no recorded volume) fails closed under both rules.
        state.cleanup_queue.lock().unwrap().entries[0].volume = None;
        install_in(
            &state,
            Slot::Disk(n.disk.clone()),
            "disk2",
            snapshot(n.disk.clone()),
        );
        assert_eq!(first_entry(&app).await.status, "needsScan");
        install_in(&state, Slot::Folder, "own2", snapshot(n.folder.clone()));
        assert_eq!(first_entry(&app).await.status, "changed");
        // Before Trash, the item's volume is read again: a ticket recorded on another volume trashes nothing.
        let scan = snapshot(n.folder.clone());
        let id = node_of(&scan, &n.item);
        let own = install_in(&state, Slot::Folder, "own3", scan);
        let review = prepare_trash(app.state(), "own3".into(), vec![id])
            .await
            .unwrap();
        own.ticket.lock().unwrap().as_mut().unwrap().items[0].volume = other();
        let mut trash = Recording::ok();
        let result = cleanup::commit(&own, &state.cleanup_queue, &review.token, false, |i| {
            trash.trash(i)
        });
        assert_eq!(
            result.err().as_deref(),
            Some("The volume at this location changed. Rescan before cleanup.")
        );
        assert!(trash.calls.is_empty());
        assert!(own.is_valid());
        assert!(n.item.exists());
    });
}

#[test]
fn a_firmlink_starts_its_own_volume_region_within_one_device() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let firmlink = root.join("Users");
    std::fs::create_dir_all(firmlink.join("me")).unwrap();
    std::fs::write(firmlink.join("me/big.bin"), b"x").unwrap();
    let scan = snapshot(root);
    let alias = node_of(&scan, &firmlink);
    // Same device throughout: only the scan root starts a region until the firmlink is named.
    assert_eq!(volume_regions(&scan, &[]), vec![0]);
    let outside = PathBuf::from("/not/in/this/scan");
    let regions = volume_regions(&scan, &[firmlink.clone(), outside]);
    assert_eq!(regions, vec![0, alias]);
    // An item takes its nearest region's volume, not the scan root's.
    let volumes: HashMap<_, _> = regions
        .iter()
        .map(|&id| (id, safety::Volume::Uuid(id.to_string())))
        .collect();
    let view = safety::Scan {
        id: "scan",
        disk: false,
        snapshot: &scan,
        root_ancestors: &[],
        volumes: Some(&volumes),
    };
    let item = node_of(&scan, &firmlink.join("me/big.bin"));
    assert_eq!(
        safety::checked_item(&view, item).unwrap().volume,
        Some(safety::Volume::Uuid(alias.to_string()))
    );
    assert_eq!(
        safety::checked_item(&view, 0).unwrap().volume,
        Some(safety::Volume::Uuid("0".into()))
    );
}
