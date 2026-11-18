//! New room and Room settings: a room's name, its members and its lead.
//! Save sends `room_command`, so the dialog and the service hold one set of
//! rules; with a room, Delete room asks once inline before it sends.

use bruhs_models::crew::room::{
    room_command, Room, RoomForm, DELETE_ROOM_CONFIRM, NEW_ROOM, NO_LEAD, ROOM_SETTINGS,
};
use bruhs_models::crew::{BruhState, RAIL_FACE_PX};
use bruhs_models::reactive::ReactiveCommand;
use leptos::prelude::*;

use super::data::use_crew_snapshot;
use super::face::BruhFace;
use crate::state::{AppView, SetActiveView};
use crate::ui::{
    button_class, Button, ButtonSize, ButtonVariant, Dialog, Dropdown, DropdownOption, Icon, Input,
};

const LABEL: &str = "block min-w-0 space-y-1 text-label-sm text-rambutan-300";
const MEMBER: &str = "bruhs-control flex min-h-control w-full min-w-0 items-center gap-2 rounded-grape px-2 py-1 text-left transition-colors hover:bg-longan-700/40 focus-visible:outline-2 focus-visible:outline-persimmon-400";

/// The lead picker: no lead, then each checked member in crew order.
fn lead_options(form: &RoomForm, bruhs: &[BruhState]) -> Vec<DropdownOption> {
    std::iter::once(DropdownOption::new("", NO_LEAD))
        .chain(
            bruhs
                .iter()
                .filter(|bruh| form.members.contains(&bruh.agent_id))
                .map(|bruh| DropdownOption::new(bruh.agent_id.clone(), bruh.name.clone())),
        )
        .collect()
}

/// The form a dialog opens with: blank for a new room, else the room
/// without members who have left the crew, so an untouched form still saves.
fn form_for(room: Option<&Room>, bruhs: &[BruhState]) -> RoomForm {
    let Some(room) = room else {
        return RoomForm::default();
    };
    let mut form = RoomForm::from_room(room);
    form.members
        .retain(|id| bruhs.iter().any(|bruh| bruh.agent_id == *id));
    if form
        .lead
        .as_ref()
        .is_some_and(|lead| !form.members.contains(lead))
    {
        form.lead = None;
    }
    form
}

/// What the dialog sent and is waiting on.
#[derive(Clone, PartialEq)]
enum Pending {
    Create(String),
    Save,
    Delete,
}

