//! Live meeting suggestions on the web. The shared rules live in
//! `bruhs_models::meeting_suggestions`; this module feeds them the live
//! transcript and the user's tasks, events and people, sends each round to
//! the server, and turns the user's decisions into tasks or clipboard text.

use std::collections::BTreeMap;

use bruhs_models::meeting_suggestions::{
    event_in_window, ContextItem, LiveInput, LiveSuggestions, MeetingSuggestionPrefs, Proactivity,
    Round, SuggestionState,
};
use bruhs_models::{
    CalendarEvent, Meeting, PersonProfile, TranscriptSegment, WorkItem, WorkItemSource,
};
use gloo_timers::callback::Interval;
use leptos::prelude::*;

use crate::api::{self, ApiError};
use crate::state::{
    load_json_pref, save_json_pref, CalendarEventsSignal, LiveMeetingIdSignal,
    MeetingSegmentsSignal, MeetingsSignal, PeopleSignal, SetWorkItemsSignal, WorkItemsSignal,
};

/// Where the preferences are stored.
pub const PREF: &str = "bruhs.meetingSuggestions";
/// How often the automatic loop checks the transcript. The shared rules
/// decide whether a round is due; most ticks do nothing.
const TICK_MS: u32 = 2_000;

#[derive(Copy, Clone)]
pub struct MeetingSuggestions {
    pub prefs: RwSignal<MeetingSuggestionPrefs>,
    pub live: RwSignal<LiveSuggestions>,
    /// A short status for the last decision, such as a failed copy.
    pub note: RwSignal<Option<String>>,
    abort: StoredValue<Option<web_sys::AbortController>, LocalStorage>,
    timer: StoredValue<Option<Interval>, LocalStorage>,
    live_id: ReadSignal<Option<String>>,
    meetings: ReadSignal<Vec<Meeting>>,
    segments: ReadSignal<BTreeMap<String, Vec<TranscriptSegment>>>,
    work_items: ReadSignal<Vec<WorkItem>>,
    set_work_items: WriteSignal<Vec<WorkItem>>,
    events: ReadSignal<Vec<CalendarEvent>>,
    people: ReadSignal<Vec<PersonProfile>>,
}

pub fn stored_prefs() -> MeetingSuggestionPrefs {
    load_json_pref::<MeetingSuggestionPrefs>(PREF)
        .unwrap_or_default()
        .sanitized()
}

impl MeetingSuggestions {
    /// Needs the meetings, tasks and calendar state.
    pub fn install() {
        let this = Self {
            prefs: RwSignal::new(stored_prefs()),
            live: RwSignal::new(LiveSuggestions::default()),
            note: RwSignal::new(None),
            abort: StoredValue::new_local(None),
            timer: StoredValue::new_local(None),
            live_id: expect_context::<LiveMeetingIdSignal>().0,
            meetings: expect_context::<MeetingsSignal>().0,
            segments: expect_context::<MeetingSegmentsSignal>().0,
            work_items: expect_context::<WorkItemsSignal>().0,
            set_work_items: expect_context::<SetWorkItemsSignal>().0,
            events: expect_context::<CalendarEventsSignal>().0,
            people: expect_context::<PeopleSignal>().0,
        };
        let prefs = this.prefs;
        Effect::new(move |_| prefs.with(|prefs| save_json_pref(PREF, prefs)));
        // A different meeting starts a fresh session and cancels a round in
        // flight; a late reply could otherwise land on the next meeting.
        Effect::new(move |_| {
            let id = this.live_id.get();
            let changed = this
                .live
                .with_untracked(|live| live.meeting_id() != id.as_deref());
            if changed {
                this.stop();
                this.live.update(|live| live.follow(id.as_deref()));
                this.note.set(None);
                if id.is_some() {
                    this.prefs.update(|prefs| {
                        prefs.calibration.relax();
                    });
                }
            }
        });
        // The timer runs only while a meeting is live and suggestions are
        // automatic. Turning them off also cancels a running round. Only the
        // level matters here: decisions rewrite calibration in the same prefs.
        let proactivity = Memo::new(move |_| this.prefs.with(|prefs| prefs.proactivity));
        Effect::new(move |_| {
            let live = this.live_id.with(Option::is_some);
            let level = proactivity.get();
            this.timer.set_value(None);
            if level == Proactivity::Off {
                this.stop();
            }
            if live && level.is_automatic() {
                this.timer
                    .set_value(Some(Interval::new(TICK_MS, move || this.tick())));
            }
        });
        on_cleanup(move || {
            this.timer.set_value(None);
            this.stop();
        });
        provide_context(this);
    }

