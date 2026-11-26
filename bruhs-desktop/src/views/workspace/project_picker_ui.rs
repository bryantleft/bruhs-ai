//! The project picker: browsing directories off the UI thread, recents, and opening a project session.

use super::*;
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;

pub(super) const PROJECT_PICKER_BODY_FONT_FAMILY: &str = "Monaspace Neon";
pub(super) const PROJECT_PICKER_LABEL_FONT_FAMILY: &str = "Monaspace Argon";
pub(super) const PROJECT_PICKER_PATH_FONT_FAMILY: &str = "Monaspace Neon";
pub(super) const PROJECT_PICKER_ROW_RADIUS_PX: f32 = 8.0;
/// Row icons, beside the rows' 12px names.
pub(super) const PROJECT_PICKER_ICON_SIZE_PX: f32 = theme::ICON_XS_PX;
pub(super) const PROJECT_PICKER_PARENT_ICON_SLOT_WIDTH_PX: f32 = 14.0;
pub(super) const PROJECT_PICKER_CONTENT_MAX_HEIGHT_PX: f32 = 288.0;
pub(super) const PROJECT_PICKER_LABEL_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const PROJECT_PICKER_SECTION_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const PROJECT_PICKER_FOOTER_TEXT_SIZE_PX: f32 = 12.0;
#[cfg(test)]
pub(super) const PROJECT_PICKER_OVERLAY_OPACITY: f32 = bruhs_ui::dialog::DIALOG_OVERLAY_OPACITY;
pub(super) const PROJECT_PICKER_HEADER_X_PADDING_PX: f32 = 16.0;
pub(super) const PROJECT_PICKER_HEADER_Y_PADDING_PX: f32 = 12.0;
pub(super) const PROJECT_PICKER_HEADER_GAP_PX: f32 = 12.0;
pub(super) const PROJECT_PICKER_PATH_TOP_MARGIN_PX: f32 = 2.0;
pub(super) const PROJECT_PICKER_SELECT_MARGIN_LEFT_PX: f32 = 12.0;
pub(super) const PROJECT_PICKER_SELECT_X_PADDING_PX: f32 = 12.0;
pub(super) const PROJECT_PICKER_SELECT_Y_PADDING_PX: f32 = 6.0;
pub(super) const PROJECT_PICKER_SELECT_BG_OPACITY: f32 = 0.20;
pub(super) const PROJECT_PICKER_SELECT_HOVER_BG_OPACITY: f32 = 0.30;
pub(super) const PROJECT_PICKER_RECENTS_X_PADDING_PX: f32 = 16.0;
pub(super) const PROJECT_PICKER_RECENTS_Y_PADDING_PX: f32 = 12.0;
pub(super) const PROJECT_PICKER_RECENTS_GAP_PX: f32 = 2.0;
pub(super) const PROJECT_PICKER_RECENTS_LABEL_BOTTOM_MARGIN_PX: f32 = 4.0;
pub(super) const PROJECT_PICKER_RECENTS_BORDER_OPACITY: f32 = 0.50;
pub(super) const PROJECT_PICKER_RECENT_ROW_GAP_PX: f32 = 8.0;
pub(super) const PROJECT_PICKER_RECENT_ROW_X_PADDING_PX: f32 = 8.0;
pub(super) const PROJECT_PICKER_RECENT_ROW_Y_PADDING_PX: f32 = 6.0;
pub(super) const PROJECT_PICKER_RECENT_ROW_HOVER_BG_OPACITY: f32 = 0.50;
pub(super) const PROJECT_PICKER_LIST_Y_PADDING_PX: f32 = 4.0;
pub(super) const PROJECT_PICKER_ROW_GAP_PX: f32 = 8.0;
pub(super) const PROJECT_PICKER_ROW_X_PADDING_PX: f32 = 16.0;
pub(super) const PROJECT_PICKER_ROW_Y_PADDING_PX: f32 = 8.0;
pub(super) const PROJECT_PICKER_ROW_HOVER_BG_OPACITY: f32 = 0.40;
pub(super) const PROJECT_PICKER_EMPTY_X_PADDING_PX: f32 = 16.0;
pub(super) const PROJECT_PICKER_EMPTY_Y_PADDING_PX: f32 = 24.0;
pub(super) const PROJECT_PICKER_FOOTER_X_PADDING_PX: f32 = 16.0;
pub(super) const PROJECT_PICKER_FOOTER_Y_PADDING_PX: f32 = 10.0;
pub(super) const PROJECT_PICKER_FOOTER_GAP_PX: f32 = 12.0;
pub(super) const PROJECT_PICKER_FOOTER_BORDER_OPACITY: f32 = 0.50;
pub(super) const PROJECT_PICKER_LIST_ERROR: &str = "Could not list directory";
pub(super) const MAX_RECENT_PROJECTS: usize = 5;

