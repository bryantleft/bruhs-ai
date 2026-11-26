//! Browser prompt links use fragments so the prompt stays in the browser.

use std::collections::BTreeMap;

use bruhs_models::logic::app_link::{
    new_prompt_link, new_prompt_link_with_context, parse_prompt_link, LinkedPrompt,
    PromptDestination, PromptLink, PromptLinkError, PromptPullRequest, PromptRepository,
    PromptShareContext, PULL_CHECKOUT_SETUP_NOTICE,
};
use futures_util::{stream, StreamExt};
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::banner::{BannerKind, BannerStackSignal};
use crate::components::composer::TextAttachments;
use crate::components::file_attachments::FileOffers;
use crate::state::{
    ActiveProject, ActiveThreadId, ChipRetargetGuard, ComposerChips, ComposerDraft,
    ProjectPickerMode, ProjectPickerModeSignal, ProjectPickerOpen, SetProjectPickerOpen,
};
use crate::ui::{Button, ButtonSize, ButtonVariant, Dialog};

#[derive(Clone)]
pub enum LinkedDraft {
    Incoming(LinkedPrompt),
    Review(LinkedPrompt),
    Placed { removed_hidden_characters: bool },
}

#[derive(Clone, Copy)]
pub struct PromptLinks {
    pub pending: RwSignal<Option<PromptLink>>,
    cloned: RwSignal<Option<(PromptLink, ActiveProject)>>,
    target_review: RwSignal<Option<TargetReview>>,
    pub checked_out: RwSignal<Vec<CheckedOutPull>>,
    drafts: RwSignal<BTreeMap<String, LinkedDraft>>,
    banners: BannerStackSignal,
}

impl PromptLinks {
    pub fn provide() -> Self {
        let links = Self {
            pending: RwSignal::new(None),
            cloned: RwSignal::new(None),
            target_review: RwSignal::new(None),
            checked_out: RwSignal::new(Vec::new()),
            drafts: RwSignal::new(BTreeMap::new()),
            banners: expect_context(),
        };
        provide_context(links);
        links
    }

    /// Consume a prompt fragment and remove its text from the address bar.
    pub fn receive_route(self, hash: &str, current: Option<String>) -> Option<String> {
        let (destination, result) = parse_fragment(hash)?;
        let destination = current.unwrap_or(destination);
        let window = window();
        if let Ok(history) = window.history() {
            let state = history.state().unwrap_or(wasm_bindgen::JsValue::NULL);
            let _ = history.replace_state_with_url(&state, "", Some(&destination));
        }
        if !self.operation_running() {
            self.target_review.set(None);
        }
        match result {
            Ok(prompt) => self.pending.set(Some(prompt)),
            Err(error) => {
                self.pending.set(None);
                self.warn(error.to_string());
            }
        }
        Some(destination)
    }

    pub fn warn(self, message: impl Into<String>) {
        self.banners.push(BannerKind::Warning, message, None);
    }

    pub fn review(self, request: PromptLink, project: Option<ActiveProject>) {
        self.target_review.set(Some(TargetReview {
            request,
            project,
            checkout_id: uuid::Uuid::new_v4(),
            running: false,
            error: None,
        }));
    }

    pub fn operation_running(self) -> bool {
        self.target_review
            .with(|review| review.as_ref().is_some_and(|review| review.running))
    }

    pub fn route_request(self) -> Option<(PromptLink, Option<ActiveProject>)> {
        self.cloned
            .get()
            .map(|(link, project)| (link, Some(project)))
            .or_else(|| self.pending.get().map(|link| (link, None)))
    }

    pub fn consume_route(self, is_clone: bool) {
        if is_clone {
            self.cloned.set(None);
        } else {
            self.pending.set(None);
        }
    }

    pub fn clone_repository_label(self) -> Option<String> {
        self.target_review
            .with(|review| Some(review.as_ref()?.clone_repository()?.slug()))
    }

    pub fn clone_error(self) -> Option<String> {
        self.target_review
            .with(|review| review.as_ref()?.error.clone())
    }

