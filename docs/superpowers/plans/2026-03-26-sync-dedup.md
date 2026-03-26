# Cross-Source Sync Dedup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent duplicate tasks when the same work item exists as both a GitHub PR and a Linear issue, using Linear's GitHub PR attachment data as the source of truth.

**Architecture:** Add `github_pr_urls` to `LinearIssue`, fetch attachments in the Linear GraphQL query, add two new `SyncAction` variants for linking to existing tasks, and update `reconcile_linear` to check for existing GitHub tasks before creating duplicates.

**Tech Stack:** Rust, todomocop-sync crate, Linear GraphQL API.

**Spec:** `docs/superpowers/specs/2026-03-26-sync-dedup-design.md`

---

## File Structure

**Modified files:**
- `sync/src/types.rs` — add `github_pr_urls` field to `LinearIssue`, add two new `SyncAction` variants
- `sync/src/linear.rs` — add `attachments` to GraphQL query, parse PR URLs
- `sync/src/reconcile.rs` — update `reconcile_linear` with dedup logic, add tests
- `sync/src/runner.rs` — handle new action variants

---

### Task 1: Add types for dedup

**Files:**
- Modify: `sync/src/types.rs`

- [ ] **Step 1: Add `github_pr_urls` to `LinearIssue`**

In `sync/src/types.rs`, add the field to the `LinearIssue` struct:

```rust
#[derive(Debug, Clone)]
pub struct LinearIssue {
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub state_type: String,
    pub priority: Option<i64>,
    pub github_pr_urls: Vec<String>,  // NEW
}
```

- [ ] **Step 2: Add new `SyncAction` variants**