pub(super) fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

pub(super) fn path_display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| path_to_string(path), str::to_string)
}

pub(super) fn home_dir_path() -> Option<PathBuf> {
    std::env::home_dir()
}

pub(super) fn project_picker_start_path() -> PathBuf {
    home_dir_path().unwrap_or_else(|| {
        std::env::current_dir()
            .ok()
            .unwrap_or_else(|| PathBuf::from("."))
    })
}

pub(super) fn read_project_directory(
    path: Option<&str>,
) -> Result<DesktopDirectoryListing, String> {
    let target = path
        .filter(|path| !path.is_empty())
        .map_or_else(project_picker_start_path, PathBuf::from);
    let resolved = fs::canonicalize(&target).map_err(|_| "Cannot read directory".to_string())?;

    let home = home_dir_path().and_then(|path| fs::canonicalize(path).ok());
    if let Some(home) = &home {
        if !resolved.starts_with(home) {
            return Err("Path must be within your home directory".to_string());
        }
    }

    let mut dirs = fs::read_dir(&resolved)
        .map_err(|_| "Cannot read directory".to_string())?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            let is_dir = entry.file_type().is_ok_and(|file_type| file_type.is_dir());
            is_dir.then(|| DesktopProjectLocation {
                path: path_to_string(&entry.path()),
                name,
            })
        })
        .collect::<Vec<_>>();
    dirs.sort_by(|a, b| a.name.cmp(&b.name));

    let parent = match &home {
        Some(home) if resolved == *home => None,
        _ => resolved.parent().map(path_to_string),
    };

    Ok(DesktopDirectoryListing {
        current: path_to_string(&resolved),
        name: path_display_name(&resolved),
        parent,
        dirs,
    })
}

pub(super) fn canonical_project_location(
    path: &str,
    fallback_name: &str,
) -> Result<DesktopProjectLocation, String> {
    if path.is_empty() {
        return Err("Path is required".to_string());
    }

    let resolved = fs::canonicalize(path).map_err(|_| "Cannot read directory".to_string())?;
    let meta = fs::metadata(&resolved).map_err(|_| "Cannot read directory".to_string())?;
    if !meta.is_dir() {
        return Err("Path is not a directory".to_string());
    }

    let home = home_dir_path().and_then(|path| fs::canonicalize(path).ok());
    if let Some(home) = &home {
        if !resolved.starts_with(home) {
            return Err("Path must be within your home directory".to_string());
        }
    }

    let name = path_display_name(&resolved);
    Ok(DesktopProjectLocation {
        path: path_to_string(&resolved),
        name: if name.is_empty() {
            fallback_name.to_string()
        } else {
            name
        },
    })
}

pub(super) fn push_recent_project(
    recent_projects: &mut Vec<DesktopProjectLocation>,
    project: DesktopProjectLocation,
) {
    recent_projects.retain(|existing| existing.path != project.path);
    recent_projects.insert(0, project);
    recent_projects.truncate(MAX_RECENT_PROJECTS);
}

pub(super) fn project_picker_directory_error_like_web(_error: &str) -> String {
    PROJECT_PICKER_LIST_ERROR.to_string()
}

