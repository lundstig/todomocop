# Todomocop Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the core library, MCP server, and CLI for a personal TODO management system backed by SQLite.

**Architecture:** Cargo workspace with `core` (library), `mcp` (binary), `cli` (binary). Core owns all DB logic via rust-query. MCP server uses rmcp for stdio JSON-RPC. CLI uses clap.

**Tech Stack:** Rust (stable), rust-query (SQLite ORM), rmcp 1.2+ (MCP SDK), clap (CLI), chrono (dates), serde/serde_json, ureq (HTTP client)

**Spec:** `docs/superpowers/specs/2026-03-24-todomocop-core-design.md`

---

## File Structure

```
todomocop/
├── Cargo.toml                  # workspace root
├── flake.nix                   # nix dev shell (rust-overlay + sqlite)
├── .envrc                      # direnv: use flake
├── .gitignore
├── core/
│   ├── Cargo.toml              # todomocop-core library
│   └── src/
│       ├── lib.rs              # re-exports, Db struct, open/migrate
│       ├── schema.rs           # rust-query schema (all tables)
│       ├── types.rs            # TaskStatus enum, AddTask, EditTask, Task, TaskFilter, etc.
│       ├── task.rs             # add_task, edit_task, delete_task, snooze
│       ├── query.rs            # list_tasks, search
│       ├── attachment.rs       # add_attachment
│       ├── http.rs             # HttpClient trait + config
│       ├── github.rs           # link_github_pr, refresh logic
│       └── linear.rs           # link_linear, refresh logic
├── mcp/
│   ├── Cargo.toml              # todomocop-mcp binary
│   └── src/
│       └── main.rs             # rmcp server, stdio transport, all tools
└── cli/
    ├── Cargo.toml              # todomocop-cli binary
    └── src/
        └── main.rs             # clap subcommands
```

---

## Task 1: Project Scaffolding

**Files:**
- Create: `Cargo.toml`
- Create: `core/Cargo.toml`
- Create: `core/src/lib.rs`
- Create: `mcp/Cargo.toml`
- Create: `mcp/src/main.rs`
- Create: `cli/Cargo.toml`
- Create: `cli/src/main.rs`
- Create: `flake.nix`
- Create: `.envrc`
- Create: `.gitignore`

- [ ] **Step 1: Create workspace Cargo.toml**

```toml
[workspace]
members = ["core", "mcp", "cli"]
resolver = "2"
```

- [ ] **Step 2: Create core/Cargo.toml**

```toml
[package]
name = "todomocop-core"
version = "0.1.0"
edition = "2021"

[dependencies]
rust-query = "0.4"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
anyhow = "1"
```

- [ ] **Step 3: Create core/src/lib.rs (stub)**

```rust
pub fn hello() -> &'static str {
    "todomocop-core"
}
```

- [ ] **Step 4: Create mcp/Cargo.toml**

```toml
[package]
name = "todomocop-mcp"
version = "0.1.0"
edition = "2021"

[dependencies]
todomocop-core = { path = "../core" }
rmcp = { version = "1.2", features = ["server", "transport-io"] }
schemars = "1.0"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
anyhow = "1"
ureq = "3"
clap = { version = "4", features = ["derive"] }
```

- [ ] **Step 5: Create mcp/src/main.rs (stub)**

```rust
fn main() {
    println!("todomocop-mcp");
}
```

- [ ] **Step 6: Create cli/Cargo.toml**

```toml
[package]
name = "todomocop-cli"
version = "0.1.0"
edition = "2021"

[dependencies]
todomocop-core = { path = "../core" }
clap = { version = "4", features = ["derive"] }
serde_json = "1"
anyhow = "1"
ureq = "3"
```

- [ ] **Step 7: Create cli/src/main.rs (stub)**

```rust
fn main() {
    println!("todomocop-cli");
}
```

- [ ] **Step 8: Create flake.nix**

Uses rust-overlay for stable Rust toolchain. Research confirmed this is the idiomatic pattern.

```nix
{
  description = "todomocop - personal TODO management with MCP";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };
          rustToolchain = pkgs.rust-bin.stable.latest.default;
        in
        {
          default = pkgs.mkShell {
            nativeBuildInputs = [
              rustToolchain
              pkgs.pkg-config
            ];
            buildInputs = [
              pkgs.sqlite
            ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
              pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
            ];
          };
        }
      );
    };
}
```

- [ ] **Step 9: Create .envrc**

```
use flake
```

- [ ] **Step 10: Create .gitignore**

```
/target
.direnv
```

- [ ] **Step 11: Verify the project compiles**

Run: `cargo check`
Expected: Compiles with no errors.

- [ ] **Step 12: Commit**

```bash
git add Cargo.toml core/ mcp/ cli/ flake.nix .envrc .gitignore
git commit -m "feat: scaffold cargo workspace, nix flake, and crate stubs"
```

---

## Task 2: Schema, Types & DB Wrapper

**Files:**
- Create: `core/src/schema.rs`
- Create: `core/src/types.rs`
- Create: `core/src/http.rs`
- Modify: `core/src/lib.rs`

- [ ] **Step 1: Write test — open in-memory database**

Add to `core/src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let db = Db::open_in_memory();
        assert!(db.is_ok());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p todomocop-core`
Expected: FAIL — `Db` doesn't exist yet.

- [ ] **Step 3: Create core/src/schema.rs**

Defines all tables using rust-query's schema macro. Each task has an explicit `id: i64` with `#[unique]` so we have a stable external identifier.

**Note:** rust-query's exact macro syntax should be verified against docs. The `#[unique]` attribute ensures we can look up tasks by ID. `TableRow<Task>` on foreign key columns creates the FK relationship. `Option<T>` makes columns nullable. `Vec<u8>` stores as a SQLite blob.

```rust
use rust_query::TableRow;

#[rust_query::schema(TodoSchema)]
#[version(0..=0)]
pub mod vN {
    pub struct Task {
        #[unique]
        pub id: i64,
        pub title: String,
        pub description: String,
        pub status: String,
        pub priority: Option<i64>,
        pub workspace: String,
        pub deadline: Option<String>,
        pub snooze_until: Option<String>,
        pub planned_date: Option<String>,
        pub deleted_at: Option<String>,
        pub created_at: String,
        pub updated_at: String,
    }

    pub struct Attachment {
        #[unique]
        pub id: i64,
        pub task: TableRow<Task>,
        pub file_name: String,
        pub content_type: String,
        pub data: Vec<u8>,
        pub caption: String,
        pub created_at: String,
    }

    pub struct GithubPrContext {
        pub task: TableRow<Task>,
        pub url: String,
        pub repo: String,
        pub number: i64,
        pub state: String,
        pub last_refreshed: String,
    }

    pub struct LinearContext {
        pub task: TableRow<Task>,
        pub url: String,
        pub identifier: String,
        pub data: String,
        pub last_refreshed: String,
    }
}

pub use v0::*;
```

- [ ] **Step 4: Create core/src/types.rs**

API types independent of the storage layer.

