//! A per-meeting, nonpersistent opt-in. No AX tree, windows, titles or URLs.
use super::*;
use bruhs_models::{LocalFocus, LocalFocusObservation};
use std::time::Instant;

const POLL: Duration = Duration::from_secs(2);
const MAX_GAP: Duration = Duration::from_secs(10);

#[derive(Default)]
pub struct FocusState {
    session: Option<Session>,
    task: Option<gpui::Task<()>>,
    epoch: u64,
    pub rows: BTreeMap<String, Vec<LocalFocusObservation>>,
    loading: HashSet<String>,
    pub load_errors: BTreeMap<String, String>,
    pub feedback: String,
}

struct Session {
    meeting_id: String,
    last_tick: Instant,
    last_wall_ms: i64,
    last: Option<LocalFocusObservation>,
    pending: bool,
}

impl FocusState {
    pub fn enabled_for(&self, id: &str) -> bool {
        self.session.as_ref().is_some_and(|s| s.meeting_id == id)
    }

    pub fn current_label(&self, id: &str) -> String {
        let Some(session) = self.session.as_ref().filter(|s| s.meeting_id == id) else {
            return "Local focus: off".into();
        };
        if session.last_tick.elapsed() > MAX_GAP {
            return "Local focus: stale / unavailable".into();
        }
        session.last.as_ref().map_or_else(
            || "Local focus: waiting for observation".into(),
            LocalFocusObservation::label,
        )
    }

    /// Runs the sampler only inside a current, unexpired session; at most one
    /// network write is outstanding. A stalled writer never builds a queue.
    fn sample(
        &mut self,
        live_id: Option<&str>,
        now: Instant,
        at_ms: i64,
        probe: impl FnOnce() -> LocalFocus,
    ) -> Option<LocalFocusObservation> {
        let session = self.session.as_mut()?;
        if live_id != Some(session.meeting_id.as_str())
            || now.duration_since(session.last_tick) > MAX_GAP
            || at_ms
                .checked_sub(session.last_wall_ms)
                .is_none_or(|gap| !(0..=10_000).contains(&gap))
        {
            self.session = None;
            self.feedback = "Local app context paused after a session change or polling gap. Enable it again to resume.".into();
            return None;
        }
        if session.pending {
            return None;
        }
        session.last_tick = now;
        session.last_wall_ms = at_ms;
        let focus = probe();
        if let Some(last) = session.last.as_mut().filter(|last| last.focus == focus) {
            last.observed_at_ms = at_ms;
            return None;
        }
        let observation = LocalFocusObservation {
            id: uuid::Uuid::new_v4().to_string(),
            meeting_id: session.meeting_id.clone(),
            observed_at_ms: at_ms,
            focus,
        };
        if !observation.is_valid() {
            self.session = None;
            self.feedback = "Local app context paused: invalid app metadata.".into();
            return None;
        }
        session.last = Some(observation.clone());
        session.pending = true;
        Some(observation)
    }
}

impl MeetingsHub {
    pub fn stop_local_focus(&mut self, message: &str, cx: &mut Context<'_, Self>) {
        self.local_focus.epoch = self.local_focus.epoch.wrapping_add(1);
        self.local_focus.session = None;
        self.local_focus.task = None;
        self.local_focus.feedback = message.into();
        cx.notify();
    }

