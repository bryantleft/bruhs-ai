//! Live meeting suggestions: private, optional notes an agent offers while a
//! meeting is recorded.
//!
//! The loop follows the Inner Thoughts framework (Liu, UCLA 2026): a trigger
//! (new speech or a pause), retrieval of salient context, thought formation,
//! an evaluation that argues for and against raising each thought, and a
//! threshold that decides whether it is shown. The app that owns the tasks,
//! events and people runs the loop; the server only answers one isolated,
//! tool-free model request per round. Suggestions never act on their own:
//! the user adds a task, copies a note, or dismisses it.
//!
//! Everything here is pure so the web and desktop apps run the same rules.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::agent::{model_info, AgentProvider};
use crate::{Speaker, TranscriptSegment};

#[cfg(test)]
mod tests;

/// How much initiative the meeting assistant takes. Off by default would hide
/// the feature, and automatic by default would spend usage nobody asked for,
/// so the default only answers when asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Proactivity {
    /// No suggestions and no requests.
    Off,
    /// Only when the user asks with Suggest now.
    #[default]
    OnRequest,
    /// Automatically after new speech or a pause, shown in the meeting dock.
    Automatic,
    /// Automatic, and the collapsed meeting pill announces new suggestions.
    Alert,
}

impl Proactivity {
    pub const ALL: [Self; 4] = [Self::Off, Self::OnRequest, Self::Automatic, Self::Alert];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::OnRequest => "on_request",
            Self::Automatic => "automatic",
            Self::Alert => "alert",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.as_str() == value)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::OnRequest => "When I ask",
            Self::Automatic => "Automatic",
            Self::Alert => "Automatic with alerts",
        }
    }

    pub fn is_automatic(self) -> bool {
        matches!(self, Self::Automatic | Self::Alert)
    }
}

pub const PROACTIVITY_HELP: &str = "Live meeting suggestions in the dock. Uses an extra model request and pauses at usage limits.";

/// Lowest and highest score a thought needs before it is shown. Scores are
/// the model's 1 to 5 rating of how strongly it should speak up now.
pub const MIN_THRESHOLD: u8 = 3;
pub const MAX_THRESHOLD: u8 = 5;
pub const DEFAULT_THRESHOLD: u8 = 4;
/// Outcomes needed before the threshold moves, and the window kept.
const CALIBRATION_MIN_OUTCOMES: usize = 6;
const CALIBRATION_WINDOW: usize = 12;

/// The shown threshold adapts to what the user keeps. Mostly dismissed
/// suggestions raise the bar; mostly kept ones lower it. The thesis tuned
/// these by hand and named that its main weakness; feedback replaces it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Calibration {
    #[serde(default = "default_threshold")]
    pub threshold: u8,
    /// Most recent outcomes, oldest first: true when kept, false when dismissed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outcomes: Vec<bool>,
}

fn default_threshold() -> u8 {
    DEFAULT_THRESHOLD
}

impl Default for Calibration {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_THRESHOLD,
            outcomes: Vec::new(),
        }
    }
}

impl Calibration {
    /// Record one decision. Returns true when the threshold moved.
    pub fn record(&mut self, kept: bool) -> bool {
        self.outcomes.push(kept);
        if self.outcomes.len() > CALIBRATION_WINDOW {
            self.outcomes.remove(0);
        }
        if self.outcomes.len() < CALIBRATION_MIN_OUTCOMES {
            return false;
        }
        let kept_count = self.outcomes.iter().filter(|kept| **kept).count();
        let total = self.outcomes.len();
        // Integer comparisons: at most a quarter kept raises the bar, at
        // least three quarters kept lowers it.
        let before = self.threshold;
        if kept_count * 4 <= total && self.threshold < MAX_THRESHOLD {
            self.threshold += 1;
        } else if kept_count * 4 >= total * 3 && self.threshold > MIN_THRESHOLD {
            self.threshold -= 1;
        }
        if self.threshold != before {
            self.outcomes.clear();
            return true;
        }
        false
    }

