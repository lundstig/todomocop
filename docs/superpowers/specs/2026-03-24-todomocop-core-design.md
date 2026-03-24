# Todomocop Core Design

Personal TODO management system with an MCP interface for Claude Code. Manages tasks from multiple sources (manual, Slack, GitHub PRs, Linear tickets, screenshots) in a unified SQLite database, queryable through lightweight MCP tools.

## Architecture

Cargo workspace with three crates:

```
todomocop/
├── Cargo.toml              # workspace root
├── flake.nix
├── .envrc
├── core/                   # todomocop-core (library)
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs
│   │   ├── schema.rs       # rust-query schema definition
│   │   ├── task.rs          # task CRUD operations
│   │   ├── attachment.rs    # attachment operations
│   │   ├── github.rs        # GitHub PR context operations
│   │   ├── linear.rs        # Linear context operations
│   │   └── query.rs         # compound queries (list_tasks, search)
│   └── tests/               # integration tests (in-memory DB)
├── mcp/                    # todomocop-mcp (binary)
│   ├── Cargo.toml
│   └── src/
│       └── main.rs          # MCP server over stdio JSON-RPC
└── cli/                    # todomocop-cli (binary)
    ├── Cargo.toml
    └── src/
        └── main.rs
```

**core** is a pure library owning all DB logic and external refresh. **mcp** and **cli** are thin wrappers.

## Data Model

All stored in a single SQLite database. Workspace is a column, not a separate DB — sync (future) will filter by workspace.

### tasks

| Column       | Type              | Notes                                         |
|--------------|-------------------|-----------------------------------------------|
| title        | String            | Required                                      |
| description  | String            | Defaults to empty                             |
| status       | String            | `idea`, `ready`, `in_progress`, `done`        |
| priority     | Option\<i64\>     | 0 = critical, 3 = low, None = unset           |
| workspace    | String            | e.g. `"personal"`, `"work"`                   |
| deadline     | Option\<String\>  | ISO 8601 date                                 |
| snooze_until | Option\<String\>  | ISO 8601 date, hidden from queries until then |
| planned_date | Option\<String\>  | ISO 8601 date, when you plan to work on it    |
| deleted_at   | Option\<String\>  | ISO 8601 timestamp, soft delete               |
| created_at   | String            | ISO 8601 timestamp                            |
| updated_at   | String            | ISO 8601 timestamp                            |

Status is stored as a string in SQLite, represented as a Rust enum with serialization.

### attachments

| Column       | Type           | Notes                          |
|--------------|----------------|--------------------------------|
| task         | FK → tasks     |                                |
| file_name    | String         | e.g. `screenshot.png`          |
| content_type | String         | e.g. `image/png`               |
| data         | Vec\<u8\>      | Raw file bytes stored as blob  |
| caption      | String         | AI-generated or manual         |
| created_at   | String         | ISO 8601 timestamp             |

### github_pr_context

| Column         | Type           | Notes                        |
|----------------|----------------|------------------------------|
| task           | FK → tasks     |                              |
| url            | String         | PR URL                       |
| repo           | String         | `owner/repo`                 |
| number         | i64            | PR number                    |
| state          | String         | Cached: open/closed/merged   |
| last_refreshed | String         | ISO 8601 timestamp           |

### linear_context

| Column         | Type           | Notes                              |
|----------------|----------------|------------------------------------|
| task           | FK → tasks     |                                    |
| url            | String         | Linear issue URL                   |
| identifier     | String         | e.g. `ENG-123`                     |
| data           | String         | Full Linear API response as JSON   |
| last_refreshed | String         | ISO 8601 timestamp                 |

## Core Library API