    pub fn clone_here(self, parent: String, on_complete: Callback<ActiveProject>) {
        let Some(review) = self
            .target_review
            .get_untracked()
            .filter(|review| !review.running)
        else {
            return;
        };
        let Some(repository) = review.clone_repository().cloned() else {
            return;
        };
        let id = review.checkout_id;
        self.start_operation();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::clone_prompt_repository(&parent, &repository.slug(), id).await;
            let result = match result {
                Ok(project) if is_expected_clone(&parent, &repository, &project) => {
                    self.cloned.set(Some((review.request, project.clone())));
                    on_complete.run(project);
                    Ok(())
                }
                Ok(_) => Err("The host returned a different clone destination.".to_owned()),
                Err(error) => Err(error.to_string()),
            };
            self.finish_operation(id, result);
        });
    }

    fn start_operation(self) {
        self.target_review.update(|review| {
            if let Some(review) = review {
                review.running = true;
                review.error = None;
            }
        });
    }

    fn finish_operation(self, id: uuid::Uuid, result: Result<(), String>) {
        self.target_review.update(|next| {
            if let Some(current) = next.as_mut().filter(|current| current.checkout_id == id) {
                match result {
                    Ok(()) => *next = None,
                    Err(error) => {
                        current.error = Some(error);
                        current.running = false;
                    }
                }
            }
        });
    }

    fn checkout(self, id: uuid::Uuid) {
        let Some(review) = self
            .target_review
            .get_untracked()
            .filter(|review| review.checkout_id == id && !review.running)
        else {
            return;
        };
        let (Some(project), PromptDestination::PullRequest(pull)) =
            (review.project, review.request.destination)
        else {
            return;
        };
        self.start_operation();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::checkout_prompt_pull(&project.path, &pull.url(), id).await;
            let result = match result {
                Ok(placed)
                    if placed.branch == pull.checkout_branch(id) && !placed.path.is_empty() =>
                {
                    self.checked_out.update(|queue| {
                        queue.push(CheckedOutPull {
                            project,
                            pull,
                            placed,
                            prompt: review.request.prompt,
                        })
                    });
                    Ok(())
                }
                Ok(_) => Err("The host returned a different pull request checkout.".to_owned()),
                Err(error) => Err(error.to_string()),
            };
            self.finish_operation(id, result);
        });
    }

    pub fn get(self, id: &str) -> Option<LinkedDraft> {
        self.drafts.with(|drafts| drafts.get(id).cloned())
    }

    pub fn set(self, id: String, draft: LinkedDraft) {
        self.drafts.update(|drafts| {
            drafts.insert(id, draft);
        });
    }

    pub fn forget(self, id: &str) {
        self.drafts.update(|drafts| {
            drafts.remove(id);
        });
    }
}

#[derive(Clone)]
struct TargetReview {
    request: PromptLink,
    project: Option<ActiveProject>,
    checkout_id: uuid::Uuid,
    running: bool,
    error: Option<String>,
}

impl TargetReview {
    fn clone_repository(&self) -> Option<&PromptRepository> {
        if self.project.is_some() {
            return None;
        }
        match &self.request.destination {
            PromptDestination::New { repository, .. } => repository.as_ref(),
            PromptDestination::PullRequest(pull) => Some(&pull.repository),
            PromptDestination::Thread(_) => None,
        }
    }
}

fn is_expected_clone(parent: &str, repository: &PromptRepository, project: &ActiveProject) -> bool {
    project.name == repository.name()
        && project
            .path
            .rsplit_once(['/', '\\'])
            .is_some_and(|(actual_parent, folder)| {
                folder == repository.name()
                    && actual_parent.trim_end_matches(['/', '\\'])
                        == parent.trim_end_matches(['/', '\\'])
            })
}

pub struct CheckedOutPull {
    pub project: ActiveProject,
    pub pull: PromptPullRequest,
    pub placed: crate::api::WorktreeCreated,
    pub prompt: Option<LinkedPrompt>,
}

