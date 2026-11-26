//! Resolves and prepares unsent drafts on the paired host.
use super::*;
use bruhs_core::mobile::authority::MobileLease;
use bruhs_models::logic::app_link::{PromptDestination, PromptModel, PromptRepository};

pub(super) struct LinkedDraft {
    id: uuid::Uuid,
    target: PromptDestination,
    stage: Stage,
}

enum Stage {
    Preparing,
    Missing {
        repository: PromptRepository,
        parent: Option<PathBuf>,
        error: Option<String>,
    },
    Checkout {
        project: DesktopProjectLocation,
        error: Option<String>,
    },
    Finished(MobileLinkStatus),
}

pub(super) enum LinkWork {
    Resolve,
    Clone {
        repository: PromptRepository,
        parent: PathBuf,
    },
    Checkout(DesktopProjectLocation),
    WithoutProject,
}

enum WorkResult {
    Resolved(Vec<String>),
    Cloned(DesktopProjectLocation),
    CheckedOut(DesktopProjectLocation, PathBuf, String),
    Retry(Stage),
    WithoutProject,
}

impl LinkedDraft {
    fn review(&self) -> MobileLinkStatus {
        match &self.stage {
            Stage::Preparing => MobileLinkStatus::Preparing,
            Stage::Missing {
                repository,
                parent,
                error,
            } => MobileLinkStatus::MissingRepository {
                repository: repository.slug(),
                clone_parent: parent.as_deref().map(path_to_string),
                error: error.clone(),
            },
            Stage::Checkout { project, error } => MobileLinkStatus::Checkout {
                project: crate::views::chat::mobile_label(&project.name, 512),
                error: error.clone(),
            },
            Stage::Finished(status) => status.clone(),
        }
    }
}

impl Workspace {
    pub(super) fn mobile_link_status(
        &self,
        device: uuid::Uuid,
        link_id: uuid::Uuid,
    ) -> MobileResult<MobileReadReply> {
        let link = self
            .mobile
            .linked_drafts
            .get(&device)
            .filter(|link| link.id == link_id)
            .ok_or_else(stale_link)?;
        Ok(MobileReadReply::LinkedDraft(MobileLinkedDraft {
            link_id,
            state: link.review(),
        }))
    }

    pub(super) fn reserve_mobile_link(
        &mut self,
        device: uuid::Uuid,
        command_id: uuid::Uuid,
        action: &MobileAction,
    ) -> MobileResult<LinkWork> {
        match action {
            MobileAction::OpenLinkedDraft { target } => {
                if self
                    .mobile
                    .linked_drafts
                    .get(&device)
                    .is_some_and(|link| matches!(link.stage, Stage::Preparing))
                {
                    return Err(MobileError::Conflict(
                        "A linked draft is still being prepared.".into(),
                    ));
                }
                let target = target
                    .destination()
                    .map_err(|error| MobileError::Invalid(error.to_string()))?;
                self.mobile.linked_drafts.insert(
                    device,
                    LinkedDraft {
                        id: command_id,
                        target,
                        stage: Stage::Preparing,
                    },
                );
                Ok(LinkWork::Resolve)
            }
            MobileAction::ContinueLinkedDraft { link_id, choice } => {
                let link = self
                    .mobile
                    .linked_drafts
                    .get_mut(&device)
                    .filter(|link| link.id == *link_id)
                    .ok_or_else(stale_link)?;
                let work = match (&link.stage, choice) {
                    (
                        Stage::Missing {
                            repository,
                            parent: Some(parent),
                            ..
                        },
                        MobileLinkChoice::Clone,
                    ) => LinkWork::Clone {
                        repository: repository.clone(),
                        parent: parent.clone(),
                    },
                    (Stage::Checkout { project, .. }, MobileLinkChoice::Checkout) => {
                        LinkWork::Checkout(project.clone())
                    }
                    (
                        Stage::Missing { .. } | Stage::Checkout { .. },
                        MobileLinkChoice::WithoutProject,
                    ) => LinkWork::WithoutProject,
                    _ => return Err(stale_link()),
                };
                link.stage = Stage::Preparing;
                Ok(work)
            }
            _ => Err(MobileError::Invalid("Choose a linked draft action.".into())),
        }
    }

