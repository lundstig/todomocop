use std::collections::HashMap;

use anyhow::{bail, Result};
use chrono::Utc;
use rust_query::{optional, Select};

use crate::schema;
use crate::types::{GithubPrContextData, GithubPrContextUpdate, Task, TaskId};
use crate::Db;

/// Parse a GitHub PR URL into (repo, number).
/// Expected format: `https://github.com/{owner}/{repo}/pull/{number}`
fn parse_github_pr_url(url: &str) -> Result<(String, i64)> {
    let stripped = url
        .strip_prefix("https://github.com/")
        .ok_or_else(|| anyhow::anyhow!("invalid GitHub PR URL: must start with https://github.com/"))?;

    let parts: Vec<&str> = stripped.split('/').collect();
    // Expected: [owner, repo, "pull", number]
    if parts.len() < 4 || parts[2] != "pull" {
        bail!("invalid GitHub PR URL format: expected https://github.com/{{owner}}/{{repo}}/pull/{{number}}");
    }

    let owner = parts[0];
    let repo_name = parts[1];
    let number: i64 = parts[3]
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid PR number in URL"))?;

    Ok((format!("{owner}/{repo_name}"), number))
}

#[derive(Select)]
struct GithubContextSelect {
    url: String,
    repo: String,
    number: i64,
    state: String,
    title: String,
    author: String,
    reviewers: String,
    review_state: String,
    last_refreshed: String,
}

impl Db {
    pub fn link_github_pr(&self, task_id: TaskId, url: &str) -> Result<()> {
        let (repo, number) = parse_github_pr_url(url)?;
        let now = Utc::now().to_rfc3339();

        self.database.transaction_mut(|txn| {
            let task_row = txn
                .query_one(optional(|row| {
                    let task = row.and(schema::Task.external_id(task_id));
                    row.then(task)
                }))
                .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"))?;

            txn.insert(schema::GithubPrContext {
                task: task_row,
                url: url.to_owned(),
                repo: repo.clone(),
                number,
                state: "unknown".to_owned(),
                last_refreshed: now.clone(),
                title: String::new(),
                author: String::new(),
                reviewers: "[]".to_owned(),
                review_state: "{}".to_owned(),
            })
            .map_err(|_| anyhow::anyhow!("failed to insert GitHub PR context"))?;

            Ok(())
        })
    }

    pub fn find_task_by_github_pr(&self, pr_url: &str) -> Result<Option<Task>> {
        let task_ids: Vec<i64> = self.database.transaction(|txn| {
            txn.query(|q| {
                let task = q.join(schema::Task);
                let ctx = q.join(schema::GithubPrContext);
                q.filter(ctx.task.eq(&task));
                q.filter(ctx.url.eq(pr_url));
                q.into_vec(&task.external_id)
            })
        });

        match task_ids.into_iter().next() {
            Some(id) => self.get_task(id),
            None => Ok(None),
        }
    }

    pub fn update_github_pr_context(
        &self,
        task_id: TaskId,
        update: &GithubPrContextUpdate,
    ) -> Result<()> {
        let reviewers = serde_json::to_string(&update.reviewers)?;
        let review_state = serde_json::to_string(&update.review_state)?;
        let now = Utc::now().to_rfc3339();

        self.database.transaction_mut(|txn| {
            let task_row = txn
                .query_one(optional(|row| {
                    let task = row.and(schema::Task.external_id(task_id));
                    row.then(task)
                }))
                .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"))?;

            let context = txn
                .query_one(optional(|row| {
                    let context = row.and(schema::GithubPrContext.url(update.url.clone()));
                    row.then(context)
                }))
                .ok_or_else(|| anyhow::anyhow!("GitHub PR context not found: {}", update.url))?;

            let belongs_to_task = txn.query(|q| {
                let context = q.join(schema::GithubPrContext);
                q.filter(context.task.eq(&task_row));
                q.filter(context.url.eq(&update.url));
                q.into_vec(())
            });
            if belongs_to_task.is_empty() {
                anyhow::bail!("GitHub PR context does not belong to task {task_id}: {}", update.url);
            }

            let mut context = txn.mutable(&context);
            context.state = update.state.clone();
            context.title = update.title.clone();
            context.author = update.author.clone();
            context.reviewers = reviewers.clone();
            context.review_state = review_state.clone();
            context.last_refreshed = now.clone();
            Ok(())
        })
    }

