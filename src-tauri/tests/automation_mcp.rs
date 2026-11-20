use rmcp::{
    model::{CallToolRequestParams, CallToolResult},
    service::RunningService,
    transport::TokioChildProcess,
    Peer, RoleClient, ServiceExt,
};
use serde_json::{json, Value};
use std::{process::Stdio, time::Duration};

fn command() -> tokio::process::Command {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_hahakigami"));
    command.arg("mcp");
    command
}

async fn connect() -> RunningService<RoleClient, ()> {
    ().serve(TokioChildProcess::new(command()).unwrap())
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
        let client = connect().await;
        let info = client.peer_info().unwrap();
        assert_eq!(info.server_info.as_ref().unwrap().name, "Hahakigami");
        assert!(info.instructions.as_deref().unwrap().contains("metadata"));
        let tools = client.list_all_tools().await.unwrap();
        let mut names: Vec<_> = tools.iter().map(|tool| tool.name.as_ref()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            ["cancel", "children", "issues", "node", "scan", "search", "status", "summary"]
        );
        let scan = tools.iter().find(|tool| tool.name == "scan").unwrap();
        assert_eq!(scan.input_schema["properties"]["path"]["type"], "string");
        // Only the query tools accept the optional scan_id.
        for tool in &tools {
            let takes = tool
                .input_schema
                .get("properties")
                .is_some_and(|properties| properties.get("scan_id").is_some());
            assert_eq!(
                takes,
                ["node", "children", "search", "issues"].contains(&tool.name.as_ref()),
                "{}",
                tool.name
            );
            assert!(!tool.input_schema.get("required").is_some_and(|required| {
                required
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|name| name == "scan_id")
            }));
        }
        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP initialization must finish within 20 seconds");
}