pub fn existing_pull_thread(
    threads: &[crate::state::Thread],
    pull: &PromptPullRequest,
) -> Option<String> {
    threads
        .iter()
        .filter(|thread| {
            thread.worktree_path.is_some()
                && thread
                    .linked_pull
                    .as_deref()
                    .and_then(PromptPullRequest::parse)
                    .is_some_and(|known| pull.matches(&known))
        })
        .max_by_key(|thread| thread.updated_at)
        .map(|thread| thread.id.clone())
}

#[derive(Clone, Default)]
pub struct ResolvedTarget {
    pub project: Option<ActiveProject>,
    pub model: Option<String>,
    pub model_fallback: bool,
}

pub fn candidate_projects(
    threads: &[crate::state::Thread],
    recent: Vec<ActiveProject>,
) -> Vec<ActiveProject> {
    let mut activity = threads
        .iter()
        .filter_map(|thread| {
            let path = thread.project_path.as_ref()?;
            (!thread.project_name.is_empty()).then_some((
                thread.updated_at,
                ActiveProject {
                    path: path.clone(),
                    name: thread.project_name.clone(),
                },
            ))
        })
        .collect::<Vec<_>>();
    activity.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    let mut seen = std::collections::BTreeSet::new();
    activity
        .into_iter()
        .map(|(_, project)| project)
        .chain(recent)
        .filter(|project| seen.insert(project.path.clone()))
        .collect()
}

pub async fn resolve_target(
    destination: &PromptDestination,
    projects: Vec<ActiveProject>,
    selected_project: Option<ActiveProject>,
) -> ResolvedTarget {
    let (repository, model) = match destination {
        PromptDestination::New { repository, model } => (repository.as_ref(), model.as_ref()),
        PromptDestination::PullRequest(pull) => (Some(&pull.repository), None),
        PromptDestination::Thread(_) => (None, None),
    };
    let project = if selected_project.is_some() {
        selected_project
    } else if let Some(repository) = repository {
        let repository = repository.slug();
        let candidates = stream::iter(projects)
            .map(|project| {
                let repository = &repository;
                async move {
                    crate::api::prompt_repository_matches(&project.path, repository)
                        .await
                        .unwrap_or(false)
                        .then_some(project)
                }
            })
            .buffered(4);
        futures_util::pin_mut!(candidates);
        candidates.filter_map(std::future::ready).next().await
    } else {
        None
    };
    let mut result = ResolvedTarget {
        project,
        model: None,
        model_fallback: false,
    };
    if repository.is_some() && result.project.is_none() {
        return result;
    }
    if let Some(model) = model {
        let catalog = crate::api::prompt_writer_models(
            result.project.as_ref().map(|project| project.path.as_str()),
        )
        .await;
        let (preferred, exact) = catalog.as_ref().map_or((None, false), |catalog| {
            model.resolve(
                catalog
                    .models
                    .iter()
                    .map(|choice| (choice.provider, choice.id.as_str())),
            )
        });
        result.model = preferred.map(str::to_owned);
        result.model_fallback = !exact;
    }
    result
}

