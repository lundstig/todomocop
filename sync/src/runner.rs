use anyhow::Result;
use todomocop_core::Db;
use todomocop_core::types::{AddTask, EditTask, TaskStatus};
use crate::types::SyncAction;

#[derive(Debug, Default)]
pub struct SyncSummary {
    pub created: u32,
    pub completed: u32,
    pub canceled: u32,
    pub updated: u32,
    pub errors: Vec<String>,
}

impl std::fmt::Display for SyncSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Created {}, completed {}, canceled {}, updated {}",
            self.created, self.completed, self.canceled, self.updated)?;
        if !self.errors.is_empty() {
            write!(f, " ({} errors)", self.errors.len())?;
        }
        Ok(())
    }
}

impl SyncSummary {
    pub fn merge(&mut self, other: &SyncSummary) {
        self.created += other.created;
        self.completed += other.completed;
        self.canceled += other.canceled;
        self.updated += other.updated;
        self.errors.extend(other.errors.iter().cloned());
    }
}

pub fn apply_actions(db: &Db, actions: &[SyncAction]) -> SyncSummary {
    let mut summary = SyncSummary::default();
    for action in actions {
        match apply_one(db, action) {
            Ok(kind) => match kind {
                ActionKind::Created => summary.created += 1,
                ActionKind::Completed => summary.completed += 1,
                ActionKind::Canceled => summary.canceled += 1,
                ActionKind::Updated => summary.updated += 1,
            },
            Err(e) => summary.errors.push(format!("{e:#}")),
        }
    }
    summary
}

enum ActionKind { Created, Completed, Canceled, Updated }

