# Todomocop

Personal TODO management system with MCP interface for Claude Code.

## Build & Test

```bash
nix develop
cargo test --workspace
cargo build --release
```

## Project Structure

- `core/` — library crate (all DB logic, rust-query 0.8)
- `mcp/` — MCP server binary (rmcp, stdio)
- `cli/` — CLI binary (clap)

## Conventions

- rust-query uses `external_id` (not `id`, which is reserved)
- All `Db` methods take `&self` (transactions are closure-based)
- Tests in `core/src/task.rs` — can use separate `Db::open_in_memory()` per test
- Dates are ISO 8601 strings (YYYY-MM-DD), validated on input

See `docs/development.md` for architecture details and rust-query patterns.
