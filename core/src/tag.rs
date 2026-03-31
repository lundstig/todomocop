use anyhow::Result;
use rust_query::{optional, Select};

use crate::schema;
use crate::types::{TagData, TagInfo, TaskId};
use crate::Db;

#[derive(Select)]
struct TagSelect {
    name: String,
    description: String,
}

impl Db {
    /// Add a tag to a task. Auto-creates the tag if it doesn't exist. Idempotent.
    pub fn tag_task(&self, task_id: TaskId, tag_name: &str) -> Result<()> {
        self.database.transaction_mut(|txn| {
            // Look up the task
            let task_row = txn
                .query_one(optional(|row| {
                    let task = row.and(schema::Task.external_id(task_id));
                    row.then(task)
                }))
                .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"))?;

            // Find or create the tag
            let tag_row = match txn.query_one(optional(|row| {
                let tag = row.and(schema::Tag.name(tag_name.to_owned()));
                row.then(tag)
            })) {
                Some(existing) => existing,
                None => {
                    txn.insert(schema::Tag {
                        name: tag_name.to_owned(),
                        description: String::new(),
                    })
                    .map_err(|_| anyhow::anyhow!("failed to create tag: {tag_name}"))?
                }
            };

            // Check if already tagged
            let already_tagged = txn.query(|q| {
                let tt = q.join(schema::TaskTag);
                q.filter(tt.task.eq(&task_row));
                q.filter(tt.tag.eq(&tag_row));
                q.into_vec(())
            });

            if already_tagged.is_empty() {
                txn.insert_ok(schema::TaskTag {
                    task: task_row,
                    tag: tag_row,
                });
            }

            Ok(())
        })
    }

    /// Remove a tag from a task. No-op if the tag isn't on the task.
    pub fn untag_task(&self, task_id: TaskId, tag_name: &str) -> Result<()> {
        self.database.transaction_mut(|txn| {
            let task_row = txn
                .query_one(optional(|row| {
                    let task = row.and(schema::Task.external_id(task_id));
                    row.then(task)
                }))
                .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"))?;

            let tag_row = match txn.query_one(optional(|row| {
                let tag = row.and(schema::Tag.name(tag_name.to_owned()));
                row.then(tag)
            })) {
                Some(t) => t,
                None => return Ok(()), // tag doesn't exist, nothing to untag
            };

            // Find the task-tag association
            let associations: Vec<_> = txn.query(|q| {
                let tt = q.join(schema::TaskTag);
                q.filter(tt.task.eq(&task_row));
                q.filter(tt.tag.eq(&tag_row));
                q.into_vec(tt)
            });

            if let Some(tt_row) = associations.into_iter().next() {
                let txn = txn.downgrade();
                let _ = txn.delete(tt_row);
            }

            Ok(())
        })
    }

    /// Returns all tags for a given task.
    pub fn load_tags_for_task(&self, task_id: TaskId) -> Result<Vec<TagData>> {
        let results: Vec<TagSelect> = self.database.transaction(|txn| {
            txn.query(|q| {
                let task = q.join(schema::Task);
                let tag = q.join(schema::Tag);
                let tt = q.join(schema::TaskTag);
                q.filter(tt.task.eq(&task));
                q.filter(tt.tag.eq(&tag));
                q.filter(task.external_id.eq(task_id));
                q.into_vec(TagSelectSelect {
                    name: &tag.name,
                    description: &tag.description,
                })
            })
        });

        Ok(results
            .into_iter()
            .map(|t| TagData {
                name: t.name,
                description: t.description,
            })
            .collect())
    }

