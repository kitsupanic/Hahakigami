//! Explicitly compiled benchmark harness. Never enabled by normal desktop builds.
//! All persistence belongs to a caller-owned disposable directory; no OS grants are changed.
use super::*;
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufWriter, Write},
    path::Path,
    sync::{atomic::AtomicBool, OnceLock},
    time::Duration,
};
use tauri::{Emitter, Listener};

#[derive(Clone, Deserialize)]
pub struct Config {
    pub schema_version: u32,
    pub root: PathBuf,
    pub scope: String,
    pub mode: String,
    pub state_dir: PathBuf,
    pub coverage_output: Option<PathBuf>,
    #[serde(default)]
    pub interactions: bool,
    #[serde(default)]
    pub settle_ms: u64,
}
static CONFIG: OnceLock<Config> = OnceLock::new();
static LAUNCHED: OnceLock<Instant> = OnceLock::new();
static LAUNCH_MS: OnceLock<f64> = OnceLock::new();
static BEGAN: Mutex<Option<Instant>> = Mutex::new(None);
static REPORTED: AtomicBool = AtomicBool::new(false);
static RENDER_PASS: Mutex<RenderPass> = Mutex::new(RenderPass {
    seed_id: None,
    preparation_ms: None,
});

#[derive(Default)]
struct RenderPass {
    seed_id: Option<String>,
    preparation_ms: Option<f64>,
}

#[derive(Debug, PartialEq)]
enum RenderAction {
    Prepare,
    Ignore,
    Report,
}

impl RenderPass {
    fn is_seed(&self, id: &str) -> bool {
        self.seed_id.as_deref() == Some(id)
    }

    fn completed(&mut self, mode: &str, id: &str, elapsed_ms: f64) -> RenderAction {
        if !["same_process", "changed"].contains(&mode) {
            return RenderAction::Report;
        }
        if self.is_seed(id) {
            return RenderAction::Ignore;
        }
        if self.seed_id.is_none() {
            self.seed_id = Some(id.into());
            self.preparation_ms = Some(elapsed_ms);
            RenderAction::Prepare
        } else {
            RenderAction::Report
        }
    }
}

pub(crate) fn app_config() -> Option<&'static Config> {
    CONFIG.get()
}

pub fn read_config() -> Result<Config, String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 || args[0] != "--config" {
        return Err("Benchmark requires --config ABSOLUTE_JSON_PATH".into());
    }
    let path = PathBuf::from(&args[1]);
    if !path.is_absolute() {
        return Err("Config path must be absolute".into());
    }
    let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 {
        return Err("Invalid benchmark config".into());
    }
    let mut config: Config = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if config.schema_version != 2
        || !config.root.is_absolute()
        || !config.state_dir.is_absolute()
        || !["full", "same_process", "persisted", "changed", "seed"].contains(&config.mode.as_str())
    {
        return Err("Unsupported benchmark request".into());
    }
    if config.settle_ms > 10000 {
        return Err("Settle interval exceeds 10 seconds".into());
    }
    config.root = fs::canonicalize(&config.root).map_err(|e| e.to_string())?;
    if config
        .coverage_output
        .as_ref()
        .is_some_and(|p| !p.is_absolute())
    {
        return Err("Coverage output must be absolute".into());
    }
    if config.mode == "changed" && config.scope != "fixture" {
        return Err("Changed mode requires an owned fixture".into());
    }
    fs::create_dir_all(&config.state_dir).map_err(|e| e.to_string())?;
    let canonical = fs::canonicalize(&config.state_dir).map_err(|e| e.to_string())?;
    if canonical != config.state_dir || config.root.starts_with(&canonical) {
        return Err(
            "Benchmark state must be separate from source fixture and have no symlink ancestors"
                .into(),
        );
    }
    let marker = canonical.join(".hahakigami-benchmark-state");
    if marker.exists() {
        if fs::read_to_string(&marker).map_err(|e| e.to_string())?
            != "owned disposable benchmark state v1\n"
        {
            return Err("State ownership mismatch".into());
        }
    } else {
        if fs::read_dir(&canonical)
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
        {
            return Err("Refusing nonempty unowned benchmark state".into());
        }
        fs::write(marker, "owned disposable benchmark state v1\n").map_err(|e| e.to_string())?;
    }
    Ok(config)
}

