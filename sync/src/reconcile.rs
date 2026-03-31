use std::collections::HashMap;

use todomocop_core::types::{Task, TaskStatus};

use crate::types::{
    CreateGithubLink, CreateLinearLink, GithubPr, GithubPrState, LinearIssue, SyncAction,
};

/// Reconcile GitHub PRs against existing tasks, producing sync actions.
///
/// Logic:
/// - Index existing tasks by their linked PR URLs
/// - For each PR, determine role (author vs reviewer) and produce actions
pub fn reconcile_github(prs: &[GithubPr], existing: &[Task], me: &str) -> Vec<SyncAction> {
    // Index tasks by PR URL for quick lookup
    let mut task_by_pr_url: HashMap<&str, &Task> = HashMap::new();
    for task in existing {
        for ctx in &task.github_pr_contexts {
            task_by_pr_url.insert(&ctx.url, task);
        }
    }

    let mut actions = Vec::new();
    let mut handled_task_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();

    for pr in prs {
        let is_author = pr.author == me;
        let is_reviewer = pr.reviewers.contains(&me.to_string());

        if !is_author && !is_reviewer {
            continue;
        }

        match task_by_pr_url.get(pr.url.as_str()) {
            None => {
                // No existing task — skip if PR is already merged or closed
                if pr.state != GithubPrState::Open {
                    continue;
                }

                let title = if is_reviewer && !is_author {
                    format!("Review: {}", pr.title)
                } else {
                    pr.title.clone()
                };

                actions.push(SyncAction::CreateTaskWithGithub {
                    title,
                    pr: CreateGithubLink {
                        url: pr.url.clone(),
                        repo: pr.repo.clone(),
                        number: pr.number,
                        title: pr.title.clone(),
                        author: pr.author.clone(),
                        reviewers: pr.reviewers.clone(),
                        review_state: pr.review_state.clone(),
                    },
                    status: TaskStatus::Ready,
                });
            }
            Some(task) => {
                // Skip if we already emitted an action for this task
                if handled_task_ids.contains(&task.id) {
                    continue;
                }

                // Existing task — skip if already done or canceled
                if task.status == TaskStatus::Done || task.status == TaskStatus::Canceled {
                    continue;
                }

                // Reviewer task: check if review is no longer pending
                if is_reviewer && !is_author {
                    if let Some(state) = pr.review_state.get(me) {
                        if state != "pending" {
                            handled_task_ids.insert(task.id);
                            actions.push(SyncAction::MarkDone { task_id: task.id });
                        }
                    }
                    continue;
                }

                // Author task with Linear links: skip (Linear is authoritative)
                if !task.linear_contexts.is_empty() {
                    continue;
                }

                // Author task without Linear links: check if ALL non-closed PRs on this task are merged
                let all_non_closed_merged = task.github_pr_contexts.iter().all(|ctx| {
                    let pr_data = prs.iter().find(|p| p.url == ctx.url);
                    match pr_data {
                        Some(p) => p.state == GithubPrState::Merged || p.state == GithubPrState::Closed,
                        None => ctx.state == "merged" || ctx.state == "closed",
                    }
                });

                // Check that at least one PR is merged (not just all closed)
                let any_merged = task.github_pr_contexts.iter().any(|ctx| {
                    let pr_data = prs.iter().find(|p| p.url == ctx.url);
                    match pr_data {
                        Some(p) => p.state == GithubPrState::Merged,
                        None => ctx.state == "merged",
                    }
                });

                if all_non_closed_merged && any_merged {
                    handled_task_ids.insert(task.id);
                    actions.push(SyncAction::MarkDone { task_id: task.id });
                }
            }
        }
    }

    actions
}