```rust
use serde::{Deserialize, Serialize};
use std::fmt;

/// Task status. Stored as a string in SQLite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Idea,
    Ready,
    InProgress,
    Done,
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskStatus::Idea => write!(f, "idea"),
            TaskStatus::Ready => write!(f, "ready"),
            TaskStatus::InProgress => write!(f, "in_progress"),
            TaskStatus::Done => write!(f, "done"),
        }
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "idea" => Ok(TaskStatus::Idea),
            "ready" => Ok(TaskStatus::Ready),
            "in_progress" => Ok(TaskStatus::InProgress),
            "done" => Ok(TaskStatus::Done),
            other => anyhow::bail!("unknown status: {other}"),
        }
    }
}

pub type TaskId = i64;
pub type AttachmentId = i64;

/// Parameters for creating a new task.
#[derive(Debug, Clone)]
pub struct AddTask {
    pub title: String,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub priority: Option<i64>,
    pub workspace: String,
    pub deadline: Option<String>,
    pub planned_date: Option<String>,
}

/// Parameters for editing a task. Only `Some` fields are updated.
#[derive(Debug, Clone, Default)]
pub struct EditTask {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub priority: Option<Option<i64>>,  // Some(None) = clear priority, None = don't touch
    pub workspace: Option<String>,
    pub deadline: Option<Option<String>>,
    pub planned_date: Option<Option<String>>,
    pub snooze_until: Option<Option<String>>,
}

/// A task as returned by queries.
#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub priority: Option<i64>,
    pub workspace: String,
    pub deadline: Option<String>,
    pub snooze_until: Option<String>,
    pub planned_date: Option<String>,
    pub deleted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub github_pr_context: Option<GithubPrContextData>,
    pub linear_context: Option<LinearContextData>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GithubPrContextData {
    pub url: String,
    pub repo: String,
    pub number: i64,
    pub state: String,
    pub last_refreshed: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinearContextData {
    pub url: String,
    pub identifier: String,
    pub data: serde_json::Value,
    pub last_refreshed: String,
}

/// Filter for list_tasks queries. All fields optional.
#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    pub status: Option<TaskStatus>,
    pub workspace: Option<String>,
    pub has_planned_date: Option<bool>,
    pub priority_max: Option<i64>,
    pub include_snoozed: bool,
}

/// Parameters for adding an attachment.
#[derive(Debug, Clone)]
pub struct NewAttachment {
    pub file_name: String,
    pub content_type: String,
    pub data: Vec<u8>,
    pub caption: Option<String>,
}
```

- [ ] **Step 5: Create core/src/http.rs**

```rust
use anyhow::Result;
use std::time::Duration;

/// Trait for HTTP requests. Injected into Db for testability.
pub trait HttpClient: Send + Sync {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>>;
}

/// No-op HTTP client for when external refresh is not needed.
pub struct NoopHttpClient;

impl HttpClient for NoopHttpClient {
    fn get(&self, _url: &str, _headers: &[(&str, &str)]) -> Result<Vec<u8>> {
        anyhow::bail!("HTTP client not configured")
    }
}

/// Configuration for external integrations.
#[derive(Debug, Clone)]
pub struct IntegrationConfig {
    pub github_token: Option<String>,
    pub linear_api_key: Option<String>,
    pub staleness_threshold: Duration,
}

impl Default for IntegrationConfig {
    fn default() -> Self {
        Self {
            github_token: None,
            linear_api_key: None,
            staleness_threshold: Duration::from_secs(300), // 5 minutes
        }
    }
}
```

- [ ] **Step 6: Create Db wrapper in core/src/lib.rs**

```rust
pub mod schema;
pub mod types;
pub mod http;

use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

pub use types::*;
pub use http::{HttpClient, IntegrationConfig, NoopHttpClient};

pub struct Db {
    db: rust_query::Database<schema::TodoSchema>,
    http: Arc<dyn HttpClient>,
    config: IntegrationConfig,
}

impl Db {
    /// Open a database at the given path.
    pub fn open(path: &Path, http: Arc<dyn HttpClient>, config: IntegrationConfig) -> Result<Self> {
        let db = rust_query::Config::open(path)
            .finish()
            .ok_or_else(|| anyhow::anyhow!("database schema version too new"))?;
        Ok(Self { db, http, config })
    }

    /// Open an in-memory database (for tests).
    pub fn open_in_memory() -> Result<Self> {
        Self::open(
            Path::new(":memory:"),
            Arc::new(NoopHttpClient),
            IntegrationConfig::default(),
        )
    }

    /// Generate the next task ID inside a transaction.
    /// Uses max(id) + 1. Safe because we're inside a transaction.
    fn next_task_id(/* txn */) -> i64 {
        // Implementation: query max task id, return + 1 (or 1 if empty)
        // Exact rust-query aggregation syntax to be determined
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let db = Db::open_in_memory();
        assert!(db.is_ok());
    }
}
```

- [ ] **Step 7: Run test to verify it passes**

Run: `cargo test -p todomocop-core`
Expected: PASS

**Note:** The `open` method's exact rust-query API may need adjustment. `rust_query::Config::open()` may take a `&str` instead of `&Path`, and `.finish()` returns `Option<Database<S>>`. Verify and adjust during implementation.

- [ ] **Step 8: Commit**

```bash
git add core/src/
git commit -m "feat: add schema, types, and DB wrapper"
```

---

## Task 3: Task CRUD (add, edit, delete, snooze)

**Files:**
- Create: `core/src/task.rs`
- Modify: `core/src/lib.rs` (add `pub mod task;` and Db methods)

- [ ] **Step 1: Write test — add_task creates a task and returns an ID**

Create `core/src/task.rs` with the module declaration, and add to the test in `lib.rs` or create a test file:

```rust
// In core/src/lib.rs tests, or core/tests/task_crud.rs
#[test]
fn test_add_task() {
    let db = Db::open_in_memory().unwrap();
    let id = db.add_task(AddTask {
        title: "Buy groceries".into(),
        description: None,
        status: None,
        priority: None,
        workspace: "personal".into(),
        deadline: None,
        planned_date: None,
    }).unwrap();
    assert_eq!(id, 1);

    // Verify we can retrieve it
    let tasks = db.list_tasks(TaskFilter::default()).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "Buy groceries");
    assert_eq!(tasks[0].status, TaskStatus::Idea); // default
    assert_eq!(tasks[0].workspace, "personal");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p todomocop-core test_add_task`
Expected: FAIL — `add_task` not implemented.

- [ ] **Step 3: Implement add_task in core/src/task.rs**

```rust
use crate::{Db, AddTask, TaskId, TaskStatus};
use anyhow::Result;
use chrono::Utc;

impl Db {
    pub fn add_task(&self, params: AddTask) -> Result<TaskId> {
        let now = Utc::now().to_rfc3339();
        let status = params.status.unwrap_or(TaskStatus::Idea).to_string();

        self.db.transaction_mut(|txn| {
            // Generate next ID
            let next_id = /* query max id + 1, or 1 if empty */;

            txn.insert_ok(crate::schema::Task {
                id: next_id,
                title: params.title,
                description: params.description.unwrap_or_default(),
                status,
                priority: params.priority,
                workspace: params.workspace,
                deadline: params.deadline,
                snooze_until: None,
                planned_date: params.planned_date,
                deleted_at: None,
                created_at: now.clone(),
                updated_at: now,
            });

            Ok(next_id)
        })
    }
}
```