#[tokio::test]
async fn stdio_scans_and_queries_the_latest_scan_and_reports_errors() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::write(fixture.path().join("needle.txt"), b"1234567").unwrap();
        std::fs::write(fixture.path().join("other.txt"), b"xyz").unwrap();
        let client = connect().await;
        assert_eq!(value(&client, "status", json!({})).await["running"], false);
        assert_eq!(value(&client, "status", json!({})).await["state"], "idle");
        // Nothing to query before the first scan.
        let early = call(&client, "summary", json!({})).await;
        assert_eq!(early.is_error, Some(true), "{early:?}");
        let scanned = value(&client, "scan", json!({"path": fixture.path()})).await;
        assert_eq!(scanned["state"], "completed");
        assert_eq!(scanned["rootId"], 0);
        assert_eq!(scanned["incomplete"], false);
        assert!(scanned.get("accounting").is_some());
        assert!(scanned.get("coverage").is_some());
        let scan_id = scanned["scanId"].as_u64().unwrap();
        let found = value(&client, "search", json!({"query": "needle"})).await;
        assert_eq!(found["total"], 1);
        assert_eq!(found["items"][0]["name"], "needle.txt");
        assert_eq!(found["scanId"], scan_id);
        let node_id = found["items"][0]["id"].as_u64().unwrap();
        let node = value(&client, "node", json!({"node_id": node_id})).await;
        assert!(node.get("logicalBytes").is_none());
        assert!(node.get("allocatedBytes").is_some());
        assert_eq!(node["scanId"], scan_id);
        let first = value(&client, "children", json!({"node_id": 0, "limit": 1})).await;
        let second = value(
            &client,
            "children",
            json!({"node_id": 0, "offset": 1, "limit": 1}),
        )
        .await;
        assert_eq!(first["total"], 2);
        assert_eq!(first["items"].as_array().unwrap().len(), 1);
        assert_eq!(second["items"].as_array().unwrap().len(), 1);
        assert_ne!(first["items"][0]["id"], second["items"][0]["id"]);
        let issues = value(&client, "issues", json!({"scan_id": scan_id})).await;
        assert_eq!(issues["total"], 0);
        assert_eq!(issues["scanId"], scan_id);
        let summary = value(&client, "summary", json!({})).await;
        assert!(summary.get("logicalBytes").is_none());
        assert_eq!(summary["allocatedBytes"], scanned["allocatedBytes"]);
        assert_eq!(summary["state"], "completed");
        for (name, args) in [
            ("node", json!({"node_id": u32::MAX})),
            ("issues", json!({"limit": 0})),
            ("search", json!({"query": ""})),
            // An ID from another scan is refused, and a file has no children.
            ("node", json!({"node_id": 0, "scan_id": scan_id + 1})),
            ("issues", json!({"scan_id": scan_id + 1})),
            ("children", json!({"node_id": node_id, "scan_id": scan_id})),
            ("scan", json!({"path": fixture.path().join("missing")})),
        ] {
            let result = call(&client, name, args).await;
            assert_eq!(result.is_error, Some(true), "{name}: {result:?}");
            assert!(result.structured_content.unwrap()["error"].is_string());
            assert!(!result.content.is_empty());
        }
        // A relative path is refused, and the finished scan stays queryable.
        let relative = call(&client, "scan", json!({"path": "relative/dir"})).await;
        assert_eq!(relative.is_error, Some(true), "{relative:?}");
        assert!(relative.structured_content.unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("absolute"));
        assert_eq!(
            value(&client, "summary", json!({})).await["scanId"],
            scan_id
        );
        assert!(client
            .call_tool(CallToolRequestParams::new("list"))
            .await
            .is_err());
        assert!(client
            .call_tool(CallToolRequestParams::new("unknown"))
            .await
            .is_err());
        assert_eq!(
            call(&client, "scan", json!({"path": 42})).await.is_error,
            Some(true)
        );
        // A scan_id is a number, not a name.
        assert_eq!(
            call(
                &client,
                "search",
                json!({"scan_id": "current", "query": "needle"})
            )
            .await
            .is_error,
            Some(true)
        );
        // Logical size no longer exists: tools take no size metric or sort option.
        for (name, args) in [
            ("search", json!({"query": "needle", "metric": "logical"})),
            ("children", json!({"node_id": 0, "sort": "logical"})),
        ] {
            assert_eq!(
                call(&client, name, args).await.is_error,
                Some(true),
                "{name}"
            );
        }
        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP scan and queries must finish within 30 seconds");
}

#[cfg(unix)]
#[tokio::test]
async fn stdio_native_paths_round_trip_and_search_pages_can_exceed_five_hundred() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("native-кіця \n%");
        std::fs::create_dir(&root).unwrap();
        for index in 0..501 {
            std::fs::write(root.join(format!("match-{index}.txt")), b"x").unwrap();
        }
        let client = connect().await;
        let summary = value(&client, "scan", json!({"path": root})).await;
        assert!(summary["pathKey"]
            .as_str()
            .unwrap()
            .ends_with("native-%D0%BA%D1%96%D1%86%D1%8F%20%0A%25"));
        let found = value(&client, "search", json!({"query": "match-", "limit": 1000})).await;
        assert_eq!(found["total"], 501);
        assert_eq!(found["items"].as_array().unwrap().len(), 501);
        assert!(found["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row.get("logicalBytes").is_none()));
        client.cancel().await.unwrap();
    })
    .await
    .expect("MCP native path scan must finish within 30 seconds");
}

#[tokio::test]
async fn stdio_cancel_keeps_partial_results_and_disconnect_exits() {
    // Built outside the timeout: 40,000 hard links take ~9 s idle and far longer on a busy machine.
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
    tokio::time::timeout(Duration::from_secs(45), async {
        // Keep the process handle so this verifies EOF exit, not the SDK's kill-on-drop cleanup.
        let mut child = command()
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
        assert_eq!(partial["incomplete"], true);
        assert_eq!(
            value(&client, "summary", json!({})).await["state"],
            "cancelled"
        );
        assert_eq!(value(&client, "status", json!({})).await["running"], false);
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