    pub fn toggle_local_focus(&mut self, cx: &mut Context<'_, Self>) {
        if self.local_focus.session.is_some() {
            self.stop_local_focus(
                "Local app context is off. Previously saved observations remain in this meeting.",
                cx,
            );
            return;
        }
        let Some(id) = self.live_meeting_id.clone() else {
            return;
        };
        if self.auto_record_in_flight || self.capture_stopped_session.as_ref() == Some(&id) {
            return;
        }
        let recording = self
            .capture_status
            .recording()
            .map(|recording| recording.meeting_id.as_str());
        if recording != Some(id.as_str()) {
            self.local_focus.feedback =
                "Start capture for this meeting before enabling local app context.".into();
            cx.notify();
            return;
        }
        if !cfg!(target_os = "macos") {
            self.local_focus.feedback =
                "Local app context is currently supported on macOS only.".into();
            cx.notify();
            return;
        }
        if let Err(error) = self.client.local_context_url(&id) {
            self.local_focus.feedback = error.to_string();
            cx.notify();
            return;
        }
        self.local_focus.epoch = self.local_focus.epoch.wrapping_add(1);
        self.local_focus.session = Some(Session {
            meeting_id: id,
            last_tick: Instant::now(),
            last_wall_ms: EpochMs::now().as_millis(),
            last: None,
            pending: false,
        });
        self.local_focus.feedback =
            "On for this meeting only. App changes stay local; no screenshots, titles or URLs."
                .into();
        let epoch = self.local_focus.epoch;
        self.local_focus.task = Some(cx.spawn(async move |this, cx: &mut AsyncApp| loop {
            cx.background_executor().timer(POLL).await;
            let keep_running = this.update(cx, |hub, cx| {
                if hub.local_focus.epoch != epoch { return false; }
                let observation = hub.local_focus.sample(hub.live_meeting_id.as_deref(), Instant::now(), EpochMs::now().as_millis(), probe);
                let changed = observation.is_some() || hub.local_focus.session.is_none();
                if let Some(observation) = observation {
                    let client = hub.client.clone();
                    let saved = observation.clone();
                    hub.run(async move { client.append_local_focus(&observation).await }, move |hub, result, cx| {
                        // A previously authorized observation may finish saving after Off.
                        // It cannot restart tracking or change a newer session's feedback.
                        if result.is_ok() && !hub.deleted_meeting_ids.contains(&saved.meeting_id) {
                            let rows = hub.local_focus.rows.entry(saved.meeting_id.clone()).or_default();
                            if !rows.iter().any(|r| r.id == saved.id) {
                                let index = rows.partition_point(|row| (row.observed_at_ms, &row.id) < (saved.observed_at_ms, &saved.id));
                                rows.insert(index, saved);
                            }
                        }
                        if hub.local_focus.epoch == epoch {
                            match result {
                                Ok(()) => if let Some(session) = &mut hub.local_focus.session { session.pending = false; },
                                Err(_) => hub.stop_local_focus("Local app context paused: observation could not be saved. Check the local server, then enable again.", cx),
                            }
                        }
                        cx.notify();
                    });
                }
                if changed { cx.notify(); }
                hub.local_focus.session.is_some()
            }).unwrap_or(false);
            if !keep_running { break; }
        }));
        cx.notify();
    }

    pub fn load_local_focus(&mut self, id: &str) {
        if cfg!(test)
            || self.client.local_context_url(id).is_err()
            || !self.local_focus.loading.insert(id.into())
        {
            return;
        }
        let key = id.to_owned();
        let request_id = key.clone();
        let client = self.client.clone();
        self.run(
            async move { client.list_local_focus(&request_id).await },
            move |hub, result, cx| {
                hub.local_focus.loading.remove(&key);
                if hub.deleted_meeting_ids.contains(&key) {
                    return;
                }
                match result {
                    Ok(rows)
                        if rows.len() <= bruhs_models::MAX_LOCAL_FOCUS_OBSERVATIONS
                            && rows.iter().all(|r| r.is_valid() && r.meeting_id == key) =>
                    {
                        hub.local_focus.load_errors.remove(&key);
                        let saved = hub.local_focus.rows.entry(key).or_default();
                        let mut merged: BTreeMap<_, _> =
                            rows.into_iter().map(|row| (row.id.clone(), row)).collect();
                        for row in saved.drain(..) {
                            merged.insert(row.id.clone(), row);
                        }
                        *saved = merged.into_values().collect();
                        saved.sort_by(|a, b| {
                            (a.observed_at_ms, &a.id).cmp(&(b.observed_at_ms, &b.id))
                        });
                    }
                    _ => { hub.local_focus.load_errors.insert(key,
                        "Saved local app context could not be loaded. Reopen the meeting to retry.".into()); },
                }
                cx.notify();
            },
        );
    }
}

