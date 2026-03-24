# Development Guide

## Building

```bash
nix develop  # enter dev shell with Rust + SQLite
cargo build --release -p todomocop-mcp -p todomocop-cli
cargo test --workspace
```

## Architecture

Cargo workspace with three crates:
- **core/** — library, owns all DB logic via rust-query 0.8, external refresh via HttpClient trait
- **mcp/** — MCP server binary (rmcp 1.2, stdio transport)
- **cli/** — CLI binary (clap)

All DB operations go through `core::Db`. Both binaries are thin wrappers.

## Key Design Decisions

- **`external_id` not `id`**: rust-query reserves `id` as an internal column. Our user-facing task IDs are in `external_id`, mapped to `TaskId` (i64) in the API.
- **Attachment blobs in DB**: `Vec<u8>` stored directly in SQLite (not file paths). Works since rust-query 0.8 fixed the BLOB support.
- **All methods take `&self`**: rust-query 0.8 uses closure-based transactions on `Database`, which handles locking internally.
- **MCP server uses DbHandle (thread dispatch)**: `Db` contains `Database` which is `!Send` (rust-query requirement). The MCP server wraps it in a `DbHandle` that dispatches closures to a dedicated thread via channels. The CLI uses `Db` directly since it's single-threaded.
- **Filtering in Rust, not SQL**: `list_tasks` fetches all rows and filters in Rust. Fine for a personal tool with <10k tasks. Could push filters to rust-query queries if needed.
- **External context refresh is transparent**: `list_tasks` automatically refreshes stale GitHub PR / Linear contexts. Controlled by `IntegrationConfig.staleness_threshold` (default 5 min). Failures return stale data silently.

## rust-query 0.8 Patterns

```rust
// Read transaction
let result = db.database.transaction(|txn| { ... });

// Write transaction (commit on Ok, rollback on Err)
let result = db.database.transaction_mut(|txn| { ... Ok(value) });

// Insert
let row = txn.insert_ok(schema::Task { field: value.clone(), ... });

// Lookup by unique column
let row = txn.query_one(optional(|row| {
    let t = row.and(schema::Task.external_id(id));
    row.then(t)
}));

// Update (mutable access, writes on drop)
let mut task = txn.mutable(&row);
task.field = new_value;

// Aggregate
let max = txn.query_one(aggregate(|rows| {
    let t = rows.join(schema::Task);
    rows.max(&t.external_id)
}));

// Query all rows with Select derive
txn.query(|q| {
    let t = q.join(schema::Task);
    q.into_vec(MySelectStruct { field: &t.field, ... })
});
```

**Gotchas:**
- `id` is reserved — use `external_id` or similar
- Insert struct fields are not generic — use `.clone()` / `.to_owned()`
- Column access via `&field` references, not method calls
- `#[derive(Select)]` generates a private companion struct (`*Select`) — can't import across modules. Use a helper method pattern (see `TaskSelect::query_all`).
- `Config::open_in_memory()` can be called multiple times (fixed from 0.4)

## Database

Default location: `~/Library/Application Support/todomocop/todomocop.db` (macOS)
Override: `TODOMOCOP_DB` env var or `--db` flag (CLI only)

## Known Limitations / Tech Debt

- **N+1 queries in list_tasks**: Each task triggers 2 extra queries to load GitHub/Linear contexts. Fine for now, batch if task count grows.
- **search triggers refresh**: `search` calls `list_tasks` which refreshes all stale contexts, even for tasks that won't match. Could filter before loading contexts.
- **Duplicated UreqHttpClient**: Both mcp and cli define the same `UreqHttpClient`. Could extract to a shared crate or feature-gated impl in core.
- **Db fields are pub**: Should be `pub(crate)` or private with accessor methods.
- **No --db flag on MCP server**: clap is a dependency but unused. CLI has --db.
- **Linear refresh uses filter query**: Parses "ENG-123" into team key + number for a GraphQL filter. This should work but hasn't been tested against a real Linear API.

## Future Work (from spec)

- **Sync**: CRDT-style merge between local and remote DBs, filtered by workspace
- **Recurrence**: Separate table with cron expressions, materializes tasks on query
- **Slack ingestion**: Fetch reminders from Slack API, summarize, create tasks
- **Screenshot capture**: macOS app, captures screenshot, AI-captions, creates task with attachment
- **Full-text search**: Upgrade from LIKE to SQLite FTS5
