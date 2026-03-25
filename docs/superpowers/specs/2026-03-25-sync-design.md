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

`canceled` tasks are hidden from default list views, same as `done`.

### Multiple external links per task

A task can have multiple GitHub PRs (e.g. frontend + backend PRs for one Linear ticket) and theoretically multiple Linear issues.

**Current state:** `Task` holds `Option<GithubPrContextData>` and `Option<LinearContextData>` (singular). `load_github_context` / `load_linear_context` return only the first match.

**Change:** `Task` fields become `Vec<GithubPrContextData>` and `Vec<LinearContextData>`. Load methods return all linked contexts. Refresh methods iterate over all. MCP/CLI output includes all links.

This is a prerequisite for both sync and correct completion logic.

### Lookup-by-source queries

New methods on `Db`:

- `find_task_by_github_pr(pr_url: &str) -> Result<Option<Task>>` — find the task linked to a given PR URL.
- `find_task_by_linear_issue(identifier: &str) -> Result<Option<Task>>` — find the task linked to a given Linear identifier (e.g. "ENG-123").

These are the dedup primitives used by sync.

## Sync Crate

### GitHub sync

**Input:** GitHub API — fetch PRs where the authenticated user is author or requested reviewer.

**API calls:**
- `GET /search/issues?q=type:pr+author:{user}+is:open` — open authored PRs
- `GET /search/issues?q=type:pr+review-requested:{user}+is:open` — open review requests
- `GET /search/issues?q=type:pr+author:{user}+is:closed+merged:>={since}` — recently merged/closed authored PRs (for completion)
- `GET /search/issues?q=type:pr+reviewed-by:{user}+is:closed+merged:>={since}` — recently closed PRs the user reviewed (for review task completion)

The `{user}` is fetched once via `GET /user`. The `{since}` window is configurable (default: 7 days).

**Reconciliation rules:**

| Condition | Action |
|-----------|--------|
| Authored PR, no matching task | Create task (title from PR title), link PR |
| Review-requested PR, no matching task | Create task ("Review: {PR title}"), link PR |
| Authored PR merged, task has no Linear link | Mark task `done` |
| Authored PR closed (not merged) | No status change; link is ignored for completion |
| User submitted review on PR | Mark review task `done` |

**How to detect "user submitted review":** `GET /repos/{owner}/{repo}/pulls/{number}/reviews` — check if any review has `user.login == authenticated_user`. If yes, the review task is done.

### Linear sync

**Input:** Linear GraphQL API — fetch issues assigned to the authenticated user.

**API call:**
```graphql
{
  viewer {
    id
    assignedIssues(
      filter: { state: { type: { nin: ["triage"] } } }
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

This gives us all non-triage issues assigned to the user. We use `state.type` (Linear's built-in state categories: `backlog`, `unstarted`, `started`, `completed`, `canceled`) for logic.

**Reconciliation rules:**

| Condition | Action |
|-----------|--------|
| Assigned issue, no matching task | Create task (title from issue title), link Linear |
| Issue state type `completed` | Mark task `done` |
| Issue state type `canceled` | Mark task `canceled` |
| Issue previously assigned, now unassigned (not in API results but task exists) | Mark task `canceled` |

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

**Review tasks** (GitHub only, never linked to Linear):
- Done when the user has submitted a review, regardless of PR state.

**No external links:**
- Manual status management only.

### Task creation details

- **Workspace:** `"work"` for both GitHub and Linear tasks (configurable later if needed).
- **Priority:** For Linear tasks, map Linear priority (1=urgent, 2=high, 3=medium, 4=low) directly to todomocop priority. GitHub tasks get no priority.
- **Status:** New tasks are created as `ready`. If Linear state type is `started`, create as `in_progress`.

## CLI Interface

```
todo sync              # sync both GitHub and Linear
todo sync github       # sync GitHub PRs only
todo sync linear       # sync Linear issues only
```

All commands read `GITHUB_TOKEN` and `LINEAR_API_KEY` from environment variables (same as existing CLI).

Output: summary of actions taken (e.g. "Created 2 tasks, updated 1, completed 1").

## Testing Strategy

### Unit tests (sync crate)

The reconciliation logic is pure: given a list of external items and a list of existing tasks, produce a list of actions (create, update status, etc.). This core is tested with mock data, no API or DB needed.

```rust
struct SyncAction {
    kind: SyncActionKind, // Create, MarkDone, MarkCanceled
    // ...
}

fn reconcile_github(prs: &[GithubPr], existing_tasks: &[Task]) -> Vec<SyncAction>;
fn reconcile_linear(issues: &[LinearIssue], existing_tasks: &[Task]) -> Vec<SyncAction>;
```

Test cases:
- New PR with no existing task → Create action
- Existing task, PR merged, no Linear link → MarkDone action
- Existing task, PR merged, has Linear link → no action (Linear is authoritative)
- Existing task, PR closed (not merged) → no action
- Existing task, two PRs: one merged, one open, no Linear → no action (not all done)
- Existing task, two PRs: one merged, one closed (not merged), no Linear → MarkDone (closed is ignored)
- Review PR, user has submitted review → MarkDone action
- Linear issue completed → MarkDone action
- Linear issue canceled → MarkCanceled action
- Linear issue unassigned → MarkCanceled action
- Linear issue assigned, existing task already done → no action (don't resurrect)

### Integration tests (sync crate + core)

Use `Db::open_in_memory()` with mock `HttpClient` to test the full flow: API response → reconcile → DB mutations → verify task state.

### Core tests

Test new `find_task_by_github_pr` and `find_task_by_linear_issue` queries against in-memory DB.
Test `canceled` status works correctly in list/search filters.
