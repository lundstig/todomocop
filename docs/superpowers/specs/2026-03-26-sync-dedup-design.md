# Cross-Source Dedup Design

Prevent duplicate tasks when the same work item exists as both a GitHub PR and a Linear issue.

## Problem

GitHub sync and Linear sync independently create tasks. When a PR corresponds to a Linear issue (common case), both syncs create separate tasks for the same work. The cross-source completion logic (Linear is authoritative when both links exist) only works when both links are on the *same* task.

## Approach

Linear's API exposes GitHub PR attachments on issues (`attachments(filter: { sourceType: { eq: "github" } })`). This is the source of truth for the PR↔issue relationship. We use it for dedup.

## Data Changes

### LinearIssue type (sync crate)

Add a field for attached PR URLs:

```rust
pub struct LinearIssue {
    // ... existing fields ...
    pub github_pr_urls: Vec<String>,  // NEW: from Linear attachments
}
```

### Linear GraphQL query

Extend the existing query with:

```graphql
attachments(filter: { sourceType: { eq: "github" } }) {
  nodes { url }
}
```

### New SyncAction variants

```rust
LinkLinearToExistingTask {
    task_id: TaskId,
    linear: CreateLinearLink,
}
LinkGithubPrToExistingTask {
    task_id: TaskId,
    pr_url: String,
}
```

## Reconciliation Changes

### Linear sync — dedup on create

Before creating a task for a new Linear issue, check if any attached PR URL matches an existing task via the task's `github_pr_contexts`. If yes, emit `LinkLinearToExistingTask` instead of `CreateTaskWithLinear`.

```
for issue in new_issues:
  for pr_url in issue.github_pr_urls:
    if existing_task has github_pr_context with pr_url:
      emit LinkLinearToExistingTask(task_id, linear_link)
      skip to next issue
  emit CreateTaskWithLinear (as before)
```

### Linear sync — link new PRs to existing tasks

For Linear issues that already have a linked task, check if there are PR attachments not yet in the task's `github_pr_contexts`. If yes, emit `LinkGithubPrToExistingTask`.

```
for issue in issues_with_existing_task:
  for pr_url in issue.github_pr_urls:
    if pr_url not in task.github_pr_contexts:
      emit LinkGithubPrToExistingTask(task_id, pr_url)
```

### GitHub sync — unchanged

`reconcile_github` already calls `find_task_by_github_pr` for dedup. If Linear sync already linked the PR to a task, GitHub sync finds the task and skips creation.

## Ordering Guarantee

Sync ordering does not matter:

- **Linear first**: creates task with both Linear link and PR link → GitHub sync finds PR already linked, skips
- **GitHub first**: creates PR-only task → Linear sync finds task via PR URL, links Linear to it
- **Both re-run**: idempotent, no duplicates

## Runner Changes

Two new action handlers:

- `LinkLinearToExistingTask`: calls `db.link_linear(task_id, &linear.url)`
- `LinkGithubPrToExistingTask`: calls `db.link_github_pr(task_id, &pr_url)`

## Testing

### Unit tests (reconcile)

- Linear issue with attached PR, PR task already exists → `LinkLinearToExistingTask` (not Create)
- Linear issue with attached PR, no task exists → `CreateTaskWithLinear` (creates with both links later via runner)
- Existing Linear task, new PR attachment on issue → `LinkGithubPrToExistingTask`
- Existing Linear task, PR already linked → no action
- Linear issue with attached PR, PR task exists, Linear task also exists → no action (already linked)

### Integration tests

- GitHub sync first, then Linear sync → one task with both links
- Linear sync first, then GitHub sync → one task with both links
- Re-run both → idempotent
