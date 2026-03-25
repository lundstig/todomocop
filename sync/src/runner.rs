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
        SyncAction::CreateTaskWithLinear { title, linear, status, priority } => {
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
    }
}
