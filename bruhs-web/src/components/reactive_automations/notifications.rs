//! How agent updates reach the owner outside the app, and whether agents keep
//! running after the app quits.
use super::calendar::local_time_zone;
use super::{Check, Field, CARD};
use crate::ui::{Button, ButtonSize};
use bruhs_models::api::{BackgroundService, BackgroundServiceAction};
use bruhs_models::reactive::*;
use leptos::prelude::*;

/// The settings the form describes. Times and zone are checked by the server.
pub(super) fn notification_settings(
    system_notify: bool,
    system_ask: bool,
    quiet: bool,
    start: &str,
    end: &str,
    time_zone: &str,
) -> NotificationSettings {
    NotificationSettings {
        system_notify,
        system_ask,
        quiet_hours: quiet.then(|| QuietHours {
            start: start.trim().to_owned(),
            end: end.trim().to_owned(),
            time_zone: time_zone.trim().to_owned(),
        }),
    }
}

/// The background service button: its label and the action it takes.
pub(super) fn service_action(
    service: &BackgroundService,
) -> Option<(&'static str, BackgroundServiceAction)> {
    if !service.supported {
        return None;
    }
    Some(if service.installed {
        (
            "Stop running after quit",
            BackgroundServiceAction::Uninstall,
        )
    } else {
        (
            "Keep agents running after quit",
            BackgroundServiceAction::Install,
        )
    })
}

#[component]
pub(super) fn NotificationsPanel(
    snapshot: RwSignal<Option<ReactiveSnapshot>>,
    busy: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    send: Callback<ReactiveCommand>,
) -> impl IntoView {
    // The server's validation message is repeated here while it answers
    // this form's last save; any other command clears the link.
    let submitted = RwSignal::new(false);
    let sending = StoredValue::new(false);
    Effect::new(move |_| {
        if busy.get() && !sending.get_value() {
            submitted.set(false);
        }
        sending.set_value(false);
    });
    let system_notify = RwSignal::new(false);
    let system_ask = RwSignal::new(false);
    let quiet = RwSignal::new(false);
    let start = RwSignal::new("22:00".to_string());
    let end = RwSignal::new("07:00".to_string());
    let zone = RwSignal::new(local_time_zone());
    // The form follows the server's settings whenever they change there, so
    // the regular refresh never overwrites edits in progress.
    let loaded = StoredValue::new(None::<NotificationSettings>);
    Effect::new(move |_| {
        let Some(settings) = snapshot.with(|s| s.as_ref().map(|s| s.notification_settings.clone()))
        else {
            return;
        };
        if loaded.with_value(|loaded| loaded.as_ref() == Some(&settings)) {
            return;
        }
        system_notify.set(settings.system_notify);
        system_ask.set(settings.system_ask);
        quiet.set(settings.quiet_hours.is_some());
        if let Some(hours) = &settings.quiet_hours {
            start.set(hours.start.clone());
            end.set(hours.end.clone());
            zone.set(hours.time_zone.clone());
        }
        loaded.set_value(Some(settings));
    });
    view! {
        <section class=CARD aria-labelledby="agent-notifications-title">
            <h3 id="agent-notifications-title" class="text-label text-rambutan-100">"Notifications"</h3>
            <div class="mt-2 space-y-1">
                <Check label="System notification for updates" value=system_notify />
                <Check label="System notification for questions" value=system_ask />
                <Check label="Quiet hours" value=quiet />
            </div>
            <Show when=move || quiet.get()>
                <div class="mt-2 grid gap-3 sm:grid-cols-3">
                    <Field label="Start" value=start />
                    <Field label="End" value=end />
                    <Field label="Time zone" value=zone />
                </div>
            </Show>
            <p class="mt-2 text-label-sm text-rambutan-500">"Sent by the local server, so they arrive while the app is closed when the background service is on. Off by default."</p>
            <div class="mt-3">
                <Button size=ButtonSize::Small on_click=Callback::new(move |_| {
                    sending.set_value(true);
                    submitted.set(true);
                    send.run(ReactiveCommand::SetNotificationSettings {
                        settings: notification_settings(system_notify.get_untracked(), system_ask.get_untracked(), quiet.get_untracked(), &start.get_untracked(), &end.get_untracked(), &zone.get_untracked()),
                    });
                })>"Save notification settings"</Button>
            </div>
            {move || (submitted.get() && !busy.get()).then(|| error.get()).flatten().map(|message| view! {
                <p role="alert" class="mt-2 break-words text-body-sm text-lychee-200">{message}</p>
            })}
        </section>
    }
}

