use serde_json::Value;
use std::{fs, process::Command};

fn command(data: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_hahakigami"))
        .arg("--data-dir")
        .arg(data)
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
#[ignore = "saved scans disabled: SAVED_SCANS_ENABLED"]
fn scans_and_queries_retained_metadata_without_the_gui() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("fixture");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("small.txt"), b"abc").unwrap();
    fs::write(root.join("large.bin"), vec![b'x'; 8192]).unwrap();
    let data = fixture.path().join("automation");
    let summary = json(command(&data, &["scan", root.to_str().unwrap()]));
    assert_eq!(summary["state"], "completed");
    let id = summary["scanId"].as_str().unwrap();
    let root_id = summary["rootId"].as_u64().unwrap().to_string();
    let later = json(command(&data, &["summary", id]));
    assert_eq!(later["scanId"], id);
    assert_eq!(later["logicalBytes"], summary["logicalBytes"]);
    let children = json(command(&data, &["children", id, &root_id, "--limit", "1"]));
    assert_eq!(children["total"], 2);
    assert_eq!(children["items"].as_array().unwrap().len(), 1);
    let found = json(command(&data, &["search", id, "SMALL"]));
    assert_eq!(found["total"], 1);
    assert_eq!(found["items"][0]["name"], "small.txt");
    assert_eq!(found["items"][0]["logicalBytes"], 3);
    let node_id = found["items"][0]["id"].as_u64().unwrap().to_string();
    let node = json(command(&data, &["node", id, &node_id]));
    assert_eq!(node["logicalBytes"], 3);
    assert_eq!(node["coverage"]["incomplete"], false);
    let next = json(command(
        &data,
        &["children", id, &root_id, "--offset", "1", "--limit", "1"],
    ));
    assert_ne!(next["items"][0]["id"], children["items"][0]["id"]);
    let outside = json(command(
        &data,
        &[
            "children",
            id,
            &root_id,
            "--offset",
            &usize::MAX.to_string(),
        ],
    ));
    assert!(outside["items"].as_array().unwrap().is_empty());
    let retained = json(command(&data, &["list"]));
    assert_eq!(retained["total"], 1);
    assert_eq!(retained["items"][0]["id"], id);
    let issues = json(command(&data, &["issues", id]));
    assert_eq!(issues["total"], 0);
    let missing_node = command(&data, &["node", id, "4294967295"]);
    assert_eq!(missing_node.status.code(), Some(1));
    assert!(missing_node.stdout.is_empty());
}

#[test]
fn help_and_invalid_arguments_do_not_launch_the_app_or_create_storage() {
    let fixture = tempfile::tempdir().unwrap();
    let data = fixture.path().join("unused-store");
    let help = command(&data, &["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("mcp"));
    assert!(!data.exists());
    for args in [
        vec!["list", "--limit", "0"],
        vec!["list", "--limit", "1001"],
        vec!["unknown-command"],
    ] {
        let output = command(&data, &args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
    let invalid_id = command(&data, &["summary", "../../outside"]);
    assert_eq!(invalid_id.status.code(), Some(1));
    assert!(invalid_id.stdout.is_empty());
    let missing = fixture.path().join("missing");
    let output = command(&data, &["scan", missing.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!data.exists());
}

#[cfg(unix)]
#[test]
#[ignore = "saved scans disabled: SAVED_SCANS_ENABLED"]
fn native_paths_round_trip_and_search_pages_can_exceed_five_hundred() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("native-кіця \n%");
    fs::create_dir(&root).unwrap();
    for index in 0..501 {
        fs::write(root.join(format!("match-{index}.txt")), b"x").unwrap();
    }
    let data = fixture.path().join("store with spaces");
    let output = Command::new(env!("CARGO_BIN_EXE_hahakigami"))
        .arg("--data-dir")
        .arg(&data)
        .arg("scan")
        .arg(&root)
        .output()
        .unwrap();
    let summary = json(output);
    assert!(summary["pathKey"]
        .as_str()
        .unwrap()
        .ends_with("native-%D0%BA%D1%96%D1%86%D1%8F%20%0A%25"));
    let id = summary["scanId"].as_str().unwrap();
    let found = json(command(&data, &["search", id, "match-", "--limit", "1000"]));
    assert_eq!(found["total"], 501);
    assert_eq!(found["items"].as_array().unwrap().len(), 501);
    assert!(found["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["logicalBytes"] == 1));
}
