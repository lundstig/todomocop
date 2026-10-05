use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use clap::{Args, Parser, Subcommand};
use colored::Colorize;
use fs2::FileExt;
use todomocop_core::http::{HttpClient, IntegrationConfig};
use todomocop_core::types::{AddTask, EditTask, Task, TaskFilter, TaskStatus};
use todomocop_core::Db;
use todomocop_sync::runner::SyncSummary;
use todomocop_sync::schedule::{should_skip, SkipReason, SyncState, SyncStatus};

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
    /// Print machine-readable JSON
    #[arg(long, global = true)]
    json: bool,
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
        #[arg(long)]
        next_action: Option<String>,
        #[arg(long = "tag")]
        tags: Vec<String>,
    },
    /// List tasks
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        include_snoozed: bool,
        #[arg(long)]
        tag: Option<String>,
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
        #[arg(long)]
        next_action: Option<String>,
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
        #[arg(long)]
        tag: Option<String>,
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
        target: Option<SyncTarget>,
        #[command(flatten)]
        options: SyncOptions,
    },
    /// Tag a task
    Tag {
        /// Task ID
        id: i64,
        /// Tag name
        tag: String,
    },
    /// Remove a tag from a task
    Untag {
        /// Task ID
        id: i64,
        /// Tag name
        tag: String,
    },
    /// List all tags
    Tags {
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Edit a tag (rename or set description)
    TagEdit {
        /// Current tag name
        tag: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
    /// Delete a tag
    TagDelete {
        /// Tag name
        tag: String,
    },
}

#[derive(Subcommand)]
enum SyncTarget {
    /// Sync GitHub PRs
    Github {
        #[command(flatten)]
        options: SyncOptions,
    },
    /// Sync Linear issues
    Linear {
        #[command(flatten)]
        options: SyncOptions,
    },
    /// Show persisted sync status
    Status,
}

#[derive(Clone, Copy)]
enum SyncSource {
    Github,
    Linear,
}

#[derive(Args, Default)]
struct SyncOptions {
    /// Lookback window for recently closed PRs (default: 7d)
    #[arg(long, default_value = "7d")]
    since: String,
    /// Skip when the most recent successful sync is newer than this duration
    #[arg(long)]
    if_stale: Option<String>,
    /// Preview changes without writing tasks or sync status
    #[arg(long)]
    dry_run: bool,
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

fn parse_duration(s: &str) -> anyhow::Result<Duration> {
    let value = s.trim();
    let (number, unit) = value.split_at(value.len().saturating_sub(1));
    let number: i64 = number
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid duration: {value}"))?;
    if number < 0 {
        anyhow::bail!("invalid duration: {value}");
    }
    match unit {
        "m" => Duration::try_minutes(number),
        "h" => Duration::try_hours(number),
        "d" => Duration::try_days(number),
        _ => None,
    }
    .ok_or_else(|| anyhow::anyhow!("invalid duration: {value}, expected format like '5m' or '1h'"))
}

fn format_task_plain(task: &Task) -> String {
    let mut parts = vec![format!("#{}", task.id), format!("[{}]", task.status)];
    if let Some(p) = task.priority {
        parts.push(format!("P{p}"));
    }
    if let Some(ref na) = task.next_action {
        parts.push(format!("[{na}]"));
    }
    parts.push(task.title.clone());
    if !task.tags.is_empty() {
        let tag_str: Vec<&str> = task.tags.iter().map(|t| t.name.as_str()).collect();
        parts.push(format!("[{}]", tag_str.join(", ")));
    }
    parts.join(" ")
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
    let max_na_w = tasks
        .iter()
        .map(|t| t.next_action.as_ref().map_or(0, |na| na.chars().count().min(40)))
        .max()
        .unwrap_or(0);
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

        print!(" {}  {}  {}", id.bold(), status, pri);
        if max_na_w > 0 {
            let na = task.next_action.as_deref().unwrap_or("");
            let na = truncate(na, 40);
            print!("  {:<w$}", na.dimmed(), w = max_na_w);
        }
        print!("  {}  {}", title_padded, ws.dimmed());
        if !date.is_empty() {
            print!("  {date}");
        }
        if !task.tags.is_empty() {
            let tag_str: Vec<&str> = task.tags.iter().map(|t| t.name.as_str()).collect();
            print!("  {}", format!("[{}]", tag_str.join(", ")).cyan());
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
    if let Some(reason) = summary.reason {
        let message = match reason {
            SkipReason::Fresh => "Sync skipped: recent successful sync.",
            SkipReason::Running => "Sync skipped: another sync is running.",
        };
        println!("{}", message.dimmed());
        return;
    }
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

struct SyncLock {
    _file: File,
}

impl SyncLock {
    fn try_acquire(path: &Path) -> Result<Option<Self>> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("open sync lock {}", path.display()))?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error).with_context(|| format!("lock sync file {}", path.display())),
        }
    }
}