**Note:** The exact rust-query transaction API (`transaction_mut` vs `transaction_mut_ok`) and ID generation (aggregate query for max id) need to be verified against the rust-query docs. The pattern is correct but syntax may need adjustment.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p todomocop-core test_add_task`
Expected: PASS

- [ ] **Step 5: Write test — edit_task updates specific fields**

```rust
#[test]
fn test_edit_task() {
    let db = Db::open_in_memory().unwrap();
    let id = db.add_task(AddTask {
        title: "Buy groceries".into(),
        description: None,
        status: None,
        priority: None,
        workspace: "personal".into(),
        deadline: None,
        planned_date: None,
    }).unwrap();

    db.edit_task(id, EditTask {
        title: Some("Buy organic groceries".into()),
        status: Some(TaskStatus::Ready),
        priority: Some(Some(1)),
        ..Default::default()
    }).unwrap();

    let tasks = db.list_tasks(TaskFilter::default()).unwrap();
    assert_eq!(tasks[0].title, "Buy organic groceries");
    assert_eq!(tasks[0].status, TaskStatus::Ready);
    assert_eq!(tasks[0].priority, Some(1));
    assert_eq!(tasks[0].workspace, "personal"); // unchanged
}
```

- [ ] **Step 6: Run test to verify it fails**

Run: `cargo test -p todomocop-core test_edit_task`
Expected: FAIL — `edit_task` not implemented.

- [ ] **Step 7: Implement edit_task**

```rust
impl Db {
    pub fn edit_task(&self, id: TaskId, params: EditTask) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        self.db.transaction_mut(|txn| {
            let task = txn.lazy(crate::schema::Task.id(id))
                .ok_or_else(|| anyhow::anyhow!("task not found: {id}"))?;

            let row = txn.mutable(task);
            if let Some(title) = params.title {
                row.title = title;
            }
            if let Some(description) = params.description {
                row.description = description;
            }
            if let Some(status) = params.status {
                row.status = status.to_string();
            }
            if let Some(priority) = params.priority {
                row.priority = priority;
            }
            if let Some(workspace) = params.workspace {
                row.workspace = workspace;
            }
            if let Some(deadline) = params.deadline {
                row.deadline = deadline;
            }
            if let Some(planned_date) = params.planned_date {
                row.planned_date = planned_date;
            }
            if let Some(snooze_until) = params.snooze_until {
                row.snooze_until = snooze_until;
            }
            row.updated_at = now;

            Ok(())
        })
    }
}
```

**Note:** `txn.lazy()` and `txn.mutable()` are rust-query patterns. The exact API (e.g., whether `mutable` returns a mutable reference with field assignment or uses setter methods) needs verification. The pattern shows the intent.

- [ ] **Step 8: Run test to verify it passes**

Run: `cargo test -p todomocop-core test_edit_task`
Expected: PASS

- [ ] **Step 9: Write test — delete_task sets deleted_at**

```rust
#[test]
fn test_delete_task() {
    let db = Db::open_in_memory().unwrap();
    let id = db.add_task(AddTask {
        title: "Delete me".into(),
        description: None,
        status: None,
        priority: None,
        workspace: "personal".into(),
        deadline: None,
        planned_date: None,
    }).unwrap();

    db.delete_task(id).unwrap();

    // Default list_tasks excludes deleted
    let tasks = db.list_tasks(TaskFilter::default()).unwrap();
    assert_eq!(tasks.len(), 0);
}
```

- [ ] **Step 10: Run test to verify it fails**

Run: `cargo test -p todomocop-core test_delete_task`
Expected: FAIL — `delete_task` not implemented.

- [ ] **Step 11: Implement delete_task**

```rust
impl Db {
    pub fn delete_task(&self, id: TaskId) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        self.db.transaction_mut(|txn| {
            let task = txn.lazy(crate::schema::Task.id(id))
                .ok_or_else(|| anyhow::anyhow!("task not found: {id}"))?;
            let row = txn.mutable(task);
            row.deleted_at = Some(now);
            row.updated_at = Utc::now().to_rfc3339();
            Ok(())
        })
    }
}
```

- [ ] **Step 12: Run test to verify it passes**

Run: `cargo test -p todomocop-core test_delete_task`
Expected: PASS

- [ ] **Step 13: Write test — snooze sets snooze_until**

```rust
#[test]
fn test_snooze_task() {
    let db = Db::open_in_memory().unwrap();
    let id = db.add_task(AddTask {
        title: "Snoozeable".into(),
        description: None,
        status: None,
        priority: None,
        workspace: "personal".into(),
        deadline: None,
        planned_date: None,
    }).unwrap();

    let until = chrono::NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
    db.snooze(id, until).unwrap();

    // With include_snoozed=false (default), task is hidden
    let tasks = db.list_tasks(TaskFilter::default()).unwrap();
    assert_eq!(tasks.len(), 0);

    // With include_snoozed=true, task appears
    let tasks = db.list_tasks(TaskFilter {
        include_snoozed: true,
        ..Default::default()
    }).unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].snooze_until.is_some());
}
```

- [ ] **Step 14: Run test to verify it fails**

Run: `cargo test -p todomocop-core test_snooze_task`
Expected: FAIL — `snooze` not implemented.

- [ ] **Step 15: Implement snooze**

```rust
impl Db {
    pub fn snooze(&self, id: TaskId, until: chrono::NaiveDate) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        self.db.transaction_mut(|txn| {
            let task = txn.lazy(crate::schema::Task.id(id))
                .ok_or_else(|| anyhow::anyhow!("task not found: {id}"))?;
            let row = txn.mutable(task);
            row.snooze_until = Some(until.to_string());
            row.updated_at = now;
            Ok(())
        })
    }
}
```

- [ ] **Step 16: Run test to verify it passes**

Run: `cargo test -p todomocop-core test_snooze_task`
Expected: PASS

- [ ] **Step 17: Commit**

```bash
git add core/src/task.rs core/src/lib.rs
git commit -m "feat: implement task CRUD — add, edit, delete, snooze"
```

---

## Task 4: Task Queries (list_tasks, search)

**Files:**
- Create: `core/src/query.rs`
- Modify: `core/src/lib.rs` (add `pub mod query;`)

- [ ] **Step 1: Write test — list_tasks filters by status**

```rust
#[test]
fn test_list_tasks_filter_status() {
    let db = Db::open_in_memory().unwrap();

    db.add_task(AddTask {
        title: "Idea task".into(),
        status: Some(TaskStatus::Idea),
        workspace: "personal".into(),
        ..default_add_task()
    }).unwrap();

    db.add_task(AddTask {
        title: "Ready task".into(),
        status: Some(TaskStatus::Ready),
        workspace: "personal".into(),
        ..default_add_task()
    }).unwrap();

    let tasks = db.list_tasks(TaskFilter {
        status: Some(TaskStatus::Idea),
        ..Default::default()
    }).unwrap();

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "Idea task");
}

