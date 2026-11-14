//! Host-saved engineering reports and their seven-day automation preset.
use crate::{ids::ThreadId, EpochMs};
use serde::{Deserialize, Serialize};

/// This preset reports and repeats over exactly seven days, independent of the
/// browser's timezone. Calendar weekday scheduling is a separate capability.
pub const WEEK_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// These identities are edited through the report preset, preserving its
/// seven-day cadence and source/model configuration as one owned setting.
pub fn managed_preset_id(id: &str) -> Option<&str> {
    [
        "weekly-report-clock-",
        "weekly-report-subscription-",
        "weekly-report-",
    ]
    .into_iter()
    .filter_map(|prefix| id.strip_prefix(prefix))
    .find(|suffix| uuid::Uuid::parse_str(suffix).is_ok())
}

pub fn editable_automation_snapshot(
    mut snapshot: crate::reactive::ReactiveSnapshot,
) -> crate::reactive::ReactiveSnapshot {
    snapshot
        .agents
        .retain(|agent| managed_preset_id(&agent.id).is_none());
    snapshot
        .sources
        .retain(|source| managed_preset_id(&source.id).is_none());
    snapshot
        .subscriptions
        .retain(|subscription| managed_preset_id(&subscription.id).is_none());
    snapshot
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeeklyReportPreset {
    pub id: String,
    pub project_path: String,
    pub model: String,
    pub local_git: bool,
    pub github_pull_requests: bool,
    #[serde(default)]
    pub github_deployments: bool,
    pub scheduled: bool,
}

impl WeeklyReportPreset {
    pub fn agent_id(&self) -> String {
        format!("weekly-report-{}", self.id)
    }
    pub fn source_id(&self) -> String {
        format!("weekly-report-clock-{}", self.id)
    }
    pub fn subscription_id(&self) -> String {
        format!("weekly-report-subscription-{}", self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportFact {
    pub title: String,
    pub reference: String,
    pub url: Option<String>,
    pub occurred_at: EpochMs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ReportCoverage {
    Available {
        source: String,
        facts: Vec<ReportFact>,
        truncated: bool,
    },
    Unavailable {
        source: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineeringReport {
    pub id: String,
    pub preset_id: String,
    pub run_id: String,
    pub thread_id: ThreadId,
    pub project_path: String,
    pub model: String,
    pub window_start: EpochMs,
    pub window_end: EpochMs,
    pub title: String,
    pub body: String,
    pub coverage: Vec<ReportCoverage>,
    pub created_at: EpochMs,
    pub updated_at: EpochMs,
    pub revision: u64,
}

/// List metadata does not include all report bodies or collected source data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineeringReportRef {
    pub id: String,
    pub preset_id: String,
    pub title: String,
    pub project_path: String,
    pub window_end: EpochMs,
    pub revision: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeeklyReportsSnapshot {
    pub presets: Vec<WeeklyReportPreset>,
    pub reports: Vec<EngineeringReportRef>,
    /// Resume the list with this cursor to reach older saved reports.
    pub next_before: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum WeeklyReportCommand {
    SavePreset {
        preset: WeeklyReportPreset,
    },
    RunNow {
        preset_id: String,
        request_id: String,
        #[serde(default)]
        window_end_utc: Option<String>,
    },
    EditReport {
        id: String,
        expected_revision: u64,
        title: String,
        body: String,
    },
}
