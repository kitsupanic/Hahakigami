use serde_json::Value;
use std::{fs, process::Command};

fn command(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_hahakigami"))
        .args(args)
        .output()
        .unwrap()
}

fn json(output: std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout is a single JSON value")
}

#[test]
fn scans_and_prints_metadata_without_the_gui() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("fixture");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("small.txt"), b"abc").unwrap();
    fs::write(root.join("large.bin"), vec![b'x'; 8192]).unwrap();
    let summary = json(command(&["scan", root.to_str().unwrap()]));
    assert_eq!(summary["state"], "completed");
    assert_eq!(summary["rootId"], 0);
    assert_eq!(summary["logicalBytes"], 3 + 8192);
    assert_eq!(summary["entries"], 3);
    assert_eq!(summary["incomplete"], false);
    assert!(summary.get("scanId").is_none());
    assert!(summary.get("accounting").is_some());
    assert!(summary.get("coverage").is_some());
}

#[test]
fn help_and_invalid_arguments_do_not_launch_the_app() {
    let help = command(&["--help"]);
    assert!(help.status.success());
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(text.contains("scan") && text.contains("mcp"));
    for removed in ["list", "summary", "children", "search", "issues"] {
        assert!(!text.contains(&format!("\n  {removed} ")), "{removed}");
    }
    for args in [
        vec!["unknown-command"],
        vec!["list"],
        vec!["--data-dir", "unused", "scan", "."],
    ] {
        let output = command(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
    let fixture = tempfile::tempdir().unwrap();
    let missing = fixture.path().join("missing");
    let output = command(&["scan", missing.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn native_paths_round_trip_through_the_scan_summary() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("native-кіця \n%");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("file.txt"), b"x").unwrap();
    let summary = json(command(&["scan", root.to_str().unwrap()]));
    assert!(summary["pathKey"]
        .as_str()
        .unwrap()
        .ends_with("native-%D0%BA%D1%96%D1%86%D1%8F%20%0A%25"));
}
