//! Make a new bruh: its name, job, working folder and face, optionally
//! starting from a role. A role is copied once, as a snapshot: later edits to
//! the role never reach the bruh.

use bruhs_models::agent::DEFAULT_AGENT_MODEL;
use bruhs_models::crew::{create_command, draft_from_role, BruhDraft, BruhLook};
use bruhs_models::roles::ResolvedRole;
use leptos::prelude::*;

use super::data::use_crew_snapshot;
use super::face_editor::FaceEditor;
use crate::state::{AppView, RolesSignal, SetActiveView};
use crate::ui::{
    button_class, Button, ButtonSize, ButtonVariant, Dialog, Dropdown, DropdownOption, Input,
    Textarea,
};

const LABEL: &str = "block min-w-0 space-y-1 text-label-sm text-rambutan-300";
const NO_ROLE: &str = "";

/// The working folder the owner typed, or `None` so the server makes one.
fn folder_choice(text: &str) -> Option<String> {
    let folder = text.trim();
    (!folder.is_empty()).then(|| folder.to_string())
}

/// The draft a role starts from, when `id` names one of `roles`.
fn role_draft(roles: &[ResolvedRole], id: &str) -> Option<BruhDraft> {
    roles
        .iter()
        .find(|role| role.id == id)
        .map(|role| draft_from_role(role, DEFAULT_AGENT_MODEL))
}

fn role_options(roles: &[ResolvedRole]) -> Vec<DropdownOption> {
    std::iter::once(DropdownOption::new(NO_ROLE, "None"))
        .chain(
            roles
                .iter()
                .map(|role| DropdownOption::new(role.id.clone(), role.name.clone())),
        )
        .collect()
}

