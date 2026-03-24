use anyhow::Result;
use chrono::Utc;
use rust_query::{Select, aggregate, optional, Update};

use crate::schema;
use crate::types::{AddTask, EditTask, Task, TaskId, TaskStatus};
use crate::Db;

/// Internal struct for selecting all task columns from the database.
#[derive(Select)]
struct TaskSelect {
    external_id: i64,
    title: String,
    description: String,
    status: String,
    priority: Option<i64>,
    workspace: String,
    deadline: Option<String>,
    snooze_until: Option<String>,
    planned_date: Option<String>,
    deleted_at: Option<String>,
    created_at: String,
    updated_at: String,
}

impl TaskSelect {
    fn into_task(self) -> Result<Task> {
        let status: TaskStatus = self.status.parse()?;
        Ok(Task {
            id: self.external_id,
            title: self.title,
            description: self.description,
            status,
            priority: self.priority,
            workspace: self.workspace,
            deadline: self.deadline,
            snooze_until: self.snooze_until,
            planned_date: self.planned_date,
            deleted_at: self.deleted_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
            github_pr_context: None,
            linear_context: None,
        })
    }
}

impl Db {
    pub fn add_task(&mut self, params: AddTask) -> Result<TaskId> {
        let now = Utc::now().to_rfc3339();
        let status = params.status.unwrap_or(TaskStatus::Idea).to_string();

        let mut txn = self.client.transaction_mut(&self.database);

        // Generate next external_id: max(external_id) + 1, or 1 if empty
        let next_id: i64 = txn.query_one(aggregate(|rows| {
            let task = rows.join(schema::Task);
            rows.max(task.external_id())
        })).map_or(1, |max| max + 1);

        txn.insert(schema::Task {
            external_id: next_id,
            title: &*params.title,
            description: &*params.description.unwrap_or_default(),
            status: &*status,
            priority: params.priority,
            workspace: &*params.workspace,
            deadline: params.deadline.as_deref(),
            snooze_until: None::<&str>,
            planned_date: params.planned_date.as_deref(),
            deleted_at: None::<&str>,
            created_at: &*now,
            updated_at: &*now,
        }).map_err(|_| anyhow::anyhow!("task with this external_id already exists"))?;

        txn.commit();
        Ok(next_id)
    }

    pub fn edit_task(&mut self, id: TaskId, params: EditTask) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        let mut txn = self.client.transaction_mut(&self.database);

        let task_row = txn
            .query_one(optional(|row| {
                let task = row.and(schema::Task::unique(id));
                row.then(task)
            }))
            .ok_or_else(|| anyhow::anyhow!("task not found: {id}"))?;

        // Build update with only the fields that changed
        let title_update = match &params.title {
            Some(t) => Update::set(&**t),
            None => Update::default(),
        };
        let description_update = match &params.description {
            Some(d) => Update::set(&**d),
            None => Update::default(),
        };
        let status_str;
        let status_update = match &params.status {
            Some(s) => {
                status_str = s.to_string();
                Update::set(&*status_str)
            }
            None => Update::default(),
        };
        let priority_update = match params.priority {
            Some(p) => Update::set(p),
            None => Update::default(),
        };
        let workspace_update = match &params.workspace {
            Some(w) => Update::set(&**w),
            None => Update::default(),
        };
        let deadline_update = match &params.deadline {
            Some(d) => Update::set(d.as_deref()),
            None => Update::default(),
        };
        let snooze_update = match &params.snooze_until {
            Some(s) => Update::set(s.as_deref()),
            None => Update::default(),
        };
        let planned_date_update = match &params.planned_date {
            Some(p) => Update::set(p.as_deref()),
            None => Update::default(),
        };

        txn.update(
            task_row,
            schema::Task {
                external_id: Update::default(),
                title: title_update,
                description: description_update,
                status: status_update,
                priority: priority_update,
                workspace: workspace_update,
                deadline: deadline_update,
                snooze_until: snooze_update,
                planned_date: planned_date_update,
                deleted_at: Update::default(),
                created_at: Update::default(),
                updated_at: Update::set(&*now),
            },
        )
        .map_err(|_| anyhow::anyhow!("update conflict on task {id}"))?;