    /// List all tags with task counts. If workspace is given, only includes tags
    /// that have at least one non-deleted task in that workspace.
    pub fn list_tags(&self, workspace: Option<&str>) -> Result<Vec<TagInfo>> {
        #[derive(Select)]
        struct TagTaskJoin {
            tag_name: String,
            task_workspace: String,
            task_deleted_at: Option<String>,
        }

        let rows: Vec<TagTaskJoin> = self.database.transaction(|txn| {
            txn.query(|q| {
                let tag = q.join(schema::Tag);
                let task = q.join(schema::Task);
                let tt = q.join(schema::TaskTag);
                q.filter(tt.task.eq(&task));
                q.filter(tt.tag.eq(&tag));
                q.into_vec(TagTaskJoinSelect {
                    tag_name: &tag.name,
                    task_workspace: &task.workspace,
                    task_deleted_at: &task.deleted_at,
                })
            })
        });

        // Also get tags with zero associations
        let all_tags: Vec<TagSelect> = self.database.transaction(|txn| {
            txn.query(|q| {
                let tag = q.join(schema::Tag);
                q.into_vec(TagSelectSelect {
                    name: &tag.name,
                    description: &tag.description,
                })
            })
        });

        use std::collections::HashMap;
        let mut tag_map: HashMap<String, TagInfo> = HashMap::new();

        // Initialize all tags
        for t in &all_tags {
            tag_map.entry(t.name.clone()).or_insert_with(|| TagInfo {
                name: t.name.clone(),
                description: t.description.clone(),
                task_count: 0,
            });
        }

        // Count non-deleted tasks, respecting workspace filter
        for row in &rows {
            // Skip deleted tasks
            if row.task_deleted_at.is_some() {
                continue;
            }
            // Apply workspace filter
            if let Some(ws) = workspace {
                if row.task_workspace != ws {
                    continue;
                }
            }
            if let Some(info) = tag_map.get_mut(&row.tag_name) {
                info.task_count += 1;
            }
        }

        let mut result: Vec<TagInfo> = if workspace.is_some() {
            // Only include tags that have at least one matching task
            tag_map.into_values().filter(|t| t.task_count > 0).collect()
        } else {
            tag_map.into_values().collect()
        };

        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }

    /// Rename and/or update description of a tag.
    pub fn edit_tag(
        &self,
        current_name: &str,
        new_name: Option<&str>,
        description: Option<&str>,
    ) -> Result<()> {
        self.database.transaction_mut(|txn| {
            let tag_row = txn
                .query_one(optional(|row| {
                    let tag = row.and(schema::Tag.name(current_name.to_owned()));
                    row.then(tag)
                }))
                .ok_or_else(|| anyhow::anyhow!("tag not found: {current_name}"))?;

            if let Some(name) = new_name {
                // Unique fields can't be mutated, so we need to:
                // 1. Create a new tag with the new name
                // 2. Re-link all task associations
                // 3. Delete old associations and tag

                let old_desc = txn.query(|q| {
                    let tag = q.join(schema::Tag);
                    q.filter(tag.name.eq(current_name));
                    q.into_vec(&tag.description)
                });
                let current_desc = old_desc.into_iter().next().unwrap_or_default();
                let final_desc = description.unwrap_or(&current_desc).to_owned();

                // Get all task rows associated with old tag
                let task_rows: Vec<_> = txn.query(|q| {
                    let tt = q.join(schema::TaskTag);
                    q.filter(tt.tag.eq(&tag_row));
                    q.into_vec(tt)
                });

                // Create new tag
                let new_tag_row = txn
                    .insert(schema::Tag {
                        name: name.to_owned(),
                        description: final_desc,
                    })
                    .map_err(|_| anyhow::anyhow!("tag already exists: {name}"))?;

                // Create new associations: query task refs from old associations
                let task_refs: Vec<_> = txn.query(|q| {
                    let task = q.join(schema::Task);
                    let tt = q.join(schema::TaskTag);
                    q.filter(tt.tag.eq(&tag_row));
                    q.filter(tt.task.eq(&task));
                    q.into_vec(task)
                });

                for task_ref in &task_refs {
                    txn.insert_ok(schema::TaskTag {
                        task: *task_ref,
                        tag: new_tag_row,
                    });
                }

                // Downgrade and delete old associations + old tag
                let txn = txn.downgrade();
                for tt_row in task_rows {
                    let _ = txn.delete(tt_row);
                }
                let _ = txn.delete(tag_row);
            } else {
                // Only updating description, no rename needed
                if let Some(desc) = description {
                    let mut tag = txn.mutable(&tag_row);
                    tag.description = desc.to_owned();
                }
            }

            Ok(())
        })
    }