```rust
pub struct Db { /* owns rust-query Database handle + HttpClient */ }

impl Db {
    pub fn open(path: &Path, http: impl HttpClient) -> Result<Self>;

    // Tasks
    pub fn add_task(&self, params: AddTask) -> Result<TaskId>;
    pub fn edit_task(&self, id: TaskId, params: EditTask) -> Result<()>;
    pub fn delete_task(&self, id: TaskId) -> Result<()>;
    pub fn list_tasks(&self, filter: TaskFilter) -> Result<Vec<Task>>;
    pub fn search(&self, query: &str, filter: TaskFilter) -> Result<Vec<Task>>;
    pub fn snooze(&self, id: TaskId, until: NaiveDate) -> Result<()>;

    // Attachments
    pub fn add_attachment(&self, task_id: TaskId, att: NewAttachment) -> Result<AttachmentId>;

    // External links
    pub fn link_github_pr(&self, task_id: TaskId, url: &str) -> Result<()>;
    pub fn link_linear(&self, task_id: TaskId, url: &str) -> Result<()>;
}
```

### Key types

- **TaskFilter**: Optional fields — `status`, `workspace`, `has_planned_date` (bool), `priority_max`, `include_snoozed` (default false). Deleted tasks always excluded.
- **Task**: All task fields plus optionally populated `github_pr_context` and `linear_context`.
- **AddTask**: `title` required, everything else optional with sensible defaults (status=idea, no priority).
- **EditTask**: All fields optional — only set fields are updated.

### HttpClient trait

```rust
pub trait HttpClient: Send + Sync {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>>;
}
```

Core uses this for GitHub/Linear refresh. Binaries inject a real client (e.g. `ureq`). Tests inject a mock. Core never reads environment variables — tokens are passed as config.

## External Context Refresh

Refresh logic lives in core, transparent to callers.

When `list_tasks` returns tasks with a `github_pr_context` or `linear_context`:
1. Check `last_refreshed` against a configurable staleness threshold (default: 5 minutes)
2. If stale, fetch fresh data via the `HttpClient`
3. Update the context row in the DB
4. Return fresh data

If tokens are not configured or the API call fails, return stale data (not an error). Auth tokens (`GITHUB_TOKEN`, `LINEAR_API_KEY`) are read by the binary and passed as config to core.

## MCP Interface

Stdio JSON-RPC transport. Tools:

| Tool              | Args                                                                 | Description                                |
|-------------------|----------------------------------------------------------------------|--------------------------------------------|
| `add_task`        | title, description?, priority?, workspace?, deadline?, planned_date?, status? | Create a task, returns ID           |
| `edit_task`       | id, any subset of task fields                                        | Update fields                              |
| `delete_task`     | id                                                                   | Soft-delete (sets deleted_at)              |
| `list_tasks`      | status?, workspace?, has_planned_date?, priority_max?, include_snoozed? | Flexible query, sorted by priority then deadline |
| `search`          | query, status?, workspace?                                           | Substring match on title + description     |
| `snooze`          | id, until (ISO 8601 date)                                           | Hide task until date                       |
| `add_attachment`  | task_id, file_name, content_type, data (base64)                     | Attach a file                              |
| `link_github_pr`  | task_id, url                                                         | Link a GitHub PR to a task                 |
| `link_linear`     | task_id, url                                                         | Link a Linear issue to a task              |

## Dev Environment

- **Nix flake** with stable Rust toolchain and SQLite. Exact flake pattern to be researched during implementation (not the ESP32-style `eachDefaultSystem`).
- **.envrc**: `use flake`
- **DB location**: `~/.local/share/todomocop/todomocop.db`, overridable via `--db` flag or `TODOMOCOP_DB` env var.

## Testing

Integration tests in `core/tests/`. Each test opens an in-memory SQLite database — fast, isolated, no cleanup. The `HttpClient` trait is mocked for testing refresh logic.

## Future Work (out of scope)

- **Sync**: CRDT-style merge between local and remote DBs, filtered by workspace.
- **Recurrence**: Separate table with cron expressions, materializes tasks on query.
- **Slack ingestion**: Fetch reminders from Slack API, summarize, create tasks.
- **Screenshot capture**: macOS app, captures screenshot, AI-captions, creates task with attachment.
- **Full-text search**: Upgrade from LIKE to SQLite FTS5.
