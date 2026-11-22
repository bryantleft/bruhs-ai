//! Make a new bruh: its name, job, working folder and face, optionally
//! starting from a role. A role is copied once, as a snapshot: later edits to
//! the role never reach the bruh. **Make this bruh** opens it with a
//! persona's name and job (D11); the persona's memory waits as a proposal
//! the owner approves on the bruh's Memory tab (D12).

use bruhs_models::agent::DEFAULT_AGENT_MODEL;
use bruhs_models::crew::{create_command, draft_from_role, BruhDraft, BruhLook};
use bruhs_models::imports::{after_created, ImportPrefill};
use bruhs_models::roles::ResolvedRole;
use leptos::prelude::*;

use super::data::use_crew_snapshot;
use super::face_editor::FaceEditor;
use crate::components::banner::{BannerKind, BannerStackSignal};
use crate::state::{
    AppView, CreateBruhOpen, CreateBruhPrefill, RolesSignal, SetActiveView, SetSidebarOpen,
};
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

/// The dialog's starting values: a persona's when it opens from **Make this
/// bruh**, else a blank bruh on the default model. The lines go under the job.
fn start_values(prefill: Option<&ImportPrefill>) -> (BruhDraft, Vec<String>) {
    match prefill {
        Some(prefill) => (prefill.draft.clone(), prefill.lines.clone()),
        None => (
            BruhDraft {
                name: String::new(),
                instructions: String::new(),
                model: DEFAULT_AGENT_MODEL.to_string(),
                effort: None,
            },
            Vec::new(),
        ),
    }
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

/// The New bruh dialog, mounted once in the shell behind [`CreateBruhOpen`]
/// (F28), so the crew rail, Move in and By prompt all reach it, whether the
/// sidebar is open or not.
#[component]
pub fn CreateBruhDialog() -> impl IntoView {
    let CreateBruhOpen(open) = expect_context::<CreateBruhOpen>();
    let prefill = use_context::<CreateBruhPrefill>().map(|prefill| prefill.0);
    // Closing, by any path, forgets the persona: the next opening is blank.
    let close = Callback::new(move |()| {
        open.set(false);
        if let Some(prefill) = prefill {
            prefill.set(None);
        }
    });

    view! {
        <Dialog open=Signal::derive(move || open.get()) on_close=close max_width="max-w-2xl".to_string() label="New bruh">
            <CreateBruhForm close=close prefill=prefill />
        </Dialog>
    }
}

/// One opening of the dialog. Its fields and crew data are made fresh each
/// time, so a reply for a bruh asked for in an earlier opening can't open it.
#[component]
fn CreateBruhForm(
    close: Callback<()>,
    prefill: Option<RwSignal<Option<ImportPrefill>>>,
) -> impl IntoView {
    let data = use_crew_snapshot();
    let roles = use_context::<RolesSignal>().map(|roles| roles.0);
    let set_view = use_context::<SetActiveView>().map(|set| set.0);
    let set_sidebar_open = use_context::<SetSidebarOpen>().map(|set| set.0);
    let banners = use_context::<BannerStackSignal>();
    let resolved = Memo::new(move |_| {
        roles
            .map(|roles| roles.with(|book| book.resolved("")))
            .unwrap_or_default()
    });

    // The persona this opening started from, if any (Make this bruh).
    let start = StoredValue::new(prefill.and_then(|prefill| prefill.get_untracked()));
    let (draft, lines) = start.with_value(|start| start_values(start.as_ref()));
    let job_rows = if draft.instructions.is_empty() { 2 } else { 6 };
    let name = RwSignal::new(draft.name);
    let job = RwSignal::new(draft.instructions);
    let folder = RwSignal::new(String::new());
    let role = RwSignal::new(NO_ROLE.to_string());
    let model = RwSignal::new(draft.model);
    let effort = RwSignal::new(draft.effort);
    let look = RwSignal::new(BruhLook::default());
    let invalid = RwSignal::new(None::<String>);
    // The bruh this dialog asked the service to make, until it appears.
    let pending = RwSignal::new(None::<String>);
    // The bruh made, while its imported memory is on its way as a proposal.
    let proposing = RwSignal::new(None::<String>);

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

    // Opens the new bruh's page, closing a phone's sidebar as a rail row
    // does, then the dialog.
    let finish = move |agent_id: String| {
        if let Some(set_view) = set_view {
            set_view.set(AppView::Bruh { agent_id });
        }
        if let Some(set_sidebar_open) = set_sidebar_open {
            let width = web_sys::window()
                .and_then(|window| window.inner_width().ok())
                .and_then(|width| width.as_f64());
            if width.is_some_and(|width| width < 640.0) {
                set_sidebar_open.set(false);
            }
        }
        close.run(());
    };

    // Open the new bruh once the service has made it; a failure keeps the
    // dialog open with the service's reason. A bruh made from a persona
    // gets its memory as a proposal first, never in the bruh itself (D12).
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
            match start
                .with_value(|start| start.as_ref().and_then(|start| after_created(start, &id)))
            {
                Some(proposal) => {
                    proposing.set(Some(id));
                    data.send.run(proposal);
                }
                None => finish(id),
            }
        } else if !data.busy.get() && data.error.with(Option::is_some) {
            pending.set(None);
        }
    });
    // The proposal answered. The bruh exists either way, so its page opens;
    // a refused proposal says why over it, and By prompt can send it again.
    Effect::new(move |_| {
        let Some(id) = proposing.get() else {
            return;
        };
        if data.busy.get() {
            return;
        }
        proposing.set(None);
        if let (Some(message), Some(banners)) = (data.error.get_untracked(), banners) {
            banners.push(BannerKind::Error, message, None);
        }
        finish(id);
    });

    let create = move || {
        if pending.get_untracked().is_some()
            || proposing.get_untracked().is_some()
            || data.busy.get_untracked()
            || !data.crew_ready()
        {
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
        pending.with(Option::is_some)
            || proposing.with(Option::is_some)
            || data.busy.get()
            || !data.crew_ready()
    });
    let prefill_lines = (!lines.is_empty()).then(|| {
        view! {
            <div data-create-prefill="" class="space-y-1">
                {lines
                    .into_iter()
                    .map(|line| view! { <p data-create-line="" class="text-label-sm text-rambutan-400">{line}</p> })
                    .collect_view()}
            </div>
        }
    });

    view! {
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
                    <Textarea aria_label="Job" placeholder="What this bruh does" value=Signal::derive(move || job.get()) on_input=Callback::new(move |text| job.set(text)) rows=job_rows />
                </label>
                {prefill_lines}
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::crew::AccessLevel;
    use bruhs_models::imports::{
        persona_lines, prefill_from_persona, FileUse, PersonaCandidate, PersonaFile, MEMORY_WAITS,
    };
    use bruhs_models::logic::portable::ImportSource;
    use bruhs_models::reactive::{ReactiveChange, ReactiveCommand};

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

    fn persona(memory: &str) -> PersonaCandidate {
        PersonaCandidate {
            id: "p1".into(),
            source: ImportSource::OpenClaw,
            folder: "/home/example/clawd".into(),
            name: "Pinch".into(),
            instructions: "## SOUL.md\n\nBe kind.".into(),
            memory: memory.into(),
            files: vec![PersonaFile {
                name: "SOUL.md".into(),
                use_: FileUse::Instructions,
            }],
            clipped: false,
            dropped_identity: Vec::new(),
            left_out_logs: 0,
        }
    }

    #[test]
    fn a_persona_prefills_the_create_dialog() {
        let persona = persona("## Memory: USER.md\n\nLikes tea.");
        let prefill = prefill_from_persona(&persona);
        let (draft, lines) = start_values(Some(&prefill));
        assert_eq!(draft.name, "Pinch");
        assert_eq!(draft.instructions, persona.instructions);
        assert_eq!(draft.model, DEFAULT_AGENT_MODEL);
        assert_eq!(draft.effort, None);
        assert_eq!(lines, persona_lines(&persona));
        assert!(lines.iter().any(|line| line == MEMORY_WAITS), "{lines:?}");
        assert_eq!(prefill.memory, persona.memory);

        // Without a prefill the dialog starts blank, on the default model.
        let (blank, lines) = start_values(None);
        assert_eq!(
            blank,
            BruhDraft {
                name: String::new(),
                instructions: String::new(),
                model: DEFAULT_AGENT_MODEL.to_string(),
                effort: None,
            }
        );
        assert!(lines.is_empty());
    }

    #[test]
    fn create_sends_memory_only_as_a_proposal() {
        let prefill = prefill_from_persona(&persona("Likes tea."));
        let (draft, _) = start_values(Some(&prefill));
        // The bruh is made exactly as any other: Read only, memory empty,
        // and with no folder the server makes a private one.
        match create_command("new".into(), &draft, BruhLook::default(), None) {
            ReactiveCommand::CreateBruh { agent, access, .. } => {
                assert_eq!(agent.memory, "");
                assert_eq!(agent.project_path, "");
                assert_eq!(access.level, AccessLevel::ReadOnly);
                assert!(!access.send_as_me && !access.can_deliver);
            }
            other => panic!("{other:?}"),
        }
        // Once it exists, its memory goes as a proposal for the owner.
        assert_eq!(
            after_created(&prefill, "new"),
            Some(ReactiveCommand::Propose {
                change: ReactiveChange::SetMemory {
                    agent_id: "new".into(),
                    memory: "Likes tea.".into(),
                },
            })
        );
        assert_eq!(
            after_created(&prefill_from_persona(&persona("  ")), "new"),
            None
        );
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