    /// Delete a tag and all its task-tag associations.
    pub fn delete_tag(&self, tag_name: &str) -> Result<()> {
        self.database.transaction_mut(|txn| {
            let tag_row = txn
                .query_one(optional(|row| {
                    let tag = row.and(schema::Tag.name(tag_name.to_owned()));
                    row.then(tag)
                }))
                .ok_or_else(|| anyhow::anyhow!("tag not found: {tag_name}"))?;

            // Gather all task-tag associations for this tag
            let associations: Vec<_> = txn.query(|q| {
                let tt = q.join(schema::TaskTag);
                q.filter(tt.tag.eq(&tag_row));
                q.into_vec(tt)
            });

            // Downgrade to allow deletions — no more inserts/queries after this
            let txn = txn.downgrade();

            // Delete associations first
            for tt_row in associations {
                let _ = txn.delete(tt_row);
            }

            // Delete the tag itself
            let _ = txn.delete(tag_row);

            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::types::AddTask;
    use crate::Db;

    fn make_add_task(title: &str, workspace: &str) -> AddTask {
        AddTask {
            title: title.into(),
            description: None,
            status: None,
            priority: None,
            workspace: workspace.into(),
            deadline: None,
            planned_date: None,
            next_action: None,
            tags: vec![],
        }
    }

    #[test]
    fn test_tag_and_load() {
        let db = Db::open_in_memory().unwrap();
        let task_id = db.add_task(make_add_task("Tagged task", "work")).unwrap();

        db.tag_task(task_id, "urgent").unwrap();

        let tags = db.load_tags_for_task(task_id).unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name, "urgent");
        assert_eq!(tags[0].description, "");
    }

    #[test]
    fn test_multiple_tags_on_task() {
        let db = Db::open_in_memory().unwrap();
        let task_id = db.add_task(make_add_task("Multi-tag task", "work")).unwrap();

        db.tag_task(task_id, "urgent").unwrap();
        db.tag_task(task_id, "backend").unwrap();
        db.tag_task(task_id, "v2").unwrap();

        let mut tags: Vec<String> = db
            .load_tags_for_task(task_id)
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        tags.sort();

        assert_eq!(tags, vec!["backend", "urgent", "v2"]);
    }

    #[test]
    fn test_shared_tag_across_tasks() {
        let db = Db::open_in_memory().unwrap();
        let t1 = db.add_task(make_add_task("Task 1", "work")).unwrap();
        let t2 = db.add_task(make_add_task("Task 2", "work")).unwrap();

        db.tag_task(t1, "shared").unwrap();
        db.tag_task(t2, "shared").unwrap();

        let tags1 = db.load_tags_for_task(t1).unwrap();
        let tags2 = db.load_tags_for_task(t2).unwrap();
        assert_eq!(tags1.len(), 1);
        assert_eq!(tags2.len(), 1);
        assert_eq!(tags1[0].name, "shared");
        assert_eq!(tags2[0].name, "shared");
    }

    #[test]
    fn test_tag_nonexistent_task() {
        let db = Db::open_in_memory().unwrap();
        let result = db.tag_task(9999, "urgent");
        assert!(result.is_err());
    }

    #[test]
    fn test_tag_idempotent() {
        let db = Db::open_in_memory().unwrap();
        let task_id = db.add_task(make_add_task("Idempotent task", "work")).unwrap();

        db.tag_task(task_id, "urgent").unwrap();
        db.tag_task(task_id, "urgent").unwrap(); // second time should be no-op

        let tags = db.load_tags_for_task(task_id).unwrap();
        assert_eq!(tags.len(), 1);
    }

    #[test]
    fn test_untag_task() {
        let db = Db::open_in_memory().unwrap();
        let task_id = db.add_task(make_add_task("Untag task", "work")).unwrap();

        db.tag_task(task_id, "urgent").unwrap();
        db.tag_task(task_id, "backend").unwrap();

        db.untag_task(task_id, "urgent").unwrap();

        let tags: Vec<String> = db
            .load_tags_for_task(task_id)
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(tags, vec!["backend"]);
    }

    #[test]
    fn test_untag_nonexistent_tag_is_noop() {
        let db = Db::open_in_memory().unwrap();
        let task_id = db.add_task(make_add_task("No tags", "work")).unwrap();

        // Untag a tag that was never applied — should be fine
        db.untag_task(task_id, "nonexistent").unwrap();

        let tags = db.load_tags_for_task(task_id).unwrap();
        assert!(tags.is_empty());
    }

    #[test]
    fn test_list_tags_with_workspace_filter() {
        let db = Db::open_in_memory().unwrap();
        let t1 = db.add_task(make_add_task("Work task", "work")).unwrap();
        let t2 = db.add_task(make_add_task("Personal task", "personal")).unwrap();

        db.tag_task(t1, "shared").unwrap();
        db.tag_task(t2, "shared").unwrap();
        db.tag_task(t1, "work-only").unwrap();
        db.tag_task(t2, "personal-only").unwrap();

        // No filter: all tags
        let all = db.list_tags(None).unwrap();
        assert_eq!(all.len(), 3);

        // Work filter: shared (count=1) and work-only (count=1)
        let work_tags = db.list_tags(Some("work")).unwrap();
        let names: Vec<&str> = work_tags.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"shared"));
        assert!(names.contains(&"work-only"));
        assert!(!names.contains(&"personal-only"));
        for tag in &work_tags {
            assert_eq!(tag.task_count, 1);
        }

        // Personal filter
        let personal_tags = db.list_tags(Some("personal")).unwrap();
        let names: Vec<&str> = personal_tags.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"shared"));
        assert!(names.contains(&"personal-only"));
        assert!(!names.contains(&"work-only"));
    }

    #[test]
    fn test_list_tags_excludes_deleted_tasks() {
        let db = Db::open_in_memory().unwrap();
        let t1 = db.add_task(make_add_task("Active task", "work")).unwrap();
        let t2 = db.add_task(make_add_task("Deleted task", "work")).unwrap();

        db.tag_task(t1, "important").unwrap();
        db.tag_task(t2, "important").unwrap();

        // Before deletion: count should be 2
        let tags = db.list_tags(None).unwrap();
        assert_eq!(tags[0].task_count, 2);

        // Delete task t2
        db.delete_task(t2).unwrap();

        // After deletion: count should be 1
        let tags = db.list_tags(None).unwrap();
        assert_eq!(tags[0].task_count, 1);
    }

    #[test]
    fn test_edit_tag_rename() {
        let db = Db::open_in_memory().unwrap();
        let task_id = db.add_task(make_add_task("Rename tag task", "work")).unwrap();

        db.tag_task(task_id, "old-name").unwrap();
        db.edit_tag("old-name", Some("new-name"), None).unwrap();

        // The task should now have the renamed tag
        let tags = db.load_tags_for_task(task_id).unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name, "new-name");
    }

    #[test]
    fn test_edit_tag_description() {
        let db = Db::open_in_memory().unwrap();
        let task_id = db.add_task(make_add_task("Desc tag task", "work")).unwrap();

        db.tag_task(task_id, "mytag").unwrap();
        db.edit_tag("mytag", None, Some("A useful description"))
            .unwrap();

        let tags = db.load_tags_for_task(task_id).unwrap();
        assert_eq!(tags[0].description, "A useful description");
    }

    #[test]
    fn test_delete_tag_cascades() {
        let db = Db::open_in_memory().unwrap();
        let t1 = db.add_task(make_add_task("Task 1", "work")).unwrap();
        let t2 = db.add_task(make_add_task("Task 2", "work")).unwrap();

        db.tag_task(t1, "doomed").unwrap();
        db.tag_task(t2, "doomed").unwrap();
        db.tag_task(t1, "keeper").unwrap();

        db.delete_tag("doomed").unwrap();

        // "doomed" tag should be gone from both tasks
        let tags1 = db.load_tags_for_task(t1).unwrap();
        let tags2 = db.load_tags_for_task(t2).unwrap();
        assert_eq!(tags1.len(), 1);
        assert_eq!(tags1[0].name, "keeper");
        assert!(tags2.is_empty());

        // Tag should not appear in list_tags
        let all = db.list_tags(None).unwrap();
        assert!(all.iter().all(|t| t.name != "doomed"));
    }

    #[test]
    fn test_delete_nonexistent_tag() {
        let db = Db::open_in_memory().unwrap();
        let result = db.delete_tag("nonexistent");
        assert!(result.is_err());
    }
}