    fn stop(self) {
        self.abort.update_value(|abort| {
            if let Some(controller) = abort.take() {
                controller.abort();
            }
        });
        // An aborted round must not leave the session waiting forever.
        if self.live.with_untracked(LiveSuggestions::is_thinking) {
            self.live.update(LiveSuggestions::cancel);
        }
    }

    fn tick(self) {
        let Some(id) = self.live_id.get_untracked() else {
            return;
        };
        let round = self.with_meeting(&id, |input, context| {
            let mut round = None;
            self.live
                .update_untracked(|live| round = live.poll(input, context));
            round
        });
        if let Some(round) = round {
            self.live.notify();
            self.send(round);
        }
    }

    /// Suggest now, from the dock.
    pub fn suggest_now(self) {
        let Some(id) = self.live_id.get_untracked() else {
            return;
        };
        let round = self.with_meeting(&id, |input, context| {
            let mut round = None;
            self.live
                .update(|live| round = live.request(input, context()));
            round
        });
        if let Some(round) = round {
            self.send(round);
        }
    }

    pub fn set_paused(self, paused: bool) {
        self.live.update(|live| live.paused = paused);
    }

    pub fn mark_seen(self) {
        if self.live.with_untracked(|live| live.unseen() > 0) {
            self.live.update(|live| {
                live.mark_seen();
            });
        }
    }

