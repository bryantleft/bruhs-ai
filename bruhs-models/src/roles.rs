//! Roles and models by task: how the user spreads work across models.
//!
//! A **role** is a named setup for a thread: a model, a Claude effort, an
//! access mode and instructions. An agent starts a thread with a role
//! (`message_thread`'s `role`), so one thread can orchestrate while others
//! write or execute on the models the user chose for that work. A project
//! may override any part of a role.
//!
//! **Task models** pick the model for the jobs that are not threads:
//! meeting summaries and second-opinion reviews.
//!
//! Both are stored by each app (web local storage, the desktop state file)
//! and read through here, so the two apps resolve them the same way.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::agent::{model_info, AgentProvider, PermissionMode};
use crate::logic::run_place::claude_effort;

/// Most roles one user keeps.
pub const MAX_ROLES: usize = 12;
/// Longest role name, in characters.
pub const MAX_NAME_CHARS: usize = 40;
/// Longest "use for" line, in characters.
pub const MAX_USE_FOR_CHARS: usize = 160;
/// Longest instructions for one role, in characters.
pub const MAX_INSTRUCTIONS_CHARS: usize = 4_000;

/// A named setup for a thread.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    /// Stable id. Threads and project overrides refer to it, so a rename
    /// keeps them.
    pub id: String,
    pub name: String,
    /// One line telling an agent what to hand this role.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub use_for: String,
    /// Model id. `None` keeps the model of the thread that starts it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Claude effort. `None` keeps the user's default. Codex ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Access mode. It can only narrow the mode the user is in; see
    /// [`narrow_access`]. `None` keeps that mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<PermissionMode>,
    /// Added to every turn of a thread with this role.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub instructions: String,
}

/// What one project changes about a role. `None` keeps the role's value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleOverride {
    /// Id of the role it changes.
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

impl RoleOverride {
    fn is_empty(&self) -> bool {
        self.model.is_none()
            && self.effort.is_none()
            && self.access.is_none()
            && self.instructions.is_none()
    }
}

/// A role with one project's overrides applied: what a thread runs with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedRole {
    pub id: String,
    pub name: String,
    pub use_for: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub access: Option<PermissionMode>,
    pub instructions: String,
}

/// Every role, and what each project changes about them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleBook {
    #[serde(default)]
    pub roles: Vec<Role>,
    /// Overrides by project name, the key thread title instructions use.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_project: BTreeMap<String, Vec<RoleOverride>>,
}

/// The first `max` characters of `text`.
fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// A model id the pickers offer, or `None`.
fn known_model(model: Option<String>) -> Option<String> {
    model.filter(|id| model_info(id).is_some())
}

/// A Claude effort level, or `None`.
fn known_effort(effort: Option<String>) -> Option<String> {
    effort
        .as_deref()
        .and_then(claude_effort)
        .map(str::to_string)
}

/// One line with no control characters, clipped.
fn one_line(text: &str, max: usize) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    clip(cleaned.trim(), max)
}

impl Role {
    fn sanitize(&mut self) {
        self.name = one_line(&self.name, MAX_NAME_CHARS);
        self.use_for = one_line(&self.use_for, MAX_USE_FOR_CHARS);
        self.model = known_model(self.model.take());
        self.effort = known_effort(self.effort.take());
        self.instructions = clip(self.instructions.trim(), MAX_INSTRUCTIONS_CHARS);
    }
}

impl RoleBook {
    pub fn is_empty(&self) -> bool {
        self.roles.is_empty()
    }

    /// The role `reference` names: its id, or its name in any case.
    pub fn find(&self, reference: &str) -> Option<&Role> {
        let reference = reference.trim();
        self.roles
            .iter()
            .find(|role| role.id == reference)
            .or_else(|| {
                self.roles
                    .iter()
                    .find(|role| role.name.eq_ignore_ascii_case(reference))
            })
    }

    /// What `project` changes about the role `id`.
    pub fn override_for(&self, project: &str, id: &str) -> Option<&RoleOverride> {
        self.by_project
            .get(project)?
            .iter()
            .find(|change| change.role == id)
    }

    /// The role `reference` names, as it runs in `project`.
    pub fn resolve(&self, reference: &str, project: &str) -> Option<ResolvedRole> {
        let role = self.find(reference)?;
        let change = self.override_for(project, &role.id);
        Some(ResolvedRole {
            id: role.id.clone(),
            name: role.name.clone(),
            use_for: role.use_for.clone(),
            model: change
                .and_then(|change| change.model.clone())
                .or_else(|| role.model.clone()),
            effort: change
                .and_then(|change| change.effort.clone())
                .or_else(|| role.effort.clone()),
            access: change.and_then(|change| change.access).or(role.access),
            instructions: change
                .and_then(|change| change.instructions.clone())
                .unwrap_or_else(|| role.instructions.clone()),
        })
    }

