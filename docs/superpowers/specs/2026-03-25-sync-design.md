# Sync Design

Automatic synchronization of GitHub PRs and Linear issues into todomocop tasks.

## Architecture

```
core/     — DB, task CRUD (+ canceled status, lookup-by-source queries)
sync/     — new lib crate: GitHub + Linear API clients, reconciliation logic
mcp/      — MCP server (unchanged)
cli/      — CLI (+ `todo sync github`, `todo sync linear`, `todo sync`)
```

`sync` depends on `core` (for DB access) and reuses `core::http::HttpClient` for API calls.
`cli` depends on `sync` and `core`.

## Core Changes

### New status: `canceled`

Added to `TaskStatus`: `idea | ready | in_progress | done | canceled`.

Currently neither `done` nor `canceled` tasks are excluded by default in `list_tasks` — the caller must pass a status filter. This is unchanged; the CLI and MCP layers decide what to show.

### Multiple external links per task

A task can have multiple GitHub PRs (e.g. frontend + backend PRs for one Linear ticket) and theoretically multiple Linear issues.

**Current state:** `Task` holds `Option<GithubPrContextData>` and `Option<LinearContextData>` (singular). `load_github_context` / `load_linear_context` return only the first match.

**Change:** `Task` fields become `Vec<GithubPrContextData>` and `Vec<LinearContextData>`. Load methods return all linked contexts. Refresh methods iterate over all. MCP/CLI output includes all links.

This is a breaking change to the MCP JSON output (field goes from `null`/object to array). Since this is a personal tool with no external consumers, we change it in place.

### Uniqueness constraints

Add `#[unique]` to `GithubPrContext.url` and `LinearContext.identifier`. A given PR or Linear issue can only be linked to one task. This makes `find_task_by_*` unambiguous and prevents duplicate links on re-sync.

### Lookup-by-source queries

New methods on `Db`:

- `find_task_by_github_pr(pr_url: &str) -> Result<Option<Task>>` — find the task linked to a given PR URL.
- `find_task_by_linear_issue(identifier: &str) -> Result<Option<Task>>` — find the task linked to a given Linear identifier (e.g. "ENG-123").

These are the dedup primitives used by sync.

## Sync Crate

### Design

The reconciliation logic is **pure**: given external items + existing tasks, it produces a list of `SyncAction`s. The sync runner then applies each action as an individual DB transaction. This means partial failures are safe — re-running sync will skip already-applied actions via the dedup queries.

```rust
enum SyncAction {
    CreateTask { title, link, workspace, priority, status },
    MarkDone { task_id },
    MarkCanceled { task_id },
    UpdateStatus { task_id, status },  // e.g. Linear started → in_progress
}

fn reconcile_github(prs: &[GithubPr], existing: &[Task]) -> Vec<SyncAction>;
fn reconcile_linear(issues: &[LinearIssue], existing: &[Task]) -> Vec<SyncAction>;
```

### GitHub sync

**Input:** GitHub API — fetch PRs where the authenticated user is author or requested reviewer.

**API calls:**
- `GET /search/issues?q=type:pr+author:{user}+is:open` — open authored PRs
- `GET /search/issues?q=type:pr+review-requested:{user}+is:open` — open review requests
- `GET /search/issues?q=type:pr+author:{user}+is:closed+merged:>={since}` — recently merged/closed authored PRs (for completion)

For review-requested PRs that are still open, also check `GET /repos/{owner}/{repo}/pulls/{number}/reviews` to detect if the user has already submitted a review.

The `{user}` is fetched once via `GET /user`. The `{since}` window defaults to 7 days, configurable via `--since` CLI flag.

**Known limitations:** GitHub search API is rate-limited (30 req/min) and capped at 1000 results. Acceptable for personal use.

**Reconciliation rules:**

