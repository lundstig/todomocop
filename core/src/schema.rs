use rust_query::migration::schema;

#[schema(TodoSchema)]
pub mod vN {
    pub struct Task {
        #[unique]
        pub external_id: i64,
        pub title: String,
        pub description: String,
        pub status: String,
        pub priority: Option<i64>,
        pub workspace: String,
        pub deadline: Option<String>,
        pub snooze_until: Option<String>,
        pub planned_date: Option<String>,
        pub deleted_at: Option<String>,
        pub created_at: String,
        pub updated_at: String,
    }

    pub struct Attachment {
        #[unique]
        pub external_id: i64,
        pub task: Task,
        pub file_name: String,
        pub content_type: String,
        /// Binary data stored as base64-encoded string.
        /// rust-query 0.4.4 has a bug where Vec<u8> (BLOB) columns cause
        /// a panic during schema validation, so we use String instead.
        pub data: String,
        pub caption: String,
        pub created_at: String,
    }

    pub struct GithubPrContext {
        pub task: Task,
        pub url: String,
        pub repo: String,
        pub number: i64,
        pub state: String,
        pub last_refreshed: String,
    }

    pub struct LinearContext {
        pub task: Task,
        pub url: String,
        pub identifier: String,
        pub data: String,
        pub last_refreshed: String,
    }
}

// Re-export the generated schema types from v0 so they're accessible from outside this module.
pub use v0::*;