        txn.commit();
        Ok(())
    }

    pub fn delete_task(&mut self, id: TaskId) -> Result<()> {
        let now = Utc::now().to_rfc3339();

        let mut txn = self.client.transaction_mut(&self.database);

        let task_row = txn
            .query_one(optional(|row| {
                let task = row.and(schema::Task::unique(id));
                row.then(task)
            }))
            .ok_or_else(|| anyhow::anyhow!("task not found: {id}"))?;

        txn.update(
            task_row,
            schema::Task {
                external_id: Update::default(),
                title: Update::default(),
                description: Update::default(),
                status: Update::default(),
                priority: Update::default(),
                workspace: Update::default(),
                deadline: Update::default(),
                snooze_until: Update::default(),
                planned_date: Update::default(),
                deleted_at: Update::set(Some(&*now)),
                created_at: Update::default(),
                updated_at: Update::set(&*now),
            },
        )
        .map_err(|_| anyhow::anyhow!("update conflict on task {id}"))?;

        txn.commit();
        Ok(())
    }

    pub fn snooze(&mut self, id: TaskId, until: chrono::NaiveDate) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let until_str = until.to_string();

        let mut txn = self.client.transaction_mut(&self.database);

        let task_row = txn
            .query_one(optional(|row| {
                let task = row.and(schema::Task::unique(id));
                row.then(task)
            }))
            .ok_or_else(|| anyhow::anyhow!("task not found: {id}"))?;

        txn.update(
            task_row,
            schema::Task {
                external_id: Update::default(),
                title: Update::default(),
                description: Update::default(),
                status: Update::default(),
                priority: Update::default(),
                workspace: Update::default(),
                deadline: Update::default(),
                snooze_until: Update::set(Some(&*until_str)),
                planned_date: Update::default(),
                deleted_at: Update::default(),
                created_at: Update::default(),
                updated_at: Update::set(&*now),
            },
        )
        .map_err(|_| anyhow::anyhow!("update conflict on task {id}"))?;

        txn.commit();
        Ok(())
    }

    pub fn get_task(&mut self, id: TaskId) -> Result<Option<Task>> {
        let txn = self.client.transaction(&self.database);

        let result: Option<TaskSelect> = txn.query_one(optional(|row| {
            let t = row.and(schema::Task::unique(id));
            row.then(TaskSelectSelect {
                external_id: t.external_id(),
                title: t.title(),
                description: t.description(),
                status: t.status(),
                priority: t.priority(),
                workspace: t.workspace(),
                deadline: t.deadline(),
                snooze_until: t.snooze_until(),
                planned_date: t.planned_date(),
                deleted_at: t.deleted_at(),
                created_at: t.created_at(),
                updated_at: t.updated_at(),
            })
        }));

        match result {
            None => Ok(None),
            Some(ts) => Ok(Some(ts.into_task()?)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AddTask;

    fn make_add_task(title: &str, workspace: &str) -> AddTask {
        AddTask {
            title: title.into(),
            description: None,
            status: None,
            priority: None,
            workspace: workspace.into(),
            deadline: None,
            planned_date: None,
        }
    }

    /// rust-query 0.4 only allows one Config::open_in_memory() per process,
    /// so all task CRUD tests share a single Db instance.
    #[test]
    fn test_task_crud() {
        let mut db = Db::open_in_memory().unwrap();

        // --- test_add_task ---
        let id = db.add_task(make_add_task("Buy groceries", "personal")).unwrap();
        assert_eq!(id, 1);

        let task = db.get_task(id).unwrap().expect("task should exist");
        assert_eq!(task.title, "Buy groceries");
        assert_eq!(task.status, TaskStatus::Idea);
        assert_eq!(task.workspace, "personal");
        assert_eq!(task.description, "");
        assert!(task.priority.is_none());
        assert!(task.deadline.is_none());
        assert!(task.snooze_until.is_none());
        assert!(task.planned_date.is_none());
        assert!(task.deleted_at.is_none());

        // --- test_add_multiple_tasks ---
        let id2 = db.add_task(make_add_task("Task 2", "work")).unwrap();
        let id3 = db.add_task(make_add_task("Task 3", "personal")).unwrap();
        assert_eq!(id2, 2);
        assert_eq!(id3, 3);

        // --- test_edit_task ---
        let edit_id = db.add_task(make_add_task("Edit me", "personal")).unwrap();

        db.edit_task(
            edit_id,
            EditTask {
                title: Some("Edited title".into()),
                status: Some(TaskStatus::Ready),
                priority: Some(Some(1)),
                ..Default::default()
            },
        )
        .unwrap();

        let task = db.get_task(edit_id).unwrap().expect("task should exist");
        assert_eq!(task.title, "Edited title");
        assert_eq!(task.status, TaskStatus::Ready);
        assert_eq!(task.priority, Some(1));
        assert_eq!(task.workspace, "personal"); // unchanged
        assert_eq!(task.description, ""); // unchanged

        // --- test_delete_task ---
        let del_id = db.add_task(make_add_task("Delete me", "personal")).unwrap();

        db.delete_task(del_id).unwrap();

        let task = db.get_task(del_id).unwrap().expect("task should still exist in db");
        assert!(task.deleted_at.is_some());

        // --- test_snooze_task ---
        let snooze_id = db.add_task(make_add_task("Snoozeable", "personal")).unwrap();

        let until = chrono::NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();
        db.snooze(snooze_id, until).unwrap();

        let task = db.get_task(snooze_id).unwrap().expect("task should exist");
        assert_eq!(task.snooze_until, Some("2026-04-01".to_string()));

        // =====================================================================
        // Query tests (list_tasks + search)
        // =====================================================================
        // Current DB state (non-deleted):
        //   1: "Buy groceries",  personal, Idea,  pri=None, deadline=None, snooze=None, planned=None
        //   2: "Task 2",         work,     Idea,  pri=None, deadline=None, snooze=None, planned=None
        //   3: "Task 3",         personal, Idea,  pri=None, deadline=None, snooze=None, planned=None
        //   4: "Edited title",   personal, Ready, pri=1,    deadline=None, snooze=None, planned=None
        //   5: DELETED
        //   6: "Snoozeable",     personal, Idea,  pri=None, deadline=None, snooze=2026-04-01, planned=None
        //
        // Add more tasks to exercise sorting and filtering:

        use crate::types::TaskFilter;

        // Task with priority 0 (critical) and early deadline
        let id7 = db.add_task(AddTask {
            title: "Critical bug".into(),
            description: Some("Fix the critical production bug".into()),
            status: Some(TaskStatus::InProgress),
            priority: Some(0),
            workspace: "work".into(),
            deadline: Some("2026-03-25".into()),
            planned_date: Some("2026-03-24".into()),
        }).unwrap();
        assert_eq!(id7, 7);

        // Task with priority 2 and later deadline
        let id8 = db.add_task(AddTask {
            title: "Write tests".into(),
            description: Some("Add unit tests for query module".into()),
            status: Some(TaskStatus::Ready),
            priority: Some(2),
            workspace: "work".into(),
            deadline: Some("2026-04-01".into()),
            planned_date: None,
        }).unwrap();
        assert_eq!(id8, 8);

        // Task with priority 1 and no deadline (should sort after pri=1 with deadline)
        let id9 = db.add_task(AddTask {
            title: "Refactor code".into(),
            description: None,
            status: Some(TaskStatus::Ready),
            priority: Some(1),
            workspace: "personal".into(),
            deadline: None,
            planned_date: Some("2026-03-30".into()),
        }).unwrap();
        assert_eq!(id9, 9);

        // Done task
        let id10 = db.add_task(AddTask {
            title: "Old done task".into(),
            description: None,
            status: Some(TaskStatus::Done),
            priority: None,
            workspace: "work".into(),
            deadline: None,
            planned_date: None,
        }).unwrap();
        assert_eq!(id10, 10);

        // --- test: list_tasks excludes deleted tasks ---
        {
            let all = db.list_tasks(TaskFilter::default()).unwrap();
            assert!(all.iter().all(|t| t.deleted_at.is_none()), "deleted tasks should be excluded");
            assert!(!all.iter().any(|t| t.id == 5), "task 5 was deleted");
        }

        // --- test: list_tasks filters by status ---
        {
            let ready_tasks = db.list_tasks(TaskFilter {
                status: Some(TaskStatus::Ready),
                ..Default::default()
            }).unwrap();
            assert!(ready_tasks.iter().all(|t| t.status == TaskStatus::Ready));
            // IDs 4, 8, 9 are Ready
            let ids: Vec<_> = ready_tasks.iter().map(|t| t.id).collect();
            assert!(ids.contains(&4));
            assert!(ids.contains(&8));
            assert!(ids.contains(&9));
        }

        // --- test: list_tasks filters by workspace ---
        {
            let work_tasks = db.list_tasks(TaskFilter {
                workspace: Some("work".into()),
                ..Default::default()
            }).unwrap();
            assert!(work_tasks.iter().all(|t| t.workspace == "work"));
            let ids: Vec<_> = work_tasks.iter().map(|t| t.id).collect();
            assert!(ids.contains(&2));
            assert!(ids.contains(&7));
            assert!(ids.contains(&8));
            assert!(ids.contains(&10));
        }

        // --- test: list_tasks sorts by priority then deadline ---
        {
            // Get all non-snoozed tasks (include_snoozed=false is default)
            // Snoozed task 6 is excluded by default since snooze_until > today
            let tasks = db.list_tasks(TaskFilter {
                include_snoozed: true,
                ..Default::default()
            }).unwrap();

            // Expected priority ordering: 0 first, then 1s, then 2s, then None
            // Within same priority, earlier deadlines first, None-deadline last
            // Verify ordering: all Some(p) before None, and within Some ascending
            let mut prev_pri: Option<i64> = Some(i64::MIN);
            let mut seen_none = false;
            for t in &tasks {
                match t.priority {
                    Some(p) => {
                        assert!(!seen_none, "task with priority appeared after None-priority task");
                        assert!(p >= prev_pri.unwrap_or(i64::MIN), "priorities should be ascending");
                        prev_pri = Some(p);
                    }
                    None => {
                        seen_none = true;
                    }
                }
            }

            // Check the first task is priority 0
            assert_eq!(tasks[0].priority, Some(0));
            assert_eq!(tasks[0].title, "Critical bug");
        }

        // --- test: list_tasks filters snoozed tasks ---
        {
            // Default: snoozed tasks excluded
            let without_snoozed = db.list_tasks(TaskFilter::default()).unwrap();
            assert!(!without_snoozed.iter().any(|t| t.id == 6), "snoozed task should be excluded by default");

            // With include_snoozed: snoozed tasks included
            let with_snoozed = db.list_tasks(TaskFilter {
                include_snoozed: true,
                ..Default::default()
            }).unwrap();
            assert!(with_snoozed.iter().any(|t| t.id == 6), "snoozed task should be included when flag set");
        }

        // --- test: list_tasks filters by has_planned_date ---
        {
            let with_planned = db.list_tasks(TaskFilter {
                has_planned_date: Some(true),
                ..Default::default()
            }).unwrap();
            assert!(with_planned.iter().all(|t| t.planned_date.is_some()));
            let ids: Vec<_> = with_planned.iter().map(|t| t.id).collect();
            assert!(ids.contains(&7)); // "Critical bug" has planned_date
            assert!(ids.contains(&9)); // "Refactor code" has planned_date

            let without_planned = db.list_tasks(TaskFilter {
                has_planned_date: Some(false),
                ..Default::default()
            }).unwrap();
            assert!(without_planned.iter().all(|t| t.planned_date.is_none()));
            assert!(!without_planned.iter().any(|t| t.id == 7));
        }

        // --- test: search matches title substring ---
        {
            let results = db.search("critical", TaskFilter::default()).unwrap();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].title, "Critical bug");
        }

        // --- test: search matches description substring ---
        {
            let results = db.search("unit tests", TaskFilter::default()).unwrap();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].title, "Write tests");
        }

        // --- test: search respects workspace filter ---
        {
            // "task" appears in titles across workspaces
            let results = db.search("task", TaskFilter {
                workspace: Some("work".into()),
                ..Default::default()
            }).unwrap();
            assert!(results.iter().all(|t| t.workspace == "work"));
            // "Task 2" is in work, "Task 3" is in personal
            let ids: Vec<_> = results.iter().map(|t| t.id).collect();
            assert!(ids.contains(&2));
            assert!(!ids.contains(&3));
        }

        // --- test: add_attachment ---
        {
            use crate::types::NewAttachment;

            let att_id = db.add_attachment(
                id, // task 1: "Buy groceries"
                NewAttachment {
                    file_name: "receipt.png".into(),
                    content_type: "image/png".into(),
                    data: vec![0x89, 0x50, 0x4e, 0x47],
                    caption: Some("Store receipt".into()),
                },
            ).unwrap();
            assert_eq!(att_id, 1);

            // Second attachment gets id 2
            let att_id2 = db.add_attachment(
                id,
                NewAttachment {
                    file_name: "note.txt".into(),
                    content_type: "text/plain".into(),
                    data: b"hello".to_vec(),
                    caption: None,
                },
            ).unwrap();
            assert_eq!(att_id2, 2);

            // Adding attachment to a non-existent task should error
            let err = db.add_attachment(
                9999,
                NewAttachment {
                    file_name: "x.txt".into(),
                    content_type: "text/plain".into(),
                    data: vec![],
                    caption: None,
                },
            );
            assert!(err.is_err());
        }
    }
}