    pub(super) fn start_mobile_link(
        &mut self,
        connection: uuid::Uuid,
        lease: MobileLease,
        work: LinkWork,
        cx: &mut Context<'_, Self>,
    ) {
        let device = lease.id();
        let Some(link) = self.mobile.linked_drafts.get(&device) else {
            return;
        };
        let link_id = link.id;
        let target = link.target.clone();
        let paths = match &work {
            LinkWork::Resolve => self.prompt_project_paths(cx),
            _ => Vec::new(),
        };
        let setup = match &work {
            LinkWork::Checkout(project) => self.setup_steps(&project.path),
            _ => Vec::new(),
        };
        let work_lease = lease.clone();
        let request = crate::app_state::spawn_tokio(cx, async move {
            work_lease.check(MobileScope::LinkedDrafts)?;
            Ok::<_, MobileError>(run_link_work(work, target, paths, link_id, setup).await)
        });
        let window = self.workflow_window;
        cx.spawn(async move |workspace, cx| {
            let Ok(result) = request.await else {
                return;
            };
            let _ = window.update(cx, |_, window, cx| {
                let _ = workspace.update(cx, |this, cx| {
                    if !this.mobile.current(connection) {
                        return;
                    }
                    let _ = lease.with_scope(MobileScope::LinkedDrafts, || {
                        this.finish_mobile_link(device, link_id, result, window, cx);
                    });
                });
            });
        })
        .detach();
    }

    fn finish_mobile_link(
        &mut self,
        device: uuid::Uuid,
        link_id: uuid::Uuid,
        result: WorkResult,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(link) = self.mobile.linked_drafts.remove(&device) else {
            return;
        };
        if link.id != link_id || !matches!(link.stage, Stage::Preparing) {
            self.mobile.linked_drafts.insert(device, link);
            return;
        }
        let model = match &link.target {
            PromptDestination::New { model, .. } => model.as_ref(),
            _ => None,
        };
        let stage = match result {
            WorkResult::Resolved(paths) => {
                self.resolve_mobile_link(device, &link.target, &paths, window, cx)
            }
            WorkResult::WithoutProject => {
                self.create_mobile_link_thread(device, None, model, None, true, window, cx)
            }
            WorkResult::Cloned(project) => {
                project_picker_ui::push_recent_project(&mut self.recent_projects, project.clone());
                self.persist_and_notify(cx);
                match &link.target {
                    PromptDestination::PullRequest(_) => Stage::Checkout {
                        project,
                        error: None,
                    },
                    _ => self.create_mobile_link_thread(
                        device,
                        Some(project),
                        model,
                        None,
                        false,
                        window,
                        cx,
                    ),
                }
            }
            WorkResult::CheckedOut(project, path, branch) => {
                let PromptDestination::PullRequest(pull) = &link.target else {
                    return;
                };
                self.create_mobile_link_thread(
                    device,
                    Some(project),
                    None,
                    Some((path, branch, pull.number)),
                    false,
                    window,
                    cx,
                )
            }
            WorkResult::Retry(stage) => stage,
        };
        self.mobile
            .linked_drafts
            .insert(device, LinkedDraft { stage, ..link });
    }

