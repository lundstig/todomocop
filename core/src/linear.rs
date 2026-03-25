use anyhow::{bail, Result};
use chrono::Utc;
use rust_query::{optional, Select};

use crate::schema;
use crate::types::{LinearContextData, Task, TaskId};
use crate::Db;

/// Parse a Linear identifier (e.g. "ENG-123") into (team_key, number).
fn parse_identifier(identifier: &str) -> Result<(&str, u64)> {
    let (team_key, number_str) = identifier
        .rsplit_once('-')
        .ok_or_else(|| anyhow::anyhow!("invalid Linear identifier: expected format TEAM-NUMBER, got '{identifier}'"))?;
    let number = number_str
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("invalid Linear identifier: number part is not a valid integer in '{identifier}'"))?;
    Ok((team_key, number))
}

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
    state_type: String,
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
                state_type: String::new(),
            })
            .map_err(|_| anyhow::anyhow!("failed to insert Linear context"))?;

            Ok(())
        })
    }

    /// Load all LinearContexts for a given task (by external_id).
    pub(crate) fn load_linear_contexts(&self, task_id: TaskId) -> Result<Vec<LinearContextData>> {
        let results: Vec<LinearContextSelect> = self.database.transaction(|txn| {
            txn.query(|q| {
                let task = q.join(schema::Task);
                let ctx = q.join(schema::LinearContext);
                q.filter(ctx.task.eq(&task));
                q.filter(task.external_id.eq(task_id));
                q.into_vec(LinearContextSelectSelect {
                    url: &ctx.url,
                    identifier: &ctx.identifier,
                    state_type: &ctx.state_type,
                    data: &ctx.data,
                    last_refreshed: &ctx.last_refreshed,
                })
            })
        });

        Ok(results
            .into_iter()
            .map(|c| {
                let data_value: serde_json::Value = serde_json::from_str(&c.data)
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                LinearContextData {
                    url: c.url,
                    identifier: c.identifier,
                    state_type: c.state_type,
                    data: data_value,
                    last_refreshed: c.last_refreshed,
                }
            })
            .collect())
    }

    /// Refresh Linear contexts for the given tasks if they are stale.
    pub(crate) fn refresh_linear_contexts(&self, tasks: &mut [Task]) -> Result<()> {
        let linear_api_key = match &self.config.linear_api_key {
            Some(k) => k.clone(),
            None => return Ok(()),
        };

        let threshold = self.config.staleness_threshold;

        for task in tasks.iter_mut() {
            for ctx in &mut task.linear_contexts {
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

                // Parse identifier into team key and number for the filter query
                let (team_key, issue_number) = match parse_identifier(&ctx.identifier) {
                    Ok(parts) => parts,
                    Err(_) => continue,
                };

                // Fetch from Linear GraphQL API using filter to support human-readable identifiers
                let query_body = serde_json::json!({
                    "query": format!(
                        r#"{{ issues(filter: {{ number: {{ eq: {} }}, team: {{ key: {{ eq: "{}" }} }} }}) {{ nodes {{ id identifier title state {{ name type }} priority priorityLabel assignee {{ name }} url }} }} }}"#,
                        issue_number, team_key
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

                let resp_json: serde_json::Value = match serde_json::from_slice(&resp_body) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // Extract the first node from the issues list
                let json = match resp_json
                    .get("data")
                    .and_then(|d| d.get("issues"))
                    .and_then(|i| i.get("nodes"))
                    .and_then(|n| n.as_array())
                    .and_then(|arr| arr.first())
                    .cloned()
                {
                    Some(node) => node,
                    None => continue,
                };

                let now = Utc::now().to_rfc3339();
                let data_str = serde_json::to_string(&json).unwrap_or_else(|_| "{}".to_string());
                let state_type = json
                    .get("state")
                    .and_then(|s| s.get("type"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();

                // Update the DB row matching this context's URL
                let task_id = task.id;
                let ctx_url = ctx.url.clone();
                let data_str_clone = data_str.clone();
                let state_type_clone = state_type.clone();
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
                            q.filter(c.url.eq(&ctx_url));
                            q.into_vec(c)
                        });

                        if let Some(ctx_row) = ctx_rows.into_iter().next() {
                            let mut ctx_mut = txn.mutable(&ctx_row);
                            ctx_mut.data = data_str_clone.clone();
                            ctx_mut.state_type = state_type_clone.clone();
                            ctx_mut.last_refreshed = now_clone.clone();
                        }
                    }
                });

                // Update in-memory context
                ctx.data = json;
                ctx.state_type = state_type;
                ctx.last_refreshed = now;
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

    #[test]
    fn test_parse_identifier_valid() {
        let (team, number) = parse_identifier("ENG-123").unwrap();
        assert_eq!(team, "ENG");
        assert_eq!(number, 123);
    }

    #[test]
    fn test_parse_identifier_multi_part_team() {
        // rsplit_once splits on the last '-', so "MY-TEAM-456" -> team="MY-TEAM", number=456
        let (team, number) = parse_identifier("MY-TEAM-456").unwrap();
        assert_eq!(team, "MY-TEAM");
        assert_eq!(number, 456);
    }

    #[test]
    fn test_parse_identifier_invalid() {
        assert!(parse_identifier("NONUMBER").is_err());
        assert!(parse_identifier("ENG-abc").is_err());
        assert!(parse_identifier("").is_err());
    }
}
