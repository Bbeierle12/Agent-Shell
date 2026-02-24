use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use cron::Schedule;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::config::{ScheduleConfig, ScheduleTaskType};
use crate::error::AgentError;

/// A task fired by the scheduler, sent to the main event loop for execution.
#[derive(Debug, Clone)]
pub enum ScheduledTask {
    /// Heartbeat check-in (loads skill, sends kickoff prompt).
    Heartbeat {
        schedule_name: String,
        workspace: String,
        skill: String,
    },
    /// Run a fixed prompt through the agent loop.
    Prompt {
        schedule_name: String,
        workspace: String,
        prompt: String,
    },
    /// Custom task type for future extensibility.
    Custom {
        schedule_name: String,
        workspace: String,
    },
}

/// Persistent state for a single schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleState {
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: DateTime<Utc>,
    pub run_count: u64,
    pub last_error: Option<String>,
}

/// The cron/heartbeat scheduler.
///
/// Manages a set of scheduled tasks, sleeping until each is due, then
/// sending fired tasks through an `mpsc` channel. State is persisted
/// to disk so missed schedules fire on restart.
pub struct Scheduler {
    schedules: Vec<ScheduleConfig>,
    parsed: Vec<Option<Schedule>>,
    state: HashMap<String, ScheduleState>,
    state_path: PathBuf,
}

impl Scheduler {
    /// Create a scheduler from config entries.
    ///
    /// Invalid cron expressions are logged as warnings and skipped.
    pub fn new(schedules: Vec<ScheduleConfig>, state_path: PathBuf) -> Self {
        let now = Utc::now();
        let mut parsed = Vec::new();
        let mut state = HashMap::new();

        for config in &schedules {
            match parse_cron_expr(&config.cron) {
                Ok(schedule) => {
                    let next_run = schedule
                        .upcoming(Utc)
                        .next()
                        .unwrap_or(now + chrono::Duration::hours(24));
                    state.insert(
                        config.name.clone(),
                        ScheduleState {
                            last_run: None,
                            next_run,
                            run_count: 0,
                            last_error: None,
                        },
                    );
                    parsed.push(Some(schedule));
                }
                Err(e) => {
                    warn!("Invalid cron expression for '{}': {}", config.name, e);
                    parsed.push(None);
                }
            }
        }

        Self {
            schedules,
            parsed,
            state,
            state_path,
        }
    }

    /// Load persisted state from disk, merging with current state.
    ///
    /// Restores `last_run`, `run_count`, and `last_error` for schedules that
    /// still exist in the config. Past `next_run` values are preserved so
    /// missed schedules fire immediately on the next tick.
    pub fn load_state(&mut self) -> Result<(), AgentError> {
        if !self.state_path.exists() {
            return Ok(());
        }
        let contents = std::fs::read_to_string(&self.state_path)?;
        let loaded: HashMap<String, ScheduleState> = serde_json::from_str(&contents)?;

        for (name, loaded_state) in loaded {
            if let Some(current) = self.state.get_mut(&name) {
                current.last_run = loaded_state.last_run;
                current.run_count = loaded_state.run_count;
                current.last_error = loaded_state.last_error;
                // Keep past next_run so missed schedules fire immediately.
                current.next_run = loaded_state.next_run;
            }
        }
        Ok(())
    }

    /// Save current state to disk.
    pub fn save_state(&self) -> Result<(), AgentError> {
        if let Some(parent) = self.state_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = serde_json::to_string_pretty(&self.state)?;
        std::fs::write(&self.state_path, contents)?;
        Ok(())
    }

