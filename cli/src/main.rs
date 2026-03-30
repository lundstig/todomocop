use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use colored::Colorize;
use todomocop_core::http::{HttpClient, IntegrationConfig};
use todomocop_core::types::{AddTask, EditTask, Task, TaskFilter, TaskStatus};
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
    /// Disable colors and formatting (auto-detected when piped)
    #[arg(long, global = true)]
    plain: bool,
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
    /// Sync tasks from external sources
    Sync {
        #[command(subcommand)]
        source: Option<SyncSource>,
        /// Lookback window for recently closed PRs (default: 7d)
        #[arg(long, default_value = "7d")]
        since: String,
    },
}

#[derive(Subcommand)]
enum SyncSource {
    /// Sync GitHub PRs
    Github,
    /// Sync Linear issues
    Linear,
}

// ---------------------------------------------------------------------------
// Output helpers
// ---------------------------------------------------------------------------

fn parse_duration_days(s: &str) -> anyhow::Result<u64> {
    let s = s.trim();
    if let Some(days) = s.strip_suffix('d') {
        days.parse().map_err(|_| anyhow::anyhow!("invalid duration: {s}"))
    } else {
        s.parse().map_err(|_| anyhow::anyhow!("invalid duration: {s}, expected format like '7d'"))
    }
}

fn format_task_plain(task: &Task) -> String {
    match task.priority {
        Some(p) => format!("#{} [{}] P{} {}", task.id, task.status, p, task.title),
        None => format!("#{} [{}] {}", task.id, task.status, task.title),
    }
}

const STATUS_WIDTH: usize = "working".len(); // widest display label

fn status_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Idea => "idea",
        TaskStatus::Ready => "ready",
        TaskStatus::Working => "working",
        TaskStatus::Done => "done",
        TaskStatus::Canceled => "cancel",
    }
}

fn status_colored(status: TaskStatus) -> colored::ColoredString {
    let label = format!("{:<w$}", status_label(status), w = STATUS_WIDTH);
    match status {
        TaskStatus::Idea => label.dimmed(),
        TaskStatus::Ready => label.blue(),
        TaskStatus::Working => label.green(),
        TaskStatus::Done => label.bright_black(),
        TaskStatus::Canceled => label.bright_black().strikethrough(),
    }
}

fn priority_colored(priority: Option<i64>) -> colored::ColoredString {
    match priority {
        Some(0) => "P0".red().bold(),
        Some(1) => "P1".yellow(),
        Some(2) => "P2".cyan(),
        Some(p) => format!("P{p}").normal(),
        None => "──".dimmed(),
    }
}

