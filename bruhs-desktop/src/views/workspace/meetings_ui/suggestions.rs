//! Live meeting suggestions on the desktop, 1:1 with
//! `bruhs-web/src/meeting_suggestions.rs` and
//! `bruhs-web/src/components/meeting_suggestions_panel.rs`. The shared rules
//! live in `bruhs_models::meeting_suggestions`.

use gpui_component::zoom::px;
use std::time::Duration;

use bruhs_models::meeting_suggestions::{
    event_in_window, ContextItem, LiveInput, LiveSuggestions, MeetingSuggestionPrefs, Proactivity,
    Round, RoundStatus, SuggestionState,
};
use bruhs_models::{CalendarEvent, EpochMs, Meeting, WorkItemPlanning};
use chrono::TimeZone;

use super::*;
use crate::app_state::DesktopState;
use crate::server_client::ServerError;
use crate::views::workspace::{DesktopPriority, DesktopWorkItem, DesktopWorkItemStatus};

/// How often the automatic loop checks the transcript; the shared rules
/// decide whether a round is due.
const TICK: Duration = Duration::from_secs(2);

/// One live meeting's suggestion session and its saved preferences.
#[derive(Default)]
pub(crate) struct MeetingSuggestionsUi {
    pub(crate) prefs: MeetingSuggestionPrefs,
    live: LiveSuggestions,
    /// The suggestions panel above the live pill, like the web dock's Open.
    open: bool,
    note: Option<String>,
    round: Option<Task<()>>,
    /// The request itself runs on the tokio runtime; aborting it closes the
    /// connection, so the server frees its one round slot right away.
    request: Option<tokio::task::AbortHandle>,
    ticker: Option<Task<()>>,
}

impl MeetingSuggestionsUi {
    pub(crate) fn new(prefs: MeetingSuggestionPrefs) -> Self {
        Self {
            prefs: prefs.sanitized(),
            ..Self::default()
        }
    }

    /// Stop a running round: drop its task and abort its request.
    fn stop_round(&mut self) {
        self.round = None;
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.live.cancel();
    }
}

fn attendee_names(event: &CalendarEvent) -> Vec<String> {
    // Names only: attendee addresses never leave the app.
    event
        .attendees
        .iter()
        .filter(|attendee| !attendee.self_)
        .filter_map(|attendee| attendee.display_name.clone())
        .filter(|name| !name.trim().is_empty())
        .take(6)
        .collect()
}

/// A short local label such as "Thu 14:00", as the web formats it.
fn when(start_at_ms: f64) -> String {
    EpochMs::from_f64_floor(start_at_ms)
        .ok()
        .and_then(|at| chrono::Local.timestamp_millis_opt(at.as_millis()).single())
        .map(|at| at.format("%a %H:%M").to_string())
        .unwrap_or_default()
}

fn status_code(error: &ServerError) -> Option<u16> {
    match error {
        ServerError::Status { code, .. } => Some(code.as_u16()),
        ServerError::Decode { .. } => Some(502),
        ServerError::Transport { source, .. } if source.is_timeout() => Some(504),
        _ => None,
    }
}

