use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use todomocop_core::http::{HttpClient, IntegrationConfig};
use todomocop_core::types::{AddTask, EditTask, TaskFilter, TaskStatus};
use todomocop_core::Db;

// ---------------------------------------------------------------------------
// UreqHttpClient
// ---------------------------------------------------------------------------

struct UreqHttpClient;

impl HttpClient for UreqHttpClient {
    fn get(&self, url: &str, headers: &[(&str, &str)]) -> anyhow::Result<Vec<u8>> {
        let mut req = ureq::get(url);
        for &(name, value) in headers {
            req = req.header(name, value);
        }
        let mut resp = req.call()?;
        let body = resp.body_mut().read_to_vec()?;
        Ok(body)
    }

    fn post(&self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut req = ureq::post(url);
        for &(name, value) in headers {
            req = req.header(name, value);
        }
        let mut resp = req.send(body)?;
        let resp_body = resp.body_mut().read_to_vec()?;
        Ok(resp_body)
    }
}

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "todomocop", about = "Personal TODO management")]
struct Cli {
    #[arg(long, env = "TODOMOCOP_DB")]
    db: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new task
    Add {
        /// Task title
        title: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        priority: Option<i64>,
        #[arg(long, default_value = "personal")]
        workspace: String,
        #[arg(long)]
        deadline: Option<String>,
        #[arg(long)]
        planned_date: Option<String>,
        #[arg(long)]
        status: Option<String>,
    },
    /// List tasks
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        include_snoozed: bool,
    },
    /// Edit an existing task
    Edit {
        /// Task ID
        id: i64,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        priority: Option<i64>,
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        deadline: Option<String>,
        #[arg(long)]
        planned_date: Option<String>,
    },
    /// Delete a task
    Delete {
        /// Task ID
        id: i64,
    },
    /// Search tasks by title and description
    Search {
        /// Search query
        query: String,
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Snooze a task until a given date
    Snooze {
        /// Task ID
        id: i64,
        /// Snooze until date (e.g. 2026-04-01)
        until: String,
    },
}

// ---------------------------------------------------------------------------
// Output helpers
// ---------------------------------------------------------------------------

fn format_task_line(task: &todomocop_core::types::Task) -> String {
    match task.priority {
        Some(p) => format!("#{} [{}] P{} {}", task.id, task.status, p, task.title),
        None => format!("#{} [{}] {}", task.id, task.status, task.title),
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();

    let db_path = cli.db.unwrap_or_else(|| {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("todomocop")
            .join("todomocop.db")
    });

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let github_token = std::env::var("GITHUB_TOKEN").ok();
    let linear_api_key = std::env::var("LINEAR_API_KEY").ok();

    let config = IntegrationConfig {
        github_token,
        linear_api_key,
        ..Default::default()
    };

    let http: Arc<dyn HttpClient> = Arc::new(UreqHttpClient);
    let mut db = Db::open(&db_path, http, config)?;

    match cli.command {
        Commands::Add {
            title,
            description,
            priority,
            workspace,
            deadline,
            planned_date,
            status,
        } => {
            let status = status.as_deref().map(|s| s.parse::<TaskStatus>()).transpose()?;
            let id = db.add_task(AddTask {
                title,
                description,
                status,
                priority,
                workspace,
                deadline,
                planned_date,
            })?;
            println!("Created task #{id}");
        }

        Commands::List {
            status,
            workspace,
            include_snoozed,
        } => {
            let status = status.as_deref().map(|s| s.parse::<TaskStatus>()).transpose()?;
            let filter = TaskFilter {
                status,
                workspace,
                include_snoozed,
                ..Default::default()
            };
            let tasks = db.list_tasks(filter)?;
            if tasks.is_empty() {
                println!("No tasks found.");
            } else {
                for task in &tasks {
                    println!("{}", format_task_line(task));
                }
            }
        }

        Commands::Edit {
            id,
            title,
            description,
            status,
            priority,
            workspace,
            deadline,
            planned_date,
        } => {
            let status = status.as_deref().map(|s| s.parse::<TaskStatus>()).transpose()?;
            db.edit_task(
                id,
                EditTask {
                    title,
                    description,
                    status,
                    priority: priority.map(Some),
                    workspace,
                    deadline: deadline.map(Some),
                    planned_date: planned_date.map(Some),
                    snooze_until: None,
                },
            )?;
            println!("Updated task #{id}");
        }

        Commands::Delete { id } => {
            db.delete_task(id)?;
            println!("Deleted task #{id}");
        }

        Commands::Search { query, workspace } => {
            let filter = TaskFilter {
                workspace,
                ..Default::default()
            };
            let tasks = db.search(&query, filter)?;
            if tasks.is_empty() {
                println!("No tasks found.");
            } else {
                for task in &tasks {
                    println!("{}", format_task_line(task));
                }
            }
        }

        Commands::Snooze { id, until } => {
            let date = until
                .parse::<chrono::NaiveDate>()
                .map_err(|e| anyhow::anyhow!("invalid date '{}': {e}", until))?;
            db.snooze(id, date)?;
            println!("Snoozed task #{id} until {until}");
        }
    }

    Ok(())
}