#[component]
pub fn PromptTargetReview() -> impl IntoView {
    let links = expect_context::<PromptLinks>();
    let ProjectPickerOpen(picker_open) = expect_context::<ProjectPickerOpen>();
    let SetProjectPickerOpen(set_picker_open) = expect_context::<SetProjectPickerOpen>();
    let ProjectPickerModeSignal(picker_mode) = expect_context::<ProjectPickerModeSignal>();
    let close = Callback::new(move |_| {
        if !links
            .target_review
            .with_untracked(|review| review.as_ref().is_some_and(|review| review.running))
        {
            links.target_review.set(None);
        }
    });
    let review_id = Memo::new(move |_| {
        links
            .target_review
            .with(|review| review.as_ref().map(|review| review.checkout_id))
    });
    let content = move || {
        let id = review_id.get()?;
        let review = links.target_review.get_untracked()?;
        let clone_label = review
            .clone_repository()
            .map(|repository| format!("Clone {}…", repository.slug()));
        let (detail, model, pull) = match review.request.destination {
            PromptDestination::New {
                repository: Some(repository),
                model,
            } => (
                format!(
                    "{} could not be matched to a saved project.",
                    repository.slug()
                ),
                model,
                None,
            ),
            PromptDestination::PullRequest(pull) => {
                let detail = match &review.project {
                    Some(project) => format!(
                        "Check out {} #{} in a new worktree for {}.",
                        pull.repository.slug(),
                        pull.number,
                        project.name
                    ),
                    None => format!(
                        "{} #{} could not be matched to a saved project.",
                        pull.repository.slug(),
                        pull.number
                    ),
                };
                (detail, None, Some(pull))
            }
            _ => return None,
        };
        let can_checkout = review.project.is_some() && pull.is_some();
        let mut prompt = review.request.prompt.unwrap_or(LinkedPrompt {
            text: String::new(),
            removed_hidden_characters: false,
        });
        let review_text = prompt.text.lines().next().unwrap_or_default().to_owned();
        if let Some(pull) = pull {
            prompt.text = if prompt.text.is_empty() {
                pull.url()
            } else {
                format!("{}\n\n{}", pull.url(), prompt.text)
            };
        }
        let running = Signal::derive(move || links.operation_running());
        Some(view! {
            <div data-prompt-target-review="" class="p-4 text-label-sm text-rambutan-300">
                    <h2 class="mb-2 text-body-sm font-medium text-rambutan-100">{if can_checkout { "Open pull request" } else { "Choose where to open this link" }}</h2>
                    <p role="status">{detail}</p>
                    {can_checkout.then(|| view! { <p class="mt-1">{PULL_CHECKOUT_SETUP_NOTICE}</p> })}
                    {(!review_text.is_empty()).then(|| view! { <p class="mt-1 truncate text-rambutan-400">{review_text}</p> })}
                    {move || links.target_review.with(|review| review.as_ref().filter(|review| review.checkout_id == id).and_then(|review| review.error.clone())).map(|error| view! { <p role="alert" class="mt-1 text-dragonfruit-400">{error}</p> })}
                    <div class="mt-3 flex flex-wrap justify-end gap-2">
                        <Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=running on_click=Callback::new(move |_| close.run(()))>"Cancel"</Button>
                        <Button variant=ButtonVariant::Ghost size=ButtonSize::Small disabled=running on_click=Callback::new(move |_| {
                            links.target_review.set(None);
                            links.pending.set(Some(PromptLink {
                                destination: PromptDestination::New { repository: None, model: model.clone() },
                                prompt: Some(prompt.clone()),
                            }));
                        })>"Use without project"</Button>
                        {can_checkout.then(|| view! {
                            <Button variant=ButtonVariant::Outline size=ButtonSize::Small disabled=running on_click=Callback::new(move |_| links.checkout(id))>
                                {move || if running.get() { "Checking out…" } else { "Check out" }}
                            </Button>
                        })}
                        {clone_label.map(|label| view! {
                            <Button variant=ButtonVariant::Outline size=ButtonSize::Small disabled=running on_click=Callback::new(move |_| {
                                picker_mode.set(ProjectPickerMode::PromptClone);
                                set_picker_open.set(true);
                            })>{label}</Button>
                        })}
                    </div>
            </div>
        })
    };
    view! {
        <Dialog open=Signal::derive(move || links.target_review.with(Option::is_some)
            && !(picker_open.get() && picker_mode.get() == ProjectPickerMode::PromptClone))
            on_close=close label="Linked prompt destination" max_width="max-w-lg">
            {content}
        </Dialog>
    }
}

fn parse_fragment(hash: &str) -> Option<(String, Result<PromptLink, PromptLinkError>)> {
    let (path, query) = hash.split_once('?')?;
    let (destination, route) = if path == "#/new" {
        ("#/chat".to_string(), "bruhs://new".to_string())
    } else {
        let id = path.strip_prefix("#thread/").filter(|id| !id.is_empty())?;
        (path.to_string(), format!("bruhs://thread/{id}"))
    };
    match parse_prompt_link(&format!("{route}?{query}")) {
        Ok(Some(link)) => Some((destination, Ok(link))),
        Ok(None) => None,
        Err(error) => Some((destination, Err(error))),
    }
}