    /// Run the scheduler loop, sending fired tasks through `tx`.
    ///
    /// Blocks until the channel is closed (receiver dropped). Persists
    /// state after each batch of fired tasks.
    pub async fn run(mut self, tx: mpsc::UnboundedSender<ScheduledTask>) {
        let enabled_count = self
            .schedules
            .iter()
            .filter(|s| s.enabled)
            .zip(self.parsed.iter())
            .filter(|(_, p)| p.is_some())
            .count();
        info!(
            "Scheduler started with {} active schedule(s)",
            enabled_count
        );

        if let Err(e) = self.load_state() {
            warn!("Failed to load scheduler state: {}", e);
        }

        loop {
            let tasks = self.tick();
            let fired = !tasks.is_empty();

            for task in tasks {
                if tx.send(task).is_err() {
                    debug!("Scheduler channel closed, shutting down");
                    return;
                }
            }

            if fired {
                if let Err(e) = self.save_state() {
                    warn!("Failed to save scheduler state: {}", e);
                }
            }

            let sleep_duration = self.time_until_next_fire();
            debug!("Scheduler sleeping for {:?}", sleep_duration);
            tokio::time::sleep(sleep_duration).await;
        }
    }

    /// Check all schedules and fire any that are due.
    pub fn tick(&mut self) -> Vec<ScheduledTask> {
        let now = Utc::now();
        let mut tasks = Vec::new();

        for (i, config) in self.schedules.iter().enumerate() {
            if !config.enabled {
                continue;
            }

            let parsed = match &self.parsed[i] {
                Some(p) => p,
                None => continue,
            };

            let state = match self.state.get_mut(&config.name) {
                Some(s) => s,
                None => continue,
            };

            if now >= state.next_run {
                debug!("Firing schedule: {}", config.name);

                let workspace = config
                    .workspace
                    .clone()
                    .unwrap_or_else(|| "default".to_string());

                let task = match config.task {
                    ScheduleTaskType::Heartbeat => ScheduledTask::Heartbeat {
                        schedule_name: config.name.clone(),
                        workspace,
                        skill: config.skill.clone().unwrap_or_default(),
                    },
                    ScheduleTaskType::Prompt => ScheduledTask::Prompt {
                        schedule_name: config.name.clone(),
                        workspace,
                        prompt: config
                            .prompt
                            .clone()
                            .unwrap_or_else(|| "Perform your scheduled task.".to_string()),
                    },
                    ScheduleTaskType::Custom => ScheduledTask::Custom {
                        schedule_name: config.name.clone(),
                        workspace,
                    },
                };

                tasks.push(task);

                state.last_run = Some(now);
                state.run_count += 1;
                state.last_error = None;

                // Advance to the next fire time.
                if let Some(next) = parsed.upcoming(Utc).next() {
                    state.next_run = next;
                }
            }
        }

        tasks
    }

    /// Calculate the duration until the next schedule should fire.
    pub fn time_until_next_fire(&self) -> std::time::Duration {
        let now = Utc::now();
        let mut earliest: Option<DateTime<Utc>> = None;

        for (i, config) in self.schedules.iter().enumerate() {
            if !config.enabled || self.parsed[i].is_none() {
                continue;
            }

            if let Some(state) = self.state.get(&config.name) {
                match earliest {
                    None => earliest = Some(state.next_run),
                    Some(e) if state.next_run < e => earliest = Some(state.next_run),
                    _ => {}
                }
            }
        }

        match earliest {
            Some(next) if next > now => (next - now)
                .to_std()
                .unwrap_or(std::time::Duration::from_secs(60)),
            Some(_) => std::time::Duration::from_millis(0),
            None => std::time::Duration::from_secs(3600),
        }
    }

    /// Get a reference to the internal state (for testing/observability).
    pub fn state(&self) -> &HashMap<String, ScheduleState> {
        &self.state
    }
}

/// Parse a cron expression, normalizing 5-field standard cron to 7-field format.
///
/// The `cron` crate expects 7 fields: `sec min hour dom month dow year`.
/// Standard cron uses 5 fields: `min hour dom month dow`.
/// This function prepends `sec=0` and appends `year=*` as needed.
pub fn parse_cron_expr(expr: &str) -> Result<Schedule, AgentError> {
    let normalized = normalize_cron_fields(expr);
    Schedule::from_str(&normalized)
        .map_err(|e| AgentError::Config(format!("Invalid cron expression '{}': {}", expr, e)))
}

