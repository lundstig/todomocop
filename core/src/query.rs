use anyhow::Result;
use chrono::Utc;

use crate::task::TaskSelect;
use crate::types::{Task, TaskFilter};
use crate::Db;

impl Db {
    pub fn list_tasks(&self, filter: TaskFilter) -> Result<Vec<Task>> {
        let rows = TaskSelect::query_all(self);
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
                Some(max) => t.priority.is_some_and(|p| p <= max),
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

        // Populate external contexts
        for task in &mut tasks {
            task.github_pr_contexts = self.load_github_contexts(task.id)?;
            task.linear_contexts = self.load_linear_contexts(task.id)?;
            task.tags = self.load_tags_for_task(task.id)?;
        }

        // Apply tag filter (after tags are populated)
        if let Some(ref tag_name) = filter.tag {
            tasks.retain(|t| t.tags.iter().any(|td| td.name == *tag_name));
        }

        Ok(tasks)
    }

    pub fn search(&self, query: &str, filter: TaskFilter) -> Result<Vec<Task>> {
        let query_lower = query.to_lowercase();
        let mut tasks = self.list_tasks(filter)?;
        tasks.retain(|t| {
            t.title.to_lowercase().contains(&query_lower)
                || t.description.to_lowercase().contains(&query_lower)
        });
        Ok(tasks)
    }
}