fn projectless_option(id: &'static str, label: &'static str, icon: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_owned())
        .mx(px(16.0))
        .my(px(4.0))
        .px(px(8.0))
        .h(px(32.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .rounded(px(8.0))
        .text_size(px(12.0))
        .text_color(theme::rambutan_300())
        .role(Role::Button)
        .aria_label(label)
        .tab_index(0)
        .border_1()
        .border_color(gpui::transparent_black())
        .focus_visible(|el| el.border_color(theme::persimmon_400()))
        .hover(|el| el.bg(theme::longan_800().opacity(0.5)))
        .cursor_pointer()
        .child(
            svg()
                .path(icon)
                .size(px(PROJECT_PICKER_ICON_SIZE_PX))
                .flex_shrink_0()
                .text_color(theme::rambutan_500()),
        )
        .child(label)
}

impl Workspace {
    /// List `path` (home when `None`) in the project picker. The directory
    /// is read off the UI thread; a newer request replaces an older one.
    pub(super) fn load_project_directory(
        &mut self,
        path: Option<String>,
        cx: &mut Context<'_, Self>,
    ) {
        let read = cx.background_spawn(async move { read_project_directory(path.as_deref()) });
        self.project_picker_task = Some(cx.spawn(async move |this, cx| {
            let listing = read.await;
            // The workspace may have closed meanwhile; nothing to show then.
            let _closed = this.update(cx, |this, cx| {
                this.apply_project_directory(listing);
                cx.notify();
            });
        }));
    }

    pub(super) fn apply_project_directory(
        &mut self,
        listing: Result<DesktopDirectoryListing, String>,
    ) {
        self.project_picker_start_warning = None;
        match listing {
            Ok(listing) => {
                self.project_picker_scroll.set_offset(Point::default());
                self.project_picker_current_path = listing.current;
                self.project_picker_current_name = listing.name;
                self.project_picker_parent_path = listing.parent;
                self.project_picker_dirs = listing.dirs;
                self.project_picker_error = None;
            }
            Err(error) => {
                self.project_picker_error = Some(project_picker_directory_error_like_web(&error));
            }
        }
    }

    pub(super) fn create_project_thread(
        &mut self,
        project: DesktopProjectLocation,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let scoped_access = self
            .project_defaults
            .get(&project.path)
            .is_some_and(|p| p.permissions.is_some());
        let (model, permissions) = self.new_thread_defaults(&project.path);
        let thread = Self::create_thread(
            project.name.clone(),
            project.path.clone(),
            model,
            permissions,
            window,
            cx,
        );
        self.seed_model_options(&thread, cx);
        self.stamp_chat_run_prefs(&thread.chat, cx);
        thread.chat.update(cx, |chat, cx| {
            chat.project_permission_override = scoped_access;
            chat.tasks_panel_visible = self.tasks_panel_visible;
            chat.diff_panel_visible = self.diff_panel_visible;
            cx.notify();
        });
        self.active_thread_id = Some(thread.id.clone());
        self.threads.push(thread);
        self.active_view = AppView::Chat;
        self.close_transient_surfaces();
        self.active_project = Some(project.clone());
        self.remember_project(project, cx);
        self.persist_and_notify(cx);
    }

    pub(super) fn apply_thread_project(
        &mut self,
        project: Option<DesktopProjectLocation>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.clone_editor.linked_prompt.is_some() {
            return;
        }
        let Some(TopLayer::ProjectPicker {
            for_terminal: false,
            thread: target,
        }) = self.top_layer
        else {
            return;
        };
        if let Some(target) = target {
            let Some(thread) = self
                .threads
                .iter_mut()
                .find(|thread| thread.chat.entity_id() == target)
            else {
                self.project_picker_error = Some(
                    "This thread is no longer available. Close the picker and choose a thread."
                        .into(),
                );
                cx.notify();
                return;
            };
            let name = project.as_ref().map(|p| p.name.clone()).unwrap_or_default();
            let path = project.as_ref().map_or_else(
                || path_to_string(&project_picker_start_path()),
                |p| p.path.clone(),
            );
            let changed = thread.project_name != name || thread.project_path != path;
            let result = thread.chat.update(cx, |chat, cx| {
                chat.change_project(path.clone(), name.clone(), project.is_some(), cx)
            });
            if let Err(error) = result {
                self.project_picker_error = Some(error.into());
                cx.notify();
                return;
            }
            thread.project_name = name;
            thread.project_path = path;
            if changed {
                thread.branch = None;
                self.base_branch.clear();
            }
            let chat = thread.chat.clone();
            self.active_thread_id = Some(thread.id.clone());
            self.active_view = AppView::Chat;
            self.stamp_chat_run_prefs(&chat, cx);
            self.close_transient_surfaces();
            if self.diff_panel_visible {
                self.sync_diff_panel_project(cx);
            }
            if self.files_panel_visible {
                self.load_files(cx);
            }
            self.refresh_branch_pull(cx);
        } else {
            match project.clone() {
                Some(project) => self.create_project_thread(project, window, cx),
                None => self.new_general_thread(window, cx),
            }
        }
        if let Some(project) = project {
            self.active_project = Some(project.clone());
            self.remember_project(project, cx);
        }
        self.persist_and_notify(cx);
    }

    pub(super) fn select_project_location(
        &mut self,
        path: String,
        fallback_name: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let target = self.top_layer;
        let resolve =
            cx.background_spawn(async move { canonical_project_location(&path, &fallback_name) });
        self.project_picker_task = Some(cx.spawn_in(window, async move |this, cx| {
            let location = resolve.await;
            // The workspace may have closed meanwhile; nothing to open then.
            let _closed = this.update_in(cx, |this, window, cx| {
                if !this.project_picker_visible() || this.top_layer != target {
                    return;
                }
                match location {
                    Ok(project) if this.project_picker_for_terminal() => {
                        this.open_terminal_in_project(project, window, cx)
                    }
                    Ok(project) => this.apply_thread_project(Some(project), window, cx),
                    Err(error) => {
                        this.project_picker_error = Some(error);
                        cx.notify();
                    }
                }
            });
        }));
    }

    pub(super) fn project_start_folder(&self, cx: &App) -> Option<String> {
        let input = self.project_start_folder_input.read(cx).value();
        let value = input.trim();
        (!value.is_empty()).then(|| value.to_owned())
    }

    fn load_project_start_folder(&mut self, folder: String, cx: &mut Context<'_, Self>) {
        let read = cx.background_spawn(async move {
            let home = project_picker_start_path();
            let path = match folder.trim() {
                "~" => home,
                value => home.join(value.strip_prefix("~/").unwrap_or(value)),
            };
            match read_project_directory(path.to_str()) {
                Ok(listing) => (Ok(listing), None),
                Err(error) => (
                    read_project_directory(None),
                    Some(format!(
                        "Couldn’t open {folder}. {error}. Showing your home folder."
                    )),
                ),
            }
        });
        self.project_picker_task = Some(cx.spawn(async move |this, cx| {
            let (listing, warning) = read.await;
            let _ = this.update(cx, |this, cx| {
                this.apply_project_directory(listing);
                if this.project_picker_error.is_none() {
                    this.project_picker_start_warning = warning;
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn open_project_picker(
        &mut self,
        _: &OpenProjectPicker,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.refresh_project_identities(cx);
        self.refresh_linked_projects(false, cx);
        self.project_picker_task.take();
        self.project_picker_start_warning = None;
        let thread = self.active_thread().map(|thread| thread.chat.entity_id());
        self.project_picker_error = None;
        self.open_top_layer(TopLayer::ProjectPicker {
            for_terminal: false,
            thread,
        });
        self.focus_first_modal_tab_stop(window, cx);
        if let Some(folder) = self.project_start_folder(cx) {
            self.load_project_start_folder(folder, cx);
        } else {
            self.load_project_directory(None, cx);
        }
    }

    pub(super) fn render_project_picker(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let current_path = self.project_picker_current_path.clone();
        let current_name = self.project_picker_current_name.clone();
        let has_current_path = !current_path.is_empty();
        let header_path = current_path.clone();
        let select_current_path = current_path;
        let select_current_name = current_name;

        let mut recent_rows = Vec::new();
        for group in self.checkout_groups(!self.project_picker_for_terminal()) {
            if group.members.len() > 1 {
                let key = group.key;
                let expanded = self.expanded_project_groups.contains(&key);
                let group_id = element_key(&key);
                let button = div()
                    .id(("recent-group", group_id))
                    .debug_selector(move || format!("desktop-project-group-{group_id}"))
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_expanded(expanded)
                    .aria_label(format!(
                        "{}, {} checkouts",
                        group.label,
                        group.members.len()
                    ))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .rounded(px(PROJECT_PICKER_ROW_RADIUS_PX))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .hover(|el| el.bg(theme::longan_800()))
                    .text_size(px(PROJECT_PICKER_SECTION_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_300())
                    .child(
                        svg()
                            .path(assets::CHEVRON_DOWN_ICON_PATH)
                            .size(px(PROJECT_PICKER_ICON_SIZE_PX))
                            .when(!expanded, |el| {
                                el.with_transformation(Transformation::rotate(Radians(
                                    -std::f32::consts::FRAC_PI_2,
                                )))
                            }),
                    )
                    .child(div().min_w_0().flex_1().truncate().child(group.label))
                    .child(group.members.len().to_string());
                recent_rows.push(
                    on_press_view(button, cx, move |ws, _, cx| {
                        if !ws.expanded_project_groups.remove(&key) {
                            ws.expanded_project_groups.insert(key.clone());
                        }
                        cx.notify();
                    })
                    .into_any_element(),
                );
                if !expanded {
                    continue;
                }
            }
            for project in group.members {
                let path = project.path.clone();
                let name = project.name.clone();
                let debug_key = element_key(&project.scope());
                let remote = project.host.map(|_| project.clone());
                let display = if remote.is_some() {
                    format!(
                        "View in Settings · {} · {}",
                        project.environment, project.path
                    )
                } else {
                    project.path.clone()
                };
                let display = if project.has_custom_name {
                    format!("{} · {display}", project.name)
                } else {
                    display
                };
                recent_rows.push(
                    div()
                        .id(("desktop-project-recent", debug_key))
                        .debug_selector(move || format!("desktop-project-recent-{debug_key}"))
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(PROJECT_PICKER_RECENT_ROW_GAP_PX))
                        .rounded(px(PROJECT_PICKER_ROW_RADIUS_PX))
                        .px(px(PROJECT_PICKER_RECENT_ROW_X_PADDING_PX))
                        .py(px(PROJECT_PICKER_RECENT_ROW_Y_PADDING_PX))
                        .font_family(
                            crate::theme::FontFace::Interface
                                .with_default(PROJECT_PICKER_LABEL_FONT_FAMILY),
                        )
                        .text_size(px(PROJECT_PICKER_LABEL_TEXT_SIZE_PX))
                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                        .text_color(theme::rambutan_400())
                        .tab_index(0)
                        .role(Role::Button)
                        .aria_label(if remote.is_some() {
                            format!("View checkout {display} in Settings")
                        } else {
                            format!("Open recent project {display}")
                        })
                        .border_1()
                        .border_color(gpui::transparent_black())
                        .focus_visible(|el| el.border_color(theme::persimmon_400()))
                        .cursor_pointer()
                        .hover(|el| {
                            el.bg(theme::longan_800()
                                .opacity(PROJECT_PICKER_RECENT_ROW_HOVER_BG_OPACITY))
                                .text_color(theme::rambutan_200())
                        })
                        .map(|el| {
                            on_press_view(el, cx, move |this, window, cx| {
                                if let Some(member) = &remote {
                                    this.open_linked_checkout(member, window, cx);
                                } else {
                                    this.select_project_location(
                                        path.clone(),
                                        name.clone(),
                                        window,
                                        cx,
                                    );
                                }
                            })
                        })
                        .child(self.project_icon(&project.scope(), PROJECT_PICKER_ICON_SIZE_PX, cx))
                        .child(div().min_w_0().flex_1().truncate().child(display))
                        .into_any_element(),
                );
            }
        }
        let show_recents = !recent_rows.is_empty();

        let mut directory_rows = Vec::new();
        if let Some(parent_path) = &self.project_picker_parent_path {
            let parent_path = parent_path.clone();
            directory_rows.push(
                div()
                    .id("desktop-project-parent")
                    .debug_selector(|| "desktop-project-parent".to_string())
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(PROJECT_PICKER_ROW_GAP_PX))
                    .px(px(PROJECT_PICKER_ROW_X_PADDING_PX))
                    .py(px(PROJECT_PICKER_ROW_Y_PADDING_PX))
                    .font_family(
                        crate::theme::FontFace::Interface
                            .with_default(PROJECT_PICKER_LABEL_FONT_FAMILY),
                    )
                    .text_size(px(PROJECT_PICKER_LABEL_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_500())
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label("Go to parent folder")
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| {
                        el.bg(theme::longan_800().opacity(PROJECT_PICKER_ROW_HOVER_BG_OPACITY))
                    })
                    .map(|el| {
                        on_press_view(el, cx, move |this, _window, cx| {
                            this.load_project_directory(Some(parent_path.clone()), cx);
                        })
                    })
                    .child(
                        div()
                            .w(px(PROJECT_PICKER_PARENT_ICON_SLOT_WIDTH_PX))
                            .flex_shrink_0()
                            .child("↑"),
                    )
                    .child("..")
                    .into_any_element(),
            );
        }
        for dir in &self.project_picker_dirs {
            let browse_path = dir.path.clone();
            let browse_path_for_key = dir.path.clone();
            let select_path = dir.path.clone();
            let select_name = dir.name.clone();
            let debug_key = element_key(&dir.path);
            directory_rows.push(
                div()
                    .id(("desktop-project-dir", element_key(&dir.path)))
                    .debug_selector(move || format!("desktop-project-dir-{debug_key}"))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(PROJECT_PICKER_ROW_GAP_PX))
                    .px(px(PROJECT_PICKER_ROW_X_PADDING_PX))
                    .py(px(PROJECT_PICKER_ROW_Y_PADDING_PX))
                    .font_family(
                        crate::theme::FontFace::Interface
                            .with_default(PROJECT_PICKER_LABEL_FONT_FAMILY),
                    )
                    .text_size(px(PROJECT_PICKER_LABEL_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_400())
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label(format!("Open folder {}", dir.name))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| {
                        el.bg(theme::longan_800().opacity(PROJECT_PICKER_ROW_HOVER_BG_OPACITY))
                            .text_color(theme::rambutan_200())
                    })
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        // Enter/Space already acted on key down.
                        if ev.is_keyboard() {
                            return;
                        }
                        if ev.click_count() == 2 {
                            this.select_project_location(
                                select_path.clone(),
                                select_name.clone(),
                                window,
                                cx,
                            );
                        } else {
                            this.load_project_directory(Some(browse_path.clone()), cx);
                        }
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.load_project_directory(Some(browse_path_for_key.clone()), cx);
                        }
                    }))
                    .child(
                        svg()
                            .path(assets::FOLDER_ICON_PATH)
                            .size(px(PROJECT_PICKER_ICON_SIZE_PX))
                            .text_color(theme::rambutan_500())
                            .flex_shrink_0(),
                    )
                    .child(div().min_w_0().flex_1().truncate().child(dir.name.clone()))
                    .into_any_element(),
            );
        }
        if self.project_picker_dirs.is_empty() && self.project_picker_error.is_none() {
            directory_rows.push(
                div()
                    .px(px(PROJECT_PICKER_EMPTY_X_PADDING_PX))
                    .py(px(PROJECT_PICKER_EMPTY_Y_PADDING_PX))
                    .text_center()
                    .font_family(
                        crate::theme::FontFace::Interface
                            .with_default(PROJECT_PICKER_LABEL_FONT_FAMILY),
                    )
                    .text_size(px(PROJECT_PICKER_LABEL_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_500())
                    .child("No subdirectories")
                    .into_any_element(),
            );
        }

        bruhs_ui::dialog::dialog_backdrop(
            "desktop-project-picker-overlay",
            bruhs_ui::dialog::DialogPlacement::Top,
        )
        .debug_selector(|| "desktop-project-picker-overlay".to_string())
        .on_click(cx.listener(|this, _ev, _window, cx| {
            this.close_transient_surfaces();
            cx.notify();
        }))
        .child(
            bruhs_ui::dialog::dialog_card(
                "desktop-project-picker-dialog",
                "Choose project folder",
                Role::Dialog,
                &self.blur_scrim,
                cx,
            )
            .debug_selector(|| "desktop-project-picker-dialog".to_string())
            .w(px(DESKTOP_DIALOG_MD_WIDTH_PX))
            .max_h(modal_ui::picker_max_height(window))
            .font_family(
                crate::theme::FontFace::Interface.with_default(PROJECT_PICKER_BODY_FONT_FAMILY),
            )
            .track_focus(&self.modal_start_focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.trap_modal_tab_focus(event, window, cx);
            }))
            .on_click(cx.listener(|_this, _ev, _window, cx| {
                cx.stop_propagation();
            }))
            .child(
                div()
                    .flex_shrink_0()
                    .px(px(PROJECT_PICKER_HEADER_X_PADDING_PX))
                    .py(px(PROJECT_PICKER_HEADER_Y_PADDING_PX))
                    .border_b_1()
                    .border_color(theme::longan_800())
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(PROJECT_PICKER_HEADER_GAP_PX))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .font_family(
                                        crate::theme::FontFace::Interface
                                            .with_default(PROJECT_PICKER_LABEL_FONT_FAMILY),
                                    )
                                    .text_size(px(PROJECT_PICKER_LABEL_TEXT_SIZE_PX))
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::rambutan_200())
                                    .child("Choose project folder"),
                            )
                            .when(has_current_path, |el| {
                                el.child(
                                    div()
                                        .mt(px(PROJECT_PICKER_PATH_TOP_MARGIN_PX))
                                        .truncate()
                                        .text_size(px(PROJECT_PICKER_LABEL_TEXT_SIZE_PX))
                                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                                        .font_family(
                                            crate::theme::FontFace::Code
                                                .with_default(PROJECT_PICKER_PATH_FONT_FAMILY),
                                        )
                                        .text_color(theme::rambutan_500())
                                        .child(header_path.clone()),
                                )
                            }),
                    )
                    .when(
                        has_current_path && self.clone_editor.linked_prompt.is_none(),
                        |el| {
                            el.child(
                                div()
                                    .id("desktop-project-select-current")
                                    .debug_selector(|| "desktop-project-select-current".to_string())
                                    .ml(px(PROJECT_PICKER_SELECT_MARGIN_LEFT_PX))
                                    .flex_shrink_0()
                                    .rounded(px(PROJECT_PICKER_ROW_RADIUS_PX))
                                    .bg(theme::rambutan_500()
                                        .opacity(PROJECT_PICKER_SELECT_BG_OPACITY))
                                    .px(px(PROJECT_PICKER_SELECT_X_PADDING_PX))
                                    .py(px(PROJECT_PICKER_SELECT_Y_PADDING_PX))
                                    .font_family(
                                        crate::theme::FontFace::Interface
                                            .with_default(PROJECT_PICKER_LABEL_FONT_FAMILY),
                                    )
                                    .text_size(px(PROJECT_PICKER_LABEL_TEXT_SIZE_PX))
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::rambutan_300())
                                    .tab_index(0)
                                    .role(Role::Button)
                                    .aria_label("Select this folder")
                                    .border_1()
                                    .border_color(gpui::transparent_black())
                                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                                    .cursor_pointer()
                                    .hover(|el| {
                                        el.bg(theme::rambutan_500()
                                            .opacity(PROJECT_PICKER_SELECT_HOVER_BG_OPACITY))
                                    })
                                    .map(|el| {
                                        on_press_view(el, cx, move |this, window, cx| {
                                            this.select_project_location(
                                                select_current_path.clone(),
                                                select_current_name.clone(),
                                                window,
                                                cx,
                                            );
                                        })
                                    })
                                    .child("Select this folder"),
                            )
                        },
                    ),
            )
            .when(
                !self.project_picker_for_terminal() && has_current_path,
                |el| el.child(self.clone_project_controls(cx)),
            )
            .when(
                !self.project_picker_for_terminal() && self.clone_editor.linked_prompt.is_none(),
                |el| {
                    let general = projectless_option(
                        "desktop-project-none",
                        "No project",
                        assets::MESSAGE_SQUARE_ICON_PATH,
                    );
                    let scratch = projectless_option(
                        "new-scratch-chat",
                        "New scratch chat",
                        assets::SQUARE_PEN_ICON_PATH,
                    )
                    .tooltip(|window, cx| {
                        Tooltip::new("Start a new chat in its own folder.").build(window, cx)
                    });
                    el.child(on_press_view(general, cx, |this, window, cx| {
                        this.apply_thread_project(None, window, cx)
                    }))
                    .child(on_press_view(scratch, cx, |this, window, cx| {
                        this.new_scratch_thread(window, cx)
                    }))
                },
            )
            .when(
                show_recents && self.clone_editor.linked_prompt.is_none(),
                |el| {
                    el.child(
                        div()
                            .border_b_1()
                            .border_color(
                                theme::longan_800().opacity(PROJECT_PICKER_RECENTS_BORDER_OPACITY),
                            )
                            .px(px(PROJECT_PICKER_RECENTS_X_PADDING_PX))
                            .py(px(PROJECT_PICKER_RECENTS_Y_PADDING_PX))
                            .flex()
                            .flex_col()
                            .gap(px(PROJECT_PICKER_RECENTS_GAP_PX))
                            .child(
                                div()
                                    .mb(px(PROJECT_PICKER_RECENTS_LABEL_BOTTOM_MARGIN_PX))
                                    .font_family(
                                        crate::theme::FontFace::Interface
                                            .with_default(PROJECT_PICKER_BODY_FONT_FAMILY),
                                    )
                                    .text_size(px(PROJECT_PICKER_SECTION_TEXT_SIZE_PX))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::rambutan_500())
                                    .child("PROJECTS"),
                            )
                            .child(
                                div()
                                    .id("desktop-project-list")
                                    .max_h(px(132.))
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .children(recent_rows),
                            ),
                    )
                },
            )
            .child(
                div()
                    .id("desktop-project-directory-list")
                    .max_h(px(PROJECT_PICKER_CONTENT_MAX_HEIGHT_PX))
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.project_picker_scroll)
                    .when_some(
                        self.project_picker_error
                            .clone()
                            .or_else(|| self.project_picker_start_warning.clone()),
                        |el, error| {
                            el.child(
                                div()
                                    .px(px(PROJECT_PICKER_EMPTY_X_PADDING_PX))
                                    .py(px(PROJECT_PICKER_EMPTY_Y_PADDING_PX))
                                    .text_center()
                                    .font_family(
                                        crate::theme::FontFace::Interface
                                            .with_default(PROJECT_PICKER_LABEL_FONT_FAMILY),
                                    )
                                    .text_size(px(PROJECT_PICKER_LABEL_TEXT_SIZE_PX))
                                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                                    .text_color(theme::lychee_400())
                                    .child(error),
                            )
                        },
                    )
                    .when(self.project_picker_error.is_none(), |el| {
                        el.child(
                            div()
                                .py(px(PROJECT_PICKER_LIST_Y_PADDING_PX))
                                .children(directory_rows),
                        )
                    })
                    .vertical_scrollbar(&self.project_picker_scroll),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(theme::longan_800().opacity(PROJECT_PICKER_FOOTER_BORDER_OPACITY))
                    .px(px(PROJECT_PICKER_FOOTER_X_PADDING_PX))
                    .py(px(PROJECT_PICKER_FOOTER_Y_PADDING_PX))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(PROJECT_PICKER_FOOTER_GAP_PX))
                    .font_family(
                        crate::theme::FontFace::Interface
                            .with_default(PROJECT_PICKER_BODY_FONT_FAMILY),
                    )
                    .text_size(px(PROJECT_PICKER_FOOTER_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_500())
                    .child("Click to browse, double click to select")
                    .child(
                        div()
                            .id("desktop-project-cancel")
                            .debug_selector(|| "desktop-project-cancel".to_string())
                            .flex_shrink_0()
                            .text_color(theme::rambutan_500())
                            .tab_index(0)
                            .role(Role::Button)
                            .aria_label("Cancel")
                            .border_1()
                            .border_color(gpui::transparent_black())
                            .focus_visible(|el| el.border_color(theme::persimmon_400()))
                            .cursor_pointer()
                            .hover(|el| el.text_color(theme::rambutan_300()))
                            .map(|el| {
                                on_press_view(el, cx, |this, _window, cx| {
                                    this.close_transient_surfaces();
                                    cx.notify();
                                })
                            })
                            .child("Cancel"),
                    ),
            )
            .child(self.render_modal_focus_sentinel()),
        )
        .into_any_element()
    }
}
