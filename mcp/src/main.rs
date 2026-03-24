use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::*,
    schemars, tool, tool_handler, tool_router,
    transport::io::stdio,
};
use serde::Deserialize;
use todomocop_core::http::{HttpClient, IntegrationConfig};
use todomocop_core::types::{AddTask, EditTask, NewAttachment, TaskFilter, TaskStatus};
use todomocop_core::Db;

// ---------------------------------------------------------------------------
// UreqHttpClient
// ---------------------------------------------------------------------------

struct UreqHttpClient;

impl HttpClient for UreqHttpClient {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> anyhow::Result<Vec<u8>> {
        let mut req = ureq::get(url);
        for &(name, value) in headers {
            req = req.header(name, value);
        }
        let mut resp = req.call()?;
        let body = resp.body_mut().read_to_vec()?;
        Ok(body)
    }

    fn post(&self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut req = ureq::post(url);
        for &(name, value) in headers {
            req = req.header(name, value);
        }
        let mut resp = req.send(body)?;
        let resp_body = resp.body_mut().read_to_vec()?;
        Ok(resp_body)
    }
}

// ---------------------------------------------------------------------------
// DbHandle: send commands to a dedicated thread owning the Db
// ---------------------------------------------------------------------------

type DbFn = Box<dyn FnOnce(&Db) -> anyhow::Result<String> + Send>;

/// A Send+Sync handle that dispatches closures to a dedicated thread owning the Db.
#[derive(Clone)]
struct DbHandle {
    tx: std::sync::mpsc::Sender<(DbFn, std::sync::mpsc::Sender<anyhow::Result<String>>)>,
}

impl DbHandle {
    fn new(db_path: PathBuf, http: Arc<dyn HttpClient>, config: IntegrationConfig) -> anyhow::Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel::<(DbFn, std::sync::mpsc::Sender<anyhow::Result<String>>)>();

        std::thread::spawn(move || {
            let db = match Db::open(&db_path, http, config) {
                Ok(db) => db,
                Err(e) => {
                    eprintln!("Failed to open database: {e:#}");
                    return;
                }
            };

            while let Ok((f, reply_tx)) = rx.recv() {
                let result = f(&db);
                let _ = reply_tx.send(result);
            }
        });

        // Verify the db thread started successfully by doing a no-op call
        // We can't easily check, so we trust the thread. If it fails, first real call will error.

        Ok(Self { tx })
    }

    fn run<F>(&self, f: F) -> anyhow::Result<String>
    where
        F: FnOnce(&Db) -> anyhow::Result<String> + Send + 'static,
    {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        self.tx
            .send((Box::new(f), reply_tx))
            .map_err(|_| anyhow::anyhow!("database thread has shut down"))?;
        reply_rx
            .recv()
            .map_err(|_| anyhow::anyhow!("database thread did not respond"))?
    }
}

