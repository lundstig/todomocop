use anyhow::Result;
use chrono::Utc;
use rust_query::Select;

use crate::schema;
use crate::types::{Task, TaskFilter, TaskStatus};
use crate::Db;

#[derive(Select)]
struct QTask {
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

impl QTask {
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
    pub fn list_tasks(&mut self, filter: TaskFilter) -> Result<Vec<Task>> {
        let txn = self.client.transaction(&self.database);

        let rows: Vec<QTask> = txn.query(|q| {
            let t = q.join(schema::Task);
            q.into_vec(QTaskSelect {
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
        });

        let today = Utc::now().date_naive();

        let mut tasks: Vec<Task> = rows
            .into_iter()
            .map(|ts| ts.into_task())
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|t| t.deleted_at.is_none())
            .filter(|t| match &filter.status {
                Some(s) => t.status == *s,
                None => true,
            })
            .filter(|t| match &filter.workspace {
                Some(w) => t.workspace == *w,
                None => true,
            })
            .filter(|t| match filter.has_planned_date {
                Some(true) => t.planned_date.is_some(),
                Some(false) => t.planned_date.is_none(),
                None => true,
            })
            .filter(|t| match filter.priority_max {
                Some(max) => t.priority.map_or(false, |p| p <= max),
                None => true,
            })
            .filter(|t| {
                if filter.include_snoozed {
                    return true;
                }
                match &t.snooze_until {
                    Some(s) => match s.parse::<chrono::NaiveDate>() {
                        Ok(d) => d <= today,
                        Err(_) => true,
                    },
                    None => true,
                }
            })
            .collect();

        // Sort: priority ascending (0=critical first), None last, then deadline ascending, None last
        tasks.sort_by(|a, b| {
            let pri_cmp = match (a.priority, b.priority) {
                (Some(ap), Some(bp)) => ap.cmp(&bp),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            };
            if pri_cmp != std::cmp::Ordering::Equal {
                return pri_cmp;
            }
            match (&a.deadline, &b.deadline) {
                (Some(ad), Some(bd)) => ad.cmp(bd),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });

        Ok(tasks)
    }

    pub fn search(&mut self, query: &str, filter: TaskFilter) -> Result<Vec<Task>> {
        let query_lower = query.to_lowercase();
        let mut tasks = self.list_tasks(filter)?;
        tasks.retain(|t| {
            t.title.to_lowercase().contains(&query_lower)
                || t.description.to_lowercase().contains(&query_lower)
        });
        Ok(tasks)
    }
}