| Condition | Action |
|-----------|--------|
| Authored PR, no matching task | Create task (title from PR title), link PR |
| Review-requested PR, no matching task | Create task ("Review: {PR title}"), link PR |
| Authored PR merged, task has no Linear link | Mark task `done` |
| Authored PR closed (not merged) | No status change; link is ignored for completion |
| User submitted review on PR (open or closed) | Mark review task `done` |
| Merged PR found but no existing task | Ignore (don't create retroactive tasks) |

### Linear sync

**Input:** Linear GraphQL API — fetch issues assigned to the authenticated user.

**API call:**
```graphql
{
  viewer {
    id
    assignedIssues(
      filter: { state: { type: { nin: ["triage", "completed", "canceled"] } } }
      first: 100
    ) {
      nodes {
        id identifier title url
        state { name type }
        priority priorityLabel
      }
    }
  }
}
```

This fetches active issues only (backlog, unstarted, started). Completed/canceled detection works by absence: if a previously-synced task's Linear issue is no longer in the results, we check why.

To distinguish "completed/canceled" from "unassigned", the sync does a targeted lookup for missing issues:
```graphql
{ issue(id: "...") { state { type } assignee { id } } }
```

**Reconciliation rules:**

| Condition | Action |
|-----------|--------|
| Assigned issue, no matching task | Create task (title from issue title), link Linear |
| Assigned issue `started`, existing task not `in_progress` | Update task to `in_progress` |
| Issue no longer in results, lookup shows `completed` | Mark task `done` |
| Issue no longer in results, lookup shows `canceled` | Mark task `canceled` |
| Issue no longer in results, lookup shows reassigned | Mark task `canceled` |

### Cross-source completion logic

A task may have multiple GitHub PRs and/or multiple Linear issues. The rules:

**Has any Linear link:**
- **Linear is authoritative.** If any Linear issue is `completed` → task `done`. If any is `canceled` and none are active → task `canceled`.
- GitHub PR state alone does not change task status.
- Closed (not merged) PRs are ignored entirely.

**Has GitHub PR(s) but no Linear link:**
- All non-ignored PRs must be merged for the task to be `done`.
- Closed (not merged) PRs are ignored (neither blocking nor satisfying completion).
- If all PRs are either merged or closed-without-merge, and at least one is merged → task `done`.

**Review tasks** (created by GitHub sync for review requests):
- Done when the user has submitted a review, regardless of PR state.
- If a user manually links a Linear issue to a review task, Linear becomes authoritative (same as any other task). This is a soft convention, not enforced.

**No external links:**
- Manual status management only.

### Task creation details

- **Workspace:** `"work"` for both GitHub and Linear tasks.
- **Priority:** For Linear tasks, map Linear priority (1=urgent, 2=high, 3=medium, 4=low) directly to todomocop priority. GitHub tasks get no priority.
- **Status:** New tasks are created as `ready`. If Linear state type is `started`, create as `in_progress`.

## CLI Interface

```
todo sync              # sync both GitHub and Linear
todo sync github       # sync GitHub PRs only
todo sync linear       # sync Linear issues only
todo sync --since 14d  # override the lookback window (default: 7d)
```

All commands read `GITHUB_TOKEN` and `LINEAR_API_KEY` from environment variables (same as existing CLI).

Output: summary of actions taken (e.g. "Created 2 tasks, updated 1, completed 1").

## Testing Strategy

### Unit tests (sync crate)

The reconciliation logic is pure — tested with mock data, no API or DB needed.

Test cases:
- New PR with no existing task → Create action
- Existing task, PR merged, no Linear link → MarkDone action
- Existing task, PR merged, has Linear link → no action (Linear is authoritative)
- Existing task, PR closed (not merged) → no action
- Existing task, two PRs: one merged, one open, no Linear → no action (not all done)
- Existing task, two PRs: one merged, one closed (not merged), no Linear → MarkDone (closed is ignored)
- Review PR, user has submitted review (PR still open) → MarkDone action
- Review PR, user has not reviewed → no action
- Merged PR found, no existing task → no action (no retroactive creation)
- Linear issue completed → MarkDone action
- Linear issue canceled → MarkCanceled action
- Linear issue unassigned → MarkCanceled action
- Linear issue `started`, existing task is `ready` → UpdateStatus to `in_progress`
- Linear issue assigned, existing task already done → no action (don't resurrect)

### Integration tests (sync crate + core)

Use `Db::open_in_memory()` with mock `HttpClient` to test the full flow: API response → reconcile → DB mutations → verify task state.

### Core tests

- `find_task_by_github_pr` and `find_task_by_linear_issue` queries against in-memory DB
- `canceled` status roundtrips correctly through add/edit/list
- Multiple links per task: load and refresh all contexts
- Uniqueness: linking same PR URL to two tasks fails
