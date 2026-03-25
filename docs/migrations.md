# Database Migrations

## How rust-query migrations work

The schema is defined in `core/src/schema.rs` using `#[schema(TodoSchema)]` with `#[version(0..=N)]`. The `vN` module is a template — the macro generates separate `v0`, `v1`, etc. modules.

- New columns use `#[version(1..)]` to indicate they only exist from v1 onwards.
- The migration closure in `Db::open` (`core/src/lib.rs`) provides default values for new columns when upgrading an existing database.
- `Db::open_in_memory()` creates a fresh database at the latest version — no migration needed.

## Current schema version: v1

v0 → v1 changes:
- `GithubPrContext`: added `title`, `author`, `reviewers`, `review_state` columns, added `#[unique]` on `url`
- `LinearContext`: added `state_type` column, added `#[unique]` on `identifier`

## Important limitations

### Adding `#[unique]` to existing columns requires a fresh database

rust-query validates that the database's unique constraints match the schema definition at each version. If you add `#[unique]` to a column that existed in a prior version without it, rust-query will panic when opening an old database — even with a migration closure.

**Workarounds:**
1. **Drop and recreate the database** (acceptable for personal tool, not for production)
2. **Version-split the column**: rename the old column with `#[version(..N)]` suffix, create a new unique column with `#[version(N..)]`, and copy the value in the migration. However, rust-query maps Rust field names directly to SQLite column names, so the old column name must match exactly — this creates naming conflicts.
3. **Skip DB-level uniqueness**: remove `#[unique]` and enforce uniqueness in application code.

We chose option 1 for the v0→v1 migration. This means **existing v0 databases cannot be migrated** — they must be deleted and recreated via `todo sync`.

### Database location

```
~/Library/Application Support/todomocop/todomocop.db
```

Override: `TODOMOCOP_DB` env var or `--db` flag (CLI only).

To reset: `rm ~/Library/Application\ Support/todomocop/todomocop.db` then run `todo sync`.

## Adding new columns in the future

1. Bump the version range: `#[version(0..=2)]`
2. Add new columns with `#[version(2..)]`
3. Add a second `.migrate()` call in `Db::open` for v1→v2
4. Provide defaults for new columns in the migration closure
5. Do NOT add `#[unique]` to columns that existed in prior versions (see above)
6. Run `cargo test --workspace` — tests use in-memory DBs at the latest version
7. **Manually test** against a real database file to verify the migration works
