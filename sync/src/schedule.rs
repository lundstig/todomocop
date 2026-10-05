use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SkipReason {
    Fresh,
    Running,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
pub struct ChangeCounts {
    pub created: usize,
    pub completed: usize,
    pub canceled: usize,
    pub updated: usize,
    pub linked: usize,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SyncState {
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub last_changes: ChangeCounts,
}

#[derive(Debug, Serialize)]
pub struct SyncStatus {
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub running: bool,
    pub last_changes: ChangeCounts,
}

impl SyncState {
    pub fn status(&self, running: bool) -> SyncStatus {
        SyncStatus {
            last_attempt_at: self.last_attempt_at,
            last_success_at: self.last_success_at,
            last_error: self.last_error.clone(),
            running,
            last_changes: self.last_changes,
        }
    }

    pub fn record_attempt(&mut self, now: DateTime<Utc>) {
        self.last_attempt_at = Some(now);
    }

    pub fn record_success(&mut self, now: DateTime<Utc>, changes: ChangeCounts) {
        self.last_success_at = Some(now);
        self.last_error = None;
        self.last_changes = changes;
    }

    pub fn record_failure(&mut self, error: String) {
        self.last_error = Some(error);
    }
}

pub fn should_skip(state: &SyncState, now: DateTime<Utc>, max_age: Option<Duration>) -> bool {
    let Some(max_age) = max_age else {
        return false;
    };

    if state
        .last_success_at
        .is_some_and(|success| now.signed_duration_since(success) < max_age)
    {
        return true;
    }

    let last_attempt_failed = state
        .last_attempt_at
        .is_some_and(|attempt| state.last_success_at.is_none_or(|success| attempt > success));

    last_attempt_failed
        && state
            .last_attempt_at
            .is_some_and(|attempt| now.signed_duration_since(attempt) < Duration::minutes(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-05T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn skips_recent_success_only_when_requested() {
        let state = SyncState {
            last_success_at: Some(now() - Duration::minutes(5) + Duration::seconds(1)),
            ..Default::default()
        };

        assert!(should_skip(&state, now(), Some(Duration::minutes(5))));
        assert!(!should_skip(&state, now(), None));
        assert!(!should_skip(&state, now(), Some(Duration::minutes(4))));
    }

    #[test]
    fn failed_attempt_has_one_minute_cooldown() {
        let state = SyncState {
            last_attempt_at: Some(now() - Duration::seconds(30)),
            last_success_at: Some(now() - Duration::hours(1)),
            last_error: Some("GitHub unavailable".to_string()),
            ..Default::default()
        };

        assert!(should_skip(&state, now(), Some(Duration::hours(1))));
        assert!(!should_skip(&state, now(), None));
    }

    #[test]
    fn retries_failed_attempt_after_one_minute() {
        let state = SyncState {
            last_attempt_at: Some(now() - Duration::seconds(61)),
            last_error: Some("Linear unavailable".to_string()),
            ..Default::default()
        };

        assert!(!should_skip(&state, now(), Some(Duration::hours(1))));
    }
}
