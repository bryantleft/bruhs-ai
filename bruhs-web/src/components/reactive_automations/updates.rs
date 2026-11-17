//! What agents told the owner. Questions wait here for a reply.
use super::calendar::local_time;
use crate::ui::{Button, ButtonSize, ButtonVariant, Textarea};
use bruhs_models::reactive::*;
use leptos::prelude::*;

const ITEM: &str = "min-w-0 rounded-grape p-3 inset-ring";

/// Badges for an item: its kind, then how it was resolved.
pub(super) fn badges(item: &OutboxItem) -> Vec<&'static str> {
    let mut badges = vec![match item.kind {
        OutboxKind::Ask => "Question",
        OutboxKind::Notify => "Update",
    }];
    if item.resolved_at.is_some() {
        badges.push(if item.answer.is_some() {
            "Answered"
        } else {
            "Dismissed"
        });
    }
    badges
}

/// The answer to send, or nothing for an empty reply (the button is also disabled).
fn answer_command(id: &str, message: String) -> Option<ReactiveCommand> {
    (!message.trim().is_empty()).then(|| ReactiveCommand::AnswerOutbox {
        id: id.to_owned(),
        message,
    })
}

pub(super) fn delivery_error(item: &OutboxItem) -> Option<String> {
    match &item.system {
        SystemDelivery::Failed { error } => Some(format!("System notification failed: {error}")),
        _ => None,
    }
}