    /// Every role as it runs in `project`, in the user's order.
    pub fn resolved(&self, project: &str) -> Vec<ResolvedRole> {
        self.roles
            .iter()
            .filter_map(|role| self.resolve(&role.id, project))
            .collect()
    }

    /// A name no other role has: `name`, or `name 2`, `name 3`, ….
    fn free_name(&self, name: &str, except: Option<&str>) -> String {
        let base = one_line(name, MAX_NAME_CHARS);
        let base = if base.is_empty() {
            "Role".to_string()
        } else {
            base
        };
        let taken = |candidate: &str| {
            self.roles.iter().any(|role| {
                Some(role.id.as_str()) != except && role.name.eq_ignore_ascii_case(candidate)
            })
        };
        if !taken(&base) {
            return base;
        }
        (2..)
            .map(|n| {
                let suffix = format!(" {n}");
                format!(
                    "{}{suffix}",
                    clip(&base, MAX_NAME_CHARS.saturating_sub(suffix.len()))
                )
            })
            .find(|candidate| !taken(candidate))
            .unwrap_or(base)
    }

    /// Add a role called `name`. Returns its id, or `None` at [`MAX_ROLES`].
    pub fn add(&mut self, name: &str) -> Option<String> {
        if self.roles.len() >= MAX_ROLES {
            return None;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let name = self.free_name(name, None);
        self.roles.push(Role {
            id: id.clone(),
            name,
            ..Role::default()
        });
        Some(id)
    }

    /// Remove the role `id` and every project's override of it.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.roles.len();
        self.roles.retain(|role| role.id != id);
        for changes in self.by_project.values_mut() {
            changes.retain(|change| change.role != id);
        }
        self.by_project.retain(|_, changes| !changes.is_empty());
        self.roles.len() != before
    }

    /// Change the role `id`. Its fields are cleaned afterwards and its name
    /// kept unique; its id cannot change. `false` when there is no such role.
    pub fn update(&mut self, id: &str, change: impl FnOnce(&mut Role)) -> bool {
        let Some(index) = self.roles.iter().position(|role| role.id == id) else {
            return false;
        };
        let mut role = self.roles[index].clone();
        change(&mut role);
        role.id = id.to_string();
        role.sanitize();
        role.name = self.free_name(&role.name, Some(id));
        self.roles[index] = role;
        true
    }

    /// Change what `project` overrides for the role `id`. An override that
    /// ends up changing nothing is dropped. `false` when there is no such
    /// role or `project` is blank.
    pub fn update_override(
        &mut self,
        project: &str,
        id: &str,
        change: impl FnOnce(&mut RoleOverride),
    ) -> bool {
        if project.trim().is_empty() || !self.roles.iter().any(|role| role.id == id) {
            return false;
        }
        let changes = self.by_project.entry(project.to_string()).or_default();
        let index = match changes.iter().position(|change| change.role == id) {
            Some(index) => index,
            None => {
                changes.push(RoleOverride {
                    role: id.to_string(),
                    ..RoleOverride::default()
                });
                changes.len() - 1
            }
        };
        let entry = &mut changes[index];
        change(entry);
        entry.role = id.to_string();
        entry.model = known_model(entry.model.take());
        entry.effort = known_effort(entry.effort.take());
        entry.instructions = entry
            .instructions
            .take()
            .map(|text| clip(text.trim(), MAX_INSTRUCTIONS_CHARS))
            .filter(|text| !text.is_empty());
        if entry.is_empty() {
            changes.remove(index);
        }
        self.by_project.retain(|_, changes| !changes.is_empty());
        true
    }

    /// A stored book made safe to use: fields cleaned, unknown models and
    /// efforts dropped, ids and names unique, overrides of missing roles
    /// removed, at most [`MAX_ROLES`].
    #[must_use]
    pub fn sanitized(self) -> Self {
        let mut book = Self::default();
        for mut role in self.roles {
            if book.roles.len() >= MAX_ROLES {
                break;
            }
            if role.id.trim().is_empty() || book.roles.iter().any(|kept| kept.id == role.id) {
                continue;
            }
            role.sanitize();
            role.name = book.free_name(&role.name, None);
            book.roles.push(role);
        }
        for (project, changes) in self.by_project {
            for change in changes {
                let id = change.role.clone();
                book.update_override(&project, &id, |entry| *entry = change);
            }
        }
        book
    }
}

/// How much a mode lets an agent do without asking.
const fn access_rank(mode: PermissionMode) -> u8 {
    match mode {
        PermissionMode::Plan => 0,
        PermissionMode::Default => 1,
        PermissionMode::AcceptEdits => 2,
        PermissionMode::BypassPermissions => 3,
    }
}

