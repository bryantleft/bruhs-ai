//! The orchestration board: where each agent task stands between its first
//! turn and its merge.
//!
//! Lanes are derived from task state, verification children and linked pull
//! requests. Nobody drags a card into "Ready": a card is ready when its pull
//! request can merge, so the board cannot disagree with the work. The host
//! supplies each task's [`TaskLink`] (branch and pull requests it read); this
//! module only classifies, so every surface shows the same lanes.
use super::{CoordinatedTask, CoordinationSnapshot, Relationship, TaskState};
use crate::{
    delivery::{Check, Pull},
    ids::ThreadId,
    EpochMs,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A column on the board, in delivery order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    /// The agent is writing, or has written, changes without a reviewable PR.
    Building,
    /// A verification child or the PR's checks are proving the change.
    Validating,
    /// A PR waits on people: review, requested changes or a conflict.
    InReview,
    /// Every open PR can merge now.
    Ready,
    /// Every linked PR merged; the card leaves the lanes.
    Done,
}

impl Lane {
    /// The lanes shown as columns; [`Lane::Done`] lives with archived work.
    pub const COLUMNS: [Self; 4] = [
        Self::Building,
        Self::Validating,
        Self::InReview,
        Self::Ready,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Building => "Building",
            Self::Validating => "Validating",
            Self::InReview => "In review",
            Self::Ready => "Ready",
            Self::Done => "Done",
        }
    }
}

/// What a status asks of the reader. Surfaces map it to Bruhs tokens:
/// action is persimmon, info blueberry, warning durian, danger lychee,
/// success pandan, neutral longan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Neutral,
    Info,
    Action,
    Warning,
    Danger,
    Success,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardStatus {
    pub label: String,
    pub tone: Tone,
    /// Work is in progress right now (an agent turn or running checks).
    pub busy: bool,
}

