use super::*;
use bruhs_ui::press::on_press_view;

pub(super) struct CloneEditor {
    remote: Entity<InputState>,
    pub open: bool,
    running: bool,
    pub generation: u64,
    pub(super) linked_prompt: Option<super::app_links::CloneTarget>,
}

impl CloneEditor {
    pub(super) fn new(window: &mut Window, cx: &mut App) -> Self {
        Self {
            remote: cx.new(|cx| {
                InputState::new(window, cx).placeholder("https://github.com/owner/repo.git")
            }),
            open: false,
            running: false,
            generation: 0,
            linked_prompt: None,
        }
    }
}

impl Workspace {
    pub(super) fn open_link_clone(
        &mut self,
        repository: &bruhs_models::logic::app_link::PromptRepository,
        target: super::app_links::CloneTarget,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.open_project_picker(&OpenProjectPicker, window, cx);
        self.clone_editor.open = true;
        self.clone_editor.linked_prompt = Some(target);
        self.clone_editor.remote.update(cx, |field, cx| {
            field.set_value(
                format!("https://github.com/{}.git", repository.slug()),
                window,
                cx,
            );
        });
        cx.notify();
    }

    pub(super) fn clone_project_controls(&self, cx: &Context<'_, Self>) -> AnyElement {
        let mut form = div().px_3().py_1().flex().flex_col().gap_2();
        let toggle = web_button(
            "project-clone-show",
            WebButtonVariant::Ghost,
            WebButtonSize::Small,
            ButtonState::disabled_if(self.clone_editor.running),
        )
        .child(if self.clone_editor.running {
            "Cloning repository…"
        } else if self.clone_editor.open {
            "Cancel"
        } else {
            "Clone repository…"
        });
        form = form.child(on_press_view(toggle, cx, |this, window, cx| {
            if this.clone_editor.linked_prompt.is_some() {
                this.clone_editor
                    .remote
                    .update(cx, |field, cx| field.set_value("", window, cx));
                this.close_transient_surfaces();
                cx.notify();
                return;
            }
            this.clone_editor.open = !this.clone_editor.open;
            if this.clone_editor.open {
                this.clone_editor
                    .remote
                    .update(cx, |field, cx| field.focus(window, cx));
            }
            cx.notify();
        }));
        if self.clone_editor.open {
            let clone = web_button(
                "project-clone-submit",
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::disabled_if(self.clone_editor.running),
            )
            .child("Clone into this folder");
            form = form.child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div().min_w_0().flex_1().child(
                            Input::new(&self.clone_editor.remote)
                                .aria_label("Repository URL")
                                .disabled(
                                    self.clone_editor.running
                                        || self.clone_editor.linked_prompt.is_some(),
                                ),
                        ),
                    )
                    .child(on_press_view(clone, cx, |this, window, cx| {
                        this.begin_project_clone(window, cx)
                    })),
            );
        }
        form.into_any_element()
    }

    fn begin_project_clone(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.clone_editor.running
            || !self.project_picker_visible()
            || self.project_picker_for_terminal()
        {
            return;
        }
        let remote = self.clone_editor.remote.read(cx).value().trim().to_owned();
        if remote.is_empty() {
            self.project_picker_error = Some("Enter a repository URL.".into());
            cx.notify();
            return;
        }
        let base = self.project_picker_current_path.clone();
        let grouped = self.clone_host_folders;
        let generation = self.clone_editor.generation;
        self.clone_editor.running = true;
        self.project_picker_error = None;
        let clone = crate::app_state::spawn_tokio(cx, async move {
            let base = project_picker_ui::canonical_project_location(&base, "")?;
            let dest =
                bruhs_core::git::clone_project(std::path::Path::new(&base.path), &remote, grouped)
                    .await
                    .map_err(|e| e.to_string())?;
            project_picker_ui::canonical_project_location(&dest.to_string_lossy(), "")
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = clone.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.clone_editor.running = false;
                let same_picker =
                    this.clone_editor.generation == generation && this.project_picker_visible();
                match result {
                    Ok(project) if same_picker => {
                        this.remember_project(project.clone(), cx);
                        this.persist(cx);
                        this.clone_editor
                            .remote
                            .update(cx, |field, cx| field.set_value("", window, cx));
                        if let Some(target) = this.clone_editor.linked_prompt.take() {
                            this.finish_link_clone(project, target, window, cx);
                        } else {
                            this.apply_thread_project(Some(project), window, cx);
                        }
                    }
                    Ok(project) => {
                        let path = project.path.clone();
                        this.remember_project(project, cx);
                        this.meetings_hub.update(cx, |hub, cx| {
                            hub.show_notice(format!("Cloned to {path}"), false, cx)
                        });
                        this.persist_and_notify(cx);
                    }
                    Err(error) if same_picker => this.project_picker_error = Some(error),
                    Err(error) => this
                        .meetings_hub
                        .update(cx, |hub, cx| hub.show_notice(error, true, cx)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