fn sync_state_path(db_path: &Path) -> PathBuf {
    db_path.with_extension("sync-status.json")
}

fn sync_lock_path(db_path: &Path) -> PathBuf {
    db_path.with_extension("sync.lock")
}

fn load_sync_state(path: &Path) -> Result<SyncState> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("parse sync status {}", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(SyncState::default()),
        Err(error) => Err(error).with_context(|| format!("read sync status {}", path.display())),
    }
}

fn save_sync_state(path: &Path, state: &SyncState) -> Result<()> {
    let temporary = path.with_extension("sync-status.json.tmp");
    let bytes = serde_json::to_vec_pretty(state)?;
    let mut file = File::create(&temporary)
        .with_context(|| format!("create sync status {}", temporary.display()))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    std::fs::rename(&temporary, path)
        .with_context(|| format!("replace sync status {}", path.display()))?;
    Ok(())
}

fn print_sync_result(summary: &SyncSummary, json: bool, pretty: bool) -> Result<()> {
    if json {
        print_json(summary)
    } else if pretty {
        print_sync_summary(summary);
        Ok(())
    } else {
        println!("{summary}");
        Ok(())
    }
}

fn format_age(timestamp: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = now.signed_duration_since(timestamp).num_seconds().max(0);
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m", seconds / 60),
        3600..=86399 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86400),
    }
}

fn print_sync_status(status: &SyncStatus, json: bool) -> Result<()> {
    if json {
        return print_json(status);
    }
    if status.running {
        println!("sync running");
    } else if let (Some(error), Some(attempt)) = (&status.last_error, &status.last_attempt_at) {
        let age = format_age(*attempt, Utc::now());
        println!("last sync failed {age} ago: {error}");
    } else if let Some(success) = &status.last_success_at {
        let age = format_age(*success, Utc::now());
        let closed = status.last_changes.completed + status.last_changes.canceled;
        println!(
            "synced {age} ago (closed {closed}, created {})",
            status.last_changes.created
        );
    } else {
        println!("never synced");
    }
    Ok(())
}