    fn with_meeting<T>(
        self,
        id: &str,
        run: impl FnOnce(&LiveInput<'_>, &dyn Fn() -> Vec<ContextItem>) -> Option<T>,
    ) -> Option<T> {
        let meeting = self
            .meetings
            .with_untracked(|meetings| meetings.iter().find(|m| m.id == id).cloned())?;
        let now = js_sys::Date::now();
        self.prefs.with_untracked(|prefs| {
            self.segments.with_untracked(|map| {
                let segments = map.get(id).map_or(&[][..], Vec::as_slice);
                let input = LiveInput {
                    now,
                    prefs,
                    meeting_title: &meeting.title,
                    segments,
                    speakers: &meeting.speakers,
                };
                run(&input, &|| self.context(&meeting, now))
            })
        })
    }

    fn context(self, meeting: &Meeting, now: f64) -> Vec<ContextItem> {
        let own_event = meeting.calendar_event_id.as_deref();
        let mut items = Vec::new();
        self.events.with_untracked(|events| {
            let event = own_event.and_then(|id| {
                events
                    .iter()
                    .find(|event| event.id == id || event.google_event_id.as_deref() == Some(id))
            });
            items.push(ContextItem::meeting(
                &meeting.id,
                &meeting.title,
                &event.map(attendee_names).unwrap_or_default(),
            ));
            items.extend(
                events
                    .iter()
                    .filter(|candidate| {
                        event.is_none_or(|own| own.id != candidate.id)
                            && event_in_window(candidate.start_at_ms, now)
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
        });
        self.work_items.with_untracked(|tasks| {
            items.extend(
                tasks
                    .iter()
                    .filter(|task| {
                        !matches!(
                            task.status,
                            bruhs_models::WorkItemStatus::Done
                                | bruhs_models::WorkItemStatus::Canceled
                        )
                    })
                    .map(|task| ContextItem::task(&task.id, &task.title, task.due_date.as_deref())),
            );
        });
        self.people.with_untracked(|people| {
            items.extend(
                people
                    .iter()
                    .map(|person| ContextItem::person(&person.id, &person.display_name)),
            );
        });
        items
    }

    fn send(self, round: Round) {
        let Ok(controller) = web_sys::AbortController::new() else {
            self.live.update(|live| {
                live.fail(&round, js_sys::Date::now(), Some(500));
            });
            return;
        };
        let signal = controller.signal();
        self.abort.update_value(|abort| {
            if let Some(previous) = abort.replace(controller) {
                previous.abort();
            }
        });
        wasm_bindgen_futures::spawn_local(async move {
            let result = api::meeting_suggestion_round(&round.request, &signal).await;
            let now = js_sys::Date::now();
            match result {
                Ok(reply) => {
                    let threshold = self
                        .prefs
                        .with_untracked(|prefs| prefs.calibration.threshold);
                    self.live.update(|live| {
                        live.finish(&round, reply, threshold, now, || {
                            uuid::Uuid::new_v4().to_string()
                        });
                    });
                }
                Err(ApiError::Aborted) => {}
                Err(error) => {
                    self.live
                        .update(|live| live.fail(&round, now, failure_status(&error)));
                }
            }
        });
    }

    /// Add task, Copy or Dismiss. Each decision is final and tunes how much
    /// a thought must score before it is shown.
    pub fn decide(self, id: &str, state: SuggestionState) {
        // The suggestion session owns its source, even if capture has already
        // switched to another meeting before its UI effect runs.
        let meeting_id = self
            .live
            .with_untracked(|live| live.meeting_id().map(str::to_string));
        let Some(decision) = self
            .live
            .try_update(|live| live.decide(id, state))
            .flatten()
        else {
            return;
        };
        let suggestion = decision.suggestion;
        if let Some(kept) = decision.kept {
            self.prefs.update(|prefs| {
                prefs.calibration.record(kept);
            });
        }
        match state {
            SuggestionState::Added => {
                let meeting = meeting_id.as_ref().and_then(|id| {
                    self.meetings.with_untracked(|meetings| {
                        meetings
                            .iter()
                            .find(|m| &m.id == id)
                            .map(|m| m.title.clone())
                    })
                });
                let mut task = WorkItem::new_at(&suggestion.text, crate::clock::now_ms());
                task.description = Some(match &meeting {
                    Some(title) => format!("Suggested during the meeting {title}."),
                    None => "Suggested during a meeting.".to_string(),
                });
                task.source = meeting_id.map(|meeting_id| WorkItemSource::Meeting {
                    meeting_id,
                    title: meeting.unwrap_or_else(|| "Meeting".into()),
                });
                self.set_work_items.update(|tasks| tasks.push(task.clone()));
                crate::storage::persist_work_item(None, task);
                self.note.set(Some("Task added.".into()));
            }
            SuggestionState::Copied => {
                let note = self.note;
                wasm_bindgen_futures::spawn_local(async move {
                    let copied = crate::components::meeting_live::write_clipboard(&suggestion.text)
                        .await
                        .is_ok();
                    note.set(Some(if copied {
                        "Copied.".into()
                    } else {
                        "Could not copy that suggestion.".into()
                    }));
                });
            }
            SuggestionState::Dismissed | SuggestionState::New | SuggestionState::Seen => {
                self.note.set(None);
            }
        }
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

/// A short local label such as "Thu 14:00".
fn when(start_at_ms: f64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(start_at_ms));
    let day = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
        .get(usize::try_from(date.get_day()).unwrap_or(0))
        .copied()
        .unwrap_or("");
    format!("{day} {:02}:{:02}", date.get_hours(), date.get_minutes())
}

/// The HTTP status a failed round reports to the shared rules; None when
/// the server could not be reached.
fn failure_status(error: &ApiError) -> Option<u16> {
    match error {
        ApiError::Http { status, .. } => Some(*status),
        ApiError::Timeout { .. } => Some(504),
        ApiError::Decode(_) => Some(502),
        ApiError::Network(_) | ApiError::Aborted => None,
    }
}
