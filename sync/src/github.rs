use std::collections::HashMap;

use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use serde_json::Value;
use todomocop_core::http::HttpClient;
use todomocop_core::types::{Task, TaskStatus};

use crate::types::{GithubPr, GithubPrState};

pub struct GithubClient<'a> {
    http: &'a dyn HttpClient,
    token: String,
    user: String,
}

impl<'a> GithubClient<'a> {
    pub fn new(http: &'a dyn HttpClient, token: String) -> Result<Self> {
        let h = github_headers(&token);
        let resp = http.get("https://api.github.com/user", &headers_ref(&h))?;
        let json: Value = serde_json::from_slice(&resp).context("parse /user response")?;
        let user = json["login"]
            .as_str()
            .context("missing login in /user response")?
            .to_string();
        Ok(Self { http, token, user })
    }

    pub fn username(&self) -> &str {
        &self.user
    }

    pub fn fetch_prs(&self, since_days: u64) -> Result<Vec<GithubPr>> {
        let since = (Utc::now() - Duration::days(since_days as i64))
            .format("%Y-%m-%d")
            .to_string();
        let user = &self.user;

        let authored_open = self.search_prs(&format!(
            "type:pr+author:{user}+is:open"
        ))?;
        let review_requested = self.search_prs(&format!(
            "type:pr+user-review-requested:{user}+is:open"
        ))?;
        let authored_merged = self.search_prs(&format!(
            "type:pr+author:{user}+is:merged+merged:>={since}"
        ))?;

        let review_requested_urls: std::collections::HashSet<String> =
            review_requested.iter().map(|pr| pr.url.clone()).collect();

        // Merge and dedup by URL — authored_open first, then review_requested, then merged
        let mut seen: HashMap<String, GithubPr> = HashMap::new();
        for pr in authored_open
            .into_iter()
            .chain(review_requested.into_iter())
            .chain(authored_merged.into_iter())
        {
            seen.entry(pr.url.clone()).or_insert(pr);
        }

        let mut result: Vec<GithubPr> = Vec::with_capacity(seen.len());
        for (url, pr) in seen {
            let mut hydrated = self.fetch_pr(&pr.repo, pr.number)?;
            if review_requested_urls.contains(&url) && !hydrated.reviewers.contains(&self.user) {
                hydrated.reviewers.push(self.user.clone());
            }
            result.push(hydrated);
        }

        Ok(result)
    }

    pub fn fetch_linked_prs(&self, tasks: &[Task]) -> Result<Vec<GithubPr>> {
        let mut seen = std::collections::HashSet::new();
        let mut prs = Vec::new();
        for task in tasks {
            if task.status == TaskStatus::Done || task.status == TaskStatus::Canceled {
                continue;
            }
            for context in &task.github_pr_contexts {
                if seen.insert(context.url.clone()) {
                    prs.push(self.fetch_pr(&context.repo, context.number)?);
                }
            }
        }
        Ok(prs)
    }

    fn fetch_pr(&self, repo: &str, number: i64) -> Result<GithubPr> {
        let url = format!("https://api.github.com/repos/{repo}/pulls/{number}");
        let h = github_headers(&self.token);
        let resp = self.http.get(&url, &headers_ref(&h))?;
        let json: Value = serde_json::from_slice(&resp).context("parse pull response")?;
        let mut pr = parse_pr_item(&json).context("invalid pull response")?;
        let requested_reviewers = pr.reviewers.clone();
        pr.review_state = self.fetch_reviews(repo, number)?;
        for reviewer in pr.review_state.keys() {
            if !pr.reviewers.contains(reviewer) {
                pr.reviewers.push(reviewer.clone());
            }
        }
        for reviewer in requested_reviewers {
            pr.review_state.insert(reviewer, "pending".to_string());
        }
        Ok(pr)
    }

