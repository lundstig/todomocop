use rust_query::migration::schema;

#[schema(TodoSchema)]
#[version(0..=3)]
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
        #[version(2..)]
        pub next_action: Option<String>,
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
        #[unique]
        pub url: String,
        pub repo: String,
        pub number: i64,
        pub state: String,
        pub last_refreshed: String,
        #[version(1..)]
        pub title: String,
        #[version(1..)]
        pub author: String,
        #[version(1..)]
        pub reviewers: String,
        #[version(1..)]
        pub review_state: String,
    }

    pub struct LinearContext {
        pub task: TableRow<Task>,
        pub url: String,
        #[unique]
        pub identifier: String,
        pub data: String,
        pub last_refreshed: String,
        #[version(1..)]
        pub state_type: String,
    }

    #[version(3..)]
    pub struct Tag {
        #[unique]
        pub name: String,
        pub description: String,
    }

    #[version(3..)]
    pub struct TaskTag {
        pub task: TableRow<Task>,
        pub tag: TableRow<Tag>,
    }
}

// Re-export the generated schema types from v3 so they're accessible from outside this module.
pub use v3::*;