/// Reconcile Linear issues against existing tasks, producing sync actions.
///
/// Logic:
/// - Index issues by identifier, index tasks by linked Linear identifiers
/// - Create tasks for new issues, update/complete existing tasks
pub fn reconcile_linear(issues: &[LinearIssue], existing: &[Task]) -> Vec<SyncAction> {
    let issue_by_id: HashMap<&str, &LinearIssue> =
        issues.iter().map(|i| (i.identifier.as_str(), i)).collect();

    // Index tasks by their linked Linear identifiers
    let mut task_by_linear_id: HashMap<&str, &Task> = HashMap::new();
    for task in existing {
        for ctx in &task.linear_contexts {
            task_by_linear_id.insert(&ctx.identifier, task);
        }
    }

    // Index tasks by their linked GitHub PR URLs for cross-source dedup
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
            continue;
        }

        // Cross-source dedup: check if any PR attachment matches an existing task
        let matched_task = issue
            .github_pr_urls
            .iter()
            .find_map(|url| task_by_pr_url.get(url.as_str()).copied());

        if let Some(task) = matched_task {
            actions.push(SyncAction::LinkLinearToExistingTask {
                task_id: task.id,
                linear: CreateLinearLink {
                    identifier: issue.identifier.clone(),
                    url: issue.url.clone(),
                    state_type: issue.state_type.clone(),
                },
            });
        } else {
            let status = if issue.state_type == "started" {
                TaskStatus::Working
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
                github_pr_urls: issue.github_pr_urls.clone(),
            });
        }
    }

    // Check existing tasks with Linear links
    // We need to deduplicate by task id since a task can have multiple linear contexts
    let mut seen_tasks: HashMap<i64, bool> = HashMap::new();
    for task in existing {
        if task.linear_contexts.is_empty() {
            continue;
        }
        if task.status == TaskStatus::Done || task.status == TaskStatus::Canceled {
            continue;
        }
        if seen_tasks.contains_key(&task.id) {
            continue;
        }
        seen_tasks.insert(task.id, true);

        // Check each linked identifier's current state
        let mut all_missing = true;
        let mut all_terminal = true;
        let mut any_completed = false;
        let mut any_started = false;

        for ctx in &task.linear_contexts {
            match issue_by_id.get(ctx.identifier.as_str()) {
                None => {
                    // Issue missing from results (unassigned)
                    // counts as terminal for the "all terminal" check
                }
                Some(issue) => {
                    all_missing = false;
                    let is_terminal =
                        issue.state_type == "completed" || issue.state_type == "canceled";
                    if !is_terminal {
                        all_terminal = false;
                    }
                    if issue.state_type == "completed" {
                        any_completed = true;
                    }
                    if issue.state_type == "started" {
                        any_started = true;
                    }
                }
            }
        }

        if all_missing {
            actions.push(SyncAction::MarkCanceled { task_id: task.id });
        } else if all_terminal {
            if any_completed {
                actions.push(SyncAction::MarkDone { task_id: task.id });
            } else {
                actions.push(SyncAction::MarkCanceled { task_id: task.id });
            }
        } else if any_started && task.status != TaskStatus::Working {
            actions.push(SyncAction::UpdateStatus {
                task_id: task.id,
                status: TaskStatus::Working,
            });
        }

        // Check for PR attachments on linked issues that aren't yet linked to the task
        let existing_pr_urls: std::collections::HashSet<&str> = task
            .github_pr_contexts
            .iter()
            .map(|ctx| ctx.url.as_str())
            .collect();

        for ctx in &task.linear_contexts {
            if let Some(issue) = issue_by_id.get(ctx.identifier.as_str()) {
                for pr_url in &issue.github_pr_urls {
                    if !existing_pr_urls.contains(pr_url.as_str()) {
                        actions.push(SyncAction::LinkGithubPrToExistingTask {
                            task_id: task.id,
                            pr_url: pr_url.clone(),
                        });
                    }
                }
            }
        }
    }

    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use todomocop_core::types::{GithubPrContextData, LinearContextData};

    const ME: &str = "karl";

    fn make_task(id: i64, title: &str, status: TaskStatus) -> Task {
        Task {
            id,
            title: title.to_string(),
            description: String::new(),
            status,
            priority: None,
            workspace: "default".to_string(),
            deadline: None,
            snooze_until: None,
            planned_date: None,
            next_action: None,
            deleted_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            github_pr_contexts: vec![],
            linear_contexts: vec![],
            tags: vec![],
        }
    }

    fn make_github_ctx(url: &str, state: &str) -> GithubPrContextData {
        GithubPrContextData {
            url: url.to_string(),
            repo: "owner/repo".to_string(),
            number: 1,
            state: state.to_string(),
            title: "PR title".to_string(),
            author: ME.to_string(),
            reviewers: vec![],
            review_state: HashMap::new(),
            last_refreshed: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn make_linear_ctx(identifier: &str, state_type: &str) -> LinearContextData {
        LinearContextData {
            url: format!("https://linear.app/team/issue/{identifier}/title"),
            identifier: identifier.to_string(),
            state_type: state_type.to_string(),
            data: serde_json::Value::Object(Default::default()),
            last_refreshed: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn make_pr(url: &str, title: &str, state: GithubPrState, author: &str) -> GithubPr {
        GithubPr {
            url: url.to_string(),
            repo: "owner/repo".to_string(),
            number: 1,
            title: title.to_string(),
            state,
            author: author.to_string(),
            reviewers: vec![],
            review_state: HashMap::new(),
        }
    }

    // --- GitHub tests ---

    #[test]
    fn new_authored_pr_creates_task() {
        let pr = make_pr(
            "https://github.com/owner/repo/pull/1",
            "Add feature",
            GithubPrState::Open,
            ME,
        );
        let actions = reconcile_github(&[pr], &[], ME);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            SyncAction::CreateTaskWithGithub { title, status, .. } => {
                assert_eq!(title, "Add feature");
                assert_eq!(*status, TaskStatus::Ready);
            }
            other => panic!("expected CreateTaskWithGithub, got {other:?}"),
        }
    }

    #[test]
    fn new_review_pr_creates_review_task() {
        let mut pr = make_pr(
            "https://github.com/owner/repo/pull/2",
            "Fix bug",
            GithubPrState::Open,
            "other-dev",
        );
        pr.reviewers = vec![ME.to_string()];
        pr.review_state
            .insert(ME.to_string(), "pending".to_string());

        let actions = reconcile_github(&[pr], &[], ME);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            SyncAction::CreateTaskWithGithub { title, status, .. } => {
                assert_eq!(title, "Review: Fix bug");
                assert_eq!(*status, TaskStatus::Ready);
            }
            other => panic!("expected CreateTaskWithGithub, got {other:?}"),
        }
    }

    #[test]
    fn review_submitted_marks_done() {
        let mut pr = make_pr(
            "https://github.com/owner/repo/pull/3",
            "Fix bug",
            GithubPrState::Open,
            "other-dev",
        );
        pr.reviewers = vec![ME.to_string()];
        pr.review_state
            .insert(ME.to_string(), "approved".to_string());

        let mut task = make_task(1, "Review: Fix bug", TaskStatus::Ready);
        task.github_pr_contexts = vec![make_github_ctx(
            "https://github.com/owner/repo/pull/3",
            "open",
        )];

        let actions = reconcile_github(&[pr], &[task], ME);
        assert_eq!(actions, vec![SyncAction::MarkDone { task_id: 1 }]);
    }

    #[test]
    fn review_pending_no_action() {
        let mut pr = make_pr(
            "https://github.com/owner/repo/pull/4",
            "Fix bug",
            GithubPrState::Open,
            "other-dev",
        );
        pr.reviewers = vec![ME.to_string()];
        pr.review_state
            .insert(ME.to_string(), "pending".to_string());

        let mut task = make_task(1, "Review: Fix bug", TaskStatus::Ready);
        task.github_pr_contexts = vec![make_github_ctx(
            "https://github.com/owner/repo/pull/4",
            "open",
        )];

        let actions = reconcile_github(&[pr], &[task], ME);
        assert!(actions.is_empty());
    }

    #[test]
    fn authored_pr_merged_no_linear_marks_done() {
        let pr = make_pr(
            "https://github.com/owner/repo/pull/5",
            "Add feature",
            GithubPrState::Merged,
            ME,
        );

        let mut task = make_task(1, "Add feature", TaskStatus::Working);
        task.github_pr_contexts = vec![make_github_ctx(
            "https://github.com/owner/repo/pull/5",
            "merged",
        )];

        let actions = reconcile_github(&[pr], &[task], ME);
        assert_eq!(actions, vec![SyncAction::MarkDone { task_id: 1 }]);
    }

    #[test]
    fn authored_pr_merged_with_linear_no_done() {
        let pr = make_pr(
            "https://github.com/owner/repo/pull/6",
            "Add feature",
            GithubPrState::Merged,
            ME,
        );

        let mut task = make_task(1, "Add feature", TaskStatus::Working);
        task.github_pr_contexts = vec![make_github_ctx(
            "https://github.com/owner/repo/pull/6",
            "merged",
        )];
        task.linear_contexts = vec![make_linear_ctx("ENG-100", "started")];

        let actions = reconcile_github(&[pr], &[task], ME);
        assert!(actions.is_empty());
    }

    #[test]
    fn pr_closed_not_merged_no_action() {
        let pr = make_pr(
            "https://github.com/owner/repo/pull/7",
            "Add feature",
            GithubPrState::Closed,
            ME,
        );

        let mut task = make_task(1, "Add feature", TaskStatus::Working);
        task.github_pr_contexts = vec![make_github_ctx(
            "https://github.com/owner/repo/pull/7",
            "closed",
        )];

        let actions = reconcile_github(&[pr], &[task], ME);
        assert!(actions.is_empty());
    }

    #[test]
    fn two_prs_one_merged_one_open_no_done() {
        let pr1 = make_pr(
            "https://github.com/owner/repo/pull/8",
            "Part 1",
            GithubPrState::Merged,
            ME,
        );
        let pr2 = make_pr(
            "https://github.com/owner/repo/pull/9",
            "Part 2",
            GithubPrState::Open,
            ME,
        );

        let mut task = make_task(1, "Big feature", TaskStatus::Working);
        task.github_pr_contexts = vec![
            make_github_ctx("https://github.com/owner/repo/pull/8", "merged"),
            make_github_ctx("https://github.com/owner/repo/pull/9", "open"),
        ];

        let actions = reconcile_github(&[pr1, pr2], &[task], ME);
        assert!(actions.is_empty());
    }

    #[test]
    fn two_prs_one_merged_one_closed_marks_done() {
        let pr1 = make_pr(
            "https://github.com/owner/repo/pull/10",
            "Part 1",
            GithubPrState::Merged,
            ME,
        );
        let pr2 = make_pr(
            "https://github.com/owner/repo/pull/11",
            "Part 2",
            GithubPrState::Closed,
            ME,
        );

        let mut task = make_task(1, "Big feature", TaskStatus::Working);
        task.github_pr_contexts = vec![
            make_github_ctx("https://github.com/owner/repo/pull/10", "merged"),
            make_github_ctx("https://github.com/owner/repo/pull/11", "closed"),
        ];

        let actions = reconcile_github(&[pr1, pr2], &[task], ME);
        assert_eq!(actions, vec![SyncAction::MarkDone { task_id: 1 }]);
    }

    #[test]
    fn merged_pr_no_existing_task_no_retroactive_create() {
        let pr = make_pr(
            "https://github.com/owner/repo/pull/12",
            "Old feature",
            GithubPrState::Merged,
            ME,
        );

        let actions = reconcile_github(&[pr], &[], ME);
        assert!(actions.is_empty());
    }

    // --- Linear tests ---

    #[test]
    fn new_assigned_issue_creates_task() {
        let issue = LinearIssue {
            identifier: "ENG-200".to_string(),
            title: "Build sync".to_string(),
            url: "https://linear.app/team/issue/ENG-200/build-sync".to_string(),
            state_type: "started".to_string(),
            priority: Some(2),
            github_pr_urls: vec![],
        };

        let actions = reconcile_linear(&[issue], &[]);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            SyncAction::CreateTaskWithLinear {
                title,
                status,
                priority,
                ..
            } => {
                assert_eq!(title, "Build sync");
                assert_eq!(*status, TaskStatus::Working);
                assert_eq!(*priority, Some(2));
            }
            other => panic!("expected CreateTaskWithLinear, got {other:?}"),
        }
    }

    #[test]
    fn started_issue_updates_task_status() {
        let issue = LinearIssue {
            identifier: "ENG-201".to_string(),
            title: "Build sync".to_string(),
            url: "https://linear.app/team/issue/ENG-201/build-sync".to_string(),
            state_type: "started".to_string(),
            priority: None,
            github_pr_urls: vec![],
        };

        let mut task = make_task(1, "Build sync", TaskStatus::Ready);
        task.linear_contexts = vec![make_linear_ctx("ENG-201", "unstarted")];

        let actions = reconcile_linear(&[issue], &[task]);
        assert_eq!(
            actions,
            vec![SyncAction::UpdateStatus {
                task_id: 1,
                status: TaskStatus::Working,
            }]
        );
    }

    #[test]
    fn all_linear_completed_marks_done() {
        let issues = vec![
            LinearIssue {
                identifier: "ENG-300".to_string(),
                title: "Part 1".to_string(),
                url: "https://linear.app/team/issue/ENG-300/part-1".to_string(),
                state_type: "completed".to_string(),
                priority: None,
                github_pr_urls: vec![],
            },
            LinearIssue {
                identifier: "ENG-301".to_string(),
                title: "Part 2".to_string(),
                url: "https://linear.app/team/issue/ENG-301/part-2".to_string(),
                state_type: "completed".to_string(),
                priority: None,
                github_pr_urls: vec![],
            },
        ];

        let mut task = make_task(1, "Big project", TaskStatus::Working);
        task.linear_contexts = vec![
            make_linear_ctx("ENG-300", "started"),
            make_linear_ctx("ENG-301", "started"),
        ];

        let actions = reconcile_linear(&issues, &[task]);
        // Should get MarkDone (no CreateTask since both are linked)
        assert_eq!(actions, vec![SyncAction::MarkDone { task_id: 1 }]);
    }

    #[test]
    fn one_completed_one_started_no_done() {
        let issues = vec![
            LinearIssue {
                identifier: "ENG-400".to_string(),
                title: "Part 1".to_string(),
                url: "https://linear.app/team/issue/ENG-400/part-1".to_string(),
                state_type: "completed".to_string(),
                priority: None,
                github_pr_urls: vec![],
            },
            LinearIssue {
                identifier: "ENG-401".to_string(),
                title: "Part 2".to_string(),
                url: "https://linear.app/team/issue/ENG-401/part-2".to_string(),
                state_type: "started".to_string(),
                priority: None,
                github_pr_urls: vec![],
            },
        ];

        let mut task = make_task(1, "Big project", TaskStatus::Working);
        task.linear_contexts = vec![
            make_linear_ctx("ENG-400", "started"),
            make_linear_ctx("ENG-401", "started"),
        ];

        let actions = reconcile_linear(&issues, &[task]);
        // Not all terminal, so no MarkDone. Already InProgress, so no UpdateStatus.
        assert!(actions.is_empty());
    }

    #[test]
    fn one_completed_one_canceled_marks_done() {
        let issues = vec![
            LinearIssue {
                identifier: "ENG-500".to_string(),
                title: "Part 1".to_string(),
                url: "https://linear.app/team/issue/ENG-500/part-1".to_string(),
                state_type: "completed".to_string(),
                priority: None,
                github_pr_urls: vec![],
            },
            LinearIssue {
                identifier: "ENG-501".to_string(),
                title: "Part 2".to_string(),
                url: "https://linear.app/team/issue/ENG-501/part-2".to_string(),
                state_type: "canceled".to_string(),
                priority: None,
                github_pr_urls: vec![],
            },
        ];

        let mut task = make_task(1, "Big project", TaskStatus::Working);
        task.linear_contexts = vec![
            make_linear_ctx("ENG-500", "started"),
            make_linear_ctx("ENG-501", "started"),
        ];

        let actions = reconcile_linear(&issues, &[task]);
        assert_eq!(actions, vec![SyncAction::MarkDone { task_id: 1 }]);
    }

    #[test]
    fn all_canceled_marks_canceled() {
        let issues = vec![
            LinearIssue {
                identifier: "ENG-600".to_string(),
                title: "Part 1".to_string(),
                url: "https://linear.app/team/issue/ENG-600/part-1".to_string(),
                state_type: "canceled".to_string(),
                priority: None,
                github_pr_urls: vec![],
            },
            LinearIssue {
                identifier: "ENG-601".to_string(),
                title: "Part 2".to_string(),
                url: "https://linear.app/team/issue/ENG-601/part-2".to_string(),
                state_type: "canceled".to_string(),
                priority: None,
                github_pr_urls: vec![],
            },
        ];

        let mut task = make_task(1, "Doomed project", TaskStatus::Working);
        task.linear_contexts = vec![
            make_linear_ctx("ENG-600", "started"),
            make_linear_ctx("ENG-601", "started"),
        ];

        let actions = reconcile_linear(&issues, &[task]);
        assert_eq!(actions, vec![SyncAction::MarkCanceled { task_id: 1 }]);
    }

    #[test]
    fn unassigned_issue_cancels_task() {
        // Issue is gone from the results (unassigned from user)
        let issues: Vec<LinearIssue> = vec![];

        let mut task = make_task(1, "Was assigned", TaskStatus::Working);
        task.linear_contexts = vec![make_linear_ctx("ENG-700", "started")];

        let actions = reconcile_linear(&issues, &[task]);
        assert_eq!(actions, vec![SyncAction::MarkCanceled { task_id: 1 }]);
    }

    #[test]
    fn already_done_task_not_resurrected() {
        let issue = LinearIssue {
            identifier: "ENG-800".to_string(),
            title: "Done thing".to_string(),
            url: "https://linear.app/team/issue/ENG-800/done-thing".to_string(),
            state_type: "started".to_string(),
            priority: None,
            github_pr_urls: vec![],
        };

        let mut task = make_task(1, "Done thing", TaskStatus::Done);
        task.linear_contexts = vec![make_linear_ctx("ENG-800", "completed")];

        let actions = reconcile_linear(&[issue], &[task]);
        // Task is already done, should not produce any actions
        assert!(actions.is_empty());
    }

    // --- Cross-source dedup tests ---

    #[test]
    fn linear_issue_with_pr_attachment_deduplicates_existing_github_task() {
        let pr_url = "https://github.com/owner/repo/pull/42";
        let issue = LinearIssue {
            identifier: "ENG-900".to_string(),
            title: "Feature X".to_string(),
            url: "https://linear.app/team/issue/ENG-900/feature-x".to_string(),
            state_type: "started".to_string(),
            priority: Some(1),
            github_pr_urls: vec![pr_url.to_string()],
        };

        // Existing task linked to that PR but not to Linear
        let mut task = make_task(1, "Feature X", TaskStatus::Working);
        task.github_pr_contexts = vec![make_github_ctx(pr_url, "open")];

        let actions = reconcile_linear(&[issue], &[task]);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            SyncAction::LinkLinearToExistingTask { task_id, linear } => {
                assert_eq!(*task_id, 1);
                assert_eq!(linear.identifier, "ENG-900");
            }
            other => panic!("expected LinkLinearToExistingTask, got {other:?}"),
        }
    }

    #[test]
    fn linear_issue_with_pr_attachment_no_existing_task_creates_normally() {
        let issue = LinearIssue {
            identifier: "ENG-901".to_string(),
            title: "Feature Y".to_string(),
            url: "https://linear.app/team/issue/ENG-901/feature-y".to_string(),
            state_type: "unstarted".to_string(),
            priority: None,
            github_pr_urls: vec!["https://github.com/owner/repo/pull/99".to_string()],
        };

        // No existing tasks at all
        let actions = reconcile_linear(&[issue], &[]);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            SyncAction::CreateTaskWithLinear { title, status, .. } => {
                assert_eq!(title, "Feature Y");
                assert_eq!(*status, TaskStatus::Ready);
            }
            other => panic!("expected CreateTaskWithLinear, got {other:?}"),
        }
    }

    #[test]
    fn existing_linear_task_gets_new_pr_attachment_linked() {
        let pr_url = "https://github.com/owner/repo/pull/50";
        let issue = LinearIssue {
            identifier: "ENG-902".to_string(),
            title: "Feature Z".to_string(),
            url: "https://linear.app/team/issue/ENG-902/feature-z".to_string(),
            state_type: "started".to_string(),
            priority: None,
            github_pr_urls: vec![pr_url.to_string()],
        };

        // Task already linked to Linear, but no GitHub PR context
        let mut task = make_task(1, "Feature Z", TaskStatus::Working);
        task.linear_contexts = vec![make_linear_ctx("ENG-902", "started")];

        let actions = reconcile_linear(&[issue], &[task]);
        assert_eq!(
            actions,
            vec![SyncAction::LinkGithubPrToExistingTask {
                task_id: 1,
                pr_url: pr_url.to_string(),
            }]
        );
    }

    #[test]
    fn existing_linear_task_pr_already_linked_no_action() {
        let pr_url = "https://github.com/owner/repo/pull/55";
        let issue = LinearIssue {
            identifier: "ENG-903".to_string(),
            title: "Feature W".to_string(),
            url: "https://linear.app/team/issue/ENG-903/feature-w".to_string(),
            state_type: "started".to_string(),
            priority: None,
            github_pr_urls: vec![pr_url.to_string()],
        };

        // Task has both Linear and the same GitHub PR linked
        let mut task = make_task(1, "Feature W", TaskStatus::Working);
        task.linear_contexts = vec![make_linear_ctx("ENG-903", "started")];
        task.github_pr_contexts = vec![make_github_ctx(pr_url, "open")];

        let actions = reconcile_linear(&[issue], &[task]);
        // No new links needed, no status changes
        assert!(actions.is_empty());
    }

    #[test]
    fn linear_issue_with_pr_attachment_existing_linear_task_also_exists() {
        let pr_url = "https://github.com/owner/repo/pull/60";
        let issue = LinearIssue {
            identifier: "ENG-904".to_string(),
            title: "Feature V".to_string(),
            url: "https://linear.app/team/issue/ENG-904/feature-v".to_string(),
            state_type: "started".to_string(),
            priority: None,
            github_pr_urls: vec![pr_url.to_string()],
        };

        // A GitHub-only task exists for the PR
        let mut github_task = make_task(1, "Feature V", TaskStatus::Working);
        github_task.github_pr_contexts = vec![make_github_ctx(pr_url, "open")];

        // A Linear-linked task also exists for this issue
        let mut linear_task = make_task(2, "Feature V", TaskStatus::Working);
        linear_task.linear_contexts = vec![make_linear_ctx("ENG-904", "started")];

        let actions = reconcile_linear(&[issue], &[github_task, linear_task]);
        // Issue is already linked to task 2, so no CreateTaskWithLinear.
        // Task 2 doesn't have the PR linked yet, so we get LinkGithubPrToExistingTask.
        assert_eq!(
            actions,
            vec![SyncAction::LinkGithubPrToExistingTask {
                task_id: 2,
                pr_url: pr_url.to_string(),
            }]
        );
    }
}