fn configure(state: &AppState, config: &Config) -> Result<platform::Target, String> {
    let preferences = config.state_dir.join("config");
    fs::create_dir_all(&preferences).map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // The benchmark scans its own disposable root, so it never waits on Full Disk Access.
    let mut access = access_setup::AccessSetup::new(&exe);
    access.allow_scan_anyway();
    *state.access_setup.lock().unwrap() = access;
    *state.explorer_preferences.lock().unwrap() =
        explorer_preferences::Preferences::load(&preferences);
    *state.cleanup_queue.lock().unwrap() =
        cleanup_queue::Queue::load(config.state_dir.join("cleanup-queue"));
    state
        .baselines
        .configure(config.state_dir.join(if config.mode == "full" {
            format!("full-cache-{}", uuid::Uuid::new_v4())
        } else {
            "rescan-cache".into()
        }));
    register(
        state,
        platform::target(&config.root, "Benchmark location".into()),
        config.root.clone(),
    )
}

fn mock(
    config: &Config,
) -> Result<(tauri::App<tauri::test::MockRuntime>, platform::Target), String> {
    let app = tauri::test::mock_builder()
        .manage(AppState::default())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .map_err(|e| e.to_string())?;
    let target = configure(&app.state::<AppState>(), config)?;
    Ok((app, target))
}

