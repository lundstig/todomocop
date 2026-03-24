use rust_query::migration::schema;

#[schema(TodoSchema)]
pub mod vN {
    use rust_query::TableRow;

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
        pub task: TableRow<Task>,
        pub file_name: String,
        pub content_type: String,
        /// Binary data stored as BLOB.
        pub data: Vec<u8>,
        pub caption: String,
        pub created_at: String,
    }

    pub struct GithubPrContext {
        pub task: TableRow<Task>,
        pub url: String,
        pub repo: String,
        pub number: i64,
        pub state: String,
        pub last_refreshed: String,
    }

    pub struct LinearContext {
        pub task: TableRow<Task>,
        pub url: String,
        pub identifier: String,
        pub data: String,
        pub last_refreshed: String,
    }
}

// Re-export the generated schema types from v0 so they're accessible from outside this module.
pub use v0::*;