/// The access a thread with a role runs with: the stricter of the mode the
/// user is in and the role's. A role can make a reviewer read-only; it can
/// never give an agent more than the user's own mode, or an agent could
/// start a thread with a looser role to get around it.
pub fn narrow_access(current: PermissionMode, role: Option<PermissionMode>) -> PermissionMode {
    match role {
        Some(role) if access_rank(role) < access_rank(current) => role,
        _ => current,
    }
}

/// Models for the jobs that are not threads. `None` keeps the built-in
/// default for that job.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskModels {
    /// Meeting summaries. A Claude model: summaries run on the Claude CLI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// A second opinion written by Claude (on a Codex thread).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_claude: Option<String>,
    /// A second opinion written by Codex (on a Claude thread).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_codex: Option<String>,
}

/// A model the pickers offer that `provider` runs, or `None`.
fn provider_model(model: Option<String>, provider: AgentProvider) -> Option<String> {
    model.filter(|id| model_info(id).is_some_and(|info| info.provider == provider))
}

impl TaskModels {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The model for a review written by `reviewer`.
    pub fn review(&self, reviewer: AgentProvider) -> Option<&str> {
        match reviewer {
            AgentProvider::Claude => self.review_claude.as_deref(),
            AgentProvider::Codex => self.review_codex.as_deref(),
        }
    }