Add two new variants to the `SyncAction` enum:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncAction {
    // ... existing variants unchanged ...
    LinkLinearToExistingTask {
        task_id: i64,
        linear: CreateLinearLink,
    },
    LinkGithubPrToExistingTask {
        task_id: i64,
        pr_url: String,
    },
}
```

- [ ] **Step 3: Fix all compile errors from the new field**

The `LinearIssue` struct is constructed in several places. Add `github_pr_urls: vec![]` to all of them:
- `sync/src/linear.rs` in `parse_issue`
- `sync/src/reconcile.rs` in every test that creates a `LinearIssue`

Run: `nix develop --command cargo check -p todomocop-sync`
Expected: Compiles with no errors.

- [ ] **Step 4: Commit**

```
git commit -am "feat: add dedup types to sync crate"
```

---

### Task 2: Fetch PR attachments in Linear client

**Files:**
- Modify: `sync/src/linear.rs`

- [ ] **Step 1: Update the GraphQL query to include attachments**

In `sync/src/linear.rs`, replace the `query` string in `fetch_assigned_issues` with one that includes GitHub attachments. The current query is a raw JSON string on line 18. Replace it with:

```rust
let query = r#"{ "query": "{ viewer { assignedIssues(filter: { state: { type: { nin: [\"triage\", \"completed\", \"canceled\"] } } } first: 100) { nodes { id identifier title url state { name type } priority priorityLabel attachments(filter: { sourceType: { eq: \"github\" } }) { nodes { url } } } } } }" }"#;
```

- [ ] **Step 2: Update `parse_issue` to extract PR URLs**

```rust
fn parse_issue(node: &Value) -> Option<LinearIssue> {
    let identifier = node["identifier"].as_str()?.to_string();
    let title = node["title"].as_str()?.to_string();
    let url = node["url"].as_str()?.to_string();
    let state_type = node["state"]["type"].as_str()?.to_string();
    let priority = node["priority"].as_i64();

    let github_pr_urls: Vec<String> = node["attachments"]["nodes"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a["url"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Some(LinearIssue {
        identifier,
        title,
        url,
        state_type,
        priority,
        github_pr_urls,
    })
}
```

- [ ] **Step 3: Verify it compiles**

Run: `nix develop --command cargo check -p todomocop-sync`

- [ ] **Step 4: Commit**

```
git commit -am "feat: fetch GitHub PR attachments in Linear sync"
```

---

### Task 3: Update reconciliation with dedup logic + tests

**Files:**
- Modify: `sync/src/reconcile.rs`

- [ ] **Step 1: Write the dedup tests**

Add these tests to the existing `mod tests` in `sync/src/reconcile.rs`:

```rust
#[test]
fn linear_issue_with_pr_attachment_deduplicates_existing_github_task() {
    // GitHub sync already created a task for this PR
    let mut task = make_task(1, "Fix bug", TaskStatus::Ready);
    task.github_pr_contexts = vec![make_github_ctx(
        "https://github.com/o/r/pull/42",
        "open",
    )];

    // Linear issue has the same PR attached
    let issue = LinearIssue {
        identifier: "ENG-900".to_string(),
        title: "Fix bug".to_string(),
        url: "https://linear.app/team/issue/ENG-900/fix-bug".to_string(),
        state_type: "started".to_string(),
        priority: Some(1),
        github_pr_urls: vec!["https://github.com/o/r/pull/42".to_string()],
    };

    let actions = reconcile_linear(&[issue], &[task]);

    // Should link Linear to existing task, NOT create a new one
    assert!(actions.iter().any(|a| matches!(
        a,
        SyncAction::LinkLinearToExistingTask { task_id: 1, .. }
    )));
    assert!(!actions.iter().any(|a| matches!(a, SyncAction::CreateTaskWithLinear { .. })));
}

#[test]
fn linear_issue_with_pr_attachment_no_existing_task_creates_normally() {
    // No existing task for this PR
    let issue = LinearIssue {
        identifier: "ENG-901".to_string(),
        title: "New feature".to_string(),
        url: "https://linear.app/team/issue/ENG-901/new-feature".to_string(),
        state_type: "started".to_string(),
        priority: None,
        github_pr_urls: vec!["https://github.com/o/r/pull/99".to_string()],
    };

    let actions = reconcile_linear(&[issue], &[]);

    // Should create normally since no existing task matched
    assert_eq!(actions.len(), 1);
    assert!(matches!(&actions[0], SyncAction::CreateTaskWithLinear { .. }));
}

#[test]
fn existing_linear_task_gets_new_pr_attachment_linked() {
    // Task already exists with Linear link, but no GitHub PR linked
    let mut task = make_task(1, "Feature", TaskStatus::InProgress);
    task.linear_contexts = vec![make_linear_ctx("ENG-902", "started")];

    // Linear issue now has a PR attachment
    let issue = LinearIssue {
        identifier: "ENG-902".to_string(),
        title: "Feature".to_string(),
        url: "https://linear.app/team/issue/ENG-902/feature".to_string(),
        state_type: "started".to_string(),
        priority: None,
        github_pr_urls: vec!["https://github.com/o/r/pull/55".to_string()],
    };

    let actions = reconcile_linear(&[issue], &[task]);

    // Should link the PR to the existing task
    assert!(actions.iter().any(|a| matches!(
        a,
        SyncAction::LinkGithubPrToExistingTask { task_id: 1, .. }
    )));
}

#[test]
fn existing_linear_task_pr_already_linked_no_action() {
    // Task already has both Linear and GitHub PR linked
    let mut task = make_task(1, "Feature", TaskStatus::InProgress);
    task.linear_contexts = vec![make_linear_ctx("ENG-903", "started")];
    task.github_pr_contexts = vec![make_github_ctx(
        "https://github.com/o/r/pull/60",
        "open",
    )];

    // Linear issue has the same PR (already linked)
    let issue = LinearIssue {
        identifier: "ENG-903".to_string(),
        title: "Feature".to_string(),
        url: "https://linear.app/team/issue/ENG-903/feature".to_string(),
        state_type: "started".to_string(),
        priority: None,
        github_pr_urls: vec!["https://github.com/o/r/pull/60".to_string()],
    };

    let actions = reconcile_linear(&[issue], &[task]);

    // No link actions — PR is already on the task
    assert!(!actions.iter().any(|a| matches!(
        a,
        SyncAction::LinkGithubPrToExistingTask { .. } | SyncAction::LinkLinearToExistingTask { .. }
    )));
}

#[test]
fn linear_issue_with_pr_attachment_existing_linear_task_also_exists() {
    // Both a GitHub task and a Linear task exist for the same PR
    // This shouldn't happen in normal flow, but be safe
    let mut gh_task = make_task(1, "Fix bug (GH)", TaskStatus::Ready);
    gh_task.github_pr_contexts = vec![make_github_ctx(
        "https://github.com/o/r/pull/70",
        "open",
    )];

    let mut linear_task = make_task(2, "Fix bug (Linear)", TaskStatus::InProgress);
    linear_task.linear_contexts = vec![make_linear_ctx("ENG-904", "started")];

    let issue = LinearIssue {
        identifier: "ENG-904".to_string(),
        title: "Fix bug".to_string(),
        url: "https://linear.app/team/issue/ENG-904/fix-bug".to_string(),
        state_type: "started".to_string(),
        priority: None,
        github_pr_urls: vec!["https://github.com/o/r/pull/70".to_string()],
    };

    let actions = reconcile_linear(&[issue], &[gh_task, linear_task]);

    // Linear is already linked to task 2, so no LinkLinearToExistingTask.
    // But PR on task 1 should be linked to task 2 (or at minimum, no duplicate creation).
    // The key assertion: no CreateTaskWithLinear
    assert!(!actions.iter().any(|a| matches!(a, SyncAction::CreateTaskWithLinear { .. })));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `nix develop --command cargo test -p todomocop-sync`
Expected: New tests fail (dedup logic not yet implemented).

- [ ] **Step 3: Update `reconcile_linear` with dedup logic**

Replace the "new issues" section of `reconcile_linear` (the first `for issue in issues` loop, lines 137-158) with:

```rust
    // Also index tasks by their linked PR URLs (for cross-source dedup)
    let mut task_by_pr_url: HashMap<&str, &Task> = HashMap::new();
    for task in existing {
        for ctx in &task.github_pr_contexts {
            task_by_pr_url.insert(&ctx.url, task);
        }
    }

    let mut actions = Vec::new();

    // Check for new issues not linked to any task
    for issue in issues {
        if task_by_linear_id.contains_key(issue.identifier.as_str()) {
            // Issue already linked to a task — check for new PR attachments to link
            let task = task_by_linear_id[issue.identifier.as_str()];
            for pr_url in &issue.github_pr_urls {
                let already_linked = task.github_pr_contexts.iter().any(|ctx| &ctx.url == pr_url);
                if !already_linked {
                    actions.push(SyncAction::LinkGithubPrToExistingTask {
                        task_id: task.id,
                        pr_url: pr_url.clone(),
                    });
                }
            }
            continue;
        }

        // Issue not yet linked to any task — check if any of its PR attachments
        // match an existing task (cross-source dedup)
        let mut linked_to_existing = false;
        for pr_url in &issue.github_pr_urls {
            if let Some(task) = task_by_pr_url.get(pr_url.as_str()) {
                // Found a GitHub task for this PR — link Linear to it
                actions.push(SyncAction::LinkLinearToExistingTask {
                    task_id: task.id,
                    linear: CreateLinearLink {
                        identifier: issue.identifier.clone(),
                        url: issue.url.clone(),
                        state_type: issue.state_type.clone(),
                    },
                });
                linked_to_existing = true;
                break;
            }
        }

        if linked_to_existing {
            continue;
        }

        // No existing task at all — create new
        let status = if issue.state_type == "started" {
            TaskStatus::InProgress
        } else {
            TaskStatus::Ready
        };

        actions.push(SyncAction::CreateTaskWithLinear {
            title: issue.title.clone(),
            linear: CreateLinearLink {
                identifier: issue.identifier.clone(),
                url: issue.url.clone(),
                state_type: issue.state_type.clone(),
            },
            status,
            priority: issue.priority,
        });
    }
```

The rest of the function (the "existing tasks" loop checking for completion/cancellation) stays unchanged.

- [ ] **Step 4: Run tests**

Run: `nix develop --command cargo test -p todomocop-sync`
Expected: All tests pass (old and new).

- [ ] **Step 5: Commit**

```
git commit -am "feat: cross-source dedup in Linear reconciliation"
```

---

### Task 4: Handle new actions in runner

**Files:**
- Modify: `sync/src/runner.rs`

- [ ] **Step 1: Write integration test for dedup actions**

Add to the existing `mod tests` in `sync/src/runner.rs`:

```rust
#[test]
fn integration_link_linear_to_existing_github_task() {
    let db = Db::open_in_memory().unwrap();

    // First: create a task via GitHub sync
    let create = vec![SyncAction::CreateTaskWithGithub {
        title: "Fix bug".into(),
        pr: CreateGithubLink {
            url: "https://github.com/o/r/pull/50".into(),
            repo: "o/r".into(),
            number: 50,
            title: "Fix bug".into(),
            author: "me".into(),
            reviewers: Vec::new(),
            review_state: HashMap::new(),
        },
        status: TaskStatus::Ready,
    }];
    apply_actions(&db, &create);

    let tasks = db.list_tasks(TaskFilter::default()).unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].linear_contexts.is_empty());

    // Now: link Linear to the same task
    let link = vec![SyncAction::LinkLinearToExistingTask {
        task_id: tasks[0].id,
        linear: CreateLinearLink {
            identifier: "ENG-50".into(),
            url: "https://linear.app/t/issue/ENG-50/fix-bug".into(),
            state_type: "started".into(),
        },
    }];
    let summary = apply_actions(&db, &link);
    assert_eq!(summary.updated, 1);
    assert!(summary.errors.is_empty());

    // Verify task now has both contexts
    let task = db.get_task(tasks[0].id).unwrap().unwrap();
    assert_eq!(task.github_pr_contexts.len(), 1);
    assert_eq!(task.linear_contexts.len(), 1);
    assert_eq!(task.linear_contexts[0].identifier, "ENG-50");
}

#[test]
fn integration_link_github_pr_to_existing_linear_task() {
    let db = Db::open_in_memory().unwrap();

    // First: create a task via Linear sync
    let create = vec![SyncAction::CreateTaskWithLinear {
        title: "Build feature".into(),
        linear: CreateLinearLink {
            identifier: "ENG-60".into(),
            url: "https://linear.app/t/issue/ENG-60/build-feature".into(),
            state_type: "started".into(),
        },
        status: TaskStatus::InProgress,
        priority: None,
    }];
    apply_actions(&db, &create);

    let tasks = db.list_tasks(TaskFilter::default()).unwrap();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].github_pr_contexts.is_empty());

    // Now: link a GitHub PR to the same task
    let link = vec![SyncAction::LinkGithubPrToExistingTask {
        task_id: tasks[0].id,
        pr_url: "https://github.com/o/r/pull/60".into(),
    }];
    let summary = apply_actions(&db, &link);
    assert_eq!(summary.updated, 1);
    assert!(summary.errors.is_empty());

    // Verify task now has both contexts
    let task = db.get_task(tasks[0].id).unwrap().unwrap();
    assert_eq!(task.github_pr_contexts.len(), 1);
    assert_eq!(task.linear_contexts.len(), 1);
    assert_eq!(task.github_pr_contexts[0].url, "https://github.com/o/r/pull/60");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `nix develop --command cargo test -p todomocop-sync`
Expected: New tests fail (action variants not handled yet).

- [ ] **Step 3: Add handlers for new action variants**

In `sync/src/runner.rs`, add two arms to the `match action` in `apply_one`:

```rust
SyncAction::LinkLinearToExistingTask { task_id, linear } => {
    db.link_linear(*task_id, &linear.url)?;
    Ok(ActionKind::Updated)
}
SyncAction::LinkGithubPrToExistingTask { task_id, pr_url } => {
    db.link_github_pr(*task_id, pr_url)?;
    Ok(ActionKind::Updated)
}
```

- [ ] **Step 4: Run all tests**

Run: `nix develop --command cargo test --workspace`
Expected: All tests pass.

- [ ] **Step 5: Commit**

```
git commit -am "feat: handle dedup link actions in sync runner"
```

---

### Task 5: Full round-trip integration test

**Files:**
- Modify: `sync/src/runner.rs` (add to existing tests)

- [ ] **Step 1: Write the full round-trip dedup test**

Add to the existing `mod tests` in `sync/src/runner.rs`:

```rust
#[test]
fn integration_github_first_then_linear_deduplicates() {
    let db = Db::open_in_memory().unwrap();

    // Step 1: GitHub sync creates a task for a PR
    let gh_actions = vec![SyncAction::CreateTaskWithGithub {
        title: "Fix bug".into(),
        pr: CreateGithubLink {
            url: "https://github.com/o/r/pull/100".into(),
            repo: "o/r".into(),
            number: 100,
            title: "Fix bug".into(),
            author: "me".into(),
            reviewers: Vec::new(),
            review_state: HashMap::new(),
        },
        status: TaskStatus::Ready,
    }];
    let summary = apply_actions(&db, &gh_actions);
    assert_eq!(summary.created, 1);

    // Step 2: Linear sync runs — issue has this PR attached
    let existing = db.list_tasks(TaskFilter::default()).unwrap();
    assert_eq!(existing.len(), 1);

    let issue = crate::types::LinearIssue {
        identifier: "ENG-100".into(),
        title: "Fix bug".into(),
        url: "https://linear.app/t/issue/ENG-100/fix-bug".into(),
        state_type: "started".into(),
        priority: Some(1),
        github_pr_urls: vec!["https://github.com/o/r/pull/100".into()],
    };

    let linear_actions = crate::reconcile::reconcile_linear(&[issue], &existing);

    // Should produce a link action, NOT a create action
    assert!(!linear_actions.iter().any(|a| matches!(a, SyncAction::CreateTaskWithLinear { .. })));
    assert!(linear_actions.iter().any(|a| matches!(a, SyncAction::LinkLinearToExistingTask { .. })));

    // Step 3: Apply the link actions
    let summary = apply_actions(&db, &linear_actions);
    assert!(summary.errors.is_empty());

    // Step 4: Verify — still one task, now with both links
    let tasks = db.list_tasks(TaskFilter::default()).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].github_pr_contexts.len(), 1);
    assert_eq!(tasks[0].linear_contexts.len(), 1);
    assert_eq!(tasks[0].linear_contexts[0].identifier, "ENG-100");
}

#[test]
fn integration_linear_first_then_github_deduplicates() {
    let db = Db::open_in_memory().unwrap();

    // Step 1: Linear sync creates a task with a PR attachment
    let linear_issue = crate::types::LinearIssue {
        identifier: "ENG-200".into(),
        title: "New feature".into(),
        url: "https://linear.app/t/issue/ENG-200/new-feature".into(),
        state_type: "started".into(),
        priority: None,
        github_pr_urls: vec!["https://github.com/o/r/pull/200".into()],
    };

    let linear_actions = crate::reconcile::reconcile_linear(&[linear_issue], &[]);
    // No existing tasks, so it creates
    assert!(linear_actions.iter().any(|a| matches!(a, SyncAction::CreateTaskWithLinear { .. })));
    apply_actions(&db, &linear_actions);

    let tasks = db.list_tasks(TaskFilter::default()).unwrap();
    assert_eq!(tasks.len(), 1);

    // Step 2: GitHub sync runs — same PR exists
    let pr = crate::types::GithubPr {
        url: "https://github.com/o/r/pull/200".into(),
        repo: "o/r".into(),
        number: 200,
        title: "New feature".into(),
        state: crate::types::GithubPrState::Open,
        author: "me".into(),
        reviewers: Vec::new(),
        review_state: HashMap::new(),
    };

    // But wait — the task created by Linear sync doesn't have a github_pr_context
    // because CreateTaskWithLinear only links Linear, not the PR.
    // The PR attachment linking happens via LinkGithubPrToExistingTask in the SAME
    // Linear reconcile run. Let's verify that happened:
    // Actually, CreateTaskWithLinear in the runner only calls link_linear, not link_github_pr.
    // The reconcile would need to also emit LinkGithubPrToExistingTask for the newly created task.
    // BUT the task doesn't exist yet during reconciliation (it's a Create action).
    //
    // This means: after Linear sync creates the task, GitHub sync will NOT find it via
    // find_task_by_github_pr because the PR wasn't linked. GitHub sync creates a duplicate!
    //
    // This is a gap. For now, verify the current behavior and document it.
    let gh_actions = crate::reconcile::reconcile_github(&[pr], &tasks, "me");

    // With current implementation: GitHub sync won't find the PR linked to the task,
    // so it would try to create. This test documents the gap.
    // Once we fix CreateTaskWithLinear to also link attached PRs, this should produce 0 actions.
    if gh_actions.iter().any(|a| matches!(a, SyncAction::CreateTaskWithGithub { .. })) {
        panic!(
            "BUG: GitHub sync would create a duplicate! \
             CreateTaskWithLinear must also link attached PRs. \
             Fix: either emit LinkGithubPr actions for PR attachments during creation, \
             or have the runner link PRs when creating a Linear task."
        );
    }
}
```

- [ ] **Step 2: Run tests**

Run: `nix develop --command cargo test -p todomocop-sync`

The second test (`linear_first_then_github`) will likely fail, revealing the gap: when Linear sync creates a new task, PR attachments from the issue need to also be linked to the new task. Otherwise the subsequent GitHub sync creates a duplicate.

**Fix options:**
1. Have `CreateTaskWithLinear` carry the `github_pr_urls` so the runner links them during creation
2. Have `reconcile_linear` emit separate `LinkGithubPrToExistingTask` actions after creation — but the task doesn't exist yet during reconciliation

Option 1 is cleaner. Add `github_pr_urls: Vec<String>` to the `CreateTaskWithLinear` variant, and have the runner call `link_github_pr` for each URL after creating the task.

- [ ] **Step 3: Fix `CreateTaskWithLinear` to carry PR URLs**

In `sync/src/types.rs`, update the variant:

```rust
CreateTaskWithLinear {
    title: String,
    linear: CreateLinearLink,
    status: TaskStatus,
    priority: Option<i64>,
    github_pr_urls: Vec<String>,  // NEW
},
```

In `sync/src/runner.rs`, update the handler:

```rust
SyncAction::CreateTaskWithLinear { title, linear, status, priority, github_pr_urls } => {
    let task_id = db.add_task(AddTask {
        title: title.clone(),
        description: None,
        status: Some(*status),
        priority: *priority,
        workspace: "work".into(),
        deadline: None,
        planned_date: None,
    })?;
    db.link_linear(task_id, &linear.url)?;
    for pr_url in github_pr_urls {
        // Best effort — PR might not be a valid GitHub URL if Linear has odd attachments
        let _ = db.link_github_pr(task_id, pr_url);
    }
    Ok(ActionKind::Created)
}
```

In `sync/src/reconcile.rs`, update where `CreateTaskWithLinear` is constructed (two places — the creation in the dedup logic and the original creation):

```rust
actions.push(SyncAction::CreateTaskWithLinear {
    title: issue.title.clone(),
    linear: CreateLinearLink { ... },
    status,
    priority: issue.priority,
    github_pr_urls: issue.github_pr_urls.clone(),  // NEW
});
```

Fix all test constructors that build `CreateTaskWithLinear` to include `github_pr_urls: vec![]`.

- [ ] **Step 4: Run all tests**

Run: `nix develop --command cargo test --workspace`
Expected: All tests pass, including both new integration tests.

- [ ] **Step 5: Commit**

```
git commit -am "feat: link PR attachments when creating Linear tasks, full round-trip tests"
```

---

### Task 6: End-to-end verification

- [ ] **Step 1: Run against real APIs with a fresh DB**

```bash
rm -f /tmp/todomocop-dedup-test.db
LINEAR_API_KEY=$(cat /run/secrets/linear_api_key) GITHUB_TOKEN=$(gh auth token) \
  nix develop --command cargo run -p todomocop-cli -- --db /tmp/todomocop-dedup-test.db sync
```

Expect: tasks are created, and items that exist in both GitHub and Linear produce only one task.

- [ ] **Step 2: Verify no duplicates**

```bash
nix develop --command cargo run -p todomocop-cli -- --db /tmp/todomocop-dedup-test.db list
```

Check: titles that match between GitHub PRs and Linear issues appear only once.

- [ ] **Step 3: Verify idempotency**

```bash
LINEAR_API_KEY=$(cat /run/secrets/linear_api_key) GITHUB_TOKEN=$(gh auth token) \
  nix develop --command cargo run -p todomocop-cli -- --db /tmp/todomocop-dedup-test.db sync
```

Expect: `Created 0, completed 0, canceled 0, updated 0`

- [ ] **Step 4: Clean up**

```bash
rm -f /tmp/todomocop-dedup-test.db
```