impl Workspace {
    /// Start the two-second check. Rounds only run while a meeting is live
    /// and suggestions are automatic.
    pub(in crate::views::workspace) fn start_meeting_suggestions(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) {
        if cfg!(test) {
            return;
        }
        self.meeting_suggestions.ticker = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(TICK).await;
            if this
                .update(cx, |this, cx| this.tick_meeting_suggestions(cx))
                .is_err()
            {
                break;
            }
        }));
    }

    /// Follow the live meeting: a different one starts a fresh session and
    /// drops any round in flight. Returns the live meeting's id.
    fn follow_live_meeting(&mut self, cx: &mut Context<'_, Self>) -> Option<String> {
        let live_id = self
            .meetings_hub
            .read(cx)
            .live_meeting()
            .map(|meeting| meeting.id.clone());
        let state = &mut self.meeting_suggestions;
        if state.live.meeting_id() != live_id.as_deref() {
            state.stop_round();
            state.live.follow(live_id.as_deref());
            state.open = false;
            state.note = None;
            if live_id.is_some() && state.prefs.calibration.relax() {
                self.persist(cx);
            }
            cx.notify();
        }
        live_id
    }

    pub(in crate::views::workspace) fn tick_meeting_suggestions(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(id) = self.follow_live_meeting(cx) else {
            return;
        };
        if !self.meeting_suggestions.prefs.proactivity.is_automatic() {
            return;
        }
        let round =
            self.with_live_meeting(&id, cx, |live, input, context| live.poll(input, context));
        if let Some(round) = round {
            self.send_meeting_suggestion_round(round, cx);
            cx.notify();
        }
    }

    fn suggest_meeting_now(&mut self, cx: &mut Context<'_, Self>) {
        let Some(id) = self.follow_live_meeting(cx) else {
            return;
        };
        let round = self.with_live_meeting(&id, cx, |live, input, context| {
            live.request(input, context())
        });
        if let Some(round) = round {
            self.send_meeting_suggestion_round(round, cx);
        }
        cx.notify();
    }

    fn with_live_meeting(
        &mut self,
        id: &str,
        cx: &mut Context<'_, Self>,
        run: impl FnOnce(
            &mut LiveSuggestions,
            &LiveInput<'_>,
            &dyn Fn() -> Vec<ContextItem>,
        ) -> Option<Round>,
    ) -> Option<Round> {
        let hub = self.meetings_hub.read(cx);
        let meeting = hub.meeting(id)?.clone();
        let segments = hub.segments.get(id).map_or(&[][..], Vec::as_slice);
        let now = EpochMs::now().as_f64();
        let work_items = &self.work_items;
        let context = || meeting_context(&meeting, hub, work_items, now);
        let input = LiveInput {
            now,
            prefs: &self.meeting_suggestions.prefs,
            meeting_title: &meeting.title,
            segments,
            speakers: &meeting.speakers,
        };
        let mut live = std::mem::take(&mut self.meeting_suggestions.live);
        let round = run(&mut live, &input, &context);
        self.meeting_suggestions.live = live;
        round
    }

    fn send_meeting_suggestion_round(&mut self, round: Round, cx: &mut Context<'_, Self>) {
        let client = self.meetings_hub.read(cx).client();
        let request = round.request.clone();
        let runtime = Arc::clone(&cx.global::<DesktopState>().runtime);
        let handle = runtime.spawn(async move { client.meeting_suggestion_round(&request).await });
        self.meeting_suggestions.request = Some(handle.abort_handle());
        self.meeting_suggestions.round = Some(cx.spawn(async move |this, cx| {
            // An aborted request ends here as a join error; the session was
            // already cancelled by whoever aborted it.
            let Ok(result) = handle.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                let now = EpochMs::now().as_f64();
                let state = &mut this.meeting_suggestions;
                state.request = None;
                match result {
                    Ok(reply) => {
                        let threshold = state.prefs.calibration.threshold;
                        state.live.finish(&round, reply, threshold, now, || {
                            uuid::Uuid::new_v4().to_string()
                        });
                        // An open panel shows them as they arrive.
                        if state.open {
                            state.live.mark_seen();
                        }
                    }
                    Err(error) => {
                        tracing::debug!("meeting suggestion round failed: {error}");
                        state.live.fail(&round, now, status_code(&error));
                    }
                }
                cx.notify();
            });
        }));
    }

    pub(in crate::views::workspace) fn set_meeting_suggestion_prefs(
        &mut self,
        cx: &mut Context<'_, Self>,
        change: impl FnOnce(&mut MeetingSuggestionPrefs),
    ) {
        let mut prefs = self.meeting_suggestions.prefs.clone();
        change(&mut prefs);
        let prefs = prefs.sanitized();
        if prefs == self.meeting_suggestions.prefs {
            return;
        }
        if prefs.proactivity == Proactivity::Off {
            self.meeting_suggestions.stop_round();
            self.meeting_suggestions.open = false;
        }
        self.meeting_suggestions.prefs = prefs;
        self.persist_and_notify(cx);
    }

    fn toggle_meeting_suggestions(&mut self, cx: &mut Context<'_, Self>) {
        let state = &mut self.meeting_suggestions;
        state.open = !state.open;
        if state.open {
            state.live.mark_seen();
        }
        cx.notify();
    }

    fn decide_meeting_suggestion(
        &mut self,
        id: &str,
        decision: SuggestionState,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(outcome) = self.meeting_suggestions.live.decide(id, decision) else {
            return;
        };
        let suggestion = outcome.suggestion;
        if let Some(kept) = outcome.kept {
            self.meeting_suggestions.prefs.calibration.record(kept);
        }
        let note = match decision {
            SuggestionState::Added => {
                let meeting = self
                    .meetings_hub
                    .read(cx)
                    .live_meeting()
                    .map(|meeting| meeting.title.clone());
                self.work_items.push(DesktopWorkItem {
                    id: uuid::Uuid::new_v4().to_string(),
                    project_id: None,
                    title: suggestion.text,
                    description: Some(match meeting {
                        Some(title) => format!("Suggested during the meeting {title}."),
                        None => "Suggested during a meeting.".to_string(),
                    }),
                    status: DesktopWorkItemStatus::Backlog,
                    priority: DesktopPriority::None,
                    due_date: None,
                    planning: WorkItemPlanning::default(),
                    updated_at: EpochMs::now(),
                });
                Some("Task added.".to_string())
            }
            SuggestionState::Copied => {
                cx.write_to_clipboard(ClipboardItem::new_string(suggestion.text));
                Some("Copied.".to_string())
            }
            SuggestionState::Dismissed | SuggestionState::New | SuggestionState::Seen => None,
        };
        self.meeting_suggestions.note = note;
        self.persist_and_notify(cx);
    }

    /// The pill's Suggestions toggle and, at the alert level, its count of
    /// new suggestions. None when suggestions are off.
    pub(super) fn render_meeting_suggestions_toggle(
        &self,
        cx: &Context<'_, Self>,
    ) -> Option<Vec<AnyElement>> {
        let state = &self.meeting_suggestions;
        if state.prefs.proactivity == Proactivity::Off {
            return None;
        }
        let mut parts = Vec::new();
        let unseen = state.live.unseen();
        if state.prefs.proactivity == Proactivity::Alert && !state.open && unseen > 0 {
            let text = if unseen == 1 {
                "1 new suggestion".to_string()
            } else {
                format!("{unseen} new suggestions")
            };
            parts.push(
                label(text, LABEL_SM_PX, theme::blueberry_400())
                    .debug_selector(|| "desktop-meeting-suggestions-unseen".to_string())
                    .into_any_element(),
            );
        }
        let open = state.open;
        parts.push(
            web_button(
                "desktop-meeting-suggestions-toggle",
                WebButtonVariant::Ghost,
                WebButtonSize::Default,
                ButtonState::Enabled,
            )
            .aria_label(if open {
                "Hide suggestions"
            } else {
                "Show suggestions"
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_meeting_suggestions(cx)))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                if is_web_button_activation_key(&ev.keystroke.key) {
                    cx.stop_propagation();
                    this.toggle_meeting_suggestions(cx);
                }
            }))
            .child(if open { "Hide" } else { "Suggestions" })
            .into_any_element(),
        );
        Some(parts)
    }

    /// The suggestions panel above the live pill.
    pub(super) fn render_meeting_suggestions(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let state = &self.meeting_suggestions;
        if !state.open || state.prefs.proactivity == Proactivity::Off {
            return None;
        }
        let automatic = state.prefs.proactivity.is_automatic();
        let thinking = state.live.is_thinking();
        let paused = state.live.paused;
        let mut header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(label("Suggestions", LABEL_SM_PX, theme::rambutan_400()));
        let mut actions = div().flex().items_center().gap_1();
        if automatic {
            actions = actions.child(small_action(
                "desktop-meeting-suggestions-pause",
                if paused { "Resume" } else { "Pause" },
                true,
                cx,
                move |this, cx| {
                    this.meeting_suggestions.live.paused = !paused;
                    cx.notify();
                },
            ));
        }
        actions = actions.child(small_action(
            "desktop-meeting-suggestions-now",
            "Suggest now",
            !thinking,
            cx,
            |this, cx| this.suggest_meeting_now(cx),
        ));
        header = header.child(actions);
        let status = match &state.live.status {
            RoundStatus::Thinking => Some("Reading the last two minutes…".to_string()),
            RoundStatus::Failed(message) => Some(message.clone()),
            RoundStatus::Idle if paused => Some("Paused for this meeting.".to_string()),
            RoundStatus::Idle => state.note.clone(),
        };
        let mut panel = div()
            .id("desktop-meeting-suggestions")
            .debug_selector(|| "desktop-meeting-suggestions".to_string())
            .w(px(LIVE_PANEL_WIDTH_PX))
            .max_h(px(LIVE_PANEL_MAX_HEIGHT_PX))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2()
            .rounded(px(8.0))
            .bg(theme::longan_950())
            .border_1()
            .border_color(theme::longan_800())
            .p_2()
            .child(header);
        if let Some(status) = status.filter(|status| !status.is_empty()) {
            panel = panel.child(label(status, LABEL_SM_PX, theme::rambutan_400()));
        }
        let items: Vec<_> = state.live.visible().cloned().collect();
        if items.is_empty() {
            panel = panel.child(body(
                if automatic {
                    "Suggestions appear here after new speech or a pause."
                } else {
                    "Select Suggest now for notes on the last two minutes."
                },
                BODY_SM_PX,
                theme::rambutan_500(),
            ));
        }
        for (index, item) in items.into_iter().enumerate() {
            let added = item.state == SuggestionState::Added;
            let mut buttons = div().flex().flex_wrap().items_center().gap_1();
            if item.kind.is_actionable() && !added {
                let id = item.id.clone();
                buttons = buttons.child(small_action(
                    ("desktop-meeting-suggestion-add", index),
                    "Add task",
                    true,
                    cx,
                    move |this, cx| this.decide_meeting_suggestion(&id, SuggestionState::Added, cx),
                ));
            }
            if added {
                buttons =
                    buttons.child(label("Task added", LABEL_SM_PX, theme::pandan_300()).px_2());
            }
            let id = item.id.clone();
            buttons = buttons.child(small_action(
                ("desktop-meeting-suggestion-copy", index),
                "Copy",
                true,
                cx,
                move |this, cx| this.decide_meeting_suggestion(&id, SuggestionState::Copied, cx),
            ));
            if !added {
                let id = item.id.clone();
                buttons = buttons.child(small_action(
                    ("desktop-meeting-suggestion-dismiss", index),
                    "Dismiss",
                    true,
                    cx,
                    move |this, cx| {
                        this.decide_meeting_suggestion(&id, SuggestionState::Dismissed, cx);
                    },
                ));
            }
            let mut row = div()
                .debug_selector(|| "desktop-meeting-suggestion".to_string())
                .flex()
                .flex_col()
                .gap_1()
                .rounded(px(4.0))
                .bg(theme::longan_900())
                .px_2()
                .py(px(6.0))
                .child(label(
                    item.kind.label(),
                    LABEL_SM_PX,
                    theme::blueberry_400(),
                ))
                .child(body(item.text, BODY_SM_PX, theme::rambutan_100()));
            if !item.reason.is_empty() {
                row = row.child(label(item.reason, LABEL_SM_PX, theme::rambutan_400()));
            }
            panel = panel.child(row.child(buttons));
        }
        Some(panel.into_any_element())
    }
}

