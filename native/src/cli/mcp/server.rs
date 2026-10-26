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
        let instructions = match crate::mcp_launcher::current_runtime() {
            Some(runtime) => format!("{}\nThis process uses immutable runtime {} of installation {} (generation {}). Its tool catalog stays fixed until this MCP process ends. New capabilities require the host to start a new MCP process.", catalog::INSTRUCTIONS, runtime.contract.runtime_sha256, runtime.installation.manifest.installation_id, runtime.installation.manifest.generation),
            None => catalog::INSTRUCTIONS.to_string(),
        };
        let instructions = if crate::mcp_launcher::migration_entry() {
            format!("{instructions}\nThis one-time migration was started through installed se. It uses the normal update-stop contract and will close for an update. Reconnect through the registered launcher after the profile refresh; this connection has no update retention exception.")
        } else {
            instructions
        };
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("smart-explorer", env!("CARGO_PKG_VERSION"))
                    .with_title("Smart Explorer"),
            )
            .with_instructions(instructions)
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
    let observation = matches!(
        request.name.as_ref(),
        "job_status" | "job_cancel" | "status"
    ) || (request.name == "sessions"
        && request
            .arguments
            .as_ref()
            .and_then(|a| a.get("action"))
            .and_then(|a| a.as_str())
            == Some("close"));
    let mut operation = if observation {
        None
    } else {
        let guard = crate::mcp_launcher::operation()?;
        let negotiation = session.clone();
        tokio::task::spawn_blocking(move || negotiation.negotiate_runtime())
            .await
            .map_err(|error| format!("runtime negotiation worker failed: {error}"))??;
        guard
    };
    let args = Args::new(request.arguments.as_ref());
    let owned = request.arguments.clone();
    let session_for_work = session.clone();
    match request.name.as_ref() {
        "status" => blocking_observe(move || status_tools::status(&session_for_work)).await,
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
            operation.take();
            job_view::started_job_answer(&job, &args, context).await
        }
        "search" => {
            let job =
                start(move || search_tool::start(&session_for_work, &Args::new(owned.as_ref())))
                    .await?;
            operation.take();
            job_view::started_job_answer(&job, &args, context).await
        }
        "exec" => {
            let job =
                start(move || exec_tool::start(&session_for_work, &Args::new(owned.as_ref())))
                    .await?;
            operation.take();
            job_view::started_job_answer(&job, &args, context).await
        }
        "screen" => blocking(move || screen_tool::screen(&Args::new(owned.as_ref()))).await,
        "screen_window" => {
            blocking(move || super::screen_window_tool::open(&Args::new(owned.as_ref()))).await
        }
        "tcp_forward" => {
            blocking(move || super::tcp_tool::tcp_forward(&Args::new(owned.as_ref()))).await
        }
        "apps" => blocking(move || super::apps_tool::apps(&Args::new(owned.as_ref()))).await,
        "login" => blocking(move || super::login_tool::login(&Args::new(owned.as_ref()))).await,
        "rdp" => {
            blocking(move || {
                super::rdp_tool::rdp(&Args::new(owned.as_ref()), |target| {
                    crate::daemon::apps_rdp_probe(
                        target,
                        Some(std::time::Duration::from_secs(30)),
                        || {},
                    )
                })
            })
            .await
        }
        "screen_input" => {
            blocking(move || screen_tool::screen_input(&Args::new(owned.as_ref()))).await
        }
        "se" => {
            let job = start(move || cli_tool::start(&session_for_work, &Args::new(owned.as_ref())))
                .await?;
            operation.take();
            job_view::started_job_answer(&job, &args, context).await
        }
        "job_status" => job_view::status_tool(session, &args, context).await,
        "job_input" => job_view::input_tool(session, &args),
        "job_cancel" => job_view::cancel_tool(session, &args, context).await,
        "sessions" => {
            if observation {
                blocking_observe(move || {
                    sessions_tool::sessions(&session_for_work, &Args::new(owned.as_ref()))
                })
                .await
            } else {
                blocking(move || {
                    sessions_tool::sessions(&session_for_work, &Args::new(owned.as_ref()))
                })
                .await
            }
        }
        other => Err(format!("unknown tool `{other}`")),
    }
}

async fn blocking(
    work: impl FnOnce() -> Result<CallToolResult, String> + Send + 'static,
) -> Result<CallToolResult, String> {
    tokio::task::spawn_blocking(move || {
        let _operation = crate::mcp_launcher::operation()?;
        work()
    })
    .await
    .map_err(|error| format!("tool worker failed: {error}"))?
}

async fn blocking_observe(
    work: impl FnOnce() -> Result<CallToolResult, String> + Send + 'static,
) -> Result<CallToolResult, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| format!("status worker failed: {error}"))?
}

async fn start(
    work: impl FnOnce() -> Result<Arc<Job>, String> + Send + 'static,
) -> Result<Arc<Job>, String> {
    tokio::task::spawn_blocking(move || {
        let _operation = crate::mcp_launcher::operation()?;
        work()
    })
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
    let lifecycle = crate::mcp_launcher::LifecycleWatch::arm()?;
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
    let watch_session = session.clone();
    let outcome = runtime.block_on(async move {
        let writer = tokio::fs::File::from_std(protocol);
        let running = server
            .serve((tokio::io::stdin(), writer))
            .await
            .map_err(|error| format!("MCP session did not start: {error}"))?;
        let negotiate_idle=watch_session.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(error)=negotiate_idle.negotiate_runtime() {eprintln!("se mcp: {error}");}
        });
        tokio::select! {
            ended = running.waiting() => ended
                .map(|_| ())
                .map_err(|error| format!("MCP session failed: {error}")),
            () = stop.wait(watch_session, lifecycle) => {
                eprintln!(
                    "se mcp: Smart Explorer is being updated; this session ends now. Reconnect the MCP server after the update."
                );
                Ok(())
            }
        }
    });
    session.close(CLOSE_GRACE);
    super::rdp_shutdown::drain();
    super::login_shutdown::drain();
    runtime.shutdown_timeout(Duration::from_secs(1));
    outcome.map(|()| 0)
}
