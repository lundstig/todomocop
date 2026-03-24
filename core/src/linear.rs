use anyhow::{bail, Result};
use chrono::Utc;
use rust_query::{optional, Select};

use crate::schema;
use crate::types::{LinearContextData, Task, TaskId};
use crate::Db;

/// Parse a Linear issue URL to extract the identifier (e.g. "ENG-123").
/// Expected format: `https://linear.app/{team}/issue/{IDENTIFIER}/...`
fn parse_linear_url(url: &str) -> Result<String> {
    let stripped = url
        .strip_prefix("https://linear.app/")
        .ok_or_else(|| anyhow::anyhow!("invalid Linear URL: must start with https://linear.app/"))?;

    let parts: Vec<&str> = stripped.split('/').collect();
    // Expected: [team, "issue", identifier, ...]
    if parts.len() < 3 || parts[1] != "issue" {
        bail!("invalid Linear URL format: expected https://linear.app/{{team}}/issue/{{IDENTIFIER}}/...");
    }

    let identifier = parts[2].to_string();
    if identifier.is_empty() {
        bail!("invalid Linear URL: empty identifier");
    }

    Ok(identifier)
}

#[derive(Select)]
struct LinearContextSelect {
    url: String,
    identifier: String,
    data: String,
    last_refreshed: String,
}

impl Db {
    pub fn link_linear(&self, task_id: TaskId, url: &str) -> Result<()> {
        let identifier = parse_linear_url(url)?;
        let now = Utc::now().to_rfc3339();

        self.database.transaction_mut(|txn| {
            let task_row = txn
                .query_one(optional(|row| {
                    let task = row.and(schema::Task.external_id(task_id));
                    row.then(task)
                }))
                .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"))?;

            txn.insert(schema::LinearContext {
                task: task_row,
                url: url.to_owned(),
                identifier: identifier.clone(),
                data: "{}".to_owned(),
                last_refreshed: now.clone(),
            })
            .map_err(|_| anyhow::anyhow!("failed to insert Linear context"))?;

            Ok(())
        })
    }

    /// Load the LinearContext for a given task (by external_id) if one exists.
    pub(crate) fn load_linear_context(&self, task_id: TaskId) -> Result<Option<LinearContextData>> {
        let results: Vec<LinearContextSelect> = self.database.transaction(|txn| {
            txn.query(|q| {
                let task = q.join(schema::Task);
                let ctx = q.join(schema::LinearContext);
                q.filter(ctx.task.eq(&task));
                q.filter(task.external_id.eq(task_id));
                q.into_vec(LinearContextSelectSelect {
                    url: &ctx.url,
                    identifier: &ctx.identifier,
                    data: &ctx.data,
                    last_refreshed: &ctx.last_refreshed,
                })
            })
        });

        Ok(results.into_iter().next().map(|c| {
            let data_value: serde_json::Value =
                serde_json::from_str(&c.data).unwrap_or(serde_json::Value::Object(Default::default()));
            LinearContextData {
                url: c.url,
                identifier: c.identifier,
                data: data_value,
                last_refreshed: c.last_refreshed,
            }
        }))
    }

    /// Refresh Linear contexts for the given tasks if they are stale.
    pub(crate) fn refresh_linear_contexts(&self, tasks: &mut [Task]) -> Result<()> {
        let linear_api_key = match &self.config.linear_api_key {
            Some(k) => k.clone(),
            None => return Ok(()),
        };

        let threshold = self.config.staleness_threshold;

        for task in tasks.iter_mut() {
            let ctx = match &task.linear_context {
                Some(c) => c,
                None => continue,
            };

            // Check staleness
            let last_refreshed = chrono::DateTime::parse_from_rfc3339(&ctx.last_refreshed)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            let age = Utc::now()
                .signed_duration_since(last_refreshed)
                .to_std()
                .unwrap_or_default();

            if age < threshold {
                continue;
            }

            // Fetch from Linear GraphQL API
            let query_body = serde_json::json!({
                "query": format!(
                    r#"{{ issue(id: "{}") {{ id title state {{ name }} priority priorityLabel assignee {{ name }} }} }}"#,
                    ctx.identifier
                )
            });
            let body_bytes = serde_json::to_vec(&query_body).unwrap_or_default();

            let response = self.http.post(
                "https://api.linear.app/graphql",
                &[
                    ("Authorization", linear_api_key.as_str()),
                    ("Content-Type", "application/json"),
                ],
                &body_bytes,
            );

            let resp_body = match response {
                Ok(b) => b,
                Err(_) => continue, // On failure, return stale data
            };

            let json: serde_json::Value = match serde_json::from_slice(&resp_body) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let now = Utc::now().to_rfc3339();
            let data_str = serde_json::to_string(&json).unwrap_or_else(|_| "{}".to_string());

            // Update the DB row
            let task_id = task.id;
            let data_str_clone = data_str.clone();
            let now_clone = now.clone();
            self.database.transaction_mut_ok(|txn| {
                let task_row = txn.query_one(optional(|row| {
                    let t = row.and(schema::Task.external_id(task_id));
                    row.then(t)
                }));

                if let Some(task_row) = task_row {
                    let ctx_rows: Vec<_> = txn.query(|q| {
                        let c = q.join(schema::LinearContext);
                        q.filter(c.task.eq(task_row));
                        q.into_vec(c)
                    });

                    if let Some(ctx_row) = ctx_rows.into_iter().next() {
                        let mut ctx_mut = txn.mutable(&ctx_row);
                        ctx_mut.data = data_str_clone.clone();
                        ctx_mut.last_refreshed = now_clone.clone();
                    }
                }
            });

            // Update in-memory task
            if let Some(ref mut c) = task.linear_context {
                c.data = json;
                c.last_refreshed = now;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_linear_url_valid() {
        let id = parse_linear_url("https://linear.app/myteam/issue/ENG-123/some-title").unwrap();
        assert_eq!(id, "ENG-123");
    }

    #[test]
    fn test_parse_linear_url_no_trailing() {
        let id = parse_linear_url("https://linear.app/myteam/issue/ENG-456").unwrap();
        assert_eq!(id, "ENG-456");
    }

    #[test]
    fn test_parse_linear_url_invalid() {
        assert!(parse_linear_url("https://example.com/foo").is_err());
        assert!(parse_linear_url("https://linear.app/team/project/ENG-1").is_err());
    }
}