#[cfg(debug_assertions)]
impl Workspace {
    /// `BRUHS_DESKTOP_SMOKE_SCENE=meeting-suggestions`: a live meeting with
    /// the panel open and three suggestions, for a comparison screenshot
    /// with the web dock. Nothing here is saved: use a scratch state file.
    pub(in crate::views::workspace) fn smoke_meeting_suggestions(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) {
        use bruhs_models::meeting_suggestions::{ContextKind, Suggestion, ThoughtKind};
        let now = EpochMs::now().as_f64();
        let mut meeting = Meeting::new("Launch sync", now - 120_000.0);
        meeting.status = bruhs_models::MeetingStatus::Live;
        let id = meeting.id.clone();
        self.meetings_hub.update(cx, |hub, cx| {
            hub.meetings.insert(0, meeting);
            hub.live_meeting_id = Some(id.clone());
            cx.notify();
        });
        let state = &mut self.meeting_suggestions;
        state.prefs.proactivity = Proactivity::Automatic;
        state.live.follow(Some(&id));
        let fixtures = [
            (
                ThoughtKind::Question,
                "Should the free tier be kept for this release?",
                "Nobody has decided and it changes the pricing page.",
                5,
                None,
            ),
            (
                ThoughtKind::ActionItem,
                "Find an owner to book the venue for next week's offsite.",
                "It was raised and left unowned.",
                4,
                None,
            ),
            (
                ThoughtKind::Context,
                "Dana sends the revised launch deck by Thursday.",
                "Matches your task Send launch deck.",
                4,
                Some((ContextKind::Task, "task".to_string())),
            ),
        ];
        for (index, (kind, text, reason, score, source)) in fixtures.into_iter().enumerate() {
            state.live.items.push(Suggestion {
                id: format!("smoke-{index}"),
                kind,
                text: text.to_string(),
                reason: reason.to_string(),
                score,
                source,
                created_at_ms: now,
                state: SuggestionState::Seen,
            });
        }
        state.open = true;
        cx.notify();
    }
}