#[component]
pub(super) fn UpdatesPanel(
    snapshot: RwSignal<Option<ReactiveSnapshot>>,
    send: Callback<ReactiveCommand>,
) -> impl IntoView {
    let replying = RwSignal::new(None::<String>);
    let reply = RwSignal::new(String::new());
    let items = Memo::new(move |_| {
        snapshot.with(|s| s.as_ref().map(|s| s.outbox.clone()).unwrap_or_default())
    });
    let unread = Memo::new(move |_| items.with(|items| items.iter().any(OutboxItem::is_unread)));
    // A reply closes once its item is resolved.
    Effect::new(move |_| {
        let Some(id) = replying.get() else {
            return;
        };
        if items.with(|items| {
            items
                .iter()
                .find(|item| item.id == id)
                .is_none_or(|item| item.resolved_at.is_some())
        }) {
            replying.set(None);
            reply.set(String::new());
        }
    });
    let agent_name = move |agent_id: &str| {
        snapshot.with(|s| {
            s.as_ref()
                .and_then(|s| s.agents.iter().find(|agent| agent.id == agent_id))
                .map_or_else(|| "Agent".to_string(), |agent| agent.name.clone())
        })
    };
    view! {
        <section class="min-w-0 space-y-3 border-b border-longan-700/60 pb-5" aria-labelledby="agent-updates-title">
            <div class="flex flex-wrap items-start justify-between gap-2">
                <div class="min-w-0">
                    <h3 id="agent-updates-title" class="text-label text-rambutan-100">"Updates"</h3>
                    <p class="mt-1 text-body-sm text-rambutan-400">"What your agents told you. Questions wait for your reply."</p>
                </div>
                <Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=Signal::derive(move || !unread.get()) on_click=Callback::new(move |_| send.run(ReactiveCommand::MarkOutboxRead { ids: Vec::new() }))>"Mark all read"</Button>
            </div>
            <Show when=move || items.with(Vec::is_empty)>
                <p class="text-body-sm text-rambutan-500">"No updates yet. Agents stay quiet until something needs you."</p>
            </Show>
            <ul class="m-0 min-w-0 list-none space-y-2 p-0" aria-label="Agent updates">
                {move || items.get().into_iter().map(|item| {
                    let id = item.id.clone();
                    let is_unread = item.is_unread();
                    let open_question = item.is_open_question();
                    let resolved = item.resolved_at.is_some();
                    let tone = if is_unread {
                        "bg-blueberry-500/10 inset-ring-blueberry-400/40"
                    } else {
                        "bg-longan-950/60 inset-ring-longan-700/50"
                    };
                    let name = agent_name(&item.agent_id);
                    let read_id = id.clone();
                    let dismiss_id = id.clone();
                    let reply_id = id.clone();
                    let send_id = id.clone();
                    let editing = Signal::derive(move || replying.get().as_deref() == Some(id.as_str()));
                    let reply_variant = if open_question { ButtonVariant::Primary } else { ButtonVariant::Outline };
                    view! {
                        <li class=format!("{ITEM} {tone}")>
                            <div class="flex flex-wrap items-center gap-2">
                                {badges(&item).into_iter().map(|badge| view! {
                                    <span class="rounded-seed bg-longan-800 px-2 py-0.5 text-label-sm text-rambutan-200">{badge}</span>
                                }).collect_view()}
                                {is_unread.then(|| view! { <span class="text-label-sm text-blueberry-200">"Unread"</span> })}
                                <span class="min-w-0 break-words text-label-sm text-rambutan-300">{name}</span>
                                <span class="text-label-sm text-rambutan-500">{local_time(item.created_at)}</span>
                            </div>
                            <h4 class="mt-2 break-words text-label text-rambutan-100">{item.title.clone()}</h4>
                            <p class="mt-1 whitespace-pre-wrap break-words select-text text-body-sm text-rambutan-200">{item.message.clone()}</p>
                            {item.answer.clone().map(|answer| view! {
                                <div class="mt-2 rounded-grape bg-longan-900 p-2 inset-ring inset-ring-longan-700/50">
                                    <p class="text-label-sm text-rambutan-400">"Your reply"</p>
                                    <p class="mt-1 whitespace-pre-wrap break-words select-text text-body-sm text-rambutan-200">{answer}</p>
                                </div>
                            })}
                            {delivery_error(&item).map(|error| view! { <p class="mt-2 break-words text-body-sm text-lychee-200">{error}</p> })}
                            <Show when=move || editing.get()>
                                <label class="mt-3 block space-y-1 text-label-sm text-rambutan-300">
                                    <span>"Reply"</span>
                                    <Textarea aria_label="Reply" value=Signal::derive(move || reply.get()) on_input=Callback::new(move |text| reply.set(text)) rows=3 autofocus=true />
                                </label>
                            </Show>
                            <div class="mt-3 flex flex-wrap gap-2">
                                {(!resolved).then(|| view! {
                                    <Show
                                        when=move || editing.get()
                                        fallback=move || view! {
                                            <Button variant=reply_variant size=ButtonSize::Small on_click=Callback::new({ let id = reply_id.clone(); move |_| { reply.set(String::new()); replying.set(Some(id.clone())); } })>"Reply"</Button>
                                        }
                                    >
                                        <Button size=ButtonSize::Small disabled=Signal::derive(move || reply.with(|text| text.trim().is_empty())) on_click=Callback::new({ let id = send_id.clone(); move |_| { if let Some(command) = answer_command(&id, reply.get_untracked()) { send.run(command) } } })>"Send reply"</Button>
                                        <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| replying.set(None))>"Cancel"</Button>
                                    </Show>
                                })}
                                {is_unread.then(|| view! {
                                    <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| send.run(ReactiveCommand::MarkOutboxRead { ids: vec![read_id.clone()] }))>"Mark read"</Button>
                                })}
                                {(!resolved).then(|| view! {
                                    <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| send.run(ReactiveCommand::DismissOutbox { id: dismiss_id.clone() }))>"Dismiss"</Button>
                                })}
                            </div>
                        </li>
                    }
                }).collect_view()}
            </ul>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::EpochMs;

    #[test]
    fn an_empty_reply_is_never_sent() {
        assert_eq!(answer_command("q", "  \n ".into()), None);
        assert_eq!(
            answer_command("q", "Hold it.".into()),
            Some(ReactiveCommand::AnswerOutbox {
                id: "q".into(),
                message: "Hold it.".into(),
            })
        );
    }

    fn item(kind: OutboxKind) -> OutboxItem {
        OutboxItem {
            id: "o".into(),
            agent_id: "a".into(),
            run_id: "r".into(),
            kind,
            title: "Title".into(),
            message: "Body".into(),
            created_at: EpochMs::UNIX_EPOCH,
            read_at: None,
            resolved_at: None,
            answer: None,
            system: SystemDelivery::NotRequested,
            approval: None,
        }
    }

    #[test]
    fn badges_show_kind_then_resolution() {
        let mut question = item(OutboxKind::Ask);
        assert_eq!(badges(&question), vec!["Question"]);
        question.resolved_at = Some(EpochMs::UNIX_EPOCH);
        question.answer = Some("Yes".into());
        assert_eq!(badges(&question), vec!["Question", "Answered"]);
        let mut update = item(OutboxKind::Notify);
        update.resolved_at = Some(EpochMs::UNIX_EPOCH);
        assert_eq!(badges(&update), vec!["Update", "Dismissed"]);
    }

    #[test]
    fn failed_system_delivery_is_explained() {
        let mut update = item(OutboxKind::Notify);
        assert_eq!(delivery_error(&update), None);
        update.system = SystemDelivery::Failed {
            error: "osascript missing".into(),
        };
        assert_eq!(
            delivery_error(&update).as_deref(),
            Some("System notification failed: osascript missing")
        );
    }
}