    /// A new meeting eases a raised bar one step back toward the default.
    /// At the highest bar few notes are shown, so too few decisions arrive
    /// to ever lower it by feedback alone. Returns true when it moved.
    pub fn relax(&mut self) -> bool {
        if self.threshold > DEFAULT_THRESHOLD {
            self.threshold -= 1;
            self.outcomes.clear();
            return true;
        }
        false
    }

    #[must_use]
    pub fn sanitized(mut self) -> Self {
        self.threshold = self.threshold.clamp(MIN_THRESHOLD, MAX_THRESHOLD);
        if self.outcomes.len() > CALIBRATION_WINDOW {
            let excess = self.outcomes.len() - CALIBRATION_WINDOW;
            self.outcomes.drain(..excess);
        }
        self
    }
}

/// Saved meeting suggestion preferences. Both apps store this as one value.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MeetingSuggestionPrefs {
    #[serde(default)]
    pub proactivity: Proactivity,
    /// None uses [`DEFAULT_MODEL`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub calibration: Calibration,
}

/// A lightweight Claude model: rounds repeat during a meeting.
pub const DEFAULT_MODEL: &str = "claude-haiku-4-5";

impl MeetingSuggestionPrefs {
    pub fn model(&self) -> &str {
        self.model.as_deref().unwrap_or(DEFAULT_MODEL)
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// A model only Claude or Codex can run in an isolated, tool-free request.
    pub fn supports_model(id: &str) -> bool {
        model_info(id).is_some_and(|model| {
            matches!(model.provider, AgentProvider::Claude | AgentProvider::Codex)
        })
    }

    #[must_use]
    pub fn sanitized(self) -> Self {
        Self {
            proactivity: self.proactivity,
            model: self.model.filter(|id| Self::supports_model(id)),
            calibration: self.calibration.sanitized(),
        }
    }
}

// ── Request and reply ───────────────────────────────────────────────────────

/// Product budgets for one round.
pub const WINDOW_MS: u32 = 120_000;
pub const MAX_TRANSCRIPT_LINES: usize = 40;
pub const MAX_TRANSCRIPT_CHARS: usize = 6_000;
pub const MAX_LINE_CHARS: usize = 600;
pub const MAX_CONTEXT_ITEMS: usize = 8;
pub const MAX_CONTEXT_CHARS: usize = 200;
pub const MAX_RECENT: usize = 10;
pub const MAX_THOUGHTS: usize = 3;
pub const MAX_TEXT_CHARS: usize = 160;
pub const MAX_TITLE_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptLine {
    pub speaker: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    Meeting,
    Task,
    Event,
    Person,
}

impl ContextKind {
    fn prefix(self) -> &'static str {
        match self {
            Self::Meeting => "m",
            Self::Task => "t",
            Self::Event => "e",
            Self::Person => "p",
        }
    }

    /// How much a match of this kind matters, before overlap.
    fn weight(self) -> f64 {
        match self {
            Self::Meeting | Self::Task => 1.0,
            Self::Person => 0.9,
            Self::Event => 0.8,
        }
    }
}

/// One piece of the user's own data the model may relate the talk to.
/// `id` is the app's record id and never leaves the app; the model sees
/// only `reference`, a short handle such as `t2`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextItem {
    pub reference: String,
    pub kind: ContextKind,
    pub text: String,
    #[serde(skip)]
    pub id: String,
    /// The words retrieval matches against: the title, without dates,
    /// times or attendee lists that would dilute the overlap.
    #[serde(skip)]
    pub keywords: String,
}

impl ContextItem {
    pub fn new(kind: ContextKind, id: impl Into<String>, text: impl Into<String>) -> Self {
        let text = clip(&one_line(&text.into()), MAX_CONTEXT_CHARS);
        Self {
            reference: String::new(),
            kind,
            keywords: text.clone(),
            text,
            id: id.into(),
        }
    }

    #[must_use]
    fn matching(mut self, title: &str) -> Self {
        self.keywords = clip(&one_line(title), MAX_CONTEXT_CHARS);
        self
    }

