use anyhow::{Context, Result};
use serde_json::Value;
use todomocop_core::http::HttpClient;

use crate::types::LinearIssue;

pub struct LinearClient<'a> {
    http: &'a dyn HttpClient,
    api_key: String,
}

impl<'a> LinearClient<'a> {
    pub fn new(http: &'a dyn HttpClient, api_key: String) -> Self {
        Self { http, api_key }
    }

    pub fn fetch_assigned_issues(&self) -> Result<Vec<LinearIssue>> {
        let query = r#"{ "query": "{ viewer { assignedIssues(filter: { state: { type: { nin: [\"triage\", \"completed\", \"canceled\"] } } } first: 100) { nodes { id identifier title url state { name type } priority priorityLabel attachments(filter: { sourceType: { eq: \"github\" } }) { nodes { url } } } } } }" }"#;

        let headers = [
            ("Authorization", self.api_key.as_str()),
            ("Content-Type", "application/json"),
        ];

        let resp = self
            .http
            .post("https://api.linear.app/graphql", &headers, query.as_bytes())?;

        let json: Value = serde_json::from_slice(&resp).context("parse Linear response")?;
        let nodes = json["data"]["viewer"]["assignedIssues"]["nodes"]
            .as_array()
            .context("missing nodes in Linear response")?;

        let mut issues = Vec::new();
        for node in nodes {
            if let Some(issue) = parse_issue(node) {
                issues.push(issue);
            }
        }
        Ok(issues)
    }
}

fn parse_issue(node: &Value) -> Option<LinearIssue> {
    let identifier = node["identifier"].as_str()?.to_string();
    let title = node["title"].as_str()?.to_string();
    let url = node["url"].as_str()?.to_string();
    let state_type = node["state"]["type"].as_str()?.to_string();
    let priority = node["priority"].as_i64();

    let github_pr_urls: Vec<String> = node["attachments"]["nodes"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a["url"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Some(LinearIssue {
        identifier,
        title,
        url,
        state_type,
        priority,
        github_pr_urls,
    })
}