pub fn copyable_link(text: &str) -> Result<String, PromptLinkError> {
    new_prompt_link(text).map(browser_link)
}

pub fn copyable_link_with_context(
    text: &str,
    context: PromptShareContext<'_>,
) -> Result<String, PromptLinkError> {
    new_prompt_link_with_context(text, context).map(browser_link)
}

fn browser_link(desktop: String) -> String {
    let Some((_, query)) = desktop.split_once('?') else {
        return desktop;
    };
    let location = window().location();
    let (Ok(origin), Ok(path)) = (location.origin(), location.pathname()) else {
        return desktop;
    };
    format!("{origin}{path}#/new?{query}")
}

#[component]
pub fn LinkedPromptBanner(locked: Signal<bool>) -> impl IntoView {
    let links = expect_context::<PromptLinks>();
    let ActiveThreadId(active) = expect_context::<ActiveThreadId>();
    let ComposerDraft(text) = expect_context::<ComposerDraft>();
    let ComposerChips(chips) = expect_context::<ComposerChips>();
    let ChipRetargetGuard(retarget) = expect_context::<ChipRetargetGuard>();
    let TextAttachments(attachments) = expect_context::<TextAttachments>();
    let files = expect_context::<FileOffers>();
    let drafts = expect_context::<crate::state::Drafts>();
    let has_attachments = move || !attachments.with(Vec::is_empty) || !files.with(<[_]>::is_empty);
    let current = move || {
        active
            .get()
            .and_then(|id| links.get(&id).map(|draft| (id, draft)))
    };
    let place = move |id: String, prompt: LinkedPrompt| {
        let caret = u32::try_from(prompt.text.encode_utf16().count()).unwrap_or_default();
        batch(|| {
            links.set(
                id.clone(),
                LinkedDraft::Placed {
                    removed_hidden_characters: prompt.removed_hidden_characters,
                },
            );
            retarget.set_value(true);
            chips.set(Vec::new());
            text.set(prompt.text);
        });
        request_animation_frame(move || {
            if active.get_untracked().as_ref() != Some(&id) {
                return;
            }
            if let Some(input) = document()
                .get_element_by_id("chat-input")
                .and_then(|node| node.dyn_into::<web_sys::HtmlTextAreaElement>().ok())
            {
                let _ = input.focus();
                let _ = input.set_selection_range(caret, caret);
            }
        });
    };
    Effect::new(move |_| match current() {
        Some((id, LinkedDraft::Incoming(prompt))) => {
            if !drafts.ready(&id) {
                return;
            }
            if locked.get_untracked()
                || untrack(has_attachments)
                || !chips.with_untracked(Vec::is_empty)
                || text.with_untracked(|text| !text.trim().is_empty() && *text != prompt.text)
            {
                links.set(id, LinkedDraft::Review(prompt));
            } else {
                place(id, prompt);
            }
        }
        Some((id, LinkedDraft::Placed { .. })) if text.with(|text| text.trim().is_empty()) => {
            links.forget(&id);
        }
        _ => {}
    });
    move || {
        let (id, draft) = current()?;
        let (proposal, hidden) = match draft {
            LinkedDraft::Incoming(_) => return None,
            LinkedDraft::Review(prompt) => (Some(prompt.clone()), prompt.removed_hidden_characters),
            LinkedDraft::Placed {
                removed_hidden_characters,
            } => (None, removed_hidden_characters),
        };
        let reviewing = proposal.is_some();
        let busy = reviewing && locked.get();
        let detail = if busy {
            "Finish your current edit to use the linked prompt."
        } else if reviewing && has_attachments() {
            "Replace the draft text and keep its attachments?"
        } else if reviewing {
            "Replace the existing draft with the linked prompt?"
        } else {
            "Review the draft before sending."
        };
        let target = id.clone();
        Some(view! {
            <div data-linked-prompt="" role="status"
                class="mx-3 mt-3 flex flex-wrap items-center justify-between gap-2 rounded-seed border border-longan-700 bg-longan-800/50 px-3 py-2 text-label-sm text-rambutan-300">
                <div class="min-w-0 flex-1 basis-48">
                    <p class="font-medium text-rambutan-100">
                        {if reviewing { "Linked prompt ready" } else { "Prompt added from a link" }}
                    </p>
                    <p>{detail}</p>
                    {hidden.then(|| view! { <p>"Hidden characters were removed."</p> })}
                </div>
                <div class="flex flex-wrap items-center gap-1">
                    <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| links.forget(&id))>
                        {if reviewing { "Keep draft" } else { "Dismiss" }}
                    </Button>
                    {proposal.filter(|_| !busy).map(|prompt| view! {
                        <Button variant=ButtonVariant::Outline size=ButtonSize::Small on_click=Callback::new(move |_| {
                            if !locked.get_untracked() && active.get_untracked().as_ref() == Some(&target) {
                                place(target.clone(), prompt.clone());
                            }
                        })>"Replace draft"</Button>
                    })}
                </div>
            </div>
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_reply_must_match_the_selected_parent_and_repository_name() {
        let repository = PromptRepository::parse("example/docs").unwrap();
        let project = |path: &str| ActiveProject {
            path: path.into(),
            name: "docs".into(),
        };
        assert!(is_expected_clone(
            "/fixtures/review/",
            &repository,
            &project("/fixtures/review/docs")
        ));
        assert!(is_expected_clone(
            "C:\\Review\\",
            &repository,
            &project("C:\\Review\\docs")
        ));
        assert!(!is_expected_clone(
            "/fixtures/review",
            &repository,
            &project("/fixtures/review-other/docs")
        ));
        assert!(!is_expected_clone(
            "/fixtures/review",
            &repository,
            &project("/fixtures/review/other")
        ));
    }

    #[test]
    fn linked_pull_reuses_only_a_matching_review_worktree() {
        let mut review = crate::state::Thread::new(
            "Review".into(),
            "docs".into(),
            Some("/fixture/docs".into()),
            bruhs_models::time::EpochMs::UNIX_EPOCH,
        );
        review.linked_pull = Some("https://github.com/Example/Docs/pull/42".into());
        let requested = PromptPullRequest::parse("example/docs#42").unwrap();
        assert!(existing_pull_thread(std::slice::from_ref(&review), &requested).is_none());
        review.worktree_path = Some("/fixture/review".into());
        let unrelated = PromptPullRequest::parse("example/other#42").unwrap();
        assert!(existing_pull_thread(std::slice::from_ref(&review), &unrelated).is_none());
        assert_eq!(
            existing_pull_thread(std::slice::from_ref(&review), &requested).as_deref(),
            Some(review.id.as_str())
        );
        let saved = serde_json::to_string(&review).unwrap();
        let restored: crate::state::Thread = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            existing_pull_thread(&[restored], &requested),
            Some(review.id)
        );
    }

    #[test]
    fn browser_fragments_share_prompt_validation_and_keep_ordinary_routes() {
        assert!(parse_fragment("#thread/thread-1").is_none());
        assert!(parse_fragment("#/settings?prompt=Review").is_none());
        let (route, new) = parse_fragment("#/new?prompt=Review%20this%20%E2%80%8Bnow").unwrap();
        assert_eq!(route, "#/chat");
        let new = new.unwrap();
        assert!(matches!(
            new.destination,
            PromptDestination::New {
                repository: None,
                model: None
            }
        ));
        let prompt = new.prompt.unwrap();
        assert!(prompt.removed_hidden_characters);
        assert_eq!(prompt.text, "Review this now");
        let (route, existing) = parse_fragment("#thread/thread-1?prompt=Already+saved").unwrap();
        assert_eq!(route, "#thread/thread-1");
        assert_eq!(
            existing.unwrap().destination,
            PromptDestination::Thread("thread-1".into())
        );
        assert_eq!(
            parse_fragment("#/new?prompt=a&prompt=b").unwrap().1,
            Err(PromptLinkError::Duplicate)
        );
    }
}