    pub fn task(id: impl Into<String>, title: &str, due: Option<&str>) -> Self {
        let text = match due {
            Some(due) if !due.trim().is_empty() => format!("{title} (due {due})"),
            _ => title.to_string(),
        };
        Self::new(ContextKind::Task, id, text).matching(title)
    }

    /// `when` is a short local label the app formats, such as "Thu 14:00".
    pub fn event(id: impl Into<String>, title: &str, when: &str, attendees: &[String]) -> Self {
        let mut text = format!("{title} ({when})");
        if !attendees.is_empty() {
            text.push_str(" with ");
            text.push_str(&attendees.join(", "));
        }
        Self::new(ContextKind::Event, id, text).matching(title)
    }

    pub fn person(id: impl Into<String>, name: &str) -> Self {
        Self::new(ContextKind::Person, id, name)
    }

    pub fn meeting(id: impl Into<String>, title: &str, attendees: &[String]) -> Self {
        let mut text = title.to_string();
        if !attendees.is_empty() {
            text.push_str(" with ");
            text.push_str(&attendees.join(", "));
        }
        Self::new(ContextKind::Meeting, id, text).matching(title)
    }
}

/// Events worth relating to a meeting: from a day ago to a week ahead.
pub fn event_in_window(start_at_ms: f64, now_ms: f64) -> bool {
    const DAY_MS: f64 = 86_400_000.0;
    start_at_ms >= now_ms - DAY_MS && start_at_ms <= now_ms + 7.0 * DAY_MS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// Enough new speech since the last round.
    Speech,
    /// The conversation went quiet after new speech.
    Pause,
    /// The user selected Suggest now.
    Request,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeetingSuggestionRequest {
    pub model: String,
    pub trigger: Trigger,
    pub meeting_title: String,
    pub transcript: Vec<TranscriptLine>,
    #[serde(default)]
    pub context: Vec<ContextItem>,
    /// Suggestions already shown or dismissed, so the model does not repeat them.
    #[serde(default)]
    pub recent: Vec<String>,
}

impl MeetingSuggestionRequest {
    pub fn is_valid(&self) -> bool {
        let transcript_chars: usize = self
            .transcript
            .iter()
            .map(|line| line.text.chars().count())
            .sum();
        let mut references = BTreeSet::new();
        !self.model.is_empty()
            && self.meeting_title.chars().count() <= MAX_TITLE_CHARS
            && !self.transcript.is_empty()
            && self.transcript.len() <= MAX_TRANSCRIPT_LINES
            && transcript_chars <= MAX_TRANSCRIPT_CHARS
            && self.transcript.iter().all(|line| {
                !line.text.trim().is_empty()
                    && line.text.chars().count() <= MAX_LINE_CHARS
                    && line.speaker.chars().count() <= 80
            })
            && self.context.len() <= MAX_CONTEXT_ITEMS
            && self.context.iter().all(|item| {
                item.text.chars().count() <= MAX_CONTEXT_CHARS
                    && is_reference(&item.reference)
                    && references.insert(item.reference.as_str())
            })
            && self.recent.len() <= MAX_RECENT
            && self
                .recent
                .iter()
                .all(|text| text.chars().count() <= MAX_TEXT_CHARS)
    }

    fn reference(&self, reference: &str) -> Option<&ContextItem> {
        self.context.iter().find(|item| item.reference == reference)
    }
}

fn is_reference(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('m' | 't' | 'e' | 'p'))
        && value.len() <= 4
        && chars.all(|c| c.is_ascii_digit())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThoughtKind {
    /// Someone committed to doing something.
    ActionItem,
    /// A question was asked and left open, or one worth asking.
    Question,
    /// Something said clashes with a task, an event or an earlier statement.
    Conflict,
    /// A task, event or person the talk relates to.
    Context,
    /// Something to follow up on after the meeting.
    FollowUp,
}

impl ThoughtKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::ActionItem => "Action item",
            Self::Question => "Open question",
            Self::Conflict => "Conflict",
            Self::Context => "Related",
            Self::FollowUp => "Follow up",
        }
    }

    /// Kinds the user can turn into a task with one action.
    pub fn is_actionable(self) -> bool {
        matches!(self, Self::ActionItem | Self::FollowUp)
    }
}

