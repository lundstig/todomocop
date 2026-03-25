use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

pub type TaskId = i64;
pub type AttachmentId = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Idea,
    Ready,
    InProgress,
    Done,
    Canceled,
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskStatus::Idea => write!(f, "idea"),
            TaskStatus::Ready => write!(f, "ready"),
            TaskStatus::InProgress => write!(f, "in_progress"),
            TaskStatus::Done => write!(f, "done"),
            TaskStatus::Canceled => write!(f, "canceled"),
        }
    }
}

impl FromStr for TaskStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "idea" => Ok(TaskStatus::Idea),
            "ready" => Ok(TaskStatus::Ready),
            "in_progress" => Ok(TaskStatus::InProgress),
            "done" => Ok(TaskStatus::Done),
            "canceled" => Ok(TaskStatus::Canceled),
            other => Err(anyhow::anyhow!("unknown task status: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddTask {
    pub title: String,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub priority: Option<i64>,
    pub workspace: String,
    pub deadline: Option<String>,
    pub planned_date: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EditTask {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    /// Some(None) = clear priority, None = don't touch
    pub priority: Option<Option<i64>>,
    pub workspace: Option<String>,
    /// Some(None) = clear deadline, None = don't touch
    pub deadline: Option<Option<String>>,
    /// Some(None) = clear snooze, None = don't touch
    pub snooze_until: Option<Option<String>>,
    /// Some(None) = clear planned_date, None = don't touch
    pub planned_date: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub priority: Option<i64>,
    pub workspace: String,
    pub deadline: Option<String>,
    pub snooze_until: Option<String>,
    pub planned_date: Option<String>,
    pub deleted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub github_pr_contexts: Vec<GithubPrContextData>,
    pub linear_contexts: Vec<LinearContextData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubPrContextData {
    pub url: String,
    pub repo: String,
    pub number: i64,
    pub state: String,
    pub title: String,
    pub author: String,
    pub reviewers: Vec<String>,
    pub review_state: HashMap<String, String>,
    pub last_refreshed: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinearContextData {
    pub url: String,
    pub identifier: String,
    pub state_type: String,
    pub data: serde_json::Value,
    pub last_refreshed: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskFilter {
    pub status: Option<TaskStatus>,
    pub workspace: Option<String>,
    pub has_planned_date: Option<bool>,
    pub priority_max: Option<i64>,
    pub include_snoozed: bool,
}

#[derive(Debug, Clone)]
pub struct NewAttachment {
    pub file_name: String,
    pub content_type: String,
    pub data: Vec<u8>,
    pub caption: Option<String>,
}
