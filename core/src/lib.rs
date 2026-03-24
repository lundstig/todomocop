pub mod http;
pub mod schema;
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
        let mut client = rust_query::LocalClient::try_new()
            .ok_or_else(|| anyhow::anyhow!("could not create LocalClient (already exists on this thread)"))?;
        let database = client
            .migrator(Config::open(path))
            .ok_or_else(|| anyhow::anyhow!("database version is older than supported"))?
            .finish()
            .ok_or_else(|| anyhow::anyhow!("database version is newer than supported"))?;
        Ok(Db {
            database,
            http,
            config,
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        let mut client = rust_query::LocalClient::try_new()
            .ok_or_else(|| anyhow::anyhow!("could not create LocalClient (already exists on this thread)"))?;
        let database = client
            .migrator(Config::open_in_memory())
            .ok_or_else(|| anyhow::anyhow!("database version is older than supported"))?
            .finish()
            .ok_or_else(|| anyhow::anyhow!("database version is newer than supported"))?;
        Ok(Db {
            database,
            http: Arc::new(NoopHttpClient),
            config: IntegrationConfig::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let db = Db::open_in_memory();
        assert!(db.is_ok());
    }
}
