use crate::automation::Service;
use rmcp::{
    handler::server::wrapper::Parameters, model::CallToolResult, schemars, service::RequestContext,
    tool, tool_handler, tool_router, RoleServer, ServerHandler, ServiceExt,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScanInput {
    /// Absolute path of the directory to scan. File contents are never read.
    path: String,
}

fn default_limit() -> usize {
    50
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct PageInput {
    /// Refused if a newer scan has replaced the scan with this `scanId`.
    scan_id: Option<u64>,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 1000))]
    limit: usize,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct NodeInput {
    node_id: u32,
    /// The `scanId` the ID came from; refused if a newer scan has replaced it.
    scan_id: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ChildrenInput {
    node_id: u32,
    /// The `scanId` the ID came from; refused if a newer scan has replaced it.
    scan_id: Option<u64>,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 1000))]
    limit: usize,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchInput {
    query: String,
    /// Refused if a newer scan has replaced the scan with this `scanId`.
    scan_id: Option<u64>,
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

#[tool_router]
impl Mcp {
    #[tool(
        description = "Scan a directory's metadata. One active scan per session; query its result with summary, node, children, search and issues. Results end with the session. Sizes do not represent reclaimable space."
    )]
    async fn scan(
        &self,
        Parameters(input): Parameters<ScanInput>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        // The server's working directory is unrelated to the client's.
        if !std::path::Path::new(&input.path).is_absolute() {
            return CallToolResult::structured_error(json!({"error": "Pass an absolute path"}));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        tokio::select! {
            result = self.execute(move |service| service.scan_with(PathBuf::from(input.path), flag)) => result,
            // A client's `notifications/cancelled` only cancels this request; stop this request's scan
            // (not whichever one is active), even if its blocking task has not started yet.
            _ = context.ct.cancelled() => {
                cancel.store(true, Ordering::Relaxed);
                CallToolResult::structured_error(json!({"error": "Scan cancelled"}))
            }
        }
    }

    #[tool(
        description = "Get the active scan's status and coverage. Results are metadata snapshots, not live filesystem state."
    )]
    async fn status(&self) -> CallToolResult {
        self.execute(Service::status).await
    }

    #[tool(
        description = "Request cancellation of this session's active scan. Partial results remain available to queries."
    )]
    async fn cancel(&self) -> CallToolResult {
        self.execute(Service::cancel).await
    }

    #[tool(
        description = "Read the latest scan's totals, coverage and accounting warnings. Allocated sizes do not represent reclaimable space."
    )]
    async fn summary(&self) -> CallToolResult {
        self.execute(Service::summary).await
    }

    #[tool(description = "Read a node's metadata, native path representation and allocated size.")]
    async fn node(&self, Parameters(input): Parameters<NodeInput>) -> CallToolResult {
        self.execute(move |service| service.node(input.node_id, input.scan_id))
            .await
    }

    #[tool(description = "Read a directory node's children from a metadata snapshot, paginated.")]
    async fn children(&self, Parameters(input): Parameters<ChildrenInput>) -> CallToolResult {
        self.execute(move |service| {
            service.children(input.node_id, input.scan_id, input.offset, input.limit)
        })
        .await
    }

    #[tool(
        description = "Search filenames in a metadata snapshot, paginated. Does not read file contents."
    )]
    async fn search(&self, Parameters(input): Parameters<SearchInput>) -> CallToolResult {
        self.execute(move |service| {
            service.search(&input.query, input.scan_id, input.offset, input.limit)
        })
        .await
    }

    #[tool(description = "Read scan issues explaining incomplete coverage, paginated.")]
    async fn issues(&self, Parameters(input): Parameters<PageInput>) -> CallToolResult {
        self.execute(move |service| service.issues(input.scan_id, input.offset, input.limit))
            .await
    }
}

#[tool_handler(
    name = "Hahakigami",
    instructions = "Scan and query local filesystem metadata without reading file contents. Queries read this session's latest scan; results end with the session. Results are metadata snapshots, not live filesystem state. Observe coverage and accounting warnings; allocated sizes do not represent reclaimable space. Use the cancel tool to cancel an active scan."
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