    /// Load all GithubPrContexts for a given task (by external_id).
    pub(crate) fn load_github_contexts(&self, task_id: TaskId) -> Result<Vec<GithubPrContextData>> {
        let results: Vec<GithubContextSelect> = self.database.transaction(|txn| {
            txn.query(|q| {
                let task = q.join(schema::Task);
                let ctx = q.join(schema::GithubPrContext);
                q.filter(ctx.task.eq(&task));
                q.filter(task.external_id.eq(task_id));
                q.into_vec(GithubContextSelectSelect {
                    url: &ctx.url,
                    repo: &ctx.repo,
                    number: &ctx.number,
                    state: &ctx.state,
                    title: &ctx.title,
                    author: &ctx.author,
                    reviewers: &ctx.reviewers,
                    review_state: &ctx.review_state,
                    last_refreshed: &ctx.last_refreshed,
                })
            })
        });

        Ok(results
            .into_iter()
            .map(|c| {
                let reviewers: Vec<String> =
                    serde_json::from_str(&c.reviewers).unwrap_or_default();
                let review_state: HashMap<String, String> =
                    serde_json::from_str(&c.review_state).unwrap_or_default();
                GithubPrContextData {
                    url: c.url,
                    repo: c.repo,
                    number: c.number,
                    state: c.state,
                    title: c.title,
                    author: c.author,
                    reviewers,
                    review_state,
                    last_refreshed: c.last_refreshed,
                }
            })
            .collect())
    }

    /// Explicitly refresh stale GitHub PR contexts for the given tasks.
    pub fn refresh_github_contexts(&self, tasks: &mut [Task]) -> Result<()> {
        let github_token = match &self.config.github_token {
            Some(t) => t.clone(),
            None => return Ok(()),
        };

        let threshold = self.config.staleness_threshold;

        for task in tasks.iter_mut() {
            for ctx in &mut task.github_pr_contexts {
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

                // Fetch from GitHub API
                let api_url = format!(
                    "https://api.github.com/repos/{}/pulls/{}",
                    ctx.repo, ctx.number
                );
                let auth_header = format!("Bearer {github_token}");
                let response = self.http.get(
                    &api_url,
                    &[
                        ("Authorization", &auth_header),
                        ("Accept", "application/vnd.github+json"),
                        ("User-Agent", "todomocop"),
                    ],
                );

                let body = match response {
                    Ok(b) => b,
                    Err(_) => continue, // On failure, return stale data
                };

                let json: serde_json::Value = match serde_json::from_slice(&body) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // Determine state: "merged" if merged==true, else use state field
                let state = if json.get("merged").and_then(|v| v.as_bool()).unwrap_or(false) {
                    "merged".to_string()
                } else {
                    json.get("state")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string()
                };

                let title = json
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let author = json
                    .get("user")
                    .and_then(|user| user.get("login"))
                    .and_then(|login| login.as_str())
                    .unwrap_or("")
                    .to_string();

                let mut reviewers: Vec<String> = json
                    .get("requested_reviewers")
                    .and_then(|value| value.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|reviewer| reviewer.get("login").and_then(|login| login.as_str()))
                    .map(str::to_owned)
                    .collect();
                let requested_reviewers = reviewers.clone();

                let reviews_url = format!(
                    "https://api.github.com/repos/{}/pulls/{}/reviews",
                    ctx.repo, ctx.number
                );
                let reviews_body = match self.http.get(
                        &reviews_url,
                        &[
                            ("Authorization", &auth_header),
                            ("Accept", "application/vnd.github+json"),
                            ("User-Agent", "todomocop"),
                        ],
                    ) {
                    Ok(body) => body,
                    Err(_) => continue,
                };
                let reviews: Vec<serde_json::Value> = match serde_json::from_slice::<serde_json::Value>(&reviews_body)
                    .ok()
                    .and_then(|value| value.as_array().cloned())
                {
                    Some(reviews) => reviews,
                    None => continue,
                };
                let mut review_state: HashMap<String, String> = reviews
                    .into_iter()
                    .filter_map(|review| {
                        let login = review.get("user")?.get("login")?.as_str()?;
                        let state = review.get("state")?.as_str()?;
                        Some((login.to_string(), state.to_lowercase()))
                    })
                    .collect();
                for reviewer in review_state.keys() {
                    if !reviewers.contains(reviewer) {
                        reviewers.push(reviewer.clone());
                    }
                }
                for reviewer in requested_reviewers {
                    review_state.insert(reviewer, "pending".to_string());
                }

                let update = GithubPrContextUpdate {
                    url: ctx.url.clone(),
                    state: state.clone(),
                    title: title.clone(),
                    author: author.clone(),
                    reviewers: reviewers.clone(),
                    review_state: review_state.clone(),
                };
                self.update_github_pr_context(task.id, &update)?;

                // Update in-memory context
                ctx.state = state;
                ctx.title = title;
                ctx.author = author;
                ctx.reviewers = reviewers;
                ctx.review_state = review_state;
                ctx.last_refreshed = Utc::now().to_rfc3339();
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_pr_url_valid() {
        let (repo, number) = parse_github_pr_url("https://github.com/rust-lang/rust/pull/12345").unwrap();
        assert_eq!(repo, "rust-lang/rust");
        assert_eq!(number, 12345);
    }

    #[test]
    fn test_parse_github_pr_url_invalid() {
        assert!(parse_github_pr_url("https://example.com/foo").is_err());
        assert!(parse_github_pr_url("https://github.com/owner/repo/issues/1").is_err());
        assert!(parse_github_pr_url("https://github.com/owner/repo/pull/abc").is_err());
    }
}