fn small_action(
    id: impl Into<ElementId>,
    text: &'static str,
    enabled: bool,
    cx: &Context<'_, Workspace>,
    run: impl Fn(&mut Workspace, &mut Context<'_, Workspace>) + Clone + 'static,
) -> AnyElement {
    let state = if enabled {
        ButtonState::Enabled
    } else {
        ButtonState::Disabled
    };
    let on_key = run.clone();
    let button = web_button(id, WebButtonVariant::Ghost, WebButtonSize::Small, state)
        .aria_label(text)
        .child(text);
    if !enabled {
        return button.into_any_element();
    }
    button
        .on_click(cx.listener(move |this, _, _, cx| run(this, cx)))
        .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
            if is_web_button_activation_key(&ev.keystroke.key) {
                cx.stop_propagation();
                on_key(this, cx);
            }
        }))
        .into_any_element()
}

/// The same context the web sends: the meeting with its attendees, open
/// tasks, events from a day ago to a week ahead, and people by name.
fn meeting_context(
    meeting: &Meeting,
    hub: &MeetingsHub,
    work_items: &[DesktopWorkItem],
    now: f64,
) -> Vec<ContextItem> {
    let own_event = meeting.calendar_event_id.as_deref().and_then(|id| {
        hub.google_events
            .iter()
            .find(|event| event.id == id || event.google_event_id.as_deref() == Some(id))
    });
    let mut items = vec![ContextItem::meeting(
        &meeting.id,
        &meeting.title,
        &own_event.map(attendee_names).unwrap_or_default(),
    )];
    items.extend(
        hub.google_events
            .iter()
            .filter(|event| {
                own_event.is_none_or(|own| own.id != event.id)
                    && event_in_window(event.start_at_ms, now)
            })
            .map(|event| {
                ContextItem::event(
                    &event.id,
                    &event.title,
                    &when(event.start_at_ms),
                    &attendee_names(event),
                )
            }),
    );
    items.extend(
        work_items
            .iter()
            .filter(|task| {
                !matches!(
                    task.status,
                    DesktopWorkItemStatus::Done | DesktopWorkItemStatus::Canceled
                )
            })
            .map(|task| ContextItem::task(&task.id, &task.title, task.due_date.as_deref())),
    );
    items.extend(
        hub.people
            .iter()
            .map(|person| ContextItem::person(&person.id, &person.display_name)),
    );
    items
}

#[cfg(test)]
mod tests {
    use super::{status_code, when, ServerError};
    use bruhs_models::meeting_suggestions::failure_message;

    #[test]
    fn failed_rounds_map_to_the_shared_words() {
        assert_eq!(
            failure_message(status_code(&ServerError::Invalid("no server"))),
            failure_message(None)
        );
        assert!(!when(1_700_000_000_000.0).is_empty());
        assert_eq!(when(f64::NAN), "");
    }
}