// Never invoked in tests, even if a synthetic UI test clicks the opt-in.
#[cfg(all(target_os = "macos", not(test)))]
fn probe() -> LocalFocus {
    use objc2_app_kit::NSWorkspace;
    let Some(_main_thread) = objc2::MainThreadMarker::new() else {
        return LocalFocus::Unavailable {};
    };
    let Some(app) = NSWorkspace::sharedWorkspace().frontmostApplication() else {
        return LocalFocus::Unavailable {};
    };
    let Some(name) = app.localizedName() else {
        return LocalFocus::Unavailable {};
    };
    LocalFocus::Available {
        app_name: name.to_string(),
        bundle_id: app.bundleIdentifier().map(|id| id.to_string()),
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn probe() -> LocalFocus {
    LocalFocus::Unavailable {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext;

    #[gpui::test]
    fn local_focus_consent_is_session_bound_and_end_drops_the_task(cx: &mut gpui::TestAppContext) {
        let runtime = Arc::new(Runtime::new().unwrap());
        let hub = cx.new(|cx| {
            let mut hub = MeetingsHub::new(
                ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap(),
                runtime,
                MeetingPrefs::default(),
                cx,
            );
            let meeting = Meeting::new("Synthetic consent only", 1.0);
            hub.live_meeting_id = Some(meeting.id.clone());
            hub.meetings.push(meeting);
            hub
        });
        hub.update(cx, |hub, cx| {
            let id = hub.live_meeting_id.clone().unwrap();
            hub.toggle_local_focus(cx);
            assert!(
                !hub.local_focus.enabled_for(&id),
                "unconfirmed capture must not authorize focus"
            );
            hub.mark_capture_recording(&id, EpochMs::now().as_f64());
            hub.toggle_local_focus(cx);
            if cfg!(target_os = "macos") {
                assert!(hub.local_focus.enabled_for(&id));
                assert!(hub.local_focus.task.is_some());
            }
            // Revoke before any timer can run: no app reads or network requests.
            hub.apply_meeting_event(
                MeetingEvent::Ended {
                    meeting_id: id.clone(),
                    ended_at_ms: 2.0,
                    status: MeetingStatus::Ended,
                },
                cx,
            );
            assert!(!hub.local_focus.enabled_for(&id));
            assert!(hub.local_focus.task.is_none());
            assert!(hub.local_focus.rows.is_empty());
            hub.live_meeting_id = Some("new-session".into());
            assert!(!hub.local_focus.enabled_for("new-session"));
        });
        cx.run_until_parked();
    }

    fn enabled(now: Instant) -> FocusState {
        FocusState {
            session: Some(Session {
                meeting_id: "meeting".into(),
                last_tick: now,
                last_wall_ms: 1,
                last: None,
                pending: false,
            }),
            ..Default::default()
        }
    }
    #[test]
    fn disabled_changed_session_or_sleep_never_probe() {
        let now = Instant::now();
        let forbidden = || panic!("must not inspect the desktop");
        assert!(FocusState::default()
            .sample(Some("meeting"), now, 1, forbidden)
            .is_none());
        assert!(enabled(now)
            .sample(Some("new-meeting"), now, 1, forbidden)
            .is_none());
        let mut state = enabled(now);
        assert!(state
            .sample(Some("meeting"), now + MAX_GAP + POLL, 1, forbidden)
            .is_none());
        assert!(!state.enabled_for("meeting"));
        assert!(enabled(now)
            .sample(Some("meeting"), now, 20_000, forbidden)
            .is_none());
        assert!(enabled(now)
            .sample(Some("meeting"), now, -1, forbidden)
            .is_none());
    }
    #[test]
    fn bounded_writer_and_change_only_history_do_not_reuse_previous_app() {
        let now = Instant::now();
        let mut state = enabled(now);
        let chrome = || LocalFocus::Available {
            app_name: "Chrome".into(),
            bundle_id: Some("com.google.Chrome".into()),
        };
        assert!(state.sample(Some("meeting"), now, 1, chrome).is_some());
        assert!(state
            .sample(Some("meeting"), now, 2, || panic!("one pending write only"))
            .is_none());
        state.session.as_mut().unwrap().pending = false;
        assert!(state.sample(Some("meeting"), now, 3, chrome).is_none());
        assert!(state
            .sample(Some("meeting"), now, 4, || LocalFocus::Unavailable {})
            .is_some());
        assert_eq!(state.current_label("meeting"), "Local focus: unavailable");
    }
}