// Test helper
fn default_add_task() -> AddTask {
    AddTask {
        title: String::new(),
        description: None,
        status: None,
        priority: None,
        workspace: "personal".into(),
        deadline: None,
        planned_date: None,
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p todomocop-core test_list_tasks_filter_status`
Expected: FAIL — `list_tasks` not fully implemented.

- [ ] **Step 3: Implement list_tasks in core/src/query.rs**

```rust
use crate::{Db, Task, TaskFilter, TaskStatus, GithubPrContextData, LinearContextData};
use anyhow::Result;
use chrono::Utc;

impl Db {
    pub fn list_tasks(&self, filter: TaskFilter) -> Result<Vec<Task>> {
        let today = Utc::now().naive_utc().date().to_string();

        self.db.transaction(|txn| {
            // Query all tasks, apply filters
            let results = txn.query(|rows| {
                let task = rows.join(crate::schema::Task);

                // Exclude deleted
                rows.filter(task.deleted_at.is_none());

                // Filter by status
                if let Some(ref status) = filter.status {
                    rows.filter(task.status.eq(status.to_string()));
                }

                // Filter by workspace
                if let Some(ref workspace) = filter.workspace {
                    rows.filter(task.workspace.eq(workspace.clone()));
                }

                // Filter by has_planned_date
                if let Some(has_planned) = filter.has_planned_date {
                    if has_planned {
                        rows.filter(task.planned_date.is_some());
                    } else {
                        rows.filter(task.planned_date.is_none());
                    }
                }

                // Filter by priority_max
                if let Some(max) = filter.priority_max {
                    rows.filter(task.priority.is_some());
                    rows.filter(task.priority.le(Some(max)));
                }

                // Exclude snoozed (unless include_snoozed)
                if !filter.include_snoozed {
                    // Show if snooze_until is None OR snooze_until <= today
                    rows.filter(
                        task.snooze_until.is_none()
                            .or(task.snooze_until.le(Some(today.clone())))
                    );
                }

                rows.into_vec(task)
            });

            // Convert to Task types and sort by priority then deadline
            let mut tasks: Vec<Task> = results
                .into_iter()
                .map(|row| /* convert schema row to Task type */)
                .collect();

            tasks.sort_by(|a, b| {
                // Priority: lower number = higher priority, None last
                let pa = a.priority.unwrap_or(i64::MAX);
                let pb = b.priority.unwrap_or(i64::MAX);
                pa.cmp(&pb).then_with(|| {
                    // Deadline: earlier first, None last
                    let da = a.deadline.as_deref().unwrap_or("9999-99-99");
                    let db = b.deadline.as_deref().unwrap_or("9999-99-99");
                    da.cmp(db)
                })
            });

            Ok(tasks)
        })
    }
}
```

**Note:** The rust-query query builder syntax (`rows.join`, `rows.filter`, `rows.into_vec`) needs verification. The filtering logic (especially `is_none()`, `.or()`, `.le()`) is pseudocode showing the intent. The actual rust-query API may use different method names. Consult [rust-query docs](https://docs.rs/rust-query/latest/rust_query/) during implementation.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p todomocop-core test_list_tasks_filter_status`
Expected: PASS

- [ ] **Step 5: Write test — list_tasks filters by workspace and excludes deleted**

```rust
#[test]
fn test_list_tasks_filter_workspace_and_deleted() {
    let db = Db::open_in_memory().unwrap();

    let id1 = db.add_task(AddTask {
        title: "Work task".into(),
        workspace: "work".into(),
        ..default_add_task()
    }).unwrap();

    db.add_task(AddTask {
        title: "Personal task".into(),
        workspace: "personal".into(),
        ..default_add_task()
    }).unwrap();

    // Delete the work task
    db.delete_task(id1).unwrap();

    // Filter by work workspace — should be empty (deleted)
    let tasks = db.list_tasks(TaskFilter {
        workspace: Some("work".into()),
        ..Default::default()
    }).unwrap();
    assert_eq!(tasks.len(), 0);

    // No filter — only personal shows
    let tasks = db.list_tasks(TaskFilter::default()).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].workspace, "personal");
}
```

- [ ] **Step 6: Run test to verify it passes**

Run: `cargo test -p todomocop-core test_list_tasks_filter_workspace_and_deleted`
Expected: PASS (should already work with the filter implementation)

- [ ] **Step 7: Write test — list_tasks sorts by priority then deadline**

```rust
#[test]
fn test_list_tasks_sort_order() {
    let db = Db::open_in_memory().unwrap();

    db.add_task(AddTask {
        title: "Low priority".into(),
        priority: Some(3),
        workspace: "personal".into(),
        ..default_add_task()
    }).unwrap();

    db.add_task(AddTask {
        title: "Critical".into(),
        priority: Some(0),
        workspace: "personal".into(),
        ..default_add_task()
    }).unwrap();

    db.add_task(AddTask {
        title: "No priority".into(),
        priority: None,
        workspace: "personal".into(),
        ..default_add_task()
    }).unwrap();

    let tasks = db.list_tasks(TaskFilter::default()).unwrap();
    assert_eq!(tasks[0].title, "Critical");       // priority 0
    assert_eq!(tasks[1].title, "Low priority");    // priority 3
    assert_eq!(tasks[2].title, "No priority");     // None = last
}
```

- [ ] **Step 8: Run test to verify it passes**

Run: `cargo test -p todomocop-core test_list_tasks_sort_order`
Expected: PASS

- [ ] **Step 9: Write test — search matches title and description**

```rust
#[test]
fn test_search() {
    let db = Db::open_in_memory().unwrap();

    db.add_task(AddTask {
        title: "Buy groceries".into(),
        description: Some("Get milk and eggs".into()),
        workspace: "personal".into(),
        ..default_add_task()
    }).unwrap();

    db.add_task(AddTask {
        title: "Fix login bug".into(),
        description: Some("Users can't log in".into()),
        workspace: "work".into(),
        ..default_add_task()
    }).unwrap();

    // Search by title substring
    let results = db.search("groceries", TaskFilter::default()).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Buy groceries");

    // Search by description substring
    let results = db.search("log in", TaskFilter::default()).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Fix login bug");

    // Search with workspace filter
    let results = db.search("Buy", TaskFilter {
        workspace: Some("work".into()),
        ..Default::default()
    }).unwrap();
    assert_eq!(results.len(), 0);
}
```

- [ ] **Step 10: Run test to verify it fails**

Run: `cargo test -p todomocop-core test_search`
Expected: FAIL — `search` not implemented.

- [ ] **Step 11: Implement search**

```rust
impl Db {
    pub fn search(&self, query: &str, filter: TaskFilter) -> Result<Vec<Task>> {
        // Get all tasks matching filter, then filter by LIKE on title/description
        // SQLite LIKE is case-insensitive for ASCII
        let pattern = format!("%{query}%");

        self.db.transaction(|txn| {
            // Same query as list_tasks but with additional LIKE filter
            // on title OR description
            let results = txn.query(|rows| {
                let task = rows.join(crate::schema::Task);
                rows.filter(task.deleted_at.is_none());
                rows.filter(
                    task.title.like(&pattern)
                        .or(task.description.like(&pattern))
                );
                // ... apply same filters as list_tasks ...
                rows.into_vec(task)
            });

            // Convert, sort, return
            Ok(results)
        })
    }
}
```

**Note:** rust-query may not support `LIKE` directly. If not, fetch all tasks matching the filter and do the substring match in Rust: `title.to_lowercase().contains(&query.to_lowercase())`. This is fine for a personal tool with <10k tasks.

- [ ] **Step 12: Run test to verify it passes**

Run: `cargo test -p todomocop-core test_search`
Expected: PASS

- [ ] **Step 13: Commit**

```bash
git add core/src/query.rs core/src/lib.rs
git commit -m "feat: implement list_tasks with filters and search"
```

---

## Task 5: Attachments

**Files:**
- Create: `core/src/attachment.rs`
- Modify: `core/src/lib.rs` (add `pub mod attachment;`)

- [ ] **Step 1: Write test — add attachment to a task**

```rust
#[test]
fn test_add_attachment() {
    let db = Db::open_in_memory().unwrap();
    let task_id = db.add_task(AddTask {
        title: "Screenshot task".into(),
        workspace: "personal".into(),
        ..default_add_task()
    }).unwrap();

    let att_id = db.add_attachment(task_id, NewAttachment {
        file_name: "screenshot.png".into(),
        content_type: "image/png".into(),
        data: vec![0x89, 0x50, 0x4E, 0x47],  // PNG header bytes
        caption: Some("A screenshot of the bug".into()),
    }).unwrap();

    assert_eq!(att_id, 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p todomocop-core test_add_attachment`
Expected: FAIL — `add_attachment` not implemented.

- [ ] **Step 3: Implement add_attachment**

```rust
use crate::{Db, NewAttachment, AttachmentId, TaskId};
use anyhow::Result;
use chrono::Utc;

impl Db {
    pub fn add_attachment(&self, task_id: TaskId, att: NewAttachment) -> Result<AttachmentId> {
        let now = Utc::now().to_rfc3339();

        self.db.transaction_mut(|txn| {
            let task = txn.lazy(crate::schema::Task.id(task_id))
                .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"))?;

            let next_id = /* query max attachment id + 1 */;

            txn.insert_ok(crate::schema::Attachment {
                id: next_id,
                task,
                file_name: att.file_name,
                content_type: att.content_type,
                data: att.data,
                caption: att.caption.unwrap_or_default(),
                created_at: now,
            });

            Ok(next_id)
        })
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p todomocop-core test_add_attachment`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add core/src/attachment.rs core/src/lib.rs
git commit -m "feat: implement attachments with blob storage"
```

---

## Task 6: External Contexts (GitHub PR + Linear)

**Files:**
- Create: `core/src/github.rs`
- Create: `core/src/linear.rs`
- Modify: `core/src/lib.rs`

- [ ] **Step 1: Write test — link GitHub PR to task**

```rust
#[test]
fn test_link_github_pr() {
    let db = Db::open_in_memory().unwrap();
    let task_id = db.add_task(AddTask {
        title: "Review PR".into(),
        workspace: "work".into(),
        ..default_add_task()
    }).unwrap();

    db.link_github_pr(task_id, "https://github.com/org/repo/pull/42").unwrap();

    let tasks = db.list_tasks(TaskFilter::default()).unwrap();
    let ctx = tasks[0].github_pr_context.as_ref().unwrap();
    assert_eq!(ctx.repo, "org/repo");
    assert_eq!(ctx.number, 42);
    assert_eq!(ctx.url, "https://github.com/org/repo/pull/42");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p todomocop-core test_link_github_pr`
Expected: FAIL — `link_github_pr` not implemented.

- [ ] **Step 3: Implement link_github_pr in core/src/github.rs**

Parses the URL to extract `owner/repo` and PR number. Creates a `GithubPrContext` row.

```rust
use crate::{Db, TaskId};
use anyhow::Result;
use chrono::Utc;

impl Db {
    pub fn link_github_pr(&self, task_id: TaskId, url: &str) -> Result<()> {
        // Parse URL: https://github.com/{owner}/{repo}/pull/{number}
        let parts: Vec<&str> = url.trim_end_matches('/').split('/').collect();
        let len = parts.len();
        if len < 4 || parts[len - 2] != "pull" {
            anyhow::bail!("invalid GitHub PR URL: {url}");
        }
        let number: i64 = parts[len - 1].parse()
            .map_err(|_| anyhow::anyhow!("invalid PR number in URL: {url}"))?;
        let repo = format!("{}/{}", parts[len - 4], parts[len - 3]);
        let now = Utc::now().to_rfc3339();

        self.db.transaction_mut(|txn| {
            let task = txn.lazy(crate::schema::Task.id(task_id))
                .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"))?;

            txn.insert_ok(crate::schema::GithubPrContext {
                task,
                url: url.to_string(),
                repo,
                number,
                state: "unknown".into(),
                last_refreshed: now,
            });

            Ok(())
        })
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p todomocop-core test_link_github_pr`
Expected: PASS

- [ ] **Step 5: Write test — link Linear issue to task**

```rust
#[test]
fn test_link_linear() {
    let db = Db::open_in_memory().unwrap();
    let task_id = db.add_task(AddTask {
        title: "Fix bug".into(),
        workspace: "work".into(),
        ..default_add_task()
    }).unwrap();

    db.link_linear(task_id, "https://linear.app/myteam/issue/ENG-123/fix-the-bug").unwrap();

    let tasks = db.list_tasks(TaskFilter::default()).unwrap();
    let ctx = tasks[0].linear_context.as_ref().unwrap();
    assert_eq!(ctx.identifier, "ENG-123");
    assert_eq!(ctx.url, "https://linear.app/myteam/issue/ENG-123/fix-the-bug");
}
```

- [ ] **Step 6: Run test to verify it fails**

Run: `cargo test -p todomocop-core test_link_linear`
Expected: FAIL

- [ ] **Step 7: Implement link_linear in core/src/linear.rs**

```rust
use crate::{Db, TaskId};
use anyhow::Result;
use chrono::Utc;

impl Db {
    pub fn link_linear(&self, task_id: TaskId, url: &str) -> Result<()> {
        // Parse URL: https://linear.app/{team}/issue/{IDENTIFIER}/...
        let parts: Vec<&str> = url.trim_end_matches('/').split('/').collect();
        let issue_idx = parts.iter().position(|&p| p == "issue")
            .ok_or_else(|| anyhow::anyhow!("invalid Linear URL: {url}"))?;
        let identifier = parts.get(issue_idx + 1)
            .ok_or_else(|| anyhow::anyhow!("no identifier in Linear URL: {url}"))?
            .to_string();
        let now = Utc::now().to_rfc3339();

        self.db.transaction_mut(|txn| {
            let task = txn.lazy(crate::schema::Task.id(task_id))
                .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"))?;

            txn.insert_ok(crate::schema::LinearContext {
                task,
                url: url.to_string(),
                identifier,
                data: "{}".into(),  // empty until first refresh
                last_refreshed: now,
            });

            Ok(())
        })
    }
}
```

- [ ] **Step 8: Run test to verify it passes**

Run: `cargo test -p todomocop-core test_link_linear`
Expected: PASS

- [ ] **Step 9: Write test — stale context triggers refresh via mock HttpClient**

```rust
use std::sync::{Arc, Mutex};
use crate::http::HttpClient;

struct MockHttpClient {
    responses: Mutex<Vec<Vec<u8>>>,
}

impl HttpClient for MockHttpClient {
    fn get(&self, _url: &str, _headers: &[(&str, &str)]) -> anyhow::Result<Vec<u8>> {
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            anyhow::bail!("no mock response");
        }
        Ok(responses.remove(0))
    }
}

#[test]
fn test_github_refresh_on_stale() {
    let mock = Arc::new(MockHttpClient {
        responses: Mutex::new(vec![
            // GitHub API response for PR #42
            serde_json::to_vec(&serde_json::json!({
                "state": "open",
                "merged": false
            })).unwrap(),
        ]),
    });

    let db = Db::open_with_http(
        Arc::clone(&mock) as Arc<dyn HttpClient>,
        IntegrationConfig {
            github_token: Some("test-token".into()),
            staleness_threshold: std::time::Duration::from_secs(0), // always stale
            ..Default::default()
        },
    ).unwrap();

    let task_id = db.add_task(AddTask {
        title: "Review PR".into(),
        workspace: "work".into(),
        ..default_add_task()
    }).unwrap();

    db.link_github_pr(task_id, "https://github.com/org/repo/pull/42").unwrap();

    // list_tasks should trigger refresh
    let tasks = db.list_tasks(TaskFilter::default()).unwrap();
    let ctx = tasks[0].github_pr_context.as_ref().unwrap();
    assert_eq!(ctx.state, "open");
}
```

**Note:** `Db::open_with_http` is a convenience constructor for tests that takes an HttpClient + config but uses `:memory:` DB. Add it alongside `open_in_memory`.

- [ ] **Step 10: Run test to verify it fails**

Run: `cargo test -p todomocop-core test_github_refresh_on_stale`
Expected: FAIL — refresh logic not implemented.

- [ ] **Step 11: Implement refresh logic**

In `core/src/github.rs`, add a method that `list_tasks` (in `query.rs`) calls after fetching results:

```rust
impl Db {
    /// Refresh stale GitHub PR contexts. Called by list_tasks.
    pub(crate) fn refresh_github_contexts(&self, tasks: &mut [Task]) -> Result<()> {
        let Some(ref token) = self.config.github_token else {
            return Ok(()); // no token, skip refresh
        };

        let threshold = Utc::now() - chrono::Duration::from_std(self.config.staleness_threshold)?;
        let threshold_str = threshold.to_rfc3339();

        for task in tasks.iter_mut() {
            let Some(ref ctx) = task.github_pr_context else { continue };
            if ctx.last_refreshed >= threshold_str {
                continue; // fresh enough
            }

            // Fetch from GitHub API
            let api_url = format!(
                "https://api.github.com/repos/{}/pulls/{}",
                ctx.repo, ctx.number
            );
            let headers = [
                ("Authorization", format!("Bearer {token}").as_str()),
                ("Accept", "application/vnd.github.v3+json"),
                ("User-Agent", "todomocop"),
            ];

            match self.http.get(&api_url, &headers) {
                Ok(body) => {
                    let json: serde_json::Value = serde_json::from_slice(&body)?;
                    let state = if json["merged"].as_bool() == Some(true) {
                        "merged"
                    } else {
                        json["state"].as_str().unwrap_or("unknown")
                    };
                    let now = Utc::now().to_rfc3339();

                    // Update DB
                    self.db.transaction_mut(|txn| {
                        // Update github_pr_context row
                        // Exact update syntax depends on rust-query API
                        Ok(())
                    })?;

                    // Update in-memory task
                    task.github_pr_context = Some(GithubPrContextData {
                        state: state.to_string(),
                        last_refreshed: now,
                        ..ctx.clone()
                    });
                }
                Err(_) => {
                    // Refresh failed — return stale data, don't error
                    continue;
                }
            }
        }

        Ok(())
    }
}
```

Add equivalent `refresh_linear_contexts` in `core/src/linear.rs` using the Linear GraphQL API.

Update `list_tasks` in `core/src/query.rs` to call both refresh methods before returning:

```rust
// At the end of list_tasks, before Ok(tasks):
self.refresh_github_contexts(&mut tasks)?;
self.refresh_linear_contexts(&mut tasks)?;
```

- [ ] **Step 12: Run test to verify it passes**

Run: `cargo test -p todomocop-core test_github_refresh_on_stale`
Expected: PASS

- [ ] **Step 13: Commit**

```bash
git add core/src/github.rs core/src/linear.rs core/src/http.rs core/src/query.rs core/src/lib.rs
git commit -m "feat: implement GitHub PR and Linear context linking with refresh"
```

---

## Task 7: MCP Server

**Files:**
- Modify: `mcp/Cargo.toml`
- Modify: `mcp/src/main.rs`

- [ ] **Step 1: Set up rmcp server skeleton**

```rust
use anyhow::Result;
use rmcp::{
    handler::server::tool::ToolRouter,
    model::*,
    schemars, tool, tool_handler, tool_router,
    ErrorData as McpError, ServerHandler, ServiceExt,
    transport::stdio,
};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use todomocop_core::{Db, IntegrationConfig};

#[derive(Clone)]
pub struct TodoServer {
    db: Arc<Db>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl TodoServer {
    pub fn new(db: Arc<Db>) -> Self {
        Self {
            db,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_handler]
impl ServerHandler for TodoServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2025_11_25,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "Personal TODO management. Use list_tasks, add_task, edit_task, search, etc."
                    .to_string(),
            ),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parse args for --db path
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

    let config = IntegrationConfig {
        github_token: std::env::var("GITHUB_TOKEN").ok(),
        linear_api_key: std::env::var("LINEAR_API_KEY").ok(),
        ..Default::default()
    };

    let http: Arc<dyn todomocop_core::HttpClient> = Arc::new(UreqHttpClient);
    let db = Arc::new(Db::open(&db_path, http, config)?);
    let server = TodoServer::new(db);

    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

/// Real HTTP client using ureq.
struct UreqHttpClient;

impl todomocop_core::HttpClient for UreqHttpClient {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>> {
        let mut req = ureq::get(url);
        for (key, value) in headers {
            req = req.header(key, value);
        }
        let response = req.call()?;
        let mut body = Vec::new();
        response.into_body().read_to_end(&mut body)?;
        Ok(body)
    }
}
```

- [ ] **Step 2: Run `cargo check -p todomocop-mcp` to verify skeleton compiles**

Expected: Compiles with no errors.

- [ ] **Step 3: Implement add_task tool**

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddTaskParams {
    #[schemars(description = "Task title")]
    pub title: String,
    #[schemars(description = "Task description")]
    pub description: Option<String>,
    #[schemars(description = "Priority: 0=critical, 1=high, 2=medium, 3=low")]
    pub priority: Option<i64>,
    #[schemars(description = "Workspace name (e.g. 'personal', 'work')")]
    pub workspace: Option<String>,
    #[schemars(description = "Deadline in ISO 8601 format (YYYY-MM-DD)")]
    pub deadline: Option<String>,
    #[schemars(description = "Planned work date in ISO 8601 format (YYYY-MM-DD)")]
    pub planned_date: Option<String>,
    #[schemars(description = "Status: idea, ready, in_progress, done")]
    pub status: Option<String>,
}

// Inside #[tool_router] impl TodoServer:
#[tool(description = "Create a new task")]
async fn add_task(
    &self,
    Parameters(params): Parameters<AddTaskParams>,
) -> Result<CallToolResult, McpError> {
    let status = params.status
        .map(|s| s.parse())
        .transpose()
        .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;

    let id = self.db.add_task(todomocop_core::AddTask {
        title: params.title,
        description: params.description,
        status,
        priority: params.priority,
        workspace: params.workspace.unwrap_or_else(|| "personal".into()),
        deadline: params.deadline,
        planned_date: params.planned_date,
    }).map_err(|e| McpError::internal_error(format!("{e}"), None))?;

    Ok(CallToolResult::success(vec![
        Content::text(format!("Created task #{id}")),
    ]))
}
```

- [ ] **Step 4: Implement edit_task tool**

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EditTaskParams {
    #[schemars(description = "Task ID to edit")]
    pub id: i64,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    #[schemars(description = "Priority: 0=critical, 1=high, 2=medium, 3=low, null to clear")]
    pub priority: Option<Option<i64>>,
    pub workspace: Option<String>,
    pub deadline: Option<Option<String>>,
    pub planned_date: Option<Option<String>>,
}

#[tool(description = "Edit an existing task. Only provided fields are updated.")]
async fn edit_task(
    &self,
    Parameters(params): Parameters<EditTaskParams>,
) -> Result<CallToolResult, McpError> {
    let status = params.status
        .map(|s| s.parse())
        .transpose()
        .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;

    self.db.edit_task(params.id, todomocop_core::EditTask {
        title: params.title,
        description: params.description,
        status,
        priority: params.priority,
        workspace: params.workspace,
        deadline: params.deadline,
        planned_date: params.planned_date,
        ..Default::default()
    }).map_err(|e| McpError::internal_error(format!("{e}"), None))?;

    Ok(CallToolResult::success(vec![
        Content::text(format!("Updated task #{}", params.id)),
    ]))
}
```

- [ ] **Step 5: Implement delete_task tool**

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteTaskParams {
    #[schemars(description = "Task ID to delete")]
    pub id: i64,
}

#[tool(description = "Soft-delete a task")]
async fn delete_task(
    &self,
    Parameters(params): Parameters<DeleteTaskParams>,
) -> Result<CallToolResult, McpError> {
    self.db.delete_task(params.id)
        .map_err(|e| McpError::internal_error(format!("{e}"), None))?;

    Ok(CallToolResult::success(vec![
        Content::text(format!("Deleted task #{}", params.id)),
    ]))
}
```

- [ ] **Step 6: Implement list_tasks tool**

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListTasksParams {
    #[schemars(description = "Filter by status: idea, ready, in_progress, done")]
    pub status: Option<String>,
    #[schemars(description = "Filter by workspace")]
    pub workspace: Option<String>,
    #[schemars(description = "Filter by whether task has a planned date")]
    pub has_planned_date: Option<bool>,
    #[schemars(description = "Filter: only show tasks with priority <= this value (0=critical)")]
    pub priority_max: Option<i64>,
    #[schemars(description = "Include snoozed tasks (default: false)")]
    pub include_snoozed: Option<bool>,
}

#[tool(description = "List tasks with optional filters. Sorted by priority (0=critical first), then deadline.")]
async fn list_tasks(
    &self,
    Parameters(params): Parameters<ListTasksParams>,
) -> Result<CallToolResult, McpError> {
    let status = params.status
        .map(|s| s.parse())
        .transpose()
        .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;

    let tasks = self.db.list_tasks(todomocop_core::TaskFilter {
        status,
        workspace: params.workspace,
        has_planned_date: params.has_planned_date,
        priority_max: params.priority_max,
        include_snoozed: params.include_snoozed.unwrap_or(false),
    }).map_err(|e| McpError::internal_error(format!("{e}"), None))?;

    let json = serde_json::to_string_pretty(&tasks)
        .map_err(|e| McpError::internal_error(format!("{e}"), None))?;

    Ok(CallToolResult::success(vec![Content::text(json)]))
}
```

- [ ] **Step 7: Implement search tool**

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    #[schemars(description = "Search query (matches title and description)")]
    pub query: String,
    pub status: Option<String>,
    pub workspace: Option<String>,
}

#[tool(description = "Search tasks by title and description")]
async fn search(
    &self,
    Parameters(params): Parameters<SearchParams>,
) -> Result<CallToolResult, McpError> {
    let status = params.status
        .map(|s| s.parse())
        .transpose()
        .map_err(|e| McpError::invalid_params(format!("{e}"), None))?;

    let tasks = self.db.search(&params.query, todomocop_core::TaskFilter {
        status,
        workspace: params.workspace,
        ..Default::default()
    }).map_err(|e| McpError::internal_error(format!("{e}"), None))?;

    let json = serde_json::to_string_pretty(&tasks)
        .map_err(|e| McpError::internal_error(format!("{e}"), None))?;

    Ok(CallToolResult::success(vec![Content::text(json)]))
}
```

- [ ] **Step 8: Implement snooze tool**

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SnoozeParams {
    #[schemars(description = "Task ID to snooze")]
    pub id: i64,
    #[schemars(description = "Hide task until this date (ISO 8601: YYYY-MM-DD)")]
    pub until: String,
}

#[tool(description = "Snooze a task — hides it until the given date")]
async fn snooze(
    &self,
    Parameters(params): Parameters<SnoozeParams>,
) -> Result<CallToolResult, McpError> {
    let until = params.until.parse::<chrono::NaiveDate>()
        .map_err(|e| McpError::invalid_params(format!("invalid date: {e}"), None))?;

    self.db.snooze(params.id, until)
        .map_err(|e| McpError::internal_error(format!("{e}"), None))?;

    Ok(CallToolResult::success(vec![
        Content::text(format!("Snoozed task #{} until {}", params.id, params.until)),
    ]))
}
```

- [ ] **Step 9: Implement add_attachment tool**

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddAttachmentParams {
    #[schemars(description = "Task ID to attach to")]
    pub task_id: i64,
    #[schemars(description = "File name (e.g. screenshot.png)")]
    pub file_name: String,
    #[schemars(description = "MIME content type (e.g. image/png)")]
    pub content_type: String,
    #[schemars(description = "File data as base64-encoded string")]
    pub data: String,
    #[schemars(description = "Caption describing the attachment")]
    pub caption: Option<String>,
}

#[tool(description = "Add a file attachment to a task (data as base64)")]
async fn add_attachment(
    &self,
    Parameters(params): Parameters<AddAttachmentParams>,
) -> Result<CallToolResult, McpError> {
    use base64::Engine;
    let data = base64::engine::general_purpose::STANDARD
        .decode(&params.data)
        .map_err(|e| McpError::invalid_params(format!("invalid base64: {e}"), None))?;

    let att_id = self.db.add_attachment(params.task_id, todomocop_core::NewAttachment {
        file_name: params.file_name,
        content_type: params.content_type,
        data,
        caption: params.caption,
    }).map_err(|e| McpError::internal_error(format!("{e}"), None))?;

    Ok(CallToolResult::success(vec![
        Content::text(format!("Added attachment #{att_id} to task #{}", params.task_id)),
    ]))
}
```

Add `base64 = "0.22"` to `mcp/Cargo.toml`.

- [ ] **Step 10: Implement link_github_pr and link_linear tools**

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LinkGithubPrParams {
    #[schemars(description = "Task ID")]
    pub task_id: i64,
    #[schemars(description = "GitHub PR URL (e.g. https://github.com/org/repo/pull/42)")]
    pub url: String,
}

#[tool(description = "Link a GitHub pull request to a task")]
async fn link_github_pr(
    &self,
    Parameters(params): Parameters<LinkGithubPrParams>,
) -> Result<CallToolResult, McpError> {
    self.db.link_github_pr(params.task_id, &params.url)
        .map_err(|e| McpError::internal_error(format!("{e}"), None))?;

    Ok(CallToolResult::success(vec![
        Content::text(format!("Linked GitHub PR to task #{}", params.task_id)),
    ]))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LinkLinearParams {
    #[schemars(description = "Task ID")]
    pub task_id: i64,
    #[schemars(description = "Linear issue URL")]
    pub url: String,
}

#[tool(description = "Link a Linear issue to a task")]
async fn link_linear(
    &self,
    Parameters(params): Parameters<LinkLinearParams>,
) -> Result<CallToolResult, McpError> {
    self.db.link_linear(params.task_id, &params.url)
        .map_err(|e| McpError::internal_error(format!("{e}"), None))?;

    Ok(CallToolResult::success(vec![
        Content::text(format!("Linked Linear issue to task #{}", params.task_id)),
    ]))
}
```

- [ ] **Step 11: Verify MCP server compiles**

Run: `cargo check -p todomocop-mcp`
Expected: Compiles with no errors.

- [ ] **Step 12: Commit**

```bash
git add mcp/
git commit -m "feat: implement MCP server with all tools via rmcp"
```

---

## Task 8: CLI

**Files:**
- Modify: `cli/Cargo.toml`
- Modify: `cli/src/main.rs`

- [ ] **Step 1: Implement CLI with clap subcommands**

```rust
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use todomocop_core::{Db, IntegrationConfig, TaskFilter, TaskStatus, AddTask, EditTask};

#[derive(Parser)]
#[command(name = "todomocop", about = "Personal TODO management")]
struct Cli {
    /// Database path (default: ~/.local/share/todomocop/todomocop.db)
    #[arg(long, env = "TODOMOCOP_DB")]
    db: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new task
    Add {
        title: String,
        #[arg(short, long)]
        description: Option<String>,
        #[arg(short, long)]
        priority: Option<i64>,
        #[arg(short, long, default_value = "personal")]
        workspace: String,
        #[arg(long)]
        deadline: Option<String>,
        #[arg(long)]
        planned_date: Option<String>,
        #[arg(long)]
        status: Option<String>,
    },
    /// List tasks
    List {
        #[arg(short, long)]
        status: Option<String>,
        #[arg(short, long)]
        workspace: Option<String>,
        #[arg(long)]
        include_snoozed: bool,
    },
    /// Edit a task
    Edit {
        id: i64,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        priority: Option<i64>,
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Delete a task
    Delete { id: i64 },
    /// Search tasks
    Search {
        query: String,
        #[arg(short, long)]
        workspace: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let db_path = cli.db.unwrap_or_else(|| {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("todomocop")
            .join("todomocop.db")
    });

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let config = IntegrationConfig {
        github_token: std::env::var("GITHUB_TOKEN").ok(),
        linear_api_key: std::env::var("LINEAR_API_KEY").ok(),
        ..Default::default()
    };

    // UreqHttpClient defined same as in mcp binary — consider extracting
    // to a shared location if this grows
    let http: Arc<dyn todomocop_core::HttpClient> = Arc::new(UreqHttpClient);
    let db = Db::open(&db_path, http, config)?;

    match cli.command {
        Commands::Add { title, description, priority, workspace, deadline, planned_date, status } => {
            let status = status.map(|s| s.parse()).transpose()?;
            let id = db.add_task(AddTask {
                title,
                description,
                status,
                priority,
                workspace,
                deadline,
                planned_date,
            })?;
            println!("Created task #{id}");
        }
        Commands::List { status, workspace, include_snoozed } => {
            let status = status.map(|s| s.parse()).transpose()?;
            let tasks = db.list_tasks(TaskFilter {
                status,
                workspace,
                include_snoozed,
                ..Default::default()
            })?;
            for task in &tasks {
                let priority = task.priority.map(|p| format!("P{p}")).unwrap_or_default();
                println!("#{} [{}] {priority} {}", task.id, task.status, task.title);
            }
            if tasks.is_empty() {
                println!("No tasks found.");
            }
        }
        Commands::Edit { id, title, description, status, priority, workspace } => {
            let status = status.map(|s| s.parse()).transpose()?;
            db.edit_task(id, EditTask {
                title,
                description,
                status,
                priority: priority.map(Some),
                workspace,
                ..Default::default()
            })?;
            println!("Updated task #{id}");
        }
        Commands::Delete { id } => {
            db.delete_task(id)?;
            println!("Deleted task #{id}");
        }
        Commands::Search { query, workspace } => {
            let tasks = db.search(&query, TaskFilter {
                workspace,
                ..Default::default()
            })?;
            for task in &tasks {
                println!("#{} [{}] {}", task.id, task.status, task.title);
            }
            if tasks.is_empty() {
                println!("No tasks found.");
            }
        }
    }

    Ok(())
}

struct UreqHttpClient;
impl todomocop_core::HttpClient for UreqHttpClient {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Vec<u8>> {
        let mut req = ureq::get(url);
        for (key, value) in headers {
            req = req.header(key, value);
        }
        let response = req.call()?;
        let mut body = Vec::new();
        response.into_body().read_to_end(&mut body)?;
        Ok(body)
    }
}
```

Add `dirs = "6"` and `ureq = "3"` to `cli/Cargo.toml`. Also add `dirs = "6"` to `mcp/Cargo.toml`.

- [ ] **Step 2: Verify CLI compiles**

Run: `cargo check -p todomocop-cli`
Expected: Compiles.

- [ ] **Step 3: Manual smoke test**

Run: `cargo run -p todomocop-cli -- add "Test task" -w personal`
Expected: `Created task #1`

Run: `cargo run -p todomocop-cli -- list`
Expected: `#1 [idea]  Test task`

- [ ] **Step 4: Commit**

```bash
git add cli/
git commit -m "feat: implement CLI with clap subcommands"
```

---

## Notes for the implementor

1. **rust-query API**: The code samples use pseudocode for some rust-query operations (especially queries with filters, aggregations, and mutable updates). Consult the [rust-query docs](https://docs.rs/rust-query/latest/rust_query/) for exact syntax. The intent and logic are correct.

2. **ID generation**: Tasks use a `#[unique] id: i64` column. Generate next ID by querying `max(id) + 1` inside the insert transaction. If rust-query exposes rowid natively, you can simplify.

3. **UreqHttpClient duplication**: Both `mcp` and `cli` define `UreqHttpClient`. If this bothers you, extract it to a small shared crate or put it in core behind a feature flag. For now, the duplication is fine.

4. **rmcp version**: Using rmcp 1.2+. The `Parameters` wrapper import path is `rmcp::handler::server::wrapper::Parameters`. Check the latest docs if the import path has changed.

5. **Error handling**: The plan uses `anyhow` throughout. Core returns `anyhow::Result`. MCP tools convert to `McpError`. This is fine for a personal tool.

6. **`list_tasks` populating contexts**: When building the `Task` return type, `list_tasks` needs to join against `github_pr_context` and `linear_context` tables. This can be done with a second query per task (simple) or a single query with left joins (efficient). Start simple, optimize if needed.
