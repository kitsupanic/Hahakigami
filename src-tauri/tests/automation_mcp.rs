use rmcp::{
    model::{CallToolRequestParams, CallToolResult},
    service::RunningService,
    transport::TokioChildProcess,
    Peer, RoleClient, ServiceExt,
};
use serde_json::{json, Value};
use std::{path::Path, process::Stdio, time::Duration};

fn command(store: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_hahakigami"));
    command.arg("--data-dir").arg(store).arg("mcp");
    command
}

async fn connect(store: &Path) -> RunningService<RoleClient, ()> {
    ().serve(TokioChildProcess::new(command(store)).unwrap())
        .await
        .unwrap()
}

async fn call(client: &Peer<RoleClient>, name: &str, args: Value) -> CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap()
}

async fn value(client: &Peer<RoleClient>, name: &str, args: Value) -> Value {
    let result = call(client, name, args).await;
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.unwrap();
    assert_eq!(
        result.content[0].as_text().unwrap().text,
        structured.to_string()
    );
    structured
}

#[tokio::test]
async fn stdio_initializes_and_lists_metadata_tools() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let store = tempfile::tempdir().unwrap();
        let client = connect(store.path()).await;
        let info = client.peer_info().unwrap();
        assert_eq!(info.server_info.as_ref().unwrap().name, "Hahakigami");
        assert!(info.instructions.as_deref().unwrap().contains("metadata"));
        let tools = client.list_all_tools().await.unwrap();
        let mut names: Vec<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "cancel", "children", "issues", "list", "node", "scan", "search", "status",
                "summary"
            ]
        );
        let scan = tools.iter().find(|tool| tool.name == "scan").unwrap();
        assert_eq!(scan.input_schema["properties"]["path"]["type"], "string");
        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP initialization must finish within 20 seconds");
}

#[tokio::test]
#[ignore = "saved scans disabled: SAVED_SCANS_ENABLED"]
async fn stdio_scans_queries_errors_and_retains_results_across_sessions() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let store = tempfile::tempdir().unwrap();
        let fixture = tempfile::tempdir().unwrap();
        std::fs::write(fixture.path().join("needle.txt"), b"1234567").unwrap();
        std::fs::write(fixture.path().join("other.txt"), b"xyz").unwrap();
        let client = connect(store.path()).await;
        assert_eq!(value(&client, "status", json!({})).await["running"], false);
        let scanned = value(&client, "scan", json!({"path": fixture.path()})).await;
        assert_eq!(scanned["state"], "completed");
        assert_eq!(scanned["rootId"], 0);
        assert_eq!(scanned["incomplete"], false);
        assert!(scanned.get("accounting").is_some());
        assert!(scanned.get("coverage").is_some());
        let scan_id = scanned["scanId"].as_str().unwrap();
        assert!(uuid::Uuid::parse_str(scan_id).is_ok());
        let found = value(
            &client,
            "search",
            json!({"scan_id": scan_id, "query": "needle"}),
        )
        .await;
        assert_eq!(found["total"], 1);
        assert_eq!(found["items"][0]["name"], "needle.txt");
        let node_id = found["items"][0]["id"].as_u64().unwrap();
        let node = value(
            &client,
            "node",
            json!({"scan_id": scan_id, "node_id": node_id}),
        )
        .await;
        assert_eq!(node["logicalBytes"], 7);
        assert!(node.get("allocatedBytes").is_some());
        let first = value(
            &client,
            "children",
            json!({"scan_id": scan_id, "node_id": 0, "limit": 1}),
        )
        .await;
        let second = value(
            &client,
            "children",
            json!({"scan_id": scan_id, "node_id": 0, "offset": 1, "limit": 1}),
        )
        .await;
        assert_eq!(first["total"], 2);
        assert_eq!(first["items"].as_array().unwrap().len(), 1);
        assert_eq!(second["items"].as_array().unwrap().len(), 1);
        assert_ne!(first["items"][0]["id"], second["items"][0]["id"]);
        assert_eq!(
            value(&client, "issues", json!({"scan_id": scan_id})).await["total"],
            0
        );
        assert_eq!(
            value(&client, "summary", json!({"scan_id": "current"})).await["scanId"],
            scan_id
        );
        for (name, args) in [
            ("summary", json!({"scan_id": "invalid"})),
            ("node", json!({"scan_id": scan_id, "node_id": u32::MAX})),
            ("list", json!({"limit": 0})),
            ("search", json!({"scan_id": scan_id, "query": ""})),
            ("scan", json!({"path": fixture.path().join("missing")})),
        ] {
            let result = call(&client, name, args).await;
            assert_eq!(result.is_error, Some(true), "{name}: {result:?}");
            assert!(result.structured_content.unwrap()["error"].is_string());
            assert!(!result.content.is_empty());
        }
        assert!(client
            .call_tool(CallToolRequestParams::new("unknown"))
            .await
            .is_err());
        assert_eq!(
            call(&client, "scan", json!({"path": 42})).await.is_error,
            Some(true)
        );
        client.cancel().await.unwrap();
        let client = connect(store.path()).await;
        let retained = value(&client, "list", json!({})).await;
        assert_eq!(retained["total"], 1);
        let summary = value(&client, "summary", json!({"scan_id": scan_id})).await;
        assert_eq!(summary["logicalBytes"], scanned["logicalBytes"]);
        assert_eq!(summary["state"], "completed");
        assert_eq!(
            value(
                &client,
                "search",
                json!({"scan_id": scan_id, "query": "needle"})
            )
            .await["total"],
            1
        );
        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP scan and retained queries must finish within 30 seconds");
}