fn apply_one(db: &Db, action: &SyncAction) -> Result<ActionKind> {
    match action {
        SyncAction::CreateTaskWithGithub { title, pr, status } => {
            let task_id = db.add_task(AddTask {
                title: title.clone(),
                description: None,
                status: Some(*status),
                priority: None,
                workspace: "work".into(),
                deadline: None,
                planned_date: None,
            })?;
            db.link_github_pr(task_id, &pr.url)?;
            Ok(ActionKind::Created)
        }
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
                db.link_github_pr(task_id, pr_url)?;
            }
            Ok(ActionKind::Created)
        }
        SyncAction::MarkDone { task_id } => {
            db.edit_task(*task_id, EditTask {
                status: Some(TaskStatus::Done),
                ..Default::default()
            })?;
            Ok(ActionKind::Completed)
        }
        SyncAction::MarkCanceled { task_id } => {
            db.edit_task(*task_id, EditTask {
                status: Some(TaskStatus::Canceled),
                ..Default::default()
            })?;
            Ok(ActionKind::Canceled)
        }
        SyncAction::UpdateStatus { task_id, status } => {
            db.edit_task(*task_id, EditTask {
                status: Some(*status),
                ..Default::default()
            })?;
            Ok(ActionKind::Updated)
        }
        SyncAction::LinkLinearToExistingTask { task_id, linear } => {
            db.link_linear(*task_id, &linear.url)?;
            Ok(ActionKind::Updated)
        }
        SyncAction::LinkGithubPrToExistingTask { task_id, pr_url } => {
            db.link_github_pr(*task_id, pr_url)?;
            Ok(ActionKind::Updated)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use todomocop_core::Db;
    use todomocop_core::types::{TaskFilter, TaskStatus};
    use crate::types::*;

    #[test]
    fn integration_create_and_complete_github() {
        let db = Db::open_in_memory().unwrap();

        // 1. Create a task from a GitHub PR
        let actions = vec![SyncAction::CreateTaskWithGithub {
            title: "Fix bug".into(),
            pr: CreateGithubLink {
                url: "https://github.com/o/r/pull/1".into(),
                repo: "o/r".into(),
                number: 1,
                title: "Fix bug".into(),
                author: "me".into(),
                reviewers: Vec::new(),
                review_state: HashMap::new(),
            },
            status: TaskStatus::Ready,
        }];

        let summary = apply_actions(&db, &actions);
        assert_eq!(summary.created, 1);
        assert!(summary.errors.is_empty());

        // Verify task exists
        let tasks = db.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Fix bug");
        assert_eq!(tasks[0].status, TaskStatus::Ready);
        assert_eq!(tasks[0].workspace, "work");

        // 2. Mark it done
        let done_actions = vec![SyncAction::MarkDone { task_id: tasks[0].id }];
        let summary = apply_actions(&db, &done_actions);
        assert_eq!(summary.completed, 1);

        let task = db.get_task(tasks[0].id).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Done);
    }

    #[test]
    fn integration_create_and_cancel_linear() {
        let db = Db::open_in_memory().unwrap();

        // Create a task from a Linear issue
        let actions = vec![SyncAction::CreateTaskWithLinear {
            title: "Build feature".into(),
            linear: CreateLinearLink {
                identifier: "ENG-42".into(),
                url: "https://linear.app/team/issue/ENG-42/build-feature".into(),
                state_type: "started".into(),
            },
            status: TaskStatus::InProgress,
            priority: Some(2),
            github_pr_urls: Vec::new(),
        }];

        let summary = apply_actions(&db, &actions);
        assert_eq!(summary.created, 1);
        assert!(summary.errors.is_empty());

        let tasks = db.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].status, TaskStatus::InProgress);
        assert_eq!(tasks[0].priority, Some(2));

        // Cancel it (issue unassigned)
        let cancel_actions = vec![SyncAction::MarkCanceled { task_id: tasks[0].id }];
        let summary = apply_actions(&db, &cancel_actions);
        assert_eq!(summary.canceled, 1);

        let task = db.get_task(tasks[0].id).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Canceled);
    }

    #[test]
    fn integration_update_status() {
        let db = Db::open_in_memory().unwrap();

        // Create a task
        let actions = vec![SyncAction::CreateTaskWithLinear {
            title: "Feature".into(),
            linear: CreateLinearLink {
                identifier: "ENG-1".into(),
                url: "https://linear.app/t/issue/ENG-1/f".into(),
                state_type: "unstarted".into(),
            },
            status: TaskStatus::Ready,
            priority: None,
            github_pr_urls: Vec::new(),
        }];
        apply_actions(&db, &actions);

        let tasks = db.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks[0].status, TaskStatus::Ready);

        // Update status to in_progress
        let update_actions = vec![SyncAction::UpdateStatus {
            task_id: tasks[0].id,
            status: TaskStatus::InProgress,
        }];
        let summary = apply_actions(&db, &update_actions);
        assert_eq!(summary.updated, 1);

        let task = db.get_task(tasks[0].id).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::InProgress);
    }

    #[test]
    fn integration_errors_dont_stop_processing() {
        let db = Db::open_in_memory().unwrap();

        // Mix of valid and invalid actions
        let actions = vec![
            SyncAction::MarkDone { task_id: 999 }, // nonexistent task
            SyncAction::CreateTaskWithGithub {
                title: "Valid task".into(),
                pr: CreateGithubLink {
                    url: "https://github.com/o/r/pull/2".into(),
                    repo: "o/r".into(),
                    number: 2,
                    title: "Valid task".into(),
                    author: "me".into(),
                    reviewers: Vec::new(),
                    review_state: HashMap::new(),
                },
                status: TaskStatus::Ready,
            },
        ];

        let summary = apply_actions(&db, &actions);
        assert_eq!(summary.errors.len(), 1); // first action failed
        assert_eq!(summary.created, 1); // second action succeeded
    }

    #[test]
    fn integration_link_linear_to_existing_github_task() {
        let db = Db::open_in_memory().unwrap();

        // Create a task via GitHub sync
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

        // Link Linear to the same task
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

        // Create a task via Linear sync
        let create = vec![SyncAction::CreateTaskWithLinear {
            title: "Build feature".into(),
            linear: CreateLinearLink {
                identifier: "ENG-60".into(),
                url: "https://linear.app/t/issue/ENG-60/build-feature".into(),
                state_type: "started".into(),
            },
            status: TaskStatus::InProgress,
            priority: None,
            github_pr_urls: Vec::new(),
        }];
        apply_actions(&db, &create);

        let tasks = db.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0].github_pr_contexts.is_empty());

        // Link a GitHub PR to the same task
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
        apply_actions(&db, &gh_actions);

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
        assert!(!linear_actions.iter().any(|a| matches!(a, SyncAction::CreateTaskWithLinear { .. })));
        assert!(linear_actions.iter().any(|a| matches!(a, SyncAction::LinkLinearToExistingTask { .. })));

        apply_actions(&db, &linear_actions);

        // Verify: still one task, now with both links
        let tasks = db.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].github_pr_contexts.len(), 1);
        assert_eq!(tasks[0].linear_contexts.len(), 1);
        assert_eq!(tasks[0].linear_contexts[0].identifier, "ENG-100");
    }

    #[test]
    fn integration_linear_first_then_github_deduplicates() {
        let db = Db::open_in_memory().unwrap();

        // Step 1: Linear sync creates a task — issue has a PR attachment
        let issue = crate::types::LinearIssue {
            identifier: "ENG-200".into(),
            title: "New feature".into(),
            url: "https://linear.app/t/issue/ENG-200/new-feature".into(),
            state_type: "started".into(),
            priority: None,
            github_pr_urls: vec!["https://github.com/o/r/pull/200".into()],
        };

        let linear_actions = crate::reconcile::reconcile_linear(&[issue], &[]);
        apply_actions(&db, &linear_actions);

        let tasks = db.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
        // The task should have BOTH links because CreateTaskWithLinear carries github_pr_urls
        assert_eq!(tasks[0].linear_contexts.len(), 1);
        assert_eq!(tasks[0].github_pr_contexts.len(), 1, "PR should be linked during creation");

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

        let existing = db.list_tasks(TaskFilter::default()).unwrap();
        let gh_actions = crate::reconcile::reconcile_github(&[pr], &existing, "me");

        // GitHub sync should find the PR already linked — no new task
        assert!(!gh_actions.iter().any(|a| matches!(a, SyncAction::CreateTaskWithGithub { .. })),
            "GitHub sync should NOT create a duplicate — PR is already linked to the task");

        // Still one task
        let tasks = db.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(tasks.len(), 1);
    }
}