async fn scan_once<R: tauri::Runtime>(
    app: &tauri::App<R>,
    target: &platform::Target,
) -> Result<Arc<Session>, String> {
    let id = start_scan(
        app.state(),
        target.id.clone(),
        Channel::new(|_| Ok(())),
        app.handle().clone(),
    )
    .await?;
    let s = app.state::<AppState>().scans.find(&id)?;
    let began = Instant::now();
    while session_running(&s) {
        if began.elapsed() > Duration::from_secs(600) {
            s.cancel.store(true, Ordering::Release);
            return Err("Native benchmark exceeded 600 seconds".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if s.summary().state != "completed" {
        return Err("Native scan did not complete".into());
    }
    Ok(s)
}

fn mutate(config: &Config, seed: &Snapshot) -> Result<(), String> {
    use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
    let root = disk_scan::ScanRoot::open(
        config.root.clone(),
        seed.node(0).ok_or("Missing seeded root")?.identity.clone(),
    )
    .map_err(|e| e.to_string())?;
    let raw = unsafe {
        libc::openat(
            root.as_fd().as_raw_fd(),
            c".hahakigami-performance-fixture.json".as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let marker = unsafe { fs::File::from_raw_fd(raw) };
    let info = marker.metadata().map_err(|e| e.to_string())?;
    if !info.is_file() || info.len() > 64 * 1024 {
        return Err("Invalid fixture marker".into());
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    marker
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 64 * 1024 {
        return Err("Fixture marker grew".into());
    }
    let metadata: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if metadata["owned_root"].as_str() != config.root.to_str() {
        return Err("Fixture ownership mismatch".into());
    }
    let raw = unsafe {
        libc::openat(
            root.as_fd().as_raw_fd(),
            c"shallow".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let directory = fs::File::from(unsafe { OwnedFd::from_raw_fd(raw) });
    use std::os::unix::fs::MetadataExt;
    let mut expected_directory = None;
    let mut expected_file = None;
    let mut path = PathBuf::new();
    for node in seed.iter_nodes() {
        seed.path_into(node.id, &mut path)?;
        if path == config.root.join("shallow") {
            expected_directory = Some(node.clone());
        }
        if path == config.root.join("shallow/ordinary") {
            expected_file = Some(node);
        }
    }
    let expected_directory = expected_directory.ok_or("Missing seeded fixture directory")?;
    let expected_file = expected_file.ok_or("Missing seeded fixture file")?;
    let info = directory.metadata().map_err(|e| e.to_string())?;
    if !info.is_dir()
        || info.dev() != expected_directory.identity.device
        || info.ino() != expected_directory.identity.inode
    {
        return Err("Fixture directory identity changed".into());
    }
    let raw = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            c"ordinary".as_ptr(),
            libc::O_WRONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut file = unsafe { fs::File::from_raw_fd(raw) };
    let info = file.metadata().map_err(|e| e.to_string())?;
    let producer_directory = &metadata["mutation_directory_identity"];
    let producer_file = &metadata["mutation_identity"];
    if producer_directory[0].as_u64() != Some(expected_directory.identity.device)
        || producer_directory[1].as_u64() != Some(expected_directory.identity.inode)
        || producer_file[0].as_u64() != Some(info.dev())
        || producer_file[1].as_u64() != Some(info.ino())
    {
        return Err("Fixture creation identities changed or are missing".into());
    }
    if info.dev() != expected_file.identity.device || info.ino() != expected_file.identity.inode {
        return Err("Fixture file identity changed".into());
    }
    if !info.is_file() {
        return Err("Fixture mutation target is not a regular file".into());
    }
    file.set_len(0).map_err(|e| e.to_string())?;
    file.write_all(b"changed performance fixture\n")
        .map_err(|e| e.to_string())
}

fn peak_rss() -> u64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } == 0 {
        usage.ru_maxrss as u64
    } else {
        0
    }
}

fn report(
    s: &Session,
    config: &Config,
    elapsed_ms: f64,
    rendered: bool,
    launch_ms: Option<f64>,
) -> Value {
    // Capture high-water RSS before untimed inspection/export allocates anything.
    let rss = peak_rss();
    let summary = s.summary();
    let snapshot = s.store.read().unwrap();
    let settings = settings::ScanSettings::default().automatic_info_for_scope(
        if config.root == Path::new("/") {
            settings::AutoScope::Volume
        } else {
            settings::AutoScope::Folder
        },
    );
    // Untimed buffer inspection follows the RSS capture. These estimates are
    // separate from process RSS and never expose query text or native paths.
    let folder_storage = s.list_folder.lock().unwrap().retained_buffer_storage();
    let filter_storage = s.filters.lock().unwrap().retained_buffer_storage();
    let map_storage = {
        let maps = s.sorts.lock().unwrap();
        let ids = disk_scan::BufferStorage {
            length_bytes: maps.values().map(|ids| ids.len() * 4).sum(),
            // An `Arc<[u32]>` is allocated at exactly its length.
            capacity_bytes: maps.values().map(|ids| ids.len() * 4).sum(),
        };
        json!({"id_buffers": ids,
            "index_payload_estimate_bytes": maps.capacity() * std::mem::size_of::<(u32, Arc<[u32]>)>()})
    };
    let issue_storage = {
        let issues = s.issues.lock().unwrap();
        disk_scan::BufferStorage {
            length_bytes: issues.1.len() * std::mem::size_of::<PathBuf>()
                + issues
                    .1
                    .iter()
                    .map(|path| path.as_os_str().len())
                    .sum::<usize>(),
            capacity_bytes: issues.1.capacity() * std::mem::size_of::<PathBuf>()
                + issues.1.iter().map(PathBuf::capacity).sum::<usize>(),
        }
    };
    let query_storage = json!({"folder": folder_storage, "search_filter": filter_storage,
        "map": map_storage, "issues": issue_storage,
        "map_temporary_keys": s.scan_details.lock().unwrap().map_sort_scratch,
        "map_temporary_keys_scope": "Largest combined requested key, owned lossy-name and sparse input-ID scratch sample from one map sort. Returned/cache ID arrays and borrowed snapshot names excluded. Not complete query peak, process peak, or RSS.",
        "notes": "Requested retained buffer capacity and estimated map entry payload; excludes allocator headers, hash controls and inline wrappers. Temporary map sort scratch reported separately; other transient query buffers excluded. Not RSS."});
    let scratch = json!({"reuse": s.scan_details.lock().unwrap().reuse_storage,
        "notes": "Reuse buffer estimates and largest individual materialized listing; excludes simultaneous workers, enumeration scratch and allocator overhead. Not process peak or RSS."});
    json!({"schema_version":2,"success":summary.state=="completed","state":summary.state,"elapsed_ms":elapsed_ms,"entries":summary.entries,"allocated_bytes":summary.allocated_bytes,"accounting":snapshot.accounting,"errors":summary.errors,"excluded":summary.excluded,"peak_rss_bytes":rss,"launch_clock_origin":"benchmark_install_entry","workers":summary.workers,"rendered":rendered,"launch_ms":launch_ms,"cache_mode":summary.scan_mode,"reused_entries":summary.reused_entries,"rescanned_directories":summary.rescanned_directories,"requested_mode":config.mode,"fallback":summary.fallback_reason,"coverage_complete":!summary.incomplete && summary.excluded==0,"coverage_limits":["OS access inherited; Full Disk Access is probed, never changed", "Verified full-clone data is attributed once; partial sharing and reclaimable space remain unresolved"],"phases":s.scan_details.lock().unwrap().phase_timings_us,"worker_stats":summary.worker_stats,"aggregation_us":summary.finalization_us,"storage":disk_scan::storage_stats(&snapshot),"scratch":scratch,"query_storage":query_storage,"scheduling":settings,"memory_scope":"native process high-water includes same-process/changed preparation only; persisted preparation uses a separate process; app process-tree sampling supplied by parent"})
}

pub fn native_main() -> Result<(), String> {
    // As in `run`: benchmarks must scan with the app's descriptor limit, not the shell's.
    budget::raise_descriptor_limit();
    let config = read_config()?;
    tauri::async_runtime::block_on(async {
        let (app, target) = mock(&config)?;
        if ["same_process", "changed"].contains(&config.mode.as_str()) {
            let seed = scan_once(&app, &target).await?;
            if config.mode == "changed" {
                mutate(&config, &seed.store.read().unwrap())?;
            }
        }
        let began = Instant::now();
        let s = scan_once(&app, &target).await?;
        let output = report(
            &s,
            &config,
            began.elapsed().as_secs_f64() * 1000.0,
            false,
            None,
        );
        if let Some(path) = &config.coverage_output {
            export_coverage(&s.store.read().unwrap(), path)?;
        }
        println!("{output}");
        Ok(())
    })
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let bits = ((chunk[0] as u32) << 16)
            | ((chunk.get(1).copied().unwrap_or(0) as u32) << 8)
            | chunk.get(2).copied().unwrap_or(0) as u32;
        out.push(TABLE[((bits >> 18) & 63) as usize] as char);
        out.push(TABLE[((bits >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((bits >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(bits & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

pub fn export_coverage(snapshot: &Snapshot, path: &Path) -> Result<(), String> {
    use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt};
    let file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| e.to_string())?;
    let mut writer = BufWriter::new(file);
    // Bound diagnostics independently of the streamed tree. These are recorded errors, not probes.
    let issue_samples: Vec<_> = snapshot
        .errors
        .iter()
        .take(20)
        .map(|issue| {
            let relative = issue.path.strip_prefix(&snapshot.root);
            let root_relative = relative.is_ok();
            let raw = relative.unwrap_or(&issue.path).as_os_str().as_bytes();
            json!({"path_base64":base64(&raw[..raw.len().min(8192)]),"path_truncated":raw.len()>8192,"path_scope":if root_relative {"root_relative"} else {"outside_root"},"error_kind":issue.error_kind,"raw_os_error":issue.raw_os_error,"message":issue.message.chars().take(512).collect::<String>(),"message_truncated":issue.message.chars().nth(512).is_some()})
        })
        .collect();
    let header = json!({"type":"header","schema_version":1,"complete":snapshot.state==ScanState::Completed && !snapshot.stats.incomplete && snapshot.stats.excluded==0,"node_count":snapshot.node_count(),"size_scope":"own","accounting_policy":"raw-reported-allocation-v1","attributed_accounting_model":snapshot.accounting.model,"mount_policy":"scanner-default-v1","issue_count":snapshot.stats.errors,"recorded_issue_count":snapshot.errors.len(),"issue_samples":issue_samples,"excluded_count":snapshot.stats.excluded,"limitations":if snapshot.stats.incomplete || snapshot.stats.excluded>0 {vec!["Incomplete or intentionally excluded coverage"]} else {vec![]}});
    writeln!(writer, "{header}").map_err(|e| e.to_string())?;
    let mut native_path = PathBuf::new();
    for node in snapshot.iter_nodes() {
        snapshot.path_into(node.id, &mut native_path)?;
        let relative = native_path
            .strip_prefix(&snapshot.root)
            .map_err(|e| e.to_string())?;
        let row = json!({"type":"node","path_base64":base64(relative.as_os_str().as_bytes()),"kind":match node.identity.kind {NodeKind::Directory=>"directory",NodeKind::File=>"file",NodeKind::Symlink=>"symlink",NodeKind::Other=>"other"},"length":node.length,"allocated_bytes":node.own_allocated_size,"identity":{"device":node.identity.device,"inode":node.identity.inode},"summary":false,"excluded":node.excluded});
        writeln!(writer, "{row}").map_err(|e| e.to_string())?;
    }
    let footer = json!({"type":"footer","schema_version":1,"node_count":snapshot.node_count(),"complete":true});
    writeln!(writer, "{footer}").map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())
}

pub(crate) fn record(s: &Session, phase: &str, began: Instant) {
    *s.scan_details
        .lock()
        .unwrap()
        .phase_timings_us
        .entry(phase.into())
        .or_default() += began.elapsed().as_micros() as u64;
}

pub(crate) fn install<R: tauri::Runtime>(
    app: &mut tauri::App<R>,
    config: Config,
) -> Result<(), String> {
    eprintln!("benchmark-owned-pid:{}", std::process::id());
    LAUNCHED.get_or_init(Instant::now);
    configure(&app.state::<AppState>(), &config)?;
    CONFIG
        .set(config)
        .map_err(|_| "Benchmark config already initialized")?;
    let handle = app.handle().clone();
    app.listen("performance-start", move |_| {
        let now = Instant::now();
        LAUNCH_MS
            .get_or_init(|| now.duration_since(*LAUNCHED.get().unwrap()).as_secs_f64() * 1000.0);
        *BEGAN.lock().unwrap() = Some(now);
        let _ = &handle;
    });
    let handle = app.handle().clone();
    app.listen("performance-rendered", move |event| {
        if REPORTED.load(Ordering::Acquire) {
            return;
        }
        let result = (|| -> Result<bool, String> {
            let payload: Value =
                serde_json::from_str(event.payload()).map_err(|e| e.to_string())?;
            let id = payload["scanId"]
                .as_str()
                .ok_or("Missing rendered scan ID")?;
            // The seed session is replaced on rescan. Ignore its duplicate ID before lookup.
            if RENDER_PASS.lock().unwrap().is_seed(id) {
                return Ok(false);
            }
            let s = handle.state::<AppState>().scans.find(id)?;
            if session_running(&s) || s.summary().state != "completed" {
                return Err("Rendered callback preceded scan completion".into());
            }
            let began = BEGAN.lock().unwrap().ok_or("No scan start callback")?;
            let config = CONFIG.get().ok_or("Missing benchmark config")?;
            let total_ms = began.elapsed().as_secs_f64() * 1000.0;
            let interaction_ms = payload["interactions"]["elapsedMs"].as_f64().unwrap_or(0.0);
            let elapsed_ms = total_ms - interaction_ms;
            let action = RENDER_PASS
                .lock()
                .unwrap()
                .completed(&config.mode, id, elapsed_ms);
            match action {
                RenderAction::Prepare => {
                    if config.mode == "changed" {
                        mutate(config, &s.store.read().unwrap())?;
                    }
                    handle
                        .emit("performance-rescan-ready", json!({"scanId":id}))
                        .map_err(|e| e.to_string())?;
                    return Ok(false);
                }
                RenderAction::Ignore => return Ok(false),
                RenderAction::Report => {}
            }
            if config.interactions && (!interaction_ms.is_finite() || interaction_ms < 0.0 || payload["interactions"]["success"] != true) {
                return Err(format!("Interaction workload failed: {}", payload["interactions"]));
            }

            if REPORTED.swap(true, Ordering::AcqRel) {
                return Ok(false);
            }
            let mut output = report(
                &s,
                config,
                elapsed_ms,
                true,
                LAUNCH_MS.get().copied(),
            );
            output["render_completion"] = json!("final_canvas_cpu_draw");
            if let Some(preparation_ms) = RENDER_PASS.lock().unwrap().preparation_ms {
                output["preparation"] = json!({"strategy":"same_process_completed_ui_draw","elapsed_ms":preparation_ms,"included_in_process_peak_rss":true});
            }
            output["interactions"] = payload["interactions"].clone();
            output["progress_updates"] = payload["progressUpdates"].clone();
            output["canvas_evidence"] = payload["canvas_evidence"].clone();
            output["active_elapsed_ms"] = json!(total_ms);
            output["settle_ms"] = json!(config.settle_ms);
            println!("{output}");
            std::io::stdout().flush().map_err(|e| e.to_string())?;
            // Only explicit benchmark builds pause after the result, for retained-RSS samples.
            std::thread::sleep(Duration::from_millis(config.settle_ms));
            // Coverage serialization is outside both active CPU and settled-RSS windows.
            if let Some(path) = &config.coverage_output {
                export_coverage(&s.store.read().unwrap(), path)?;
            }
            Ok(true)
        })();
        match result {
            Err(error) => {
                eprintln!("Benchmark failed: {error}");
                handle.exit(1);
            }
            Ok(true) => handle.exit(0),
            Ok(false) => {}
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coverage_diagnostics_distinguish_total_and_retained_issues() {
        let temporary = tempfile::tempdir().unwrap();
        let mut snapshot = Snapshot::empty(PathBuf::from("/fixture"));
        snapshot.stats.errors = 10_003;
        snapshot.stats.incomplete = true;
        snapshot.errors.push(disk_scan::ScanIssue {
            path: PathBuf::from("/fixture/blocked"),
            message: "Permission denied".into(),
            error_kind: Some("PermissionDenied".into()),
            raw_os_error: Some(13),
        });
        let output = temporary.path().join("coverage.ndjson");
        export_coverage(&snapshot, &output).unwrap();
        let text = fs::read_to_string(output).unwrap();
        let header: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(header["issue_count"], 10_003);
        assert_eq!(header["recorded_issue_count"], 1);
        assert_eq!(
            header["issue_samples"][0]["path_base64"],
            base64(b"blocked")
        );
        assert_eq!(header["complete"], false);
    }
    #[test]
    fn rendered_rescan_needs_two_distinct_completed_scan_ids() {
        for mode in ["same_process", "changed"] {
            let mut pass = RenderPass::default();
            assert_eq!(pass.completed(mode, "seed", 100.0), RenderAction::Prepare);
            assert!(pass.is_seed("seed")); // Works before any lookup of a replaced session.
            assert_eq!(pass.completed(mode, "seed", 999.0), RenderAction::Ignore);
            assert_eq!(pass.completed(mode, "measured", 50.0), RenderAction::Report);
            assert_eq!(pass.preparation_ms, Some(100.0));
            assert_eq!(pass.completed(mode, "seed", 999.0), RenderAction::Ignore);
        }
    }
    #[test]
    fn full_and_relaunched_runs_report_their_first_completed_draw() {
        for mode in ["full", "seed", "persisted"] {
            let mut pass = RenderPass::default();
            assert_eq!(pass.completed(mode, "scan", 50.0), RenderAction::Report);
            assert!(pass.seed_id.is_none());
            assert!(pass.preparation_ms.is_none());
        }
    }
    #[test]
    fn coverage_base64_preserves_arbitrary_native_bytes() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(&[0xff, 0, 10]), "/wAK");
    }
    #[test]
    fn fixture_mutation_rejects_symlink_hardlink_and_ancestor_substitution() {
        use std::os::unix::fs::symlink;
        for kind in ["symlink", "hardlink", "ancestor", "before-seed"] {
            let temporary = tempfile::tempdir().unwrap();
            let root = std::fs::canonicalize(temporary.path())
                .unwrap()
                .join("fixture");
            fs::create_dir_all(root.join("shallow")).unwrap();
            fs::write(root.join("shallow/ordinary"), b"fixture").unwrap();
            use std::os::unix::fs::MetadataExt;
            let original = fs::metadata(root.join("shallow/ordinary")).unwrap();
            let directory = fs::metadata(root.join("shallow")).unwrap();
            fs::write(root.join(".hahakigami-performance-fixture.json"), json!({"owned_root":root,"mutation_identity":[original.dev(),original.ino()],"mutation_directory_identity":[directory.dev(),directory.ino()]}).to_string()).unwrap();
            let mut seed = disk_scan::scan(
                root.clone(),
                Default::default(),
                Arc::new(AtomicBool::new(false)),
                |_| {},
            )
            .unwrap();
            let outside = temporary.path().join("outside");
            fs::write(&outside, b"keep this").unwrap();
            let target = root.join("shallow/ordinary");
            if kind == "ancestor" {
                let directory = temporary.path().join("outside-directory");
                fs::create_dir(&directory).unwrap();
                fs::write(directory.join("ordinary"), b"keep this").unwrap();
                fs::rename(root.join("shallow"), root.join("old-shallow")).unwrap();
                symlink(&directory, root.join("shallow")).unwrap();
            } else {
                fs::remove_file(&target).unwrap();
                if kind == "symlink" {
                    symlink(&outside, &target).unwrap();
                } else {
                    fs::hard_link(&outside, &target).unwrap();
                }
            }
            if kind == "before-seed" {
                seed = disk_scan::scan(
                    root.clone(),
                    Default::default(),
                    Arc::new(AtomicBool::new(false)),
                    |_| {},
                )
                .unwrap();
            }
            let config = Config {
                schema_version: 2,
                root,
                scope: "fixture".into(),
                mode: "changed".into(),
                state_dir: temporary.path().join("state"),
                coverage_output: None,
                interactions: false,
                settle_ms: 0,
            };
            assert!(mutate(&config, &seed).is_err());
            assert_eq!(fs::read(&outside).unwrap(), b"keep this");
            if kind == "ancestor" {
                assert_eq!(
                    fs::read(temporary.path().join("outside-directory/ordinary")).unwrap(),
                    b"keep this"
                );
            }
        }
    }
}