#[component]
pub fn RoomDialog(
    open: RwSignal<bool>,
    /// The room to edit; without one the dialog makes a new room.
    #[prop(optional)]
    room_id: Option<String>,
    /// Runs after the new room's page opens, such as to close a phone's sidebar.
    #[prop(optional)]
    on_open: Option<Callback<()>>,
) -> impl IntoView {
    let data = use_crew_snapshot();
    let set_view = use_context::<SetActiveView>().map(|set| set.0);
    let room_id = StoredValue::new(room_id);
    let editing = room_id.with_value(Option::is_some);
    let bruhs = Memo::new(move |_| {
        data.snapshot
            .with(|snapshot| snapshot.as_ref().map(|snapshot| snapshot.bruhs.clone()))
            .unwrap_or_default()
    });
    let existing = Memo::new(move |_| {
        let id = room_id.get_value()?;
        data.snapshot.with(|snapshot| {
            snapshot.as_ref().and_then(|snapshot| {
                snapshot
                    .rooms
                    .iter()
                    .find(|state| state.room.id == id)
                    .map(|state| state.room.clone())
            })
        })
    });

    let form = RwSignal::new(RoomForm::default());
    let invalid = RwSignal::new(None::<&'static str>);
    let confirming = RwSignal::new(false);
    let pending = RwSignal::new(None::<Pending>);

    // Every opening starts from the room as the service has it.
    Effect::new(move |was_open: Option<bool>| {
        let is_open = open.get();
        if is_open && was_open != Some(true) {
            form.set(untrack(|| {
                bruhs.with(|bruhs| existing.with(|room| form_for(room.as_ref(), bruhs)))
            }));
            invalid.set(None);
            confirming.set(false);
            pending.set(None);
            data.error.set(None);
        }
        is_open
    });

    // Once the service has answered: a failure keeps the dialog open with
    // its reason; otherwise it closes, and a new room opens.
    Effect::new(move |_| {
        let Some(sent) = pending.get() else {
            return;
        };
        if data.busy.get() {
            return;
        }
        pending.set(None);
        if data.error.with(Option::is_some) {
            return;
        }
        open.set(false);
        if let Pending::Create(id) = sent {
            let made = data.snapshot.with_untracked(|snapshot| {
                snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.rooms.iter().any(|state| state.room.id == id))
            });
            if made {
                if let Some(set_view) = set_view {
                    set_view.set(AppView::Room { room_id: id });
                }
                if let Some(on_open) = on_open {
                    on_open.run(());
                }
            }
        }
    });

    // Closing forgets what was sent, so a late answer can't open a room.
    let close = Callback::new(move |()| {
        pending.set(None);
        open.set(false);
        data.error.set(None);
    });
    let blocked = move || {
        pending.with_untracked(Option::is_some) || data.busy.get_untracked() || !data.rooms_ready()
    };
    let submit = Callback::new(move |()| {
        if blocked() {
            return;
        }
        let existing = existing.get_untracked();
        let id = match room_id.get_value() {
            // The room went while the dialog was open: nothing left to save.
            Some(_) if existing.is_none() => {
                close.run(());
                return;
            }
            Some(id) => id,
            None => uuid::Uuid::new_v4().to_string(),
        };
        let result = bruhs.with_untracked(|bruhs| {
            let eligible: Vec<&str> = bruhs.iter().map(|bruh| bruh.agent_id.as_str()).collect();
            form.with_untracked(|form| room_command(existing.as_ref(), &id, form, &eligible))
        });
        match result {
            Err(reason) => invalid.set(Some(reason)),
            Ok(None) => close.run(()),
            Ok(Some(command)) => {
                invalid.set(None);
                pending.set(Some(if existing.is_some() {
                    Pending::Save
                } else {
                    Pending::Create(id)
                }));
                data.send.run(command);
            }
        }
    });
    let delete = move |_| {
        let Some(id) = room_id.get_value() else {
            return;
        };
        if blocked() {
            return;
        }
        pending.set(Some(Pending::Delete));
        data.send.run(ReactiveCommand::DeleteRoom { id });
    };
    let submit_disabled = Signal::derive(move || {
        pending.with(Option::is_some) || data.busy.get() || !data.rooms_ready()
    });
    let heading = if editing { ROOM_SETTINGS } else { NEW_ROOM };

    view! {
        <Dialog open=Signal::derive(move || open.get()) on_close=close label=heading>
            <form
                aria-label=heading
                data-room-dialog=""
                on:submit=move |event| {
                    event.prevent_default();
                    submit.run(());
                }
            >
                <div class="border-b border-longan-800 px-4 py-3">
                    <h2 class="text-h4 text-rambutan-100">{heading}</h2>
                </div>
                <div class="max-h-[70vh] space-y-3 overflow-y-auto px-4 py-3">
                    <label class=LABEL>
                        <span>"Name"</span>
                        <Input
                            aria_label="Name"
                            value=Signal::derive(move || form.with(|form| form.name.clone()))
                            on_input=Callback::new(move |text| form.update(|form| form.name = text))
                        />
                    </label>
                    <div class=LABEL>
                        <span id="room-dialog-members">"Members"</span>
                        {move || bruhs.with(Vec::is_empty).then(|| view! {
                            <p class="text-label-sm text-rambutan-500">"No bruhs yet."</p>
                        })}
                        <ul role="list" aria-labelledby="room-dialog-members" class="m-0 max-h-[30vh] min-h-0 list-none space-y-0.5 overflow-y-auto p-0">
                            <For
                                each=move || bruhs.get()
                                key=|bruh| bruh.agent_id.clone()
                                children=move |bruh: BruhState| {
                                    let id = StoredValue::new(bruh.agent_id.clone());
                                    let checked = Memo::new(move |_| {
                                        id.with_value(|id| form.with(|form| form.members.contains(id)))
                                    });
                                    let state = Memo::new(move |_| {
                                        id.with_value(|id| bruhs.with(|bruhs| bruhs.iter().find(|bruh| bruh.agent_id == *id).cloned()))
                                    });
                                    let look = Signal::derive(move || state.get().map(|state| state.look).unwrap_or_default());
                                    let status = Signal::derive(move || state.get().map_or(bruh.status, |state| state.status));
                                    let name = move || state.get().map(|state| state.name).unwrap_or_default();
                                    view! {
                                        <li>
                                            <button
                                                type="button"
                                                role="checkbox"
                                                data-room-member=move || id.get_value()
                                                aria-checked=move || checked.get().to_string()
                                                class=MEMBER
                                                on:click=move |_| id.with_value(|id| form.update(|form| form.toggle_member(id)))
                                                on:keydown=move |event| {
                                                    // As in a field, Enter saves; Space checks.
                                                    if event.key() == "Enter" {
                                                        event.prevent_default();
                                                        submit.run(());
                                                    }
                                                }
                                            >
                                                <span
                                                    aria-hidden="true"
                                                    class=move || format!(
                                                        "inline-flex size-control-xs shrink-0 items-center justify-center rounded-seed transition-colors {}",
                                                        if checked.get() { "bg-blueberry-400 text-longan-950" } else { "bg-white/5 text-transparent inset-ring inset-ring-white/15" }
                                                    )
                                                >
                                                    <Icon name="check" size=14 class="stroke-[3]" />
                                                </span>
                                                <BruhFace size=f64::from(RAIL_FACE_PX) look=look status=status />
                                                <span class="min-w-0 flex-1 truncate text-label text-rambutan-100">{name}</span>
                                            </button>
                                        </li>
                                    }
                                }
                            />
                        </ul>
                    </div>
                    <div class=LABEL>
                        <span>"Lead"</span>
                        <Dropdown
                            options=Signal::derive(move || bruhs.with(|bruhs| form.with(|form| lead_options(form, bruhs))))
                            value=Signal::derive(move || form.with(|form| form.lead.clone().unwrap_or_default()))
                            on_change=Callback::new(move |value: String| form.update(|form| form.lead = Some(value).filter(|value| !value.is_empty())))
                            label="Lead"
                            class="w-full"
                        />
                    </div>
                    {move || invalid.get().map(str::to_string).or_else(|| data.error.get()).map(|message| view! {
                        <p role="alert" class="rounded-grape bg-lychee-500/10 p-3 text-body-sm text-lychee-200">{message}</p>
                    })}
                    {move || data.rooms_reason().map(|reason| view! {
                        <p class="text-label-sm text-rambutan-400">{reason}</p>
                    })}
                    <Show when=move || confirming.get()>
                        <div data-room-delete-confirm="" class="space-y-2 rounded-grape bg-longan-950/60 p-3 inset-ring inset-ring-longan-700/50">
                            <p class="text-body-sm text-rambutan-200">{DELETE_ROOM_CONFIRM}</p>
                            <div class="flex flex-wrap items-center gap-2">
                                <Button variant=ButtonVariant::Destructive size=ButtonSize::Small disabled=submit_disabled on_click=Callback::new(delete)>"Delete"</Button>
                                <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| confirming.set(false))>"Keep"</Button>
                            </div>
                        </div>
                    </Show>
                </div>
                <div class="flex items-center gap-2 border-t border-longan-800 bg-longan-900/40 px-4 py-3">
                    {editing.then(|| view! {
                        <Show when=move || !confirming.get()>
                            <Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=submit_disabled on_click=Callback::new(move |_| confirming.set(true))>"Delete room"</Button>
                        </Show>
                    })}
                    <span class="flex-1"></span>
                    <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| close.run(()))>"Cancel"</Button>
                    // The form's submit button, so Enter in a field saves.
                    <button type="submit" class=button_class(ButtonVariant::Primary, ButtonSize::Small, "") disabled=move || submit_disabled.get()>
                        {if editing { "Save" } else { "Create room" }}
                    </button>
                </div>
            </form>
        </Dialog>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::crew::{BruhAccess, BruhLook, BruhStatus};

    fn bruh(id: &str, name: &str) -> BruhState {
        BruhState {
            agent_id: id.into(),
            name: name.into(),
            look: BruhLook::default(),
            access: BruhAccess::default(),
            effort: None,
            status: BruhStatus::Asleep,
            next_wake_ms: None,
            unread: 0,
        }
    }

    #[test]
    fn the_lead_list_follows_the_checked_members() {
        let bruhs = [bruh("a", "Ada"), bruh("b", "Bo"), bruh("c", "Cy")];
        // Checked out of crew order on purpose: the list keeps crew order.
        let form = RoomForm {
            name: "Launch".into(),
            members: vec!["c".into(), "a".into()],
            lead: None,
        };
        let options: Vec<(String, String)> = lead_options(&form, &bruhs)
            .into_iter()
            .map(|option| (option.value, option.label))
            .collect();
        assert_eq!(
            options,
            [
                (String::new(), NO_LEAD.to_string()),
                ("a".into(), "Ada".into()),
                ("c".into(), "Cy".into()),
            ]
        );
        assert_eq!(lead_options(&RoomForm::default(), &bruhs).len(), 1);
    }

    #[test]
    fn a_room_opens_without_members_who_left_the_crew() {
        let bruhs = [bruh("a", "Ada"), bruh("b", "Bo")];
        assert_eq!(form_for(None, &bruhs), RoomForm::default());
        let room = Room {
            id: "r".into(),
            name: "Launch".into(),
            members: vec!["gone".into(), "b".into()],
            lead: Some("gone".into()),
        };
        let form = form_for(Some(&room), &bruhs);
        assert_eq!(form.members, ["b"]);
        assert_eq!(form.lead, None);
        // What is left saves, where the room as stored would not.
        let eligible = ["a", "b"];
        assert!(room_command(Some(&room), "r", &form, &eligible).is_ok());
        assert!(room_command(Some(&room), "r", &RoomForm::from_room(&room), &eligible).is_err());
        let kept = Room {
            lead: Some("b".into()),
            ..room
        };
        assert_eq!(form_for(Some(&kept), &bruhs).lead.as_deref(), Some("b"));
    }
}
