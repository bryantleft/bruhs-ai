//! Delivers outbox items as system notifications, so updates reach the owner
//! while no app window is open. Quiet hours hold delivery; a burst becomes one
//! digest. Every attempt's result is recorded on the item.

use super::{clock, store, ReactiveError, ReactiveRuntime};
use bruhs_models::{
    reactive::{OutboxItem, OutboxKind, SystemDelivery},
    EpochMs,
};
use std::{sync::Arc, time::Duration};

/// How often pending deliveries are checked when nothing changes.
const POLL: Duration = Duration::from_secs(30);
/// Short pause so a burst of updates from one batch becomes one notification.
const SETTLE: Duration = Duration::from_secs(3);

/// Shows one notification. Returns a plain error when the system refuses.
pub type Notifier = Arc<dyn Fn(&str, &str) -> Result<(), String> + Send + Sync>;

pub(super) fn start(runtime: ReactiveRuntime) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut changes = runtime.watch_changes();
        loop {
            if let Err(error) = deliver_pending(&runtime).await {
                tracing::warn!("Agent notifications were not delivered: {error}");
            }
            tokio::select! {
                _ = tokio::time::sleep(POLL) => {}
                changed = changes.changed() => {
                    if changed.is_err() { return; }
                    tokio::time::sleep(SETTLE).await;
                }
            }
        }
    })
}

/// One delivery pass. Public to the runtime for tests.
pub(super) async fn deliver_pending(runtime: &ReactiveRuntime) -> Result<usize, ReactiveError> {
    let (items, settings) = runtime
        .call(|connection| {
            Ok((
                store::pending_deliveries(connection)?,
                store::snapshot_settings(connection)?,
            ))
        })
        .await?;
    if items.is_empty() {
        return Ok(0);
    }
    if let Some(hours) = &settings.quiet_hours {
        if clock::in_quiet_hours(hours, EpochMs::now().as_millis())? {
            return Ok(0);
        }
    }
    let (title, body) = digest(&items);
    let notifier = runtime.notifier();
    let outcome = tokio::task::spawn_blocking(move || notifier(&title, &body))
        .await
        .unwrap_or_else(|_| Err("The notification helper stopped.".into()));
    let delivery = match outcome {
        Ok(()) => SystemDelivery::Delivered { at: EpochMs::now() },
        Err(error) => SystemDelivery::Failed { error },
    };
    let ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();
    let count = ids.len();
    runtime
        .call(move |connection| {
            store::transaction(connection, |connection| {
                store::record_delivery(connection, &ids, &delivery)
            })
        })
        .await?;
    runtime.changed();
    Ok(count)
}