fn normalize_cron_fields(expr: &str) -> String {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    match fields.len() {
        5 => format!("0 {} *", expr),
        6 => format!("0 {}", expr),
        7 => expr.to_string(),
        _ => expr.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ScheduleTaskType;

    fn make_schedule_config(
        name: &str,
        cron: &str,
        task: ScheduleTaskType,
        enabled: bool,
    ) -> ScheduleConfig {
        ScheduleConfig {
            name: name.to_string(),
            cron: cron.to_string(),
            workspace: Some("default".to_string()),
            task,
            skill: if task == ScheduleTaskType::Heartbeat {
                Some("test-skill".to_string())
            } else {
                None
            },
            prompt: if task == ScheduleTaskType::Prompt {
                Some("Test prompt.".to_string())
            } else {
                None
            },
            enabled,
        }
    }

    #[test]
    fn test_cron_parsing_every_30_min() {
        let schedule = parse_cron_expr("*/30 * * * *").unwrap();
        let next = schedule.upcoming(Utc).next().unwrap();
        let now = Utc::now();
        // Next fire should be within 30 minutes.
        let diff = next - now;
        assert!(diff.num_minutes() <= 30);
        assert!(diff.num_seconds() >= 0);
    }

    #[test]
    fn test_cron_parsing_every_4_hours() {
        let schedule = parse_cron_expr("0 */4 * * *").unwrap();
        let next = schedule.upcoming(Utc).next().unwrap();
        let now = Utc::now();
        let diff = next - now;
        assert!(diff.num_hours() <= 4);
    }

    #[test]
    fn test_cron_parsing_7_field_passthrough() {
        // 7-field expression should be passed through unchanged.
        let schedule = parse_cron_expr("0 0 */2 * * * *").unwrap();
        let next = schedule.upcoming(Utc).next().unwrap();
        assert!(next > Utc::now());
    }

    #[test]
    fn test_cron_parsing_invalid_expression() {
        let result = parse_cron_expr("not a cron");
        assert!(result.is_err());
    }

    #[test]
    fn test_schedule_fires_at_correct_time() {
        let configs = vec![make_schedule_config(
            "test",
            "*/30 * * * *",
            ScheduleTaskType::Prompt,
            true,
        )];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut scheduler = Scheduler::new(configs, tmp.path().to_path_buf());

        // Force next_run to the past so tick fires it.
        scheduler.state.get_mut("test").unwrap().next_run =
            Utc::now() - chrono::Duration::seconds(1);

        let tasks = scheduler.tick();
        assert_eq!(tasks.len(), 1);
        assert!(
            matches!(&tasks[0], ScheduledTask::Prompt { schedule_name, .. } if schedule_name == "test")
        );

        // After firing, next_run should be in the future.
        let state = scheduler.state.get("test").unwrap();
        assert!(state.next_run > Utc::now());
        assert_eq!(state.run_count, 1);
        assert!(state.last_run.is_some());
    }

    #[test]
    fn test_missed_schedule_fires_immediately() {
        let configs = vec![make_schedule_config(
            "missed",
            "*/30 * * * *",
            ScheduleTaskType::Heartbeat,
            true,
        )];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut scheduler = Scheduler::new(configs, tmp.path().to_path_buf());

        // Set next_run to 2 hours ago (simulating missed schedule).
        scheduler.state.get_mut("missed").unwrap().next_run =
            Utc::now() - chrono::Duration::hours(2);

        let tasks = scheduler.tick();
        assert_eq!(tasks.len(), 1);
        assert!(
            matches!(&tasks[0], ScheduledTask::Heartbeat { skill, .. } if skill == "test-skill")
        );

        // time_until_next_fire should return a positive duration (future).
        let wait = scheduler.time_until_next_fire();
        assert!(wait.as_secs() > 0);
    }

    #[test]
    fn test_disabled_schedule_skipped() {
        let configs = vec![make_schedule_config(
            "disabled",
            "*/5 * * * *",
            ScheduleTaskType::Prompt,
            false,
        )];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut scheduler = Scheduler::new(configs, tmp.path().to_path_buf());

        // Force next_run to the past.
        scheduler.state.get_mut("disabled").unwrap().next_run =
            Utc::now() - chrono::Duration::seconds(1);

        let tasks = scheduler.tick();
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_error_does_not_block_other_schedules() {
        let configs = vec![
            ScheduleConfig {
                name: "bad".to_string(),
                cron: "not valid cron".to_string(),
                workspace: None,
                task: ScheduleTaskType::Prompt,
                skill: None,
                prompt: None,
                enabled: true,
            },
            make_schedule_config("good", "*/10 * * * *", ScheduleTaskType::Prompt, true),
        ];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut scheduler = Scheduler::new(configs, tmp.path().to_path_buf());

        // "bad" has no parsed schedule, "good" does.
        assert!(scheduler.parsed[0].is_none());
        assert!(scheduler.parsed[1].is_some());

        // Force "good" to fire.
        scheduler.state.get_mut("good").unwrap().next_run =
            Utc::now() - chrono::Duration::seconds(1);

        let tasks = scheduler.tick();
        assert_eq!(tasks.len(), 1);
        assert!(
            matches!(&tasks[0], ScheduledTask::Prompt { schedule_name, .. } if schedule_name == "good")
        );
    }

    #[test]
    fn test_state_persists_across_restart() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let state_path = tmp.path().to_path_buf();
        let configs = vec![make_schedule_config(
            "persist",
            "*/30 * * * *",
            ScheduleTaskType::Prompt,
            true,
        )];

        // First scheduler: fire a task and save state.
        {
            let mut scheduler = Scheduler::new(configs.clone(), state_path.clone());
            scheduler.state.get_mut("persist").unwrap().next_run =
                Utc::now() - chrono::Duration::seconds(1);
            let tasks = scheduler.tick();
            assert_eq!(tasks.len(), 1);
            assert_eq!(scheduler.state.get("persist").unwrap().run_count, 1);
            scheduler.save_state().unwrap();
        }

        // Second scheduler: load state and verify persistence.
        {
            let mut scheduler = Scheduler::new(configs, state_path);
            scheduler.load_state().unwrap();

            let state = scheduler.state.get("persist").unwrap();
            assert_eq!(state.run_count, 1);
            assert!(state.last_run.is_some());
            // next_run should be in the future (set by first scheduler's tick).
            assert!(state.next_run > Utc::now());
        }
    }

    #[test]
    fn test_time_until_next_fire_no_schedules() {
        let scheduler = Scheduler::new(Vec::new(), PathBuf::from("/tmp/empty"));
        let wait = scheduler.time_until_next_fire();
        // No schedules — defaults to 1 hour.
        assert_eq!(wait.as_secs(), 3600);
    }

    #[test]
    fn test_time_until_next_fire_past_due() {
        let configs = vec![make_schedule_config(
            "past",
            "*/30 * * * *",
            ScheduleTaskType::Prompt,
            true,
        )];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut scheduler = Scheduler::new(configs, tmp.path().to_path_buf());
        scheduler.state.get_mut("past").unwrap().next_run =
            Utc::now() - chrono::Duration::seconds(10);

        let wait = scheduler.time_until_next_fire();
        assert_eq!(wait.as_millis(), 0);
    }

    #[test]
    fn test_heartbeat_task_carries_skill() {
        let configs = vec![make_schedule_config(
            "hb",
            "*/30 * * * *",
            ScheduleTaskType::Heartbeat,
            true,
        )];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let mut scheduler = Scheduler::new(configs, tmp.path().to_path_buf());
        scheduler.state.get_mut("hb").unwrap().next_run = Utc::now() - chrono::Duration::seconds(1);

        let tasks = scheduler.tick();
        assert_eq!(tasks.len(), 1);
        match &tasks[0] {
            ScheduledTask::Heartbeat {
                schedule_name,
                workspace,
                skill,
            } => {
                assert_eq!(schedule_name, "hb");
                assert_eq!(workspace, "default");
                assert_eq!(skill, "test-skill");
            }
            _ => panic!("Expected Heartbeat task"),
        }
    }
}