// ---------------------------------------------------------------------------
// Tool parameter structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AddTaskParams {
    #[schemars(description = "Title of the task")]
    title: String,
    #[schemars(description = "Description of the task")]
    description: Option<String>,
    #[schemars(description = "Priority (0 = critical, higher = lower priority)")]
    priority: Option<i64>,
    #[schemars(description = "Workspace name (default: personal)")]
    workspace: Option<String>,
    #[schemars(description = "Deadline date string (ISO 8601)")]
    deadline: Option<String>,
    #[schemars(description = "Planned date string (ISO 8601)")]
    planned_date: Option<String>,
    #[schemars(description = "Status: idea, ready, in_progress, or done")]
    status: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EditTaskParams {
    #[schemars(description = "Task ID to edit")]
    id: i64,
    #[schemars(description = "New title")]
    title: Option<String>,
    #[schemars(description = "New description")]
    description: Option<String>,
    #[schemars(description = "New status: idea, ready, in_progress, or done")]
    status: Option<String>,
    #[schemars(description = "New priority (null to clear)")]
    priority: Option<Option<i64>>,
    #[schemars(description = "New workspace")]
    workspace: Option<String>,
    #[schemars(description = "New deadline (null to clear)")]
    deadline: Option<Option<String>>,
    #[schemars(description = "New planned date (null to clear)")]
    planned_date: Option<Option<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DeleteTaskParams {
    #[schemars(description = "Task ID to delete")]
    id: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListTasksParams {
    #[schemars(description = "Filter by status: idea, ready, in_progress, or done")]
    status: Option<String>,
    #[schemars(description = "Filter by workspace")]
    workspace: Option<String>,
    #[schemars(description = "Filter by whether task has a planned date")]
    has_planned_date: Option<bool>,
    #[schemars(description = "Filter tasks with priority <= this value")]
    priority_max: Option<i64>,
    #[schemars(description = "Include snoozed tasks (default: false)")]
    include_snoozed: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchParams {
    #[schemars(description = "Search query (matches title and description)")]
    query: String,
    #[schemars(description = "Filter by status: idea, ready, in_progress, or done")]
    status: Option<String>,
    #[schemars(description = "Filter by workspace")]
    workspace: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SnoozeParams {
    #[schemars(description = "Task ID to snooze")]
    id: i64,
    #[schemars(description = "Snooze until this date (ISO 8601 date string, e.g. 2026-04-01)")]
    until: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AddAttachmentParams {
    #[schemars(description = "Task ID to attach to")]
    task_id: i64,
    #[schemars(description = "File name for the attachment")]
    file_name: String,
    #[schemars(description = "MIME content type (e.g. image/png)")]
    content_type: String,
    #[schemars(description = "Base64-encoded file data")]
    data: String,
    #[schemars(description = "Optional caption for the attachment")]
    caption: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LinkGithubPrParams {
    #[schemars(description = "Task ID to link")]
    task_id: i64,
    #[schemars(description = "GitHub PR URL (e.g. https://github.com/owner/repo/pull/123)")]
    url: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LinkLinearParams {
    #[schemars(description = "Task ID to link")]
    task_id: i64,
    #[schemars(description = "Linear issue URL (e.g. https://linear.app/team/issue/ENG-123/title)")]
    url: String,
}

// ---------------------------------------------------------------------------
// Server struct
// ---------------------------------------------------------------------------

struct TodomocopServer {
    db: DbHandle,
    tool_router: ToolRouter<TodomocopServer>,
}

// ---------------------------------------------------------------------------
// Helper to convert anyhow::Error to McpError
// ---------------------------------------------------------------------------

fn to_mcp_error(e: anyhow::Error) -> McpError {
    McpError::internal_error(format!("{e:#}"), None)
}

fn parse_status(s: &str) -> Result<TaskStatus, McpError> {
    s.parse::<TaskStatus>()
        .map_err(|e| McpError::invalid_params(format!("{e}"), None))
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

#[tool_router]
impl TodomocopServer {
    fn new(db: DbHandle) -> Self {
        Self {
            db,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Add a new task")]
    fn add_task(
        &self,
        Parameters(params): Parameters<AddTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let status = params.status.map(|s| parse_status(&s)).transpose()?;

        let add = AddTask {
            title: params.title,
            description: params.description,
            status,
            priority: params.priority,
            workspace: params.workspace.unwrap_or_else(|| "personal".into()),
            deadline: params.deadline,
            planned_date: params.planned_date,
        };

        let result = self.db.run(move |db| {
            let id = db.add_task(add)?;
            Ok(format!("Created task #{id}"))
        }).map_err(to_mcp_error)?;

        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    #[tool(description = "Edit an existing task")]
    fn edit_task(
        &self,
        Parameters(params): Parameters<EditTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let status = params.status.map(|s| parse_status(&s)).transpose()?;
        let id = params.id;

        let edit = EditTask {
            title: params.title,
            description: params.description,
            status,
            priority: params.priority,
            workspace: params.workspace,
            deadline: params.deadline,
            snooze_until: None,
            planned_date: params.planned_date,
        };

        let result = self.db.run(move |db| {
            db.edit_task(id, edit)?;
            Ok(format!("Updated task #{id}"))
        }).map_err(to_mcp_error)?;

        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    #[tool(description = "Delete a task (soft delete)")]
    fn delete_task(
        &self,
        Parameters(params): Parameters<DeleteTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = params.id;
        let result = self.db.run(move |db| {
            db.delete_task(id)?;
            Ok(format!("Deleted task #{id}"))
        }).map_err(to_mcp_error)?;

        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    #[tool(description = "List tasks with optional filters")]
    fn list_tasks(
        &self,
        Parameters(params): Parameters<ListTasksParams>,
    ) -> Result<CallToolResult, McpError> {
        let status = params.status.map(|s| parse_status(&s)).transpose()?;

        let filter = TaskFilter {
            status,
            workspace: params.workspace,
            has_planned_date: params.has_planned_date,
            priority_max: params.priority_max,
            include_snoozed: params.include_snoozed.unwrap_or(false),
        };

        let result = self.db.run(move |db| {
            let tasks = db.list_tasks(filter)?;
            Ok(serde_json::to_string_pretty(&tasks)?)
        }).map_err(to_mcp_error)?;

        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    #[tool(description = "Search tasks by title and description")]
    fn search(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let status = params.status.map(|s| parse_status(&s)).transpose()?;

        let filter = TaskFilter {
            status,
            workspace: params.workspace,
            ..Default::default()
        };
        let query = params.query;

        let result = self.db.run(move |db| {
            let tasks = db.search(&query, filter)?;
            Ok(serde_json::to_string_pretty(&tasks)?)
        }).map_err(to_mcp_error)?;

        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    #[tool(description = "Snooze a task until a given date")]
    fn snooze(
        &self,
        Parameters(params): Parameters<SnoozeParams>,
    ) -> Result<CallToolResult, McpError> {
        let until = params
            .until
            .parse::<chrono::NaiveDate>()
            .map_err(|e| McpError::invalid_params(format!("invalid date: {e}"), None))?;

        let id = params.id;
        let until_str = params.until.clone();

        let result = self.db.run(move |db| {
            db.snooze(id, until)?;
            Ok(format!("Snoozed task #{id} until {until_str}"))
        }).map_err(to_mcp_error)?;

        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    #[tool(description = "Add a file attachment to a task")]
    fn add_attachment(
        &self,
        Parameters(params): Parameters<AddAttachmentParams>,
    ) -> Result<CallToolResult, McpError> {
        let data = base64::engine::general_purpose::STANDARD
            .decode(&params.data)
            .map_err(|e| McpError::invalid_params(format!("invalid base64: {e}"), None))?;

        let task_id = params.task_id;
        let att = NewAttachment {
            file_name: params.file_name,
            content_type: params.content_type,
            data,
            caption: params.caption,
        };

        let result = self.db.run(move |db| {
            let att_id = db.add_attachment(task_id, att)?;
            Ok(format!("Added attachment #{att_id} to task #{task_id}"))
        }).map_err(to_mcp_error)?;

        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    #[tool(description = "Link a GitHub pull request to a task")]
    fn link_github_pr(
        &self,
        Parameters(params): Parameters<LinkGithubPrParams>,
    ) -> Result<CallToolResult, McpError> {
        let task_id = params.task_id;
        let url = params.url;

        let result = self.db.run(move |db| {
            db.link_github_pr(task_id, &url)?;
            Ok(format!("Linked GitHub PR to task #{task_id}"))
        }).map_err(to_mcp_error)?;

        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    #[tool(description = "Link a Linear issue to a task")]
    fn link_linear(
        &self,
        Parameters(params): Parameters<LinkLinearParams>,
    ) -> Result<CallToolResult, McpError> {
        let task_id = params.task_id;
        let url = params.url;

        let result = self.db.run(move |db| {
            db.link_linear(task_id, &url)?;
            Ok(format!("Linked Linear issue to task #{task_id}"))
        }).map_err(to_mcp_error)?;

        Ok(CallToolResult::success(vec![Content::text(result)]))
    }
}

// ---------------------------------------------------------------------------
// ServerHandler implementation
// ---------------------------------------------------------------------------

#[tool_handler]
impl ServerHandler for TodomocopServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("todomocop", env!("CARGO_PKG_VERSION")))
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_instructions(
                "Task management server. Use add_task, edit_task, delete_task, list_tasks, \
                 search, snooze, add_attachment, link_github_pr, and link_linear to manage tasks."
                    .to_string(),
            )
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let db_path = std::env::var("TODOMOCOP_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("todomocop")
                .join("todomocop.db")
        });

    // Ensure parent directory exists
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let github_token = std::env::var("GITHUB_TOKEN").ok();
    let linear_api_key = std::env::var("LINEAR_API_KEY").ok();

    let config = IntegrationConfig {
        github_token,
        linear_api_key,
        ..Default::default()
    };

    let http: Arc<dyn HttpClient> = Arc::new(UreqHttpClient);
    let db = DbHandle::new(db_path, http, config)?;
    let server = TodomocopServer::new(db);

    let transport = stdio();
    let service = server.serve(transport).await?;
    service.waiting().await?;

    Ok(())
}