fn print_persisted_sync_status(db_path: &Path, json: bool) -> Result<()> {
    let state = load_sync_state(&sync_state_path(db_path))?;
    let running = SyncLock::try_acquire(&sync_lock_path(db_path))?.is_none();
    print_sync_status(&state.status(running), json)
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn print_task_json(db: &Db, id: i64) -> Result<()> {
    let task = db
        .get_task(id)?
        .ok_or_else(|| anyhow::anyhow!("task not found: {id}"))?;
    print_json(&task)
}

fn run_github_sync(
    db: &Db,
    http: &dyn HttpClient,
    token: Option<String>,
    since_days: u64,
    dry_run: bool,
) -> Result<SyncSummary> {
    let existing = db.list_tasks(TaskFilter {
        include_snoozed: true,
        ..Default::default()
    })?;
    let Some(token) = token else {
        let mut summary = SyncSummary::default();
        summary.errors.push("GitHub: GITHUB_TOKEN not set".to_string());
        return Ok(summary);
    };
    let github = match todomocop_sync::github::GithubClient::new(http, token) {
        Ok(github) => github,
        Err(error) => {
            let mut summary = SyncSummary::default();
            summary.errors.push(format!("GitHub: authenticate: {error:#}"));
            return Ok(summary);
        }
    };
    let discovered = match github.fetch_prs(since_days) {
        Ok(prs) => prs,
        Err(error) => {
            let mut summary = SyncSummary::default();
            summary.errors.push(format!("GitHub: fetch pull requests: {error:#}"));
            return Ok(summary);
        }
    };
    let linked = match github.fetch_linked_prs(&existing) {
        Ok(prs) => prs,
        Err(error) => {
            let mut summary = SyncSummary::default();
            summary.errors.push(format!("GitHub: refresh linked pull requests: {error:#}"));
            return Ok(summary);
        }
    };
    let prs = todomocop_sync::github::merge_prs(discovered, linked);
    let mut actions = todomocop_sync::reconcile::reconcile_github_contexts(&prs, &existing);
    actions.extend(todomocop_sync::reconcile::reconcile_github_tags(
        &prs,
        &existing,
        github.username(),
    ));
    actions.extend(todomocop_sync::reconcile::reconcile_github(
        &prs,
        &existing,
        github.username(),
    ));
    Ok(if dry_run {
        todomocop_sync::runner::preview_actions(&existing, &actions)
    } else {
        todomocop_sync::runner::apply_actions(db, &actions)
    })
}

fn run_linear_sync(
    db: &Db,
    http: &dyn HttpClient,
    api_key: Option<String>,
    dry_run: bool,
) -> Result<SyncSummary> {
    let existing = db.list_tasks(TaskFilter {
        include_snoozed: true,
        ..Default::default()
    })?;
    let Some(api_key) = api_key else {
        let mut summary = SyncSummary::default();
        summary.errors.push("Linear: LINEAR_API_KEY not set".to_string());
        return Ok(summary);
    };
    let linear = todomocop_sync::linear::LinearClient::new(http, api_key);
    let issues = match linear.fetch_assigned_issues() {
        Ok(issues) => issues,
        Err(error) => {
            let mut summary = SyncSummary::default();
            summary.errors.push(format!("Linear: fetch assigned issues: {error:#}"));
            return Ok(summary);
        }
    };
    let mut actions = todomocop_sync::reconcile::reconcile_linear_tags(&existing);
    actions.extend(todomocop_sync::reconcile::reconcile_linear(&issues, &existing));
    Ok(if dry_run {
        todomocop_sync::runner::preview_actions(&existing, &actions)
    } else {
        todomocop_sync::runner::apply_actions(db, &actions)
    })
}

fn run_sync(
    db_path: &Path,
    source: Option<SyncSource>,
    since: &str,
    if_stale: Option<&str>,
    dry_run: bool,
    json: bool,
    pretty: bool,
) -> Result<()> {
    let since_days = parse_duration_days(since)?;
    let max_age = if_stale.map(parse_duration).transpose()?;
    let Some(_lock) = SyncLock::try_acquire(&sync_lock_path(db_path))? else {
        return print_sync_result(
            &SyncSummary::skipped(SkipReason::Running),
            json,
            pretty,
        );
    };

    let state_path = sync_state_path(db_path);
    let mut state = load_sync_state(&state_path)?;
    if should_skip(&state, Utc::now(), max_age) {
        return print_sync_result(&SyncSummary::skipped(SkipReason::Fresh), json, pretty);
    }

    if !dry_run {
        state.record_attempt(Utc::now());
        save_sync_state(&state_path, &state)?;
    }

    let github_token = std::env::var("GITHUB_TOKEN").ok();
    let linear_api_key = std::env::var("LINEAR_API_KEY").ok();
    let config = IntegrationConfig {
        github_token: github_token.clone(),
        linear_api_key: linear_api_key.clone(),
        ..Default::default()
    };
    let http: Arc<dyn HttpClient> = Arc::new(UreqHttpClient);
    let result = (|| -> Result<SyncSummary> {
        let db = Db::open(db_path, Arc::clone(&http), config)?;
        let do_github = source.is_none() || matches!(source, Some(SyncSource::Github));
        let do_linear = source.is_none() || matches!(source, Some(SyncSource::Linear));
        let mut summary = SyncSummary::default();
        if do_github {
            summary.merge(&run_github_sync(
                &db,
                http.as_ref(),
                github_token,
                since_days,
                dry_run,
            )?);
        }
        if do_linear {
            summary.merge(&run_linear_sync(
                &db,
                http.as_ref(),
                linear_api_key,
                dry_run,
            )?);
        }
        Ok(summary)
    })();

    let summary = match result {
        Ok(summary) => summary,
        Err(error) => {
            if !dry_run {
                state.record_failure(format!("{error:#}"));
                save_sync_state(&state_path, &state)?;
            }
            return Err(error);
        }
    };

    if !dry_run {
        if summary.errors.is_empty() {
            state.record_success(Utc::now(), summary.counts());
        } else {
            state.record_failure(summary.errors.join("; "));
        }
        save_sync_state(&state_path, &state)?;
    }

    print_sync_result(&summary, json, pretty)
}

fn main() {
    let cli = Cli::parse();
    let json = cli.json;
    if let Err(err) = run(cli) {
        if json {
            println!("{}", serde_json::json!({ "error": format!("{err:#}") }));
        } else {
            eprintln!("Error: {err:#}");
        }
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let json = cli.json;
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

    if let Commands::Sync { target, options } = &cli.command {
        let (source, options) = match target {
            None => (None, options),
            Some(SyncTarget::Github { options }) => (Some(SyncSource::Github), options),
            Some(SyncTarget::Linear { options }) => (Some(SyncSource::Linear), options),
            Some(SyncTarget::Status) => return print_persisted_sync_status(&db_path, json),
        };
        return run_sync(
            &db_path,
            source,
            &options.since,
            options.if_stale.as_deref(),
            options.dry_run,
            json,
            pretty,
        );
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
            next_action,
            tags,
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
                next_action,
                tags,
            })?;
            if json {
                print_task_json(&db, id)?;
            } else {
                println!("Created task #{id}");
            }
        }

        Commands::List {
            status,
            workspace,
            include_snoozed,
            tag,
        } => {
            let status = status.as_deref().map(|s| s.parse::<TaskStatus>()).transpose()?;
            let filter = TaskFilter {
                status,
                workspace,
                include_snoozed,
                tag,
                ..Default::default()
            };
            let tasks = db.list_tasks(filter)?;
            if json {
                print_json(&tasks)?;
            } else {
                print_tasks(&tasks, pretty);
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
            next_action,
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
                    next_action: next_action.map(Some),
                },
            )?;
            if json {
                print_task_json(&db, id)?;
            } else {
                println!("Updated task #{id}");
            }
        }

        Commands::Delete { id } => {
            db.delete_task(id)?;
            if json {
                print_json(&serde_json::json!({ "id": id, "deleted": true }))?;
            } else {
                println!("Deleted task #{id}");
            }
        }

        Commands::Search { query, workspace, tag } => {
            let filter = TaskFilter {
                workspace,
                tag,
                ..Default::default()
            };
            let tasks = db.search(&query, filter)?;
            if json {
                print_json(&tasks)?;
            } else {
                print_tasks(&tasks, pretty);
            }
        }

        Commands::Snooze { id, until } => {
            let date = until
                .parse::<chrono::NaiveDate>()
                .map_err(|e| anyhow::anyhow!("invalid date '{}': {e}", until))?;
            db.snooze(id, date)?;
            if json {
                print_task_json(&db, id)?;
            } else {
                println!("Snoozed task #{id} until {until}");
            }
        }

        Commands::Sync { .. } => unreachable!("sync handled before opening the database"),

        Commands::Tag { id, tag } => {
            db.tag_task(id, &tag)?;
            if json {
                print_task_json(&db, id)?;
            } else {
                println!("Tagged task #{id} with '{tag}'");
            }
        }

        Commands::Untag { id, tag } => {
            db.untag_task(id, &tag)?;
            if json {
                print_task_json(&db, id)?;
            } else {
                println!("Removed tag '{tag}' from task #{id}");
            }
        }

        Commands::Tags { workspace } => {
            let tags = db.list_tags(workspace.as_deref())?;
            if json {
                print_json(&tags)?;
            } else if tags.is_empty() {
                if pretty {
                    println!("{}", "No tags found.".dimmed());
                } else {
                    println!("No tags found.");
                }
            } else {
                for tag in &tags {
                    if pretty {
                        let desc = if tag.description.is_empty() {
                            String::new()
                        } else {
                            format!(" {}", format!("— {}", tag.description).dimmed())
                        };
                        println!(
                            "  {}  {}{}",
                            tag.name.bold(),
                            format!("({} tasks)", tag.task_count).dimmed(),
                            desc
                        );
                    } else {
                        let desc = if tag.description.is_empty() {
                            String::new()
                        } else {
                            format!(" — {}", tag.description)
                        };
                        println!("{} ({} tasks){desc}", tag.name, tag.task_count);
                    }
                }
            }
        }

        Commands::TagEdit { tag, name, description } => {
            db.edit_tag(&tag, name.as_deref(), description.as_deref())?;
            if json {
                let name = name.as_deref().unwrap_or(&tag);
                let info = db
                    .list_tags(None)?
                    .into_iter()
                    .find(|t| t.name == name)
                    .ok_or_else(|| anyhow::anyhow!("tag not found: {name}"))?;
                print_json(&info)?;
            } else {
                println!("Updated tag '{tag}'");
            }
        }

        Commands::TagDelete { tag } => {
            db.delete_tag(&tag)?;
            if json {
                print_json(&serde_json::json!({ "tag": tag, "deleted": true }))?;
            } else {
                println!("Deleted tag '{tag}'");
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_lock_allows_only_one_holder() {
        let path = std::env::temp_dir().join(format!(
            "todomocop-sync-lock-test-{}",
            std::process::id()
        ));
        let first = SyncLock::try_acquire(&path).unwrap().unwrap();
        assert!(SyncLock::try_acquire(&path).unwrap().is_none());
        drop(first);
        assert!(SyncLock::try_acquire(&path).unwrap().is_some());
        std::fs::remove_file(path).unwrap();
    }
}
