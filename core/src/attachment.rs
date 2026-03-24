use anyhow::Result;
use base64::Engine;
use chrono::Utc;
use rust_query::{aggregate, optional};

use crate::schema;
use crate::types::{AttachmentId, NewAttachment, TaskId};
use crate::Db;

impl Db {
    pub fn add_attachment(&mut self, task_id: TaskId, att: NewAttachment) -> Result<AttachmentId> {
        let now = Utc::now().to_rfc3339();
        let data_b64 = base64::engine::general_purpose::STANDARD.encode(&att.data);
        let caption = att.caption.unwrap_or_default();

        let mut txn = self.client.transaction_mut(&self.database);

        // Look up the task by external_id
        let task_row = txn
            .query_one(optional(|row| {
                let task = row.and(schema::Task::unique(task_id));
                row.then(task)
            }))
            .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"))?;

        // Generate next external_id for the attachment: max + 1, or 1 if empty
        let next_id: i64 = txn
            .query_one(aggregate(|rows| {
                let att = rows.join(schema::Attachment);
                rows.max(att.external_id())
            }))
            .map_or(1, |max| max + 1);

        txn.insert(schema::Attachment {
            external_id: next_id,
            task: task_row,
            file_name: &*att.file_name,
            content_type: &*att.content_type,
            data: &*data_b64,
            caption: &*caption,
            created_at: &*now,
        })
        .map_err(|_| anyhow::anyhow!("attachment with this external_id already exists"))?;

        txn.commit();
        Ok(next_id)
    }
}