fn digest(items: &[OutboxItem]) -> (String, String) {
    if let [item] = items {
        let title = match item.kind {
            OutboxKind::Ask => format!("Question: {}", item.title),
            OutboxKind::Notify => item.title.clone(),
        };
        return (title, item.message.chars().take(240).collect());
    }
    let questions = items
        .iter()
        .filter(|item| item.kind == OutboxKind::Ask)
        .count();
    let title = if questions > 0 {
        format!("{} agent updates ({questions} need an answer)", items.len())
    } else {
        format!("{} agent updates", items.len())
    };
    let body = items
        .iter()
        .take(4)
        .map(|item| item.title.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    (title, body)
}

/// The platform's notification command. Text is passed as arguments or
/// environment variables, never interpolated into script text.
pub fn system_notifier() -> Notifier {
    Arc::new(|title: &str, body: &str| {
        let output = if cfg!(target_os = "macos") {
            std::process::Command::new("osascript")
                .args([
                    "-e",
                    "on run argv",
                    "-e",
                    "display notification (item 2 of argv) with title \"bruhs\" subtitle (item 1 of argv)",
                    "-e",
                    "end run",
                    "--",
                    title,
                    body,
                ])
                .output()
        } else if cfg!(target_os = "linux") {
            std::process::Command::new("notify-send")
                .args(["--app-name=bruhs", "--", title, body])
                .output()
        } else if cfg!(target_os = "windows") {
            windows_notification_command(title, body).output()
        } else {
            return Err("System notifications are not supported on this platform.".into());
        };
        match output {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(format!(
                "The notification command failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(error) => Err(format!("The notification command could not start: {error}")),
        }
    })
}

const WINDOWS_TOAST_TITLE_ENV: &str = "BRUHS_NOTIFICATION_TITLE";
const WINDOWS_TOAST_BODY_ENV: &str = "BRUHS_NOTIFICATION_BODY";
const WINDOWS_TOAST_SCRIPT: &str = concat!(
    "$ErrorActionPreference = 'Stop';",
    "[void][Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime];",
    "[void][Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime];",
    "$xml = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent(",
    "[Windows.UI.Notifications.ToastTemplateType]::ToastText02);",
    "$nodes = $xml.GetElementsByTagName('text');",
    "$title = if ($env:BRUHS_NOTIFICATION_TITLE) { \"bruhs - $($env:BRUHS_NOTIFICATION_TITLE)\" } else { 'bruhs' };",
    "$body = if ($null -ne $env:BRUHS_NOTIFICATION_BODY) { $env:BRUHS_NOTIFICATION_BODY } else { '' };",
    "[void]$nodes.Item(0).AppendChild($xml.CreateTextNode($title));",
    "[void]$nodes.Item(1).AppendChild($xml.CreateTextNode($body));",
    "$toast = New-Object Windows.UI.Notifications.ToastNotification $xml;",
    "$appId = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe';",
    "[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($appId).Show($toast)"
);

fn windows_notification_command(title: &str, body: &str) -> std::process::Command {
    let clean = |value: &str| value.replace('\0', "");
    let mut command = std::process::Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            WINDOWS_TOAST_SCRIPT,
        ])
        .env(WINDOWS_TOAST_TITLE_ENV, clean(title))
        .env(WINDOWS_TOAST_BODY_ENV, clean(body));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: OutboxKind, title: &str) -> OutboxItem {
        OutboxItem {
            id: title.into(),
            agent_id: "a".into(),
            run_id: title.into(),
            kind,
            title: title.into(),
            message: "Body".into(),
            created_at: EpochMs::now(),
            read_at: None,
            resolved_at: None,
            answer: None,
            system: SystemDelivery::Pending,
            approval: None,
        }
    }

    #[test]
    fn one_item_is_shown_alone_and_bursts_become_a_digest() {
        assert_eq!(
            digest(&[item(OutboxKind::Ask, "Approve the invoice?")]).0,
            "Question: Approve the invoice?"
        );
        let (title, body) = digest(&[
            item(OutboxKind::Notify, "Invoice due"),
            item(OutboxKind::Ask, "Which card?"),
        ]);
        assert_eq!(title, "2 agent updates (1 need an answer)");
        assert_eq!(body, "Invoice due\nWhich card?");
    }

    #[test]
    fn windows_toast_passes_text_through_env_never_as_script() {
        let untrusted_title = "Done $(calc.exe) \0 <tag>";
        let untrusted_body = "Body '; Start-Process calc; \0 &";
        let command = windows_notification_command(untrusted_title, untrusted_body);
        assert_eq!(command.get_program(), "powershell.exe");
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                WINDOWS_TOAST_SCRIPT,
            ]
        );
        assert!(args.iter().all(|arg| !arg.contains("calc.exe")));
        assert!(args.iter().all(|arg| !arg.contains("Start-Process")));
        let envs: std::collections::BTreeMap<String, Option<String>> = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|val| val.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert_eq!(
            envs.get(WINDOWS_TOAST_TITLE_ENV),
            Some(&Some("Done $(calc.exe)  <tag>".into()))
        );
        assert_eq!(
            envs.get(WINDOWS_TOAST_BODY_ENV),
            Some(&Some("Body '; Start-Process calc;  &".into()))
        );
    }
}