#[tokio::test]
#[ignore = "saved scans disabled: SAVED_SCANS_ENABLED"]
async fn stdio_save_failure_preserves_completed_current_snapshot() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::write(fixture.path().join("needle.txt"), b"1234567").unwrap();
        let store = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(store.path(), b"existing store file").unwrap();
        let client = connect(store.path()).await;

        let failed = call(&client, "scan", json!({"path": fixture.path()})).await;
        assert_eq!(failed.is_error, Some(true));
        let error = failed.structured_content.unwrap()["error"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(error.contains("could not be saved"), "{error}");
        let status = value(&client, "status", json!({})).await;
        assert_eq!(status["state"], "completed");
        assert_eq!(status["running"], false);
        assert_eq!(status["error"], error);
        assert!(status["scanId"].is_null());
        let current = value(&client, "summary", json!({"scan_id": "current"})).await;
        assert_eq!(current["state"], "completed");
        assert_eq!(current["incomplete"], false);
        assert!(current["scanId"].is_null());
        assert_eq!(current["rootId"], 0);
        let found = value(
            &client,
            "search",
            json!({"scan_id": "current", "query": "needle"}),
        )
        .await;
        assert_eq!(found["total"], 1);
        assert_eq!(found["items"][0]["logicalBytes"], 7);
        assert_eq!(std::fs::read(store.path()).unwrap(), b"existing store file");
        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP save failure must preserve current results within 20 seconds");
}

#[tokio::test]
#[ignore = "saved scans disabled: SAVED_SCANS_ENABLED"]
async fn stdio_cancel_keeps_partial_results_and_disconnect_exits() {
    tokio::time::timeout(Duration::from_secs(45), async {
        let store = tempfile::tempdir().unwrap();
        let fixture = tempfile::tempdir().unwrap();
        let seed = fixture.path().join("seed");
        std::fs::write(&seed, b"metadata fixture").unwrap();
        for directory in 0..200 {
            let directory = fixture.path().join(format!("dir-{directory}"));
            std::fs::create_dir(&directory).unwrap();
            for file in 0..200 {
                std::fs::hard_link(&seed, directory.join(format!("file-{file}"))).unwrap();
            }
        }
        // Keep the process handle so this verifies EOF exit, not the SDK's kill-on-drop cleanup.
        let mut child = command(store.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let stdin = child.stdin.take().unwrap();
        let client = ().serve((stdout, stdin)).await.unwrap();
        let peer = client.peer().clone();
        let path = fixture.path().to_owned();
        let scan = tokio::spawn(async move { call(&peer, "scan", json!({"path": path})).await });
        wait_until_running(&client).await;
        let rejected = call(&client, "scan", json!({"path": fixture.path()})).await;
        assert_eq!(rejected.is_error, Some(true));
        assert_eq!(
            value(&client, "cancel", json!({})).await["cancelRequested"],
            true
        );
        let result = scan.await.unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
        let partial = result.structured_content.unwrap();
        assert_eq!(partial["state"], "cancelled");
        assert!(partial["scanId"].is_null());
        assert_eq!(partial["incomplete"], true);
        assert_eq!(
            value(&client, "summary", json!({"scan_id": "current"})).await["state"],
            "cancelled"
        );
        assert_eq!(value(&client, "status", json!({})).await["running"], false);
        assert_eq!(value(&client, "list", json!({})).await["total"], 0);
        assert_eq!(
            value(&client, "cancel", json!({})).await["cancelRequested"],
            false
        );

        let peer = client.peer().clone();
        let path = fixture.path().to_owned();
        let scan = tokio::spawn(async move {
            peer.call_tool(
                CallToolRequestParams::new("scan")
                    .with_arguments(json!({"path": path}).as_object().unwrap().clone()),
            )
            .await
        });
        wait_until_running(&client).await;
        client.cancel().await.unwrap();
        let exit = tokio::time::timeout(Duration::from_secs(3), child.wait())
            .await
            .expect("MCP server must exit within three seconds of disconnect")
            .unwrap();
        assert!(exit.success(), "MCP server exited with {exit}");
        assert!(scan.await.unwrap().is_err());
    })
    .await
    .expect("MCP cancellation and disconnect must finish within 45 seconds");
}

async fn wait_until_running(client: &Peer<RoleClient>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = value(client, "status", json!({})).await;
            if status["running"] == true {
                break;
            }
            assert_ne!(
                status["state"], "completed",
                "Fixture finished before cancellation could be tested"
            );
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("Scan must become active within five seconds");
}
