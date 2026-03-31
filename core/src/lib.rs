pub mod attachment;
pub mod github;
pub mod http;
pub mod linear;
pub mod query;
pub mod schema;
pub mod tag;
pub mod task;
pub mod types;

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use rust_query::migration::Config;
use rust_query::Database;

use http::{HttpClient, IntegrationConfig, NoopHttpClient};
use schema::TodoSchema;

pub struct Db {
    pub database: Database<TodoSchema>,
    pub http: Arc<dyn HttpClient>,
    pub config: IntegrationConfig,
}

impl Db {
    pub fn open(path: &Path, http: Arc<dyn HttpClient>, config: IntegrationConfig) -> Result<Self> {
        let database = Database::migrator(Config::open(path))
            .ok_or_else(|| anyhow::anyhow!("database version is older than supported"))?
            .migrate(|txn| schema::v0::migrate::TodoSchema {
                github_pr_context: txn.migrate_ok(|_old| schema::v0::migrate::GithubPrContext {
                    title: String::new(),
                    author: String::new(),
                    reviewers: "[]".to_owned(),
                    review_state: "{}".to_owned(),
                }),
                linear_context: txn.migrate_ok(|_old| schema::v0::migrate::LinearContext {
                    state_type: String::new(),
                }),
            })
            .migrate(|txn| schema::v1::migrate::TodoSchema {
                task: txn.migrate_ok(|_old| schema::v1::migrate::Task {
                    next_action: None,
                }),
            })
            .migrate(|_txn| schema::v2::migrate::TodoSchema {})
            .finish()
            .ok_or_else(|| anyhow::anyhow!("database version is newer than supported"))?;
        Ok(Db {
            database,
            http,
            config,
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        let database = Database::new(Config::open_in_memory());
        Ok(Db {
            database,
            http: Arc::new(NoopHttpClient),
            config: IntegrationConfig::default(),
        })
    }
}
