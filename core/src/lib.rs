pub mod attachment;
pub mod http;
pub mod query;
pub mod schema;
pub mod task;
pub mod types;

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use rust_query::migration::Config;
use rust_query::{Database, LocalClient};

use http::{HttpClient, IntegrationConfig, NoopHttpClient};
use schema::TodoSchema;

pub struct Db {
    pub database: Database<TodoSchema>,
    pub client: LocalClient,
    pub http: Arc<dyn HttpClient>,
    pub config: IntegrationConfig,
}

impl Db {
    pub fn open(path: &Path, http: Arc<dyn HttpClient>, config: IntegrationConfig) -> Result<Self> {
        let mut client = LocalClient::try_new()
            .ok_or_else(|| anyhow::anyhow!("could not create LocalClient (already exists on this thread)"))?;
        let database = client
            .migrator(Config::open(path))
            .ok_or_else(|| anyhow::anyhow!("database version is older than supported"))?
            .finish()
            .ok_or_else(|| anyhow::anyhow!("database version is newer than supported"))?;
        Ok(Db {
            database,
            client,
            http,
            config,
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        let mut client = LocalClient::try_new()
            .ok_or_else(|| anyhow::anyhow!("could not create LocalClient (already exists on this thread)"))?;
        let database = client
            .migrator(Config::open_in_memory())
            .ok_or_else(|| anyhow::anyhow!("database version is older than supported"))?
            .finish()
            .ok_or_else(|| anyhow::anyhow!("database version is newer than supported"))?;
        Ok(Db {
            database,
            client,
            http: Arc::new(NoopHttpClient),
            config: IntegrationConfig::default(),
        })
    }
}

// Note: tests that need Db::open_in_memory() live in task.rs (single combined test)
// because rust-query 0.4 only allows one Config::open_in_memory() per process.
