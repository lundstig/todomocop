use anyhow::Result;
use chrono::Utc;
use rust_query::{aggregate, optional};

use crate::schema;
use crate::types::{AttachmentId, NewAttachment, TaskId};
use crate::Db;

impl Db {
    pub fn add_attachment(&self, task_id: TaskId, att: NewAttachment) -> Result<AttachmentId> {
        let now = Utc::now().to_rfc3339();
        let caption = att.caption.unwrap_or_default();

        self.database.transaction_mut(|txn| {
            // Look up the task by external_id
            let task_row = txn
                .query_one(optional(|row| {
                    let task = row.and(schema::Task.external_id(task_id));
                    row.then(task)
                }))
                .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"))?;

            // Generate next external_id for the attachment: max + 1, or 1 if empty
            let next_id: i64 = txn
                .query_one(aggregate(|rows| {
                    let a = rows.join(schema::Attachment);
                    rows.max(&a.external_id)
                }))
                .map_or(1, |max| max + 1);

            txn.insert(schema::Attachment {
                external_id: next_id,
                task: task_row,
                file_name: att.file_name.clone(),
                content_type: att.content_type.clone(),
                data: att.data.clone(),
                caption: caption.clone(),
                created_at: now.clone(),
            })
            .map_err(|_| anyhow::anyhow!("attachment with this external_id already exists"))?;

            Ok(next_id)
        })
    }
}
