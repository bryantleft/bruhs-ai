use super::*;
use bruhs_models::{ActivityMonitorSession, ACTIVITY_MONITOR_RENEW_MS};

impl MeetingsHub {
    pub(super) fn set_activity_monitor(&mut self, enabled: bool, cx: &mut Context<'_, Self>) {
        self.monitor_generation = self.monitor_generation.wrapping_add(1);
        let generation = self.monitor_generation;
        if let Some(previous) = self.monitor_session.take() {
            if let Some(command) = previous.cancel() {
                self.monitor_feedback = "Stopping microphone activity checks…".into();
                let client = self.client.clone();
                self.run(async move { client.activity_monitor(Some(&command)).await }, move |hub, result, cx| {
                    if hub.monitor_session.is_none() && hub.monitor_generation == generation {
                        hub.monitor_feedback = match result {
                            Ok(_) => "Microphone activity checks are off.".into(),
                            Err(_) => "Permission renewal stopped. Could not confirm microphone shutdown; the daemon stops checks when its current 30-second permission expires.".into(),
                        };
                        cx.notify();
                    }
                });
            } else {
                self.monitor_feedback = "Microphone activity checks are off.".into();
            }
        }
        if !enabled {
            return;
        }
        let session = Arc::new(ActivityMonitorSession::default());
        self.monitor_session = Some(session.clone());
        self.monitor_feedback = "Requesting microphone activity permission…".into();
        cx.notify();
        // GPUI tests never use a real daemon or microphone. Client protocol
        // behavior is covered separately against synthetic HTTP fixtures.
        if cfg!(test) {
            return;
        }
        let client = self.client.clone();
        let jobs = self.jobs.clone();
        self.runtime.spawn(async move {
            let result = async {
                let status = client.activity_monitor(None).await.map_err(|error| error.to_string())?;
                let Some(command) = session.prepare(status).map_err(|error| error.to_string())? else { return Ok::<_, String>(()); };
                let status = client.activity_monitor(Some(&command)).await.map_err(|error| error.to_string())?;
                session.validate(&status).map_err(|error| error.to_string())?;
                if session.cancelled() { return Ok(()); }
                let active = session.clone();
                let _ = jobs.send(Box::new(move |hub, cx| {
                    if hub.monitor_session.as_ref().is_some_and(|s| Arc::ptr_eq(s, &active)) {
                        hub.monitor_feedback = "Microphone activity checks are on for this app session.".into();
                        cx.notify();
                    }
                }));
                loop {
                    tokio::time::sleep(Duration::from_millis(ACTIVITY_MONITOR_RENEW_MS)).await;
                    let Some(command) = session.renew() else { return Ok(()); };
                    let status = client.activity_monitor(Some(&command)).await.map_err(|error| error.to_string())?;
                    session.validate(&status).map_err(|error| error.to_string())?;
                    if session.cancelled() { return Ok(()); }
                }
            }.await;
            if let Err(error) = result {
                if session.cancelled() { return; }
                let shutdown = if let Some(command) = session.cancel() {
                    client.activity_monitor(Some(&command)).await.map(|_| ())
                } else { Ok(()) };
                let _ = jobs.send(Box::new(move |hub, cx| {
                    if hub.monitor_session.as_ref().is_some_and(|s| Arc::ptr_eq(s, &session)) {
                        hub.monitor_session = None;
                        hub.prefs.auto_record.audio_driven_enabled = false;
                        hub.monitor_feedback = format!("Activity checks stopped: {error} Turn the switch on to retry.");
                        if shutdown.is_err() {
                            hub.monitor_feedback.push_str(" Shutdown could not be confirmed; the current permission expires after 30 seconds without renewal.");
                        }
                        cx.emit(HubEvent::PrefsChanged);
                        cx.notify();
                    }
                }));
            }
        });
    }
}

impl Drop for MeetingsHub {
    fn drop(&mut self) {
        if let Some(command) = self.monitor_session.take().and_then(|s| s.cancel()) {
            let client = self.client.clone();
            self.runtime.spawn(async move {
                let _ = client.activity_monitor(Some(&command)).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, TestAppContext};

    #[gpui::test]
    fn persisted_audio_preference_is_not_microphone_consent(cx: &mut TestAppContext) {
        let runtime = Arc::new(Runtime::new().unwrap());
        let mut prefs = MeetingPrefs::default();
        prefs.auto_record.enabled = true;
        prefs.auto_record.audio_driven_enabled = true;
        let hub = cx.new(|cx| {
            MeetingsHub::new(
                ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap(),
                runtime,
                prefs,
                cx,
            )
        });
        hub.update(cx, |hub, _| {
            assert!(hub.prefs.auto_record.enabled);
            assert!(!hub.prefs.auto_record.audio_driven_enabled);
            assert!(hub.monitor_session.is_none());
        });
    }

    #[gpui::test]
    fn master_off_cancels_pending_permission_and_reenable_does_not_regrant(
        cx: &mut TestAppContext,
    ) {
        let runtime = Arc::new(Runtime::new().unwrap());
        let hub = cx.new(|cx| {
            MeetingsHub::new(
                ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap(),
                runtime,
                MeetingPrefs::default(),
                cx,
            )
        });
        hub.update(cx, |hub, cx| {
            hub.update_prefs(
                |p| {
                    p.auto_record.enabled = true;
                    p.auto_record.audio_driven_enabled = true;
                },
                cx,
            );
            let pending = hub.monitor_session.clone().unwrap();
            hub.update_prefs(|p| p.auto_record.enabled = false, cx);
            assert!(pending.cancelled());
            assert!(hub.monitor_session.is_none());
            assert!(!hub.prefs.auto_record.audio_driven_enabled);
            hub.update_prefs(|p| p.auto_record.enabled = true, cx);
            assert!(hub.monitor_session.is_none());
        });
    }
}