fn truncate(s: &str, max_width: usize) -> String {
    if s.chars().count() <= max_width {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_width.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

fn format_relative_date(date: NaiveDate, today: NaiveDate) -> String {
    let diff = (date - today).num_days();
    match diff {
        0 => "today".to_string(),
        1 => "tomorrow".to_string(),
        -1 => "yesterday".to_string(),
        2..=6 => date.format("%a").to_string(),
        _ => date.format("%b %-d").to_string(),
    }
}

fn format_date_info(task: &Task, today: NaiveDate) -> String {
    let mut parts = Vec::new();

    if let Some(ref deadline) = task.deadline {
        if let Ok(date) = deadline.parse::<NaiveDate>() {
            let label = format_relative_date(date, today);
            if date < today {
                parts.push(format!("{}", format!("⚠ {label}").red()));
            } else if date == today {
                parts.push(format!("{}", format!("⚠ {label}").yellow()));
            } else {
                parts.push(format!("📅 {}", label.dimmed()));
            }
        }
    }

    if let Some(ref planned) = task.planned_date {
        if let Ok(date) = planned.parse::<NaiveDate>() {
            let label = format_relative_date(date, today);
            parts.push(format!("📋 {}", label.dimmed()));
        }
    }

    parts.join("  ")
}

fn print_tasks(tasks: &[Task], pretty: bool) {
    if tasks.is_empty() {
        if pretty {
            println!("{}", "No tasks found.".dimmed());
        } else {
            println!("No tasks found.");
        }
        return;
    }

    if !pretty {
        for task in tasks {
            println!("{}", format_task_plain(task));
        }
        return;
    }

    let today = chrono::Local::now().date_naive();
    let max_id_w = tasks
        .iter()
        .map(|t| format!("#{}", t.id).len())
        .max()
        .unwrap_or(2);
    let max_title_w = tasks
        .iter()
        .map(|t| t.title.chars().count())
        .max()
        .unwrap_or(10)
        .min(80);
    let max_ws_w = tasks
        .iter()
        .map(|t| t.workspace.len())
        .max()
        .unwrap_or(8);

    for task in tasks {
        let id = format!("{:>w$}", format!("#{}", task.id), w = max_id_w);
        let status = status_colored(task.status);
        let pri = priority_colored(task.priority);
        let title = truncate(&task.title, max_title_w);
        let title_padded = format!("{:<w$}", title, w = max_title_w);
        let ws = format!("{:<w$}", task.workspace, w = max_ws_w);
        let date = format_date_info(task, today);

        print!(" {}  {}  {}  {}  {}", id.bold(), status, pri, title_padded, ws.dimmed());
        if !date.is_empty() {
            print!("  {date}");
        }
        println!();
    }
}

fn print_sync_section(icon: &str, label: colored::ColoredString, items: &[String]) {
    if items.is_empty() {
        return;
    }
    println!("  {} {} ({})", icon, label, items.len());
    for item in items {
        println!("    {}", item.dimmed());
    }
}

fn print_sync_summary(summary: &todomocop_sync::runner::SyncSummary) {
    if summary.is_empty() && summary.errors.is_empty() {
        println!("{}", "Already up to date.".dimmed());
        return;
    }
    print_sync_section("+", "created".green(), &summary.created);
    print_sync_section("✓", "completed".blue(), &summary.completed);
    print_sync_section("✗", "canceled".yellow(), &summary.canceled);
    print_sync_section("~", "updated".cyan(), &summary.updated);
    print_sync_section("⇄", "linked".dimmed(), &summary.linked);
    if !summary.errors.is_empty() {
        print_sync_section("!", "errors".red(), &summary.errors);
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    let cli = Cli::parse();
    let pretty = !cli.plain && std::io::stdout().is_terminal();

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
    let db = Db::open(&db_path, Arc::clone(&http), config)?;

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
            print_tasks(&tasks, pretty);
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
            print_tasks(&tasks, pretty);
        }

        Commands::Snooze { id, until } => {
            let date = until
                .parse::<chrono::NaiveDate>()
                .map_err(|e| anyhow::anyhow!("invalid date '{}': {e}", until))?;
            db.snooze(id, date)?;
            println!("Snoozed task #{id} until {until}");
        }

        Commands::Sync { source, since } => {
            let since_days = parse_duration_days(&since)?;

            let do_github = source.is_none() || matches!(source, Some(SyncSource::Github));
            let do_linear = source.is_none() || matches!(source, Some(SyncSource::Linear));

            let spinner = |msg: &'static str| -> Option<indicatif::ProgressBar> {
                if !pretty {
                    return None;
                }
                let sp = indicatif::ProgressBar::new_spinner();
                sp.set_style(
                    indicatif::ProgressStyle::default_spinner()
                        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
                        .template("{spinner} {msg}")
                        .unwrap(),
                );
                sp.enable_steady_tick(std::time::Duration::from_millis(80));
                sp.set_message(msg);
                Some(sp)
            };

            let mut total_summary = todomocop_sync::runner::SyncSummary::default();

            if do_github {
                let sp = spinner("Syncing GitHub PRs…");
                let existing = db.list_tasks(TaskFilter::default())?;
                let token = std::env::var("GITHUB_TOKEN")
                    .map_err(|_| anyhow::anyhow!("GITHUB_TOKEN not set"))?;
                let github = todomocop_sync::github::GithubClient::new(http.as_ref(), token)?;
                let prs = github.fetch_prs(since_days)?;
                let actions = todomocop_sync::reconcile::reconcile_github(&prs, &existing, github.username());
                let summary = todomocop_sync::runner::apply_actions(&db, &actions);
                total_summary.merge(&summary);
                if let Some(sp) = sp {
                    sp.finish_and_clear();
                }
            }

            if do_linear {
                let sp = spinner("Syncing Linear issues…");
                // Re-read tasks after GitHub sync so dedup sees newly created tasks
                let existing = db.list_tasks(TaskFilter::default())?;
                let api_key = std::env::var("LINEAR_API_KEY")
                    .map_err(|_| anyhow::anyhow!("LINEAR_API_KEY not set"))?;
                let linear = todomocop_sync::linear::LinearClient::new(http.as_ref(), api_key);
                let issues = linear.fetch_assigned_issues()?;
                let actions = todomocop_sync::reconcile::reconcile_linear(&issues, &existing);
                let summary = todomocop_sync::runner::apply_actions(&db, &actions);
                total_summary.merge(&summary);
                if let Some(sp) = sp {
                    sp.finish_and_clear();
                }
            }

            if pretty {
                print_sync_summary(&total_summary);
            } else {
                println!("{total_summary}");
            }
        }
    }

    Ok(())
}
