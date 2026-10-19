//! The rmcp server behind `se mcp`: one stdio connection per agent session.
//! rmcp negotiates either lifecycle (Codex `initialize`, Claude Code
//! `server/discover`) and runs every request on its own task; blocking file,
//! network and IPC work runs on tokio's blocking pool or on job threads.
use super::args::Args;
use super::jobs::{Job, JobKind};
use super::session::Session;
use super::update_stop::UpdateStop;
use super::{
    catalog, cli_tool, exec_tool, file_tools, file_write, job_view, response, screen_tool,
    search_tool, sessions_tool, status_tools, transfer_tools,
};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use std::sync::Arc;
use std::time::Duration;

/// How long the end of a session waits for cancelled jobs: one SSH TERM grace
/// period, inside the updater helper's 30-second wait for exiting processes.
const CLOSE_GRACE: Duration = Duration::from_secs(10);

#[derive(Clone)]
struct SeServer {
    session: Arc<Session>,
    tools: Arc<Vec<Tool>>,
}

impl ServerHandler for SeServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("smart-explorer", env!("CARGO_PKG_VERSION"))
                    .with_title("Smart Explorer"),
            )
            .with_instructions(catalog::INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(self.tools.as_ref().clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        if !self.tools.iter().any(|tool| tool.name == request.name) {
            return Err(McpError::invalid_params(
                format!("unknown tool `{}`", request.name),
                None,
            ));
        }
        let result = match dispatch(&self.session, &request, &context).await {
            Ok(result) => result,
            Err(message) => response::error(message),
        };
        Ok(result.into())
    }
}

async fn dispatch(
    session: &Arc<Session>,
    request: &CallToolRequestParams,
    context: &RequestContext<RoleServer>,
) -> Result<CallToolResult, String> {
    let args = Args::new(request.arguments.as_ref());
    let owned = request.arguments.clone();
    let session_for_work = session.clone();
    match request.name.as_ref() {
        "status" => blocking(move || status_tools::status(&session_for_work)).await,
        "connections" => blocking(status_tools::connections).await,
        "list" => {
            blocking(move || file_tools::list(&session_for_work, &Args::new(owned.as_ref()))).await
        }
        "stat" => {
            blocking(move || file_tools::stat(&session_for_work, &Args::new(owned.as_ref()))).await
        }
        "read" => {
            blocking(move || file_tools::read(&session_for_work, &Args::new(owned.as_ref()))).await
        }
        "mkdir" => {
            blocking(move || file_tools::mkdir(&session_for_work, &Args::new(owned.as_ref()))).await
        }
        "remove" => {
            blocking(move || file_tools::remove(&session_for_work, &Args::new(owned.as_ref())))
                .await
        }
        "write" => {
            blocking(move || file_write::write(&session_for_work, &Args::new(owned.as_ref()))).await
        }
        "edit" => {
            blocking(move || file_write::edit(&session_for_work, &Args::new(owned.as_ref()))).await
        }
        "copy" | "move" => {
            let kind = if request.name == "copy" {
                JobKind::Copy
            } else {
                JobKind::Move
            };
            let job = start(move || {
                transfer_tools::start(&session_for_work, kind, &Args::new(owned.as_ref()))
            })
            .await?;
            job_view::started_job_answer(&job, &args, context).await
        }
        "search" => {
            let job =
                start(move || search_tool::start(&session_for_work, &Args::new(owned.as_ref())))
                    .await?;
            job_view::started_job_answer(&job, &args, context).await
        }
        "exec" => {
            let job =
                start(move || exec_tool::start(&session_for_work, &Args::new(owned.as_ref())))
                    .await?;
            job_view::started_job_answer(&job, &args, context).await
        }
        "screen" => blocking(move || screen_tool::screen(&Args::new(owned.as_ref()))).await,
        "screen_input" => {
            blocking(move || screen_tool::screen_input(&Args::new(owned.as_ref()))).await
        }
        "se" => {
            let job = start(move || cli_tool::start(&session_for_work, &Args::new(owned.as_ref())))
                .await?;
            job_view::started_job_answer(&job, &args, context).await
        }
        "job_status" => job_view::status_tool(session, &args, context).await,
        "job_input" => job_view::input_tool(session, &args),
        "job_cancel" => job_view::cancel_tool(session, &args, context).await,
        "sessions" => {
            blocking(move || sessions_tool::sessions(&session_for_work, &Args::new(owned.as_ref())))
                .await
        }
        other => Err(format!("unknown tool `{other}`")),
    }
}

async fn blocking(
    work: impl FnOnce() -> Result<CallToolResult, String> + Send + 'static,
) -> Result<CallToolResult, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| format!("tool worker failed: {error}"))?
}

async fn start(
    work: impl FnOnce() -> Result<Arc<Job>, String> + Send + 'static,
) -> Result<Arc<Job>, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| format!("job start failed: {error}"))?
}

/// Runs the server until the client closes standard input or a desktop update
/// asks every `se` process to exit.
pub(super) fn serve() -> Result<i32, String> {
    let protocol = super::os::protect_stdout()
        .map_err(|error| format!("cannot open the MCP protocol channel: {error}"))?;
    #[cfg(debug_assertions)]
    if std::env::var_os("SE_MCP_TEST_STRAY_PRINT").is_some() {
        // Suite probe: this line must reach standard error, not the protocol.
        println!("se mcp test: stray standard output");
    }
    let stop = UpdateStop::arm();
    let session = Arc::new(Session::open()?);
    crate::cli::plugin::refresh_generated_files();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("se-mcp")
        .build()
        .map_err(|error| format!("cannot start the MCP runtime: {error}"))?;
    let server = SeServer {
        session: session.clone(),
        tools: Arc::new(catalog::tools()),
    };
    let outcome = runtime.block_on(async move {
        let writer = tokio::fs::File::from_std(protocol);
        let running = server
            .serve((tokio::io::stdin(), writer))
            .await
            .map_err(|error| format!("MCP session did not start: {error}"))?;
        tokio::select! {
            ended = running.waiting() => ended
                .map(|_| ())
                .map_err(|error| format!("MCP session failed: {error}")),
            () = stop.wait() => {
                eprintln!(
                    "se mcp: Smart Explorer is being updated; this session ends now. Reconnect the MCP server after the update."
                );
                Ok(())
            }
        }
    });
    session.close(CLOSE_GRACE);
    runtime.shutdown_timeout(Duration::from_secs(1));
    outcome.map(|()| 0)
}
