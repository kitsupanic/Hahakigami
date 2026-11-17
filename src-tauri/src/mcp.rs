use crate::automation::Service;
use rmcp::{
    handler::server::wrapper::Parameters, model::CallToolResult, schemars, tool, tool_handler,
    tool_router, ServerHandler, ServiceExt,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScanInput {
    /// Directory to scan. File contents are never read.
    path: String,
}

fn default_limit() -> usize {
    50
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct PageInput {
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 1000))]
    limit: usize,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SnapshotInput {
    /// "current" for this session's latest snapshot. Saved scan UUIDs are refused while saved scans are turned off.
    scan_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct NodeInput {
    scan_id: String,
    node_id: u32,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ChildrenInput {
    scan_id: String,
    node_id: u32,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 1000))]
    limit: usize,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchInput {
    scan_id: String,
    query: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 1000))]
    limit: usize,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct IssuesInput {
    scan_id: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 1000))]
    limit: usize,
}

#[derive(Clone)]
struct Mcp {
    service: Arc<Service>,
}

impl Mcp {
    async fn execute(
        &self,
        operation: impl FnOnce(&Service) -> Result<Value, String> + Send + 'static,
    ) -> CallToolResult {
        let service = Arc::clone(&self.service);
        match tokio::task::spawn_blocking(move || operation(&service)).await {
            Ok(Ok(value)) => CallToolResult::structured(value),
            Ok(Err(error)) => CallToolResult::structured_error(json!({"error": error})),
            Err(error) => CallToolResult::structured_error(
                json!({"error": format!("Worker failed: {error}")}),
            ),
        }
    }
}

// Tool texts say saved scans are off; reword them when SAVED_SCANS_ENABLED flips back on.
#[tool_router]
impl Mcp {
    #[tool(
        description = "Scan a directory's metadata. One active scan per session; query its result with scan_id \"current\". Saved scans are turned off in this version, so results are not kept after the session. Sizes do not represent reclaimable space."
    )]
    async fn scan(&self, Parameters(input): Parameters<ScanInput>) -> CallToolResult {
        self.execute(move |service| service.scan(PathBuf::from(input.path)))
            .await
    }

    #[tool(
        description = "Get the active scan's status and coverage. Saved results are metadata snapshots, not live filesystem state."
    )]
    async fn status(&self) -> CallToolResult {
        self.execute(Service::status).await
    }

    #[tool(
        description = "Request cancellation of this session's active scan. Partial results remain available as current."
    )]
    async fn cancel(&self) -> CallToolResult {
        self.execute(Service::cancel).await
    }

    #[tool(
        description = "List saved scans, paginated. Saved scans are turned off in this version, so this always returns an error."
    )]
    async fn list(&self, Parameters(input): Parameters<PageInput>) -> CallToolResult {
        self.execute(move |service| service.list(input.offset, input.limit))
            .await
    }

    #[tool(
        description = "Read scan totals, coverage and accounting warnings. Logical and allocated sizes do not represent reclaimable space."
    )]
    async fn summary(&self, Parameters(input): Parameters<SnapshotInput>) -> CallToolResult {
        self.execute(move |service| service.summary(&input.scan_id))
            .await
    }

    #[tool(
        description = "Read a node's saved metadata, native path representation, logical and allocated sizes."
    )]
    async fn node(&self, Parameters(input): Parameters<NodeInput>) -> CallToolResult {
        self.execute(move |service| service.node(&input.scan_id, input.node_id))
            .await
    }

    #[tool(description = "Read a directory node's children from a metadata snapshot, paginated.")]
    async fn children(&self, Parameters(input): Parameters<ChildrenInput>) -> CallToolResult {
        self.execute(move |service| {
            service.children(&input.scan_id, input.node_id, input.offset, input.limit)
        })
        .await
    }

    #[tool(
        description = "Search filenames in a metadata snapshot, paginated. Does not read file contents."
    )]
    async fn search(&self, Parameters(input): Parameters<SearchInput>) -> CallToolResult {
        self.execute(move |service| {
            service.search(&input.scan_id, &input.query, input.offset, input.limit)
        })
        .await
    }

    #[tool(description = "Read scan issues explaining incomplete coverage, paginated.")]
    async fn issues(&self, Parameters(input): Parameters<IssuesInput>) -> CallToolResult {
        self.execute(move |service| service.issues(&input.scan_id, input.offset, input.limit))
            .await
    }
}

#[tool_handler(
    name = "Hahakigami",
    instructions = "Scan and query local filesystem metadata without reading file contents. Query results with scan_id \"current\"; saved scans are turned off in this version, so results end with the session. Results are metadata snapshots, not live filesystem state. Observe coverage and accounting warnings; logical and allocated sizes do not represent reclaimable space. Use the cancel tool to cancel an active scan."
)]
impl ServerHandler for Mcp {}

pub(crate) async fn serve(service: Arc<Service>) -> Result<(), String> {
    let server = Mcp {
        service: Arc::clone(&service),
    }
    .serve(rmcp::transport::stdio())
    .await
    .map_err(|error| error.to_string())?;
    let result = server.waiting().await.map_err(|error| error.to_string());
    let _ = service.cancel();
    result.map(|_| ())
}