/// One evaluated thought, as the model returns it. The reasons come before
/// the score in the JSON so the model argues both sides before rating.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thought {
    pub kind: ThoughtKind,
    pub text: String,
    #[serde(rename = "for")]
    pub reason_for: String,
    #[serde(rename = "against")]
    pub reason_against: String,
    pub score: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeetingSuggestionReply {
    pub thoughts: Vec<Thought>,
}

impl MeetingSuggestionReply {
    pub fn is_valid(&self) -> bool {
        self.thoughts.len() <= MAX_THOUGHTS
            && self.thoughts.iter().all(|thought| {
                (1..=5).contains(&thought.score)
                    && is_clean(&thought.text)
                    && !thought.text.is_empty()
                    && is_clean(&thought.reason_for)
                    && is_clean(&thought.reason_against)
                    && thought.source.as_deref().is_none_or(is_reference)
            })
    }
}

fn is_clean(text: &str) -> bool {
    text.chars().count() <= MAX_TEXT_CHARS
        && !text.chars().any(char::is_control)
        && text == text.trim()
}

// ── Transcript window and retrieval ─────────────────────────────────────────

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The last `max` characters, on a character boundary.
fn clip_tail(text: &str, max: usize) -> String {
    let start = text
        .char_indices()
        .rev()
        .nth(max.saturating_sub(1))
        .map_or(0, |(offset, _)| offset);
    text[start..].to_string()
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// The speaker label the model sees. Live capture tags the microphone "me"
/// and system audio "them"; renamed speakers keep their names.
pub fn speaker_name(speakers: &[Speaker], speaker_id: Option<&str>) -> String {
    let named = speaker_id.and_then(|id| speakers.iter().find(|speaker| speaker.id == id));
    match (named, speaker_id) {
        (Some(speaker), _) if !speaker.label.trim().is_empty() => clip(&speaker.label, 80),
        (_, Some("me")) => "Me".to_string(),
        _ => "Others".to_string(),
    }
}

/// The newest two minutes of speech, merged by speaker run and trimmed to
/// the request budget from the oldest end.
pub fn transcript_window(
    segments: &[TranscriptSegment],
    speakers: &[Speaker],
) -> Vec<TranscriptLine> {
    let Some(latest) = segments.iter().map(|segment| segment.end_ms).max() else {
        return Vec::new();
    };
    let since = latest.saturating_sub(WINDOW_MS);
    let mut recent: Vec<&TranscriptSegment> = segments
        .iter()
        .filter(|segment| segment.end_ms > since && !segment.text.trim().is_empty())
        .collect();
    recent.sort_by_key(|segment| (segment.start_ms, segment.end_ms));
    let mut lines: Vec<TranscriptLine> = Vec::new();
    for segment in recent {
        let speaker = speaker_name(speakers, segment.speaker_id.as_deref());
        let text = one_line(&segment.text);
        match lines.last_mut() {
            Some(line) if line.speaker == speaker => {
                line.text.push(' ');
                line.text.push_str(&text);
            }
            _ => lines.push(TranscriptLine { speaker, text }),
        }
    }
    for line in &mut lines {
        if line.text.chars().count() > MAX_LINE_CHARS {
            line.text = clip_tail(&line.text, MAX_LINE_CHARS);
        }
    }
    let mut total = 0;
    let mut keep = 0;
    for line in lines.iter().rev() {
        let chars = line.text.chars().count();
        if keep == MAX_TRANSCRIPT_LINES || total + chars > MAX_TRANSCRIPT_CHARS {
            break;
        }
        total += chars;
        keep += 1;
    }
    lines.split_off(lines.len() - keep)
}

const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "are", "but", "not", "you", "all", "any", "can", "had", "her", "was",
    "one", "our", "out", "has", "him", "his", "how", "its", "may", "new", "now", "see", "two",
    "who", "did", "get", "got", "let", "say", "she", "too", "use", "that", "with", "have", "this",
    "will", "your", "from", "they", "know", "want", "been", "good", "much", "some", "time", "very",
    "when", "come", "here", "just", "like", "long", "make", "many", "more", "only", "over", "such",
    "take", "than", "them", "then", "well", "were", "what", "about", "would", "there", "their",
    "which", "could", "think", "yeah", "okay", "really", "going", "gonna", "should", "maybe",
    "right", "thing", "things", "also", "into", "because", "after", "before", "with",
];