    fn resolve_mobile_link(
        &mut self,
        device: uuid::Uuid,
        target: &PromptDestination,
        paths: &[String],
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Stage {
        let project = self.matching_prompt_project(paths);
        match target {
            PromptDestination::New {
                repository: Some(repository),
                ..
            } if project.is_none() => self.mobile_missing_repository(repository.clone(), cx),
            PromptDestination::New { model, .. } => self.create_mobile_link_thread(
                device,
                project,
                model.as_ref(),
                None,
                false,
                window,
                cx,
            ),
            PromptDestination::PullRequest(pull) => {
                if let Some(thread) = self.linked_pull_thread(pull, paths, cx) {
                    return ready(thread, false, false);
                }
                project.map_or_else(
                    || self.mobile_missing_repository(pull.repository.clone(), cx),
                    |project| Stage::Checkout {
                        project,
                        error: None,
                    },
                )
            }
            PromptDestination::Thread(_) => Stage::Finished(MobileLinkStatus::Failed {
                message: "Use a new draft link.".into(),
            }),
        }
    }

    fn mobile_missing_repository(&self, repository: PromptRepository, cx: &App) -> Stage {
        let parent = self
            .project_start_folder(cx)
            .map(|folder| {
                let home = project_picker_ui::project_picker_start_path();
                match folder.as_str() {
                    "~" => home,
                    path => home.join(path.strip_prefix("~/").unwrap_or(path)),
                }
            })
            .or_else(std::env::home_dir);
        Stage::Missing {
            repository,
            parent,
            error: None,
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Project, model, and checked out pull are independent link placement choices"
    )]
    fn create_mobile_link_thread(
        &mut self,
        device: uuid::Uuid,
        project: Option<DesktopProjectLocation>,
        model: Option<&PromptModel>,
        pull: Option<(PathBuf, String, u64)>,
        without_project: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Stage {
        let mut thread = match project {
            Some(project) => self.build_project_thread(project.name, project.path, window, cx),
            None => {
                let mut thread = Self::create_thread(
                    String::new(),
                    String::new(),
                    self.available_new_thread_model("", &self.current_model),
                    self.permission_mode,
                    window,
                    cx,
                );
                let path = match bruhs_core::worktrees::scratch::create(&thread.id) {
                    Ok(path) => path_to_string(&path),
                    Err(error) => {
                        return Stage::Finished(MobileLinkStatus::Failed {
                            message: mobile_link_error(&error),
                        })
                    }
                };
                thread.project_path = path.clone();
                thread.chat.update(cx, |chat, cx| {
                    chat.set_project_context(path.clone(), String::new(), false, cx);
                    chat.worktree_path = Some(path);
                    chat.run_locked = true;
                });
                self.seed_model_options(&thread, cx);
                self.stamp_chat_run_prefs(&thread.chat, cx);
                self.mobile.scratch_owners.insert(thread.id.clone(), device);
                thread
            }
        };
        let mut model_fallback = false;
        if let Some(model) = model {
            let path = thread.chat.read(cx).settings_project_path();
            let (preferred, exact) = self.resolve_prompt_link_model(Path::new(path), model);
            model_fallback = !exact;
            if let Some(model) = preferred {
                thread.model_id = model.clone();
                thread.chat.update(cx, |chat, cx| chat.set_model(model, cx));
            }
        }
        let is_pull = pull.is_some();
        if let Some((path, branch, number)) = pull {
            let project_path = thread.project_path.clone();
            let path = path_to_string(&path);
            thread.branch = Some(branch.clone());
            thread.chat.update(cx, |chat, cx| {
                chat.lock_place(Some(path), Some(branch.clone()), cx)
            });
            self.known_thread_pulls.insert(
                thread.id.clone(),
                pull_settlement::Observation::linked(project_path, branch, number),
            );
        }
        let stage = ready(&thread, model_fallback, without_project);
        let id = thread.id.clone();
        self.threads.push(thread);
        if is_pull {
            self.launch_thread_hooks(&id, false, cx);
        }
        self.persist_and_notify(cx);
        stage
    }
}

fn ready(thread: &DesktopThread, model_fallback: bool, without_project: bool) -> Stage {
    Stage::Finished(MobileLinkStatus::Ready {
        thread_id: thread.id.clone(),
        model: crate::views::chat::mobile_label(&thread.model_id, 512),
        model_fallback,
        without_project,
    })
}

fn stale_link() -> MobileError {
    MobileError::Conflict("This linked draft choice changed. Open the link again.".into())
}

fn mobile_link_error(error: &impl std::fmt::Display) -> String {
    crate::views::chat::mobile_label(&error.to_string(), 2048)
}

async fn run_link_work(
    work: LinkWork,
    target: PromptDestination,
    paths: Vec<String>,
    link_id: uuid::Uuid,
    setup: Vec<bruhs_core::git::SetupAction>,
) -> WorkResult {
    match work {
        LinkWork::Resolve => {
            let repository = match target {
                PromptDestination::New { repository, .. } => repository,
                PromptDestination::PullRequest(pull) => Some(pull.repository),
                PromptDestination::Thread(_) => None,
            };
            WorkResult::Resolved(app_links::matching_prompt_paths(paths, repository).await)
        }
        LinkWork::Clone { repository, parent } => {
            match bruhs_core::git::clone_prompt_repository(&parent, &repository, link_id).await {
                Ok(path) => WorkResult::Cloned(DesktopProjectLocation {
                    path: path_to_string(&path),
                    name: repository.name().to_owned(),
                }),
                Err(error) => WorkResult::Retry(Stage::Missing {
                    repository,
                    parent: Some(parent),
                    error: Some(mobile_link_error(&error)),
                }),
            }
        }
        LinkWork::Checkout(project) => {
            let PromptDestination::PullRequest(pull) = target else {
                return WorkResult::Retry(Stage::Finished(MobileLinkStatus::Failed {
                    message: "This link has no pull request.".into(),
                }));
            };
            match bruhs_core::git::checkout_linked_pull(
                Path::new(&project.path),
                &pull,
                link_id,
                &setup,
            )
            .await
            {
                Ok((path, branch)) => WorkResult::CheckedOut(project, path, branch),
                Err(error) => WorkResult::Retry(Stage::Checkout {
                    project,
                    error: Some(mobile_link_error(&error)),
                }),
            }
        }
        LinkWork::WithoutProject => WorkResult::WithoutProject,
    }
}