    /// Stored choices made safe to use: a model its job's provider cannot
    /// run is dropped, so the job falls back to its default.
    #[must_use]
    pub fn sanitized(self) -> Self {
        Self {
            summary: provider_model(self.summary, AgentProvider::Claude),
            review_claude: provider_model(self.review_claude, AgentProvider::Claude),
            review_codex: provider_model(self.review_codex, AgentProvider::Codex),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE: &str = "claude-sonnet-4-6";
    const CODEX: &str = "gpt-6.1-sol";

    fn book() -> (RoleBook, String, String) {
        let mut book = RoleBook::default();
        let writer = book.add("Writer").unwrap();
        let executor = book.add("Executor").unwrap();
        book.update(&writer, |role| {
            role.use_for = "drafting prose".into();
            role.model = Some(CLAUDE.into());
            role.effort = Some("high".into());
            role.instructions = "Write plainly.".into();
        });
        book.update(&executor, |role| {
            role.model = Some(CODEX.into());
            role.access = Some(PermissionMode::AcceptEdits);
        });
        (book, writer, executor)
    }

    #[test]
    fn roles_are_found_by_id_or_by_name_in_any_case() {
        let (book, writer, _) = book();
        assert_eq!(book.find(&writer).unwrap().name, "Writer");
        assert_eq!(book.find("  writer ").unwrap().id, writer);
        assert_eq!(book.find("EXECUTOR").unwrap().model.as_deref(), Some(CODEX));
        assert!(book.find("Reviewer").is_none());
        assert!(RoleBook::default().is_empty() && !book.is_empty());
    }

    #[test]
    fn a_project_overrides_only_what_it_sets() {
        let (mut book, writer, _) = book();
        assert!(book.update_override("bruhs-ai", &writer, |change| {
            change.model = Some("claude-haiku-4-5".into());
            change.instructions = Some("Write in British English.".into());
        }));
        let here = book.resolve("writer", "bruhs-ai").unwrap();
        assert_eq!(here.model.as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(here.instructions, "Write in British English.");
        // What the project did not set comes from the role.
        assert_eq!(here.effort.as_deref(), Some("high"));
        assert_eq!(here.use_for, "drafting prose");
        let elsewhere = book.resolve("writer", "other").unwrap();
        assert_eq!(elsewhere.model.as_deref(), Some(CLAUDE));
        assert_eq!(elsewhere.instructions, "Write plainly.");
        assert_eq!(book.resolved("bruhs-ai").len(), 2);

        // Clearing every field drops the override, and the project with it.
        assert!(book.update_override("bruhs-ai", &writer, |change| {
            change.model = None;
            change.instructions = Some("   ".into());
        }));
        assert!(book.by_project.is_empty());
        assert!(!book.update_override(" ", &writer, |_| {}));
        assert!(!book.update_override("bruhs-ai", "missing", |_| {}));
    }

    #[test]
    fn edits_are_cleaned_and_names_stay_unique() {
        let (mut book, writer, executor) = book();
        assert!(book.update(&executor, |role| {
            role.id = "hijacked".into();
            role.name = "  writer\n".into();
            role.model = Some("not-a-model".into());
            role.effort = Some("ludicrous".into());
            role.use_for = "x".repeat(MAX_USE_FOR_CHARS + 20);
            role.instructions = "y".repeat(MAX_INSTRUCTIONS_CHARS + 20);
        }));
        let edited = book.find(&executor).unwrap();
        assert_eq!(edited.id, executor);
        assert_eq!(edited.name, "writer 2", "the name was taken");
        assert_eq!(
            (edited.model.as_ref(), edited.effort.as_ref()),
            (None, None)
        );
        assert_eq!(edited.use_for.chars().count(), MAX_USE_FOR_CHARS);
        assert_eq!(edited.instructions.chars().count(), MAX_INSTRUCTIONS_CHARS);
        // Renaming a role to its own name keeps it.
        assert!(book.update(&writer, |role| role.name = "WRITER".into()));
        assert_eq!(book.find(&writer).unwrap().name, "WRITER");
        assert!(book.update(&writer, |role| role.name = String::new()));
        assert_eq!(book.find(&writer).unwrap().name, "Role");
        assert!(!book.update("missing", |_| {}));
    }

    #[test]
    fn removing_a_role_removes_its_overrides_and_the_count_is_capped() {
        let (mut book, writer, executor) = book();
        book.update_override("p", &writer, |change| change.effort = Some("low".into()));
        book.update_override("p", &executor, |change| change.effort = Some("low".into()));
        assert!(book.remove(&writer));
        assert!(!book.remove(&writer));
        assert_eq!(book.by_project["p"].len(), 1);
        assert!(book.remove(&executor));
        assert!(book.by_project.is_empty());
        for n in 0..MAX_ROLES {
            assert!(book.add(&format!("Role {n}")).is_some());
        }
        assert!(book.add("One too many").is_none());
    }

    #[test]
    fn a_stored_book_is_made_safe_to_use() {
        let stored: RoleBook = serde_json::from_value(serde_json::json!({
            "roles": [
                {"id": "a", "name": "Writer", "model": "retired-model", "effort": "high"},
                {"id": "a", "name": "Duplicate id"},
                {"id": "", "name": "No id"},
                {"id": "b", "name": "writer", "access": "plan"}
            ],
            "by_project": {
                "p": [
                    {"role": "a", "model": CLAUDE},
                    {"role": "gone", "model": CLAUDE},
                    {"role": "b"}
                ]
            }
        }))
        .unwrap();
        let book = stored.sanitized();
        assert_eq!(book.roles.len(), 2);
        assert_eq!(book.roles[0].model, None, "an unknown model is dropped");
        assert_eq!(book.roles[0].effort.as_deref(), Some("high"));
        assert_eq!(book.roles[1].name, "writer 2");
        assert_eq!(book.roles[1].access, Some(PermissionMode::Plan));
        assert_eq!(book.by_project["p"].len(), 1);
        assert_eq!(
            book.resolve("a", "p").unwrap().model.as_deref(),
            Some(CLAUDE)
        );
        // An untouched book serializes to almost nothing, and back.
        assert_eq!(
            serde_json::to_string(&RoleBook::default()).unwrap(),
            r#"{"roles":[]}"#
        );
        let round: RoleBook = serde_json::from_str(&serde_json::to_string(&book).unwrap()).unwrap();
        assert_eq!(round, book);
        assert_eq!(
            serde_json::from_str::<RoleBook>("{}").unwrap(),
            RoleBook::default()
        );
    }

    #[test]
    fn a_role_can_narrow_access_but_never_widen_it() {
        use PermissionMode::{AcceptEdits, BypassPermissions, Default, Plan};
        assert_eq!(narrow_access(AcceptEdits, Some(Plan)), Plan);
        assert_eq!(narrow_access(BypassPermissions, Some(Default)), Default);
        assert_eq!(narrow_access(Default, Some(BypassPermissions)), Default);
        assert_eq!(narrow_access(Plan, Some(AcceptEdits)), Plan);
        assert_eq!(narrow_access(AcceptEdits, Some(AcceptEdits)), AcceptEdits);
        assert_eq!(narrow_access(AcceptEdits, None), AcceptEdits);
    }

    #[test]
    fn task_models_keep_only_what_each_job_can_run() {
        let stored = TaskModels {
            summary: Some(CODEX.into()),
            review_claude: Some(CLAUDE.into()),
            review_codex: Some(CLAUDE.into()),
        };
        let models = stored.sanitized();
        assert_eq!(models.summary, None, "summaries run on Claude");
        assert_eq!(models.review(AgentProvider::Claude), Some(CLAUDE));
        assert_eq!(models.review(AgentProvider::Codex), None);
        assert!(TaskModels::default().is_default() && !models.is_default());
        assert_eq!(serde_json::to_string(&TaskModels::default()).unwrap(), "{}");
        let kept = TaskModels {
            summary: Some("claude-haiku-4-5".into()),
            review_codex: Some(CODEX.into()),
            ..TaskModels::default()
        }
        .sanitized();
        assert_eq!(kept.summary.as_deref(), Some("claude-haiku-4-5"));
        assert_eq!(kept.review(AgentProvider::Codex), Some(CODEX));
    }
}