#[component]
pub fn CreateBruhDialog(
    open: RwSignal<bool>,
    /// Runs after the new bruh's page opens, such as to close a phone's sidebar.
    #[prop(optional)]
    on_open: Option<Callback<()>>,
) -> impl IntoView {
    let data = use_crew_snapshot();
    let roles = use_context::<RolesSignal>().map(|roles| roles.0);
    let set_view = use_context::<SetActiveView>().map(|set| set.0);
    let resolved = Memo::new(move |_| {
        roles
            .map(|roles| roles.with(|book| book.resolved("")))
            .unwrap_or_default()
    });

    let name = RwSignal::new(String::new());
    let job = RwSignal::new(String::new());
    let folder = RwSignal::new(String::new());
    let role = RwSignal::new(NO_ROLE.to_string());
    let model = RwSignal::new(DEFAULT_AGENT_MODEL.to_string());
    let effort = RwSignal::new(None::<String>);
    let look = RwSignal::new(BruhLook::default());
    let invalid = RwSignal::new(None::<String>);
    // The bruh this dialog asked the service to make, until it appears.
    let pending = RwSignal::new(None::<String>);

    // Every opening starts from a blank bruh.
    Effect::new(move |was_open: Option<bool>| {
        let is_open = open.get();
        if is_open && was_open != Some(true) {
            name.set(String::new());
            job.set(String::new());
            folder.set(String::new());
            role.set(NO_ROLE.to_string());
            model.set(DEFAULT_AGENT_MODEL.to_string());
            effort.set(None);
            look.set(BruhLook::default());
            invalid.set(None);
            pending.set(None);
            data.error.set(None);
        }
        is_open
    });

    let choose_role = Callback::new(move |id: String| {
        match resolved.with_untracked(|roles| role_draft(roles, &id)) {
            Some(draft) => {
                name.set(draft.name);
                job.set(draft.instructions);
                model.set(draft.model);
                effort.set(draft.effort);
            }
            None => {
                model.set(DEFAULT_AGENT_MODEL.to_string());
                effort.set(None);
            }
        }
        role.set(id);
    });

    // Open the new bruh once the service has made it; a failure keeps the
    // dialog open with the service's reason.
    Effect::new(move |_| {
        let Some(id) = pending.get() else {
            return;
        };
        let made = data.snapshot.with(|snapshot| {
            snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.agents.iter().any(|agent| agent.id == id))
        });
        if made {
            pending.set(None);
            open.set(false);
            if let Some(set_view) = set_view {
                set_view.set(AppView::Bruh { agent_id: id });
            }
            if let Some(on_open) = on_open {
                on_open.run(());
            }
        } else if !data.busy.get() && data.error.with(Option::is_some) {
            pending.set(None);
        }
    });

    // Closing forgets the bruh on its way, so its reply can't open it later.
    let close = Callback::new(move |()| {
        pending.set(None);
        open.set(false);
        data.error.set(None);
    });
    let create = move || {
        if pending.get_untracked().is_some() || data.busy.get_untracked() || !data.crew_ready() {
            return;
        }
        let draft = BruhDraft {
            name: name.get_untracked().trim().to_string(),
            instructions: job.get_untracked().trim().to_string(),
            model: model.get_untracked(),
            effort: effort.get_untracked(),
        };
        if draft.name.is_empty() {
            invalid.set(Some("Give the bruh a name.".into()));
            return;
        }
        invalid.set(None);
        let id = uuid::Uuid::new_v4().to_string();
        let command = create_command(
            id.clone(),
            &draft,
            look.get_untracked(),
            folder.with_untracked(|folder| folder_choice(folder)),
        );
        pending.set(Some(id));
        data.send.run(command);
    };
    let create_disabled = Signal::derive(move || {
        pending.with(Option::is_some) || data.busy.get() || !data.crew_ready()
    });

    view! {
        <Dialog open=Signal::derive(move || open.get()) on_close=close max_width="max-w-2xl".to_string() label="New bruh">
            <form
                aria-label="New bruh"
                on:submit=move |event| {
                    event.prevent_default();
                    create();
                }
            >
                <div class="border-b border-longan-800 px-4 py-3">
                    <h2 class="text-h4 text-rambutan-100">"New bruh"</h2>
                </div>
                <div class="max-h-[70vh] space-y-3 overflow-y-auto px-4 py-3">
                    <label class=LABEL>
                        <span>"Name"</span>
                        <Input aria_label="Name" value=Signal::derive(move || name.get()) on_input=Callback::new(move |text| name.set(text)) />
                    </label>
                    <label class=LABEL>
                        <span>"Job"</span>
                        <Textarea aria_label="Job" placeholder="What this bruh does" value=Signal::derive(move || job.get()) on_input=Callback::new(move |text| job.set(text)) rows=2 />
                    </label>
                    <label class=LABEL>
                        <span>"Working folder"</span>
                        <Input aria_label="Working folder" placeholder="Optional" value=Signal::derive(move || folder.get()) on_input=Callback::new(move |text| folder.set(text)) />
                        <Show when=move || folder.with(|folder| folder_choice(folder).is_none())>
                            <span class="block text-label-sm text-rambutan-500">"Bruhs makes a private folder."</span>
                        </Show>
                    </label>
                    <div class=LABEL>
                        <span>"Start from role"</span>
                        <Dropdown
                            options=Signal::derive(move || resolved.with(|roles| role_options(roles)))
                            value=Signal::derive(move || role.get())
                            on_change=choose_role
                            label="Start from role"
                            class="w-full"
                        />
                        <span class="flex min-w-0 flex-wrap gap-x-3 text-label-sm text-rambutan-500">
                            <span class="break-all">{move || format!("Model: {}", model.get())}</span>
                            {move || effort.get().map(|effort| view! { <span>{format!("Effort: {effort}")}</span> })}
                        </span>
                    </div>
                    <section class="min-w-0 space-y-2" aria-labelledby="new-bruh-face">
                        <h3 id="new-bruh-face" class="text-label-sm text-rambutan-300">"Face"</h3>
                        <FaceEditor look=look />
                    </section>
                    {move || invalid.get().or_else(|| data.error.get()).map(|message| view! {
                        <p role="alert" class="rounded-grape bg-lychee-500/10 p-3 text-body-sm text-lychee-200">{message}</p>
                    })}
                    {move || data.service_reason().map(|reason| view! {
                        <p class="text-label-sm text-rambutan-400">{reason}</p>
                    })}
                </div>
                <div class="flex items-center justify-end gap-2 border-t border-longan-800 bg-longan-900/40 px-4 py-3">
                    <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| close.run(()))>"Cancel"</Button>
                    // The form's submit button, so Enter in a field creates.
                    <button type="submit" class=button_class(ButtonVariant::Primary, ButtonSize::Small, "") disabled=move || create_disabled.get()>"Create"</button>
                </div>
            </form>
        </Dialog>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(id: &str, model: Option<&str>) -> ResolvedRole {
        ResolvedRole {
            id: id.into(),
            name: format!("Role {id}"),
            use_for: String::new(),
            model: model.map(str::to_string),
            effort: Some("high".into()),
            access: None,
            instructions: format!("Do {id} work."),
        }
    }

    #[test]
    fn a_blank_folder_lets_the_server_make_one() {
        assert_eq!(folder_choice("  "), None);
        assert_eq!(folder_choice(""), None);
        assert_eq!(
            folder_choice("/Users/me/work"),
            Some("/Users/me/work".into())
        );
        assert_eq!(
            folder_choice("  /Users/me/work \n"),
            Some("/Users/me/work".into())
        );
    }

    #[test]
    fn a_role_copies_its_job_model_and_effort_or_the_default_model() {
        let roles = [role("a", Some("gpt-5.1-codex")), role("b", None)];
        assert_eq!(
            role_draft(&roles, "a"),
            Some(BruhDraft {
                name: "Role a".into(),
                instructions: "Do a work.".into(),
                model: "gpt-5.1-codex".into(),
                effort: Some("high".into()),
            })
        );
        assert_eq!(
            role_draft(&roles, "b").map(|draft| draft.model),
            Some(DEFAULT_AGENT_MODEL.to_string())
        );
        assert_eq!(role_draft(&roles, NO_ROLE), None);
        let options = role_options(&roles);
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].value, NO_ROLE);
        assert_eq!(options[1].label, "Role a");
    }
}