fn tokens(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|word| {
            let length = word.chars().count();
            (length >= 3 || (length == 2 && word.chars().any(|c| c.is_ascii_digit())))
                && !STOP_WORDS.contains(&word.as_str())
        })
        .map(|word| {
            // A light plural fold, so "decks" meets "deck".
            if word.chars().count() > 4 && word.ends_with('s') && !word.ends_with("ss") {
                word[..word.len() - 1].to_string()
            } else {
                word
            }
        })
        .collect()
}

/// Share of `a`'s tokens that appear in `b`.
fn overlap(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    if a.is_empty() {
        return 0.0;
    }
    let shared = a.intersection(b).count();
    f64::from(u32::try_from(shared).unwrap_or(u32::MAX))
        / f64::from(u32::try_from(a.len()).unwrap_or(u32::MAX))
}

/// Minimum saliency for an item to enter a round.
const MIN_SALIENCY: f64 = 0.2;

/// Keep the meeting itself and the items the talk touches, most salient
/// first, then assign each a short reference for the model. Saliency is
/// lexical overlap times the kind's weight: no embeddings run in the
/// browser, and titles are short enough for words to carry the match.
pub fn select_context(items: Vec<ContextItem>, transcript: &[TranscriptLine]) -> Vec<ContextItem> {
    let spoken = tokens(
        &transcript
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    );
    let mut meeting = None;
    let mut scored: Vec<(f64, ContextItem)> = Vec::new();
    for item in items {
        if item.text.trim().is_empty() {
            continue;
        }
        if item.kind == ContextKind::Meeting {
            meeting.get_or_insert(item);
            continue;
        }
        let saliency = overlap(&tokens(&item.keywords), &spoken) * item.kind.weight();
        if saliency >= MIN_SALIENCY {
            scored.push((saliency, item));
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut selected: Vec<ContextItem> = meeting.into_iter().collect();
    selected.extend(
        scored
            .into_iter()
            .map(|(_, item)| item)
            .take(MAX_CONTEXT_ITEMS - selected.len()),
    );
    let mut counts = [0u32; 4];
    for item in &mut selected {
        let slot = match item.kind {
            ContextKind::Meeting => 0,
            ContextKind::Task => 1,
            ContextKind::Event => 2,
            ContextKind::Person => 3,
        };
        counts[slot] += 1;
        item.reference = if item.kind == ContextKind::Meeting {
            "m".to_string()
        } else {
            format!("{}{}", item.kind.prefix(), counts[slot])
        };
    }
    selected
}

/// How much of the shorter text's vocabulary the other one repeats (the
/// overlap coefficient), for duplicate suppression. A model that rephrases a
/// note ("Should the free tier be kept" and "Decide: keep the free tier")
/// keeps most of its key words while changing the rest, which Jaccard
/// similarity misses.
fn similarity(a: &str, b: &str) -> f64 {
    let (a, b) = (tokens(a), tokens(b));
    let smaller = a.len().min(b.len());
    if smaller == 0 {
        return if a.is_empty() && b.is_empty() {
            1.0
        } else {
            0.0
        };
    }
    let shared = a.intersection(&b).count();
    f64::from(u32::try_from(shared).unwrap_or(u32::MAX))
        / f64::from(u32::try_from(smaller).unwrap_or(u32::MAX))
}

const DUPLICATE_SIMILARITY: f64 = 0.7;

// ── Scheduling ──────────────────────────────────────────────────────────────

/// New speech (characters) that justifies a round, and the spacing between
/// automatic rounds. At most two rounds a minute while people keep talking.
const SPEECH_CHARS: usize = 160;
const SPEECH_SPACING_MS: f64 = 30_000.0;
/// A pause: no new transcript for this long after some new speech.
const PAUSE_CHARS: usize = 40;
const PAUSE_MS: f64 = 10_000.0;
const PAUSE_SPACING_MS: f64 = 15_000.0;
/// After a failed round, wait before trying again automatically.
const FAILURE_BACKOFF_MS: f64 = 60_000.0;
const BUSY_BACKOFF_MS: f64 = 15_000.0;

#[derive(Debug, Clone, Default, PartialEq)]
struct Scheduler {
    /// Transcript time (ms from meeting start) covered by the last round.
    covered_ms: u32,
    /// Transcript end the running round will cover when it succeeds.
    pending_ms: Option<u32>,
    last_round_at: Option<f64>,
    /// Newest transcript end seen, and when it last grew.
    seen_ms: u32,
    grew_at: Option<f64>,
    retry_after: Option<f64>,
}

impl Scheduler {
    fn observe(&mut self, now: f64, segments: &[TranscriptSegment]) -> usize {
        let latest = segments
            .iter()
            .map(|segment| segment.end_ms)
            .max()
            .unwrap_or(0);
        if latest > self.seen_ms || self.grew_at.is_none() && latest > 0 {
            self.seen_ms = latest;
            self.grew_at = Some(now);
        }
        segments
            .iter()
            .filter(|segment| segment.end_ms > self.covered_ms)
            .map(|segment| segment.text.trim().chars().count())
            .sum()
    }

    fn due(&mut self, now: f64, segments: &[TranscriptSegment]) -> Option<Trigger> {
        let fresh = self.observe(now, segments);
        if self.pending_ms.is_some() || self.retry_after.is_some_and(|until| now < until) {
            return None;
        }
        let since_round = self.last_round_at.map_or(f64::INFINITY, |at| now - at);
        let quiet_for = self.grew_at.map_or(0.0, |at| now - at);
        if fresh >= SPEECH_CHARS && since_round >= SPEECH_SPACING_MS {
            Some(Trigger::Speech)
        } else if fresh >= PAUSE_CHARS && quiet_for >= PAUSE_MS && since_round >= PAUSE_SPACING_MS {
            Some(Trigger::Pause)
        } else {
            None
        }
    }

    fn begin(&mut self, now: f64) {
        self.pending_ms = Some(self.seen_ms);
        self.last_round_at = Some(now);
    }

    fn finish(&mut self) {
        if let Some(covered) = self.pending_ms.take() {
            self.covered_ms = self.covered_ms.max(covered);
        }
        self.retry_after = None;
    }

    fn fail(&mut self, now: f64, backoff_ms: f64) {
        self.pending_ms = None;
        self.retry_after = Some(now + backoff_ms);
    }
}

// ── Live session ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SuggestionState {
    New,
    Seen,
    Added,
    Copied,
    Dismissed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub id: String,
    pub kind: ThoughtKind,
    pub text: String,
    /// Why the assistant raised it, from the evaluation.
    pub reason: String,
    pub score: u8,
    /// The app record the thought came from, and its kind.
    pub source: Option<(ContextKind, String)>,
    pub created_at_ms: f64,
    pub state: SuggestionState,
}

impl Suggestion {
    pub fn is_visible(&self) -> bool {
        !matches!(self.state, SuggestionState::Dismissed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RoundStatus {
    #[default]
    Idle,
    Thinking,
    /// The last round failed; the message is safe to show.
    Failed(String),
}

/// Suggestions kept per meeting, newest first.
pub const MAX_SUGGESTIONS: usize = 20;

/// One live meeting's suggestion session. Both apps own one of these and
/// call it from a timer and from their buttons.
#[derive(Debug, Clone, Default)]
pub struct LiveSuggestions {
    meeting_id: Option<String>,
    scheduler: Scheduler,
    round: u64,
    pub items: Vec<Suggestion>,
    pub status: RoundStatus,
    /// Paused for this meeting only.
    pub paused: bool,
}

/// What the app knows about the live meeting at one moment.
#[derive(Debug, Clone, Copy)]
pub struct LiveInput<'a> {
    /// Wall clock, epoch milliseconds.
    pub now: f64,
    pub prefs: &'a MeetingSuggestionPrefs,
    pub meeting_title: &'a str,
    pub segments: &'a [TranscriptSegment],
    pub speakers: &'a [Speaker],
}

/// A round the app should send. `round` lets a late reply be ignored after
/// the meeting changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Round {
    pub round: u64,
    pub request: MeetingSuggestionRequest,
}

impl LiveSuggestions {
    /// Start fresh when a different meeting goes live (or none is).
    pub fn follow(&mut self, meeting_id: Option<&str>) {
        if self.meeting_id.as_deref() != meeting_id {
            *self = Self {
                meeting_id: meeting_id.map(str::to_string),
                round: self.round + 1,
                ..Self::default()
            };
        }
    }

    pub fn meeting_id(&self) -> Option<&str> {
        self.meeting_id.as_deref()
    }

    pub fn is_thinking(&self) -> bool {
        self.status == RoundStatus::Thinking
    }

    pub fn visible(&self) -> impl Iterator<Item = &Suggestion> {
        self.items.iter().filter(|item| item.is_visible())
    }

    pub fn unseen(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.state == SuggestionState::New)
            .count()
    }

    pub fn mark_seen(&mut self) -> bool {
        let mut changed = false;
        for item in &mut self.items {
            if item.state == SuggestionState::New {
                item.state = SuggestionState::Seen;
                changed = true;
            }
        }
        changed
    }

    /// Called from the app's timer. Returns a round to send when one is due.
    /// `context` runs only when a round starts.
    pub fn poll(
        &mut self,
        input: &LiveInput<'_>,
        context: impl FnOnce() -> Vec<ContextItem>,
    ) -> Option<Round> {
        if self.meeting_id.is_none() || self.paused || !input.prefs.proactivity.is_automatic() {
            self.scheduler.observe(input.now, input.segments);
            return None;
        }
        let trigger = self.scheduler.due(input.now, input.segments)?;
        self.start(input, trigger, context())
    }

    /// Suggest now. Returns None when nothing has been said yet, suggestions
    /// are off, or a round is already running.
    pub fn request(&mut self, input: &LiveInput<'_>, context: Vec<ContextItem>) -> Option<Round> {
        if self.meeting_id.is_none() || input.prefs.proactivity == Proactivity::Off {
            return None;
        }
        self.scheduler.observe(input.now, input.segments);
        if self.scheduler.pending_ms.is_some() {
            return None;
        }
        self.start(input, Trigger::Request, context)
    }

    fn start(
        &mut self,
        input: &LiveInput<'_>,
        trigger: Trigger,
        context: Vec<ContextItem>,
    ) -> Option<Round> {
        let transcript = transcript_window(input.segments, input.speakers);
        if transcript.is_empty() {
            if trigger == Trigger::Request {
                self.status = RoundStatus::Failed("Nothing has been transcribed yet.".into());
            }
            return None;
        }
        let context = select_context(context, &transcript);
        let recent = self
            .items
            .iter()
            .take(MAX_RECENT)
            .map(|item| item.text.clone())
            .collect();
        let request = MeetingSuggestionRequest {
            model: input.prefs.model().to_string(),
            trigger,
            meeting_title: clip(&one_line(input.meeting_title), MAX_TITLE_CHARS),
            transcript,
            context,
            recent,
        };
        if !request.is_valid() {
            return None;
        }
        self.scheduler.begin(input.now);
        self.round += 1;
        self.status = RoundStatus::Thinking;
        Some(Round {
            round: self.round,
            request,
        })
    }

    /// Apply a reply. Thoughts below the threshold, malformed ones and
    /// repeats are dropped. Returns how many suggestions were added.
    pub fn finish(
        &mut self,
        round: &Round,
        reply: MeetingSuggestionReply,
        threshold: u8,
        now: f64,
        new_id: impl FnMut() -> String,
    ) -> usize {
        if round.round != self.round {
            return 0;
        }
        self.scheduler.finish();
        self.status = RoundStatus::Idle;
        if !reply.is_valid() {
            return 0;
        }
        let mut new_id = new_id;
        let mut added = 0;
        let mut thoughts = reply.thoughts;
        thoughts.sort_by_key(|thought| std::cmp::Reverse(thought.score));
        for thought in thoughts {
            if thought.score < threshold {
                continue;
            }
            let repeat = self
                .items
                .iter()
                .any(|item| similarity(&item.text, &thought.text) >= DUPLICATE_SIMILARITY);
            if repeat {
                continue;
            }
            let source = thought
                .source
                .as_deref()
                .and_then(|reference| round.request.reference(reference))
                .filter(|item| item.kind != ContextKind::Meeting)
                .map(|item| (item.kind, item.id.clone()));
            self.items.insert(
                0,
                Suggestion {
                    id: new_id(),
                    kind: thought.kind,
                    text: thought.text,
                    reason: thought.reason_for,
                    score: thought.score,
                    source,
                    created_at_ms: now,
                    state: SuggestionState::New,
                },
            );
            added += 1;
        }
        self.items.truncate(MAX_SUGGESTIONS);
        added
    }

    /// Abandon a running round without penalty: a late reply is ignored and
    /// the next round may start as soon as one is due.
    pub fn cancel(&mut self) {
        if self.is_thinking() {
            self.round += 1;
            self.scheduler.pending_ms = None;
            self.status = RoundStatus::Idle;
        }
    }

    /// A failed round, from the server's HTTP status (None when it could
    /// not be reached). A busy server is retried sooner than other failures:
    /// another app may have held the one round slot for a moment.
    pub fn fail(&mut self, round: &Round, now: f64, status: Option<u16>) {
        if round.round != self.round {
            return;
        }
        let backoff = if status == Some(503) {
            BUSY_BACKOFF_MS
        } else {
            FAILURE_BACKOFF_MS
        };
        self.scheduler.fail(now, backoff);
        self.status = RoundStatus::Failed(failure_message(status).to_string());
    }

    /// Apply Add task, Copy or Dismiss. Returns the suggestion, so the app
    /// can add a task or copy text, and the outcome to record for
    /// calibration. Only the first decision on a suggestion counts; Copy may
    /// repeat; Add task and Dismiss are final.
    pub fn decide(&mut self, id: &str, decision: SuggestionState) -> Option<Decision> {
        let item = self.items.iter_mut().find(|item| item.id == id)?;
        let undecided = matches!(item.state, SuggestionState::New | SuggestionState::Seen);
        let kept = match decision {
            SuggestionState::Copied if item.state != SuggestionState::Dismissed => {
                if undecided {
                    item.state = SuggestionState::Copied;
                }
                undecided.then_some(true)
            }
            SuggestionState::Added | SuggestionState::Dismissed
                if !matches!(
                    item.state,
                    SuggestionState::Added | SuggestionState::Dismissed
                ) =>
            {
                item.state = decision;
                undecided.then_some(decision == SuggestionState::Added)
            }
            _ => return None,
        };
        Some(Decision {
            suggestion: item.clone(),
            kept,
        })
    }
}

/// What a failed round says, from the server's HTTP status (None when the
/// server could not be reached). Both apps show the same words.
pub fn failure_message(status: Option<u16>) -> &'static str {
    match status {
        None => "Could not reach the server. Suggestions will try again.",
        Some(400) => "Choose a Claude or Codex model for meeting suggestions in Settings.",
        Some(503) => "Suggestions are paused to save usage, or another round is running. They will try again.",
        Some(504) => "Suggestions took too long. They will try again.",
        Some(_) => "Suggestions are unavailable right now. They will try again.",
    }
}

/// What a decision returns: the suggestion as it now stands, and the
/// calibration outcome (true kept, false dismissed) when this decision is
/// the first one on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub suggestion: Suggestion,
    pub kept: Option<bool>,
}