#[component]
pub(super) fn BackgroundServicePanel() -> impl IntoView {
    let service = RwSignal::new(None::<BackgroundService>);
    let pending = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let alive = RwSignal::new(true);
    on_cleanup(move || alive.set(false));
    pending.set(true);
    wasm_bindgen_futures::spawn_local(async move {
        let result = crate::api::background_service().await;
        if alive.try_get_untracked() != Some(true) {
            return;
        }
        pending.set(false);
        match result {
            Ok(next) => service.set(Some(next)),
            Err(err) => error.set(Some(err.to_string())),
        }
    });
    let act = Callback::new(move |action: BackgroundServiceAction| {
        if pending.get_untracked() {
            return;
        }
        pending.set(true);
        error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::set_background_service(action).await;
            if alive.try_get_untracked() != Some(true) {
                return;
            }
            pending.set(false);
            match result {
                Ok(next) => service.set(Some(next)),
                Err(err) => error.set(Some(err.to_string())),
            }
        });
    });
    view! {
        <section class=CARD aria-labelledby="background-service-title" aria-busy=move || pending.get().to_string()>
            <h3 id="background-service-title" class="text-label text-rambutan-100">"Background service"</h3>
            {move || service.get().map(|service| {
                let action = service_action(&service);
                view! {
                    <p role="status" class="mt-2 break-words text-body-sm text-rambutan-300">{service.detail.clone()}</p>
                    {action.map(|(label, action)| view! {
                        <div class="mt-3">
                            <Button size=ButtonSize::Small disabled=Signal::derive(move || pending.get()) on_click=Callback::new(move |_| act.run(action))>{label}</Button>
                        </div>
                    })}
                }
            })}
            {move || error.get().map(|message| view! { <p role="alert" class="mt-2 break-words text-body-sm text-lychee-200">{message}</p> })}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_hours_are_sent_only_when_enabled() {
        let off = notification_settings(true, false, false, "22:00", "07:00", "UTC");
        assert_eq!(off.quiet_hours, None);
        assert!(off.system_notify && !off.system_ask);
        let on = notification_settings(false, true, true, " 22:00 ", "07:30", " Europe/Paris ");
        assert_eq!(
            on.quiet_hours,
            Some(QuietHours {
                start: "22:00".into(),
                end: "07:30".into(),
                time_zone: "Europe/Paris".into(),
            })
        );
        let wire = serde_json::to_value(ReactiveCommand::SetNotificationSettings { settings: on })
            .unwrap();
        assert_eq!(wire["action"], "set_notification_settings");
        assert_eq!(wire["settings"]["quiet_hours"]["start"], "22:00");
    }

    #[test]
    fn the_service_button_installs_or_uninstalls() {
        let mut service = BackgroundService {
            supported: true,
            ..BackgroundService::default()
        };
        assert_eq!(
            service_action(&service),
            Some((
                "Keep agents running after quit",
                BackgroundServiceAction::Install
            ))
        );
        service.installed = true;
        assert_eq!(
            service_action(&service),
            Some((
                "Stop running after quit",
                BackgroundServiceAction::Uninstall
            ))
        );
        service.supported = false;
        assert_eq!(service_action(&service), None);
        assert_eq!(
            serde_json::to_value(BackgroundServiceAction::Install).unwrap(),
            serde_json::json!({"action": "install"})
        );
    }
}