    fn search_prs(&self, query: &str) -> Result<Vec<GithubPr>> {
        // The query uses '+' as space separators. URL-encode only the special
        // characters that ureq's URI parser rejects (colons, angle brackets, etc).
        // '+' is valid in query strings (represents space) and must NOT be encoded.
        let encoded_query = query
            .replace(':', "%3A")
            .replace('>', "%3E")
            .replace('=', "%3D");
        let url = format!(
            "https://api.github.com/search/issues?q={encoded_query}&per_page=100"
        );
        let h = github_headers(&self.token);
        let resp = self.http.get(&url, &headers_ref(&h))?;
        let json: Value = serde_json::from_slice(&resp).context("parse search response")?;
        let items = json["items"].as_array().context("missing items in search response")?;

        let mut prs = Vec::new();
        for item in items {
            if let Some(pr) = parse_pr_item(item) {
                prs.push(pr);
            }
        }
        Ok(prs)
    }

    fn fetch_reviews(&self, repo: &str, number: i64) -> Result<HashMap<String, String>> {
        let url = format!("https://api.github.com/repos/{repo}/pulls/{number}/reviews");
        let h = github_headers(&self.token);
        let resp = self.http.get(&url, &headers_ref(&h))?;
        let json: Value = serde_json::from_slice(&resp).context("parse reviews response")?;
        let reviews = json.as_array().context("reviews response is not an array")?;

        // Later reviews override earlier ones for the same user
        let mut review_state: HashMap<String, String> = HashMap::new();
        for review in reviews {
            let login = review["user"]["login"].as_str().unwrap_or_default();
            let state = review["state"].as_str().unwrap_or_default();
            if !login.is_empty() && !state.is_empty() {
                review_state.insert(login.to_string(), state.to_lowercase());
            }
        }
        Ok(review_state)
    }
}

fn github_headers(token: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Authorization", format!("Bearer {token}")),
        ("Accept", "application/vnd.github+json".to_string()),
        ("User-Agent", "todomocop".to_string()),
    ]
}

fn headers_ref<'a>(headers: &'a [(&'static str, String)]) -> Vec<(&'a str, &'a str)> {
    headers.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

fn parse_pr_item(item: &Value) -> Option<GithubPr> {
    let url = item["html_url"]
        .as_str()
        .or_else(|| item["pull_request"]["html_url"].as_str())?;

    // Parse: https://github.com/{owner}/{repo}/pull/{number}
    let parts: Vec<&str> = url.trim_start_matches("https://github.com/").split('/').collect();
    if parts.len() < 4 || parts[2] != "pull" {
        return None;
    }
    let owner = parts[0];
    let repo_name = parts[1];
    let number: i64 = parts[3].parse().ok()?;
    let repo = format!("{owner}/{repo_name}");

    let title = item["title"].as_str()?.to_string();
    let author = item["user"]["login"].as_str().unwrap_or("").to_string();
    let state_str = item["state"].as_str().unwrap_or("open");
    let merged = item["merged"].as_bool().unwrap_or(false)
        || item["pull_request"]["merged_at"].is_string();

    let state = if state_str == "closed" {
        if merged {
            GithubPrState::Merged
        } else {
            GithubPrState::Closed
        }
    } else {
        GithubPrState::Open
    };

    let reviewers = item["requested_reviewers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|reviewer| reviewer["login"].as_str())
        .map(str::to_owned)
        .collect();

    Some(GithubPr {
        url: url.to_string(),
        repo,
        number,
        title,
        state,
        author,
        reviewers,
        review_state: HashMap::new(),
    })
}

pub fn merge_prs(discovered: Vec<GithubPr>, linked: Vec<GithubPr>) -> Vec<GithubPr> {
    let mut by_url: HashMap<String, GithubPr> = discovered
        .into_iter()
        .map(|pr| (pr.url.clone(), pr))
        .collect();
    for pr in linked {
        by_url.insert(pr.url.clone(), pr);
    }
    by_url.into_values().collect()
}
