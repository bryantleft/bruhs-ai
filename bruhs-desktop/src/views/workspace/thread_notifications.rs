//! Device-local alerts for interactive agent threads.
use super::*;

pub(super) const MODE_ID: &str = "settings-thread-notifications";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum NotificationMode {
    #[default]
    Off,
    Notifications,
    Sound,
    NotificationsAndSound,
}

impl NotificationMode {
    pub(super) const OPTIONS: [(Self, &'static str, &'static str); 4] = [
        (Self::Off, "off", "Off"),
        (Self::Notifications, "notifications", "Notifications"),
        (Self::Sound, "sound", "Sound"),
        (
            Self::NotificationsAndSound,
            "notifications-and-sound",
            "Notifications and sound",
        ),
    ];
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct NotificationSettings {
    pub(super) mode: NotificationMode,
    pub(super) in_app: bool,
}

impl Workspace {
    pub(super) fn notify_thread(
        &self,
        chat: &Entity<ChatView>,
        reason: &'static str,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        let prefs = self.thread_notifications;
        if prefs == NotificationSettings::default() {
            return;
        }
        let Some(thread) = self.threads.iter().find(|thread| thread.chat == *chat) else {
            return;
        };
        let title = chat.read(cx).thread_title.clone();
        if matches!(
            prefs.mode,
            NotificationMode::Sound | NotificationMode::NotificationsAndSound
        ) {
            window.play_system_bell();
        }
        if window.is_window_active() {
            let reading_thread = self.active_view == AppView::Chat
                && self.active_thread_id.as_deref() == Some(&thread.id);
            if prefs.in_app && !reading_thread {
                let action =
                    crate::views::meetings_hub::NoticeAction::OpenThread(thread.id.clone());
                self.meetings_hub.update(cx, |hub, cx| {
                    hub.show_notice_with_action(format!("{reason} · {title}"), action, cx)
                });
            }
        } else if matches!(
            prefs.mode,
            NotificationMode::Notifications | NotificationMode::NotificationsAndSound
        ) {
            cx.background_spawn(async move {
                if let Err(error) = bruhs_core::reactive::system_notifier()(reason, &title) {
                    tracing::warn!("Could not deliver thread notification: {error}");
                }
            })
            .detach();
        }
    }

    pub(super) fn thread_notification_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let modes = NotificationMode::OPTIONS;
        let current = modes
            .iter()
            .find(|(mode, _, _)| *mode == self.thread_notifications.mode)
            .unwrap()
            .1;
        let options = modes
            .into_iter()
            .map(|(_, value, label)| DropdownOption {
                value,
                label: self.ui_language().text(label).into(),
                description: None,
            })
            .collect();
        let (toggle, select) = self.settings_dropdown_handlers(MODE_ID, cx);
        vec![
            self.settings_row("desktop-settings-row-thread-notifications", "Thread notifications",
                "Alert when a thread finishes, fails, or needs input.",
                settings_ui::control_column(dropdown(MODE_ID, self.ui_language().text("Thread notifications"), current, vec![DropdownGroup { label: None, mark: None, options }],self.settings_listbox(MODE_ID), toggle, select))),
            self.settings_row("desktop-settings-row-in-app-notifications", "In-app notifications",
                "Show an in-app alert when another thread finishes.",
                settings_ui::switch("settings-in-app-notifications", self.thread_notifications.in_app,false,self.ui_language().text("In-app notifications"),cx,|this,_,cx| {
                    this.thread_notifications.in_app = !this.thread_notifications.in_app;
                    this.persist_and_notify(cx);
                })),
        ]
    }
}
