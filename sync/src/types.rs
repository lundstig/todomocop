use std::collections::HashMap;

use todomocop_core::types::TaskStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceTag {
    Review,
    MyPr,
    Linear,
}

impl SourceTag {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::MyPr => "my-pr",
            Self::Linear => "linear",
        }
    }
}

#[derive(Debug, Clone)]
pub struct GithubPr {
    pub url: String,
    pub repo: String,
    pub number: i64,
    pub title: String,
    pub state: GithubPrState,
    pub author: String,
    pub reviewers: Vec<String>,
    pub review_state: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubPrState {
    Open,
    Merged,
    Closed,
}

#[derive(Debug, Clone)]
pub struct LinearIssue {
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub state_type: String,
    pub priority: Option<i64>,
    pub github_pr_urls: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncAction {
    CreateTaskWithGithub {
        title: String,
        pr: CreateGithubLink,
        status: TaskStatus,
        source: SourceTag,
    },
    CreateTaskWithLinear {
        title: String,
        linear: CreateLinearLink,
        status: TaskStatus,
        priority: Option<i64>,
        github_pr_urls: Vec<String>,
    },
    MarkDone {
        task_id: i64,
    },
    MarkCanceled {
        task_id: i64,
    },
    UpdateStatus {
        task_id: i64,
        status: TaskStatus,
    },
    LinkLinearToExistingTask {
        task_id: i64,
        linear: CreateLinearLink,
    },
    LinkGithubPrToExistingTask {
        task_id: i64,
        pr_url: String,
    },
    EnsureTag {
        task_id: i64,
        tag: SourceTag,
    },
    UpdateGithubContext {
        task_id: i64,
        pr: CreateGithubLink,
        state: GithubPrState,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateGithubLink {
    pub url: String,
    pub repo: String,
    pub number: i64,
    pub title: String,
    pub author: String,
    pub reviewers: Vec<String>,
    pub review_state: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateLinearLink {
    pub identifier: String,
    pub url: String,
    pub state_type: String,
}

impl GithubPrState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Merged => "merged",
            Self::Closed => "closed",
        }
    }
}