impl CardStatus {
    fn new(label: &str, tone: Tone, busy: bool) -> Self {
        Self {
            label: label.into(),
            tone,
            busy,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullState {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksState {
    /// The forge reported no checks.
    None,
    Pending,
    Passing,
    Failing,
}

impl ChecksState {
    pub fn of(checks: &[Check]) -> Self {
        if checks.iter().any(Check::failed) {
            Self::Failing
        } else if checks.iter().any(Check::pending) {
            Self::Pending
        } else if checks.is_empty() {
            Self::None
        } else {
            Self::Passing
        }
    }
}

/// The part of a forge pull request the board needs, small enough to send
/// with every card.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskPull {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: PullState,
    pub checks: ChecksState,
    /// Forge review decision: `APPROVED`, `CHANGES_REQUESTED`,
    /// `REVIEW_REQUIRED`, or none when the repository requires no review.
    pub review_decision: Option<String>,
    /// `MERGEABLE`, `CONFLICTING` or `UNKNOWN` while the forge computes it.
    pub mergeable: String,
    /// Conversation comments plus submitted reviews.
    pub comments: u32,
    /// When the forge last changed the request; for a merged one, about when
    /// it merged.
    #[serde(default)]
    pub updated_at: Option<EpochMs>,
}

impl TaskPull {
    pub fn from_pull(pull: &Pull) -> Self {
        let state = match pull.state.to_ascii_uppercase().as_str() {
            "MERGED" => PullState::Merged,
            "CLOSED" => PullState::Closed,
            _ if pull.draft => PullState::Draft,
            _ => PullState::Open,
        };
        let comments = pull.comments.len().saturating_add(
            pull.reviews
                .iter()
                .filter(|review| !review.body.trim().is_empty())
                .count(),
        );
        Self {
            number: pull.number,
            title: pull.title.clone(),
            url: pull.url.clone(),
            state,
            checks: ChecksState::of(&pull.checks),
            review_decision: pull
                .review_decision
                .clone()
                .filter(|decision| !decision.is_empty()),
            mergeable: pull.mergeable.to_ascii_uppercase(),
            comments: u32::try_from(comments).unwrap_or(u32::MAX),
            updated_at: parse_utc(&pull.updated_at),
        }
    }

    fn open(&self) -> bool {
        matches!(self.state, PullState::Open | PullState::Draft)
    }

    /// The lane this one open PR puts its task in.
    fn lane(&self) -> Lane {
        if self.state == PullState::Draft {
            return Lane::Building;
        }
        if matches!(self.checks, ChecksState::Pending | ChecksState::Failing) {
            return Lane::Validating;
        }
        let approved_or_not_required =
            matches!(self.review_decision.as_deref(), None | Some("APPROVED"));
        if self.mergeable == "MERGEABLE" && approved_or_not_required {
            Lane::Ready
        } else {
            Lane::InReview
        }
    }

    /// What this PR is waiting on, for a card whose agent is not working.
    fn status(&self) -> CardStatus {
        match self.lane() {
            Lane::Building => CardStatus::new("Draft PR", Tone::Neutral, false),
            Lane::Validating if self.checks == ChecksState::Failing => {
                CardStatus::new("Checks failing", Tone::Danger, false)
            }
            Lane::Validating => CardStatus::new("Checks running", Tone::Info, true),
            Lane::InReview if self.mergeable == "CONFLICTING" => {
                CardStatus::new("Merge conflict", Tone::Danger, false)
            }
            Lane::InReview if self.review_decision.as_deref() == Some("CHANGES_REQUESTED") => {
                CardStatus::new("Changes requested", Tone::Warning, false)
            }
            Lane::InReview => CardStatus::new("Awaiting review", Tone::Warning, false),
            Lane::Ready => CardStatus::new("Mergeable", Tone::Success, false),
            Lane::Done => CardStatus::new("Merged", Tone::Success, false),
        }
    }
}

/// What the host found out about one task's code: the branch its working
/// directory is on and the pull requests opened from it or linked to it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskLink {
    pub branch: Option<String>,
    pub pulls: Vec<TaskPull>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardCard {
    pub id: ThreadId,
    pub title: String,
    pub lane: Lane,
    pub status: CardStatus,
    pub project_path: String,
    pub branch: Option<String>,
    pub pulls: Vec<TaskPull>,
    pub model: String,
    /// The catalog name for `model`, for display.
    pub model_label: String,
    pub relationship: Relationship,
    pub parent_id: Option<ThreadId>,
    /// Delegated and forked tasks (each its own card).
    pub children: u32,
    pub active_children: u32,
    /// Verification runs folded into this card.
    pub verifications: u32,
    pub approvals: u32,
    pub archived: bool,
    pub updated_at: EpochMs,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskBoard {
    pub revision: u64,
    pub cards: Vec<BoardCard>,
    /// The host is still reading branches or pull requests; ask again soon.
    #[serde(default)]
    pub refreshing: bool,
    /// The last pull request lookup that failed, for the reader to fix.
    #[serde(default)]
    pub link_error: Option<String>,
}

/// Whether a task gets its own card. Verification runs and side chats serve
/// their parent and fold into its card; a child whose parent is gone keeps one.
pub fn has_card(task: &CoordinatedTask, snapshot: &CoordinationSnapshot) -> bool {
    match task.relationship {
        Relationship::Root | Relationship::Delegate | Relationship::Fork => true,
        Relationship::Verify | Relationship::SideChat => task
            .parent_id
            .as_ref()
            .is_none_or(|parent| snapshot.tasks.iter().all(|task| &task.id != parent)),
    }
}

/// Every carded task in `snapshot`, most urgent first within each lane:
/// approvals, then failures, then the most recently updated.
pub fn cards(
    snapshot: &CoordinationSnapshot,
    links: &BTreeMap<ThreadId, TaskLink>,
) -> Vec<BoardCard> {
    let mut cards = snapshot
        .tasks
        .iter()
        .filter(|task| has_card(task, snapshot))
        .map(|task| card(task, snapshot, links.get(&task.id)))
        .collect::<Vec<_>>();
    cards.sort_by(|a, b| {
        a.lane
            .cmp(&b.lane)
            .then_with(|| urgency(b).cmp(&urgency(a)))
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });
    cards
}

fn urgency(card: &BoardCard) -> u8 {
    match card.status.tone {
        Tone::Action => 2,
        Tone::Danger => 1,
        _ => 0,
    }
}

fn card(
    task: &CoordinatedTask,
    snapshot: &CoordinationSnapshot,
    link: Option<&TaskLink>,
) -> BoardCard {
    let children = snapshot
        .tasks
        .iter()
        .filter(|child| child.parent_id.as_ref() == Some(&task.id))
        .collect::<Vec<_>>();
    let carded = children
        .iter()
        .filter(|child| has_card(child, snapshot))
        .collect::<Vec<_>>();
    let verifications = children
        .iter()
        .filter(|child| child.relationship == Relationship::Verify)
        .copied()
        .collect::<Vec<_>>();
    let pulls: Vec<TaskPull> = link
        .map(|link| {
            link.pulls
                .iter()
                .filter(|pull| belongs_to(task, pull))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let (lane, pull_status) = lane(task, &pulls, &verifications);
    let status = status(task, lane, pull_status, &verifications);
    let updated_at = children
        .iter()
        .map(|child| child.updated_at)
        .chain([task.updated_at])
        .max()
        .unwrap_or(task.updated_at);
    BoardCard {
        id: task.id.clone(),
        title: task.title.clone(),
        lane,
        status,
        project_path: task.project_path.clone(),
        branch: link.and_then(|link| link.branch.clone()),
        pulls,
        model: task.model.clone(),
        model_label: crate::agent::model_label(&task.model),
        relationship: task.relationship,
        parent_id: task.parent_id.clone(),
        children: count(carded.len()),
        active_children: count(
            carded
                .iter()
                .filter(|child| child.state.active() || child.state == TaskState::Ready)
                .count(),
        ),
        verifications: count(verifications.len()),
        approvals: count(task.approvals.len()),
        // Work that is running again belongs in the lanes even if a stale
        // archive mark survived.
        archived: task.archived_at.is_some()
            && !task.state.active()
            && task.state != TaskState::Ready,
        updated_at,
    }
}

/// Branch names get reused. A request that merged or closed before the task
/// existed is earlier work on the same name, not this task's; an open one
/// always counts, since a task may pick up an existing request.
fn belongs_to(task: &CoordinatedTask, pull: &TaskPull) -> bool {
    pull.open() || pull.updated_at.is_none_or(|at| at >= task.created_at)
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The lane, and the status of the open PR that holds the task back (the
/// least advanced one), when there is one.
fn lane(
    task: &CoordinatedTask,
    pulls: &[TaskPull],
    verifications: &[&CoordinatedTask],
) -> (Lane, Option<CardStatus>) {
    if let Some(behind) = pulls
        .iter()
        .filter(|pull| pull.open())
        .min_by_key(|pull| pull.lane())
    {
        return (behind.lane(), Some(behind.status()));
    }
    let idle = !task.state.active() && task.state != TaskState::Ready;
    if idle && merged_after_last_prompt(task, pulls) {
        return (
            Lane::Done,
            Some(CardStatus::new("Merged", Tone::Success, false)),
        );
    }
    match current_verification(task, verifications) {
        Some(_) => (Lane::Validating, None),
        None => (Lane::Building, None),
    }
}

/// The newest verification run, if the task has not been asked for more work
/// since it started. A new prompt sends the card back to Building.
fn current_verification<'a>(
    task: &CoordinatedTask,
    verifications: &[&'a CoordinatedTask],
) -> Option<&'a CoordinatedTask> {
    let latest = verifications.iter().max_by_key(|run| run.created_at)?;
    (latest.created_at >= last_prompt(task)).then_some(*latest)
}

/// Whether a merged request covers the task's latest work. A prompt sent
/// after the last merge is new work on the branch, so the card stays on the
/// board. A merge with no known time counts.
fn merged_after_last_prompt(task: &CoordinatedTask, pulls: &[TaskPull]) -> bool {
    let last_prompt = last_prompt(task);
    pulls
        .iter()
        .filter(|pull| pull.state == PullState::Merged)
        .any(|pull| pull.updated_at.is_none_or(|merged| merged >= last_prompt))
}

fn last_prompt(task: &CoordinatedTask) -> EpochMs {
    task.messages
        .iter()
        .filter(|message| message.role == "user")
        .map(|message| message.at)
        .max()
        .unwrap_or(task.created_at)
}

/// `YYYY-MM-DDTHH:MM:SS` UTC, as GitHub and GitLab report times; anything
/// else is unknown rather than guessed.
fn parse_utc(text: &str) -> Option<EpochMs> {
    let bytes = text.as_bytes();
    if bytes.len() < 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let field = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (field(0..4)?, field(5..7)?, field(8..10)?);
    let (hour, minute, second) = (field(11..13)?, field(14..16)?, field(17..19)?);
    let zone = text
        .get(19..)?
        .trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    if !matches!(zone, "Z" | "" | "+00:00")
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    EpochMs::new(seconds.checked_mul(1_000)?).ok()
}

fn status(
    task: &CoordinatedTask,
    lane: Lane,
    pull_status: Option<CardStatus>,
    verifications: &[&CoordinatedTask],
) -> CardStatus {
    match task.state {
        TaskState::Running => return CardStatus::new("Working", Tone::Info, true),
        TaskState::WaitingApproval => {
            return CardStatus::new("Needs approval", Tone::Action, false)
        }
        TaskState::WaitingChild => return CardStatus::new("Waiting on delegate", Tone::Info, true),
        TaskState::Ready => return CardStatus::new("Queued", Tone::Neutral, false),
        TaskState::Limited if task.resume_at.is_some() => {
            return CardStatus::new("Resumes at reset", Tone::Warning, false)
        }
        TaskState::Limited => return CardStatus::new("Usage limit", Tone::Warning, false),
        TaskState::Failed => return CardStatus::new("Failed", Tone::Danger, false),
        TaskState::Interrupted => return CardStatus::new("Interrupted", Tone::Warning, false),
        TaskState::Cancelled => return CardStatus::new("Stopped", Tone::Neutral, false),
        TaskState::Completed => {}
    }
    if let Some(status) = pull_status {
        return status;
    }
    if lane == Lane::Validating {
        if let Some(run) = current_verification(task, verifications) {
            return match run.state {
                TaskState::Completed => CardStatus::new("Verified", Tone::Success, false),
                TaskState::Failed | TaskState::Interrupted | TaskState::Limited => {
                    CardStatus::new("Verification failed", Tone::Danger, false)
                }
                TaskState::Cancelled => {
                    CardStatus::new("Verification stopped", Tone::Neutral, false)
                }
                TaskState::WaitingApproval => {
                    CardStatus::new("Verifier needs approval", Tone::Action, false)
                }
                TaskState::Running | TaskState::WaitingChild | TaskState::Ready => {
                    CardStatus::new("Verifying", Tone::Info, true)
                }
            };
        }
    }
    CardStatus::new("Idle", Tone::Neutral, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{agent::PermissionMode, coordination::TaskMessage};

    fn task(relationship: Relationship, state: TaskState, at: i64) -> CoordinatedTask {
        CoordinatedTask {
            id: ThreadId::new_v4(),
            parent_id: None,
            source_context: vec![],
            source_revision: None,
            relationship,
            title: "Fix toast".into(),
            project_path: "/p".into(),
            working_path: "/p".into(),
            model: "claude-sonnet-4-5".into(),
            permission_mode: PermissionMode::Default,
            spec_id: None,
            spec_revision: None,
            state,
            messages: vec![TaskMessage {
                role: "user".into(),
                text: "go".into(),
                at: EpochMs::saturating_from_i64(at),
            }],
            live_output: String::new(),
            error: None,
            session_id: None,
            approvals: vec![],
            created_at: EpochMs::saturating_from_i64(at),
            updated_at: EpochMs::saturating_from_i64(at),
            resume_at: None,
            auto_resume_on_limit: false,
            limit_reported: false,
            archived_at: None,
        }
    }

    fn child(
        parent: &CoordinatedTask,
        relationship: Relationship,
        state: TaskState,
        at: i64,
    ) -> CoordinatedTask {
        let mut child = task(relationship, state, at);
        child.parent_id = Some(parent.id.clone());
        child
    }

    fn pull(
        state: PullState,
        checks: ChecksState,
        review: Option<&str>,
        mergeable: &str,
    ) -> TaskPull {
        TaskPull {
            number: 7,
            title: "Fix toast".into(),
            url: "https://example.test/pull/7".into(),
            state,
            checks,
            review_decision: review.map(str::to_owned),
            mergeable: mergeable.into(),
            comments: 0,
            updated_at: None,
        }
    }

    fn board(tasks: Vec<CoordinatedTask>, links: &[(usize, Vec<TaskPull>)]) -> Vec<BoardCard> {
        let links = links
            .iter()
            .map(|(index, pulls)| {
                (
                    tasks[*index].id.clone(),
                    TaskLink {
                        branch: Some("bruhs/task".into()),
                        pulls: pulls.clone(),
                    },
                )
            })
            .collect();
        let snapshot = CoordinationSnapshot {
            tasks,
            ..CoordinationSnapshot::default()
        };
        cards(&snapshot, &links)
    }

    fn only(cards: &[BoardCard]) -> (Lane, &str) {
        assert_eq!(cards.len(), 1, "{cards:?}");
        (cards[0].lane, cards[0].status.label.as_str())
    }

    #[test]
    fn a_working_agent_without_a_pull_request_is_building() {
        let cards = board(vec![task(Relationship::Root, TaskState::Running, 1)], &[]);
        assert_eq!(only(&cards), (Lane::Building, "Working"));
        assert!(cards[0].status.busy);
    }

    #[test]
    fn a_finished_turn_without_a_pull_request_is_idle_building() {
        let cards = board(vec![task(Relationship::Root, TaskState::Completed, 1)], &[]);
        assert_eq!(only(&cards), (Lane::Building, "Idle"));
    }

    #[test]
    fn verification_folds_into_its_parent_and_moves_it_to_validating() {
        let parent = task(Relationship::Root, TaskState::Completed, 1);
        let running = child(&parent, Relationship::Verify, TaskState::Running, 2);
        let cards = board(vec![parent, running], &[]);
        assert_eq!(only(&cards), (Lane::Validating, "Verifying"));
        assert_eq!(cards[0].verifications, 1);
    }

    #[test]
    fn a_failed_verification_is_reported_until_the_task_gets_new_work() {
        let mut parent = task(Relationship::Root, TaskState::Completed, 1);
        let failed = child(&parent, Relationship::Verify, TaskState::Failed, 2);
        let cards = board(vec![parent.clone(), failed.clone()], &[]);
        assert_eq!(only(&cards), (Lane::Validating, "Verification failed"));

        parent.messages.push(TaskMessage {
            role: "user".into(),
            text: "fix it".into(),
            at: EpochMs::saturating_from_i64(3),
        });
        parent.state = TaskState::Running;
        let cards = board(vec![parent, failed], &[]);
        assert_eq!(only(&cards), (Lane::Building, "Working"));
    }

    #[test]
    fn pull_request_progress_sets_the_lane() {
        let cases = [
            (
                pull(PullState::Draft, ChecksState::None, None, "MERGEABLE"),
                Lane::Building,
                "Draft PR",
            ),
            (
                pull(PullState::Open, ChecksState::Pending, None, "MERGEABLE"),
                Lane::Validating,
                "Checks running",
            ),
            (
                pull(PullState::Open, ChecksState::Failing, None, "MERGEABLE"),
                Lane::Validating,
                "Checks failing",
            ),
            (
                pull(
                    PullState::Open,
                    ChecksState::Passing,
                    Some("REVIEW_REQUIRED"),
                    "MERGEABLE",
                ),
                Lane::InReview,
                "Awaiting review",
            ),
            (
                pull(
                    PullState::Open,
                    ChecksState::Passing,
                    Some("CHANGES_REQUESTED"),
                    "MERGEABLE",
                ),
                Lane::InReview,
                "Changes requested",
            ),
            (
                pull(
                    PullState::Open,
                    ChecksState::Passing,
                    Some("APPROVED"),
                    "CONFLICTING",
                ),
                Lane::InReview,
                "Merge conflict",
            ),
            (
                pull(PullState::Open, ChecksState::None, None, "UNKNOWN"),
                Lane::InReview,
                "Awaiting review",
            ),
            (
                pull(
                    PullState::Open,
                    ChecksState::Passing,
                    Some("APPROVED"),
                    "MERGEABLE",
                ),
                Lane::Ready,
                "Mergeable",
            ),
            (
                pull(PullState::Open, ChecksState::None, None, "MERGEABLE"),
                Lane::Ready,
                "Mergeable",
            ),
            (
                pull(
                    PullState::Merged,
                    ChecksState::Passing,
                    Some("APPROVED"),
                    "UNKNOWN",
                ),
                Lane::Done,
                "Merged",
            ),
        ];
        for (pull, lane, label) in cases {
            let cards = board(
                vec![task(Relationship::Root, TaskState::Completed, 1)],
                &[(0, vec![pull.clone()])],
            );
            assert_eq!(only(&cards), (lane, label), "{pull:?}");
        }
    }

    #[test]
    fn the_least_advanced_open_pull_request_holds_the_card_back() {
        let ready = pull(
            PullState::Open,
            ChecksState::Passing,
            Some("APPROVED"),
            "MERGEABLE",
        );
        let review = pull(
            PullState::Open,
            ChecksState::Passing,
            Some("REVIEW_REQUIRED"),
            "MERGEABLE",
        );
        let merged = pull(PullState::Merged, ChecksState::Passing, None, "UNKNOWN");
        let cards = board(
            vec![task(Relationship::Root, TaskState::Completed, 1)],
            &[(0, vec![ready, merged, review])],
        );
        assert_eq!(only(&cards), (Lane::InReview, "Awaiting review"));
    }

    #[test]
    fn a_closed_unmerged_pull_request_leaves_the_task_building() {
        let closed = pull(PullState::Closed, ChecksState::Passing, None, "UNKNOWN");
        let cards = board(
            vec![task(Relationship::Root, TaskState::Completed, 1)],
            &[(0, vec![closed])],
        );
        assert_eq!(only(&cards), (Lane::Building, "Idle"));
    }

    #[test]
    fn agent_activity_outranks_pull_request_status_but_keeps_the_lane() {
        let review = pull(
            PullState::Open,
            ChecksState::Passing,
            Some("CHANGES_REQUESTED"),
            "MERGEABLE",
        );
        let mut addressing = task(Relationship::Root, TaskState::WaitingApproval, 1);
        addressing
            .approvals
            .push(crate::coordination::PendingApproval {
                id: "a".into(),
                tool: "Bash".into(),
                input: serde_json::Value::Null,
            });
        let cards = board(vec![addressing], &[(0, vec![review])]);
        assert_eq!(only(&cards), (Lane::InReview, "Needs approval"));
        assert_eq!(cards[0].status.tone, Tone::Action);
        assert_eq!(cards[0].approvals, 1);
    }

    #[test]
    fn a_merged_task_that_is_working_again_is_not_done() {
        let merged = pull(PullState::Merged, ChecksState::Passing, None, "UNKNOWN");
        let cards = board(
            vec![task(Relationship::Root, TaskState::Running, 1)],
            &[(0, vec![merged])],
        );
        assert_eq!(only(&cards), (Lane::Building, "Working"));
    }

    #[test]
    fn delegates_get_their_own_cards_and_count_on_the_parent() {
        let parent = task(Relationship::Root, TaskState::WaitingChild, 1);
        let delegate = child(&parent, Relationship::Delegate, TaskState::Running, 2);
        let side = child(&parent, Relationship::SideChat, TaskState::Completed, 3);
        let parent_id = parent.id.clone();
        let cards = board(vec![parent, delegate, side], &[]);
        assert_eq!(cards.len(), 2);
        let parent = cards.iter().find(|card| card.id == parent_id).unwrap();
        assert_eq!((parent.children, parent.active_children), (1, 1));
        assert_eq!(parent.status.label, "Waiting on delegate");
        assert_eq!(parent.updated_at, EpochMs::saturating_from_i64(3));
    }

    #[test]
    fn an_orphaned_verification_keeps_a_card() {
        let mut orphan = task(Relationship::Verify, TaskState::Completed, 1);
        orphan.parent_id = Some(ThreadId::new_v4());
        assert_eq!(board(vec![orphan], &[]).len(), 1);
    }

    #[test]
    fn cards_needing_the_owner_sort_first_within_a_lane() {
        let quiet = task(Relationship::Root, TaskState::Completed, 9);
        let mut asking = task(Relationship::Root, TaskState::WaitingApproval, 1);
        asking.title = "Asking".into();
        let cards = board(vec![quiet, asking], &[]);
        assert_eq!(cards[0].title, "Asking");
    }

    #[test]
    fn checks_summary_prefers_failures_over_pending_work() {
        let check = |state: &str, conclusion: Option<&str>| Check {
            name: "ci".into(),
            state: state.into(),
            conclusion: conclusion.map(str::to_owned),
            url: None,
        };
        assert_eq!(ChecksState::of(&[]), ChecksState::None);
        assert_eq!(
            ChecksState::of(&[
                check("IN_PROGRESS", None),
                check("COMPLETED", Some("FAILURE"))
            ]),
            ChecksState::Failing
        );
        assert_eq!(
            ChecksState::of(&[check("IN_PROGRESS", None)]),
            ChecksState::Pending
        );
        assert_eq!(
            ChecksState::of(&[check("COMPLETED", Some("SUCCESS"))]),
            ChecksState::Passing
        );
    }

    #[test]
    fn new_work_after_a_merge_keeps_the_card_on_the_board() {
        let mut merged = pull(PullState::Merged, ChecksState::Passing, None, "UNKNOWN");
        merged.updated_at = Some(EpochMs::saturating_from_i64(5));
        let mut task = task(Relationship::Root, TaskState::Completed, 1);
        let cards = board(vec![task.clone()], &[(0, vec![merged.clone()])]);
        assert_eq!(only(&cards), (Lane::Done, "Merged"));

        task.messages.push(TaskMessage {
            role: "user".into(),
            text: "follow up".into(),
            at: EpochMs::saturating_from_i64(9),
        });
        let cards = board(vec![task], &[(0, vec![merged])]);
        assert_eq!(only(&cards), (Lane::Building, "Idle"));
    }

    #[test]
    fn running_work_is_never_filed_away_as_archived() {
        let mut task = task(Relationship::Root, TaskState::WaitingApproval, 1);
        task.archived_at = Some(EpochMs::saturating_from_i64(2));
        assert!(!board(vec![task.clone()], &[])[0].archived);
        task.state = TaskState::Completed;
        assert!(board(vec![task], &[])[0].archived);
    }

    #[test]
    fn forge_times_parse_as_utc_and_odd_ones_are_unknown() {
        assert_eq!(
            parse_utc("2026-11-13T00:00:00Z"),
            EpochMs::new(1_794_528_000_000).ok()
        );
        assert_eq!(
            parse_utc("1970-01-01T00:00:01.250Z"),
            EpochMs::new(1_000).ok()
        );
        assert_eq!(
            parse_utc("2024-02-29T12:00:00+00:00"),
            EpochMs::new(1_709_208_000_000).ok()
        );
        assert_eq!(parse_utc("2026-11-13T00:00:00+02:00"), None);
        assert_eq!(parse_utc("yesterday"), None);
        assert_eq!(parse_utc(""), None);
    }

    #[test]
    fn requests_settled_before_the_task_existed_are_earlier_work_on_the_branch() {
        let at = |ms| Some(EpochMs::saturating_from_i64(ms));
        let mut old_merge = pull(PullState::Merged, ChecksState::Passing, None, "UNKNOWN");
        old_merge.updated_at = at(5);
        let mut current = pull(PullState::Draft, ChecksState::None, None, "MERGEABLE");
        current.number = 8;
        current.updated_at = at(3);
        let task = task(Relationship::Root, TaskState::Completed, 10);

        let cards = board(vec![task.clone()], &[(0, vec![old_merge.clone()])]);
        assert_eq!(
            only(&cards),
            (Lane::Building, "Idle"),
            "an old merge is not Done"
        );
        assert!(cards[0].pulls.is_empty());

        let cards = board(vec![task], &[(0, vec![current, old_merge])]);
        assert_eq!(
            cards[0].pulls.len(),
            1,
            "the open request counts even if older"
        );
        assert_eq!(only(&cards), (Lane::Building, "Draft PR"));
    }
}
