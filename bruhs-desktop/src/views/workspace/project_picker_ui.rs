//! The project picker: browsing directories off the UI thread, recents, and opening a project session.

use super::*;

pub(super) fn project_session_surface_after_create_like_web(
    active_view: AppView,
) -> (AppView, DesktopSidebarTab) {
    (active_view, DesktopSidebarTab::Code)
}
pub(super) const PROJECT_PICKER_BODY_FONT_FAMILY: &str = "Monaspace Neon";
pub(super) const PROJECT_PICKER_LABEL_FONT_FAMILY: &str = "Monaspace Argon";
pub(super) const PROJECT_PICKER_PATH_FONT_FAMILY: &str = "Monaspace Neon";
pub(super) const PROJECT_PICKER_ROW_RADIUS_PX: f32 = 8.0;
pub(super) const PROJECT_PICKER_ICON_SIZE_PX: f32 = 14.0;
pub(super) const PROJECT_PICKER_PARENT_ICON_SLOT_WIDTH_PX: f32 = 14.0;
pub(super) const PROJECT_PICKER_CONTENT_MAX_HEIGHT_PX: f32 = 288.0;
pub(super) const PROJECT_PICKER_LABEL_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const PROJECT_PICKER_SECTION_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const PROJECT_PICKER_FOOTER_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const PROJECT_PICKER_OVERLAY_OPACITY: f32 = 0.70;
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
    std::env::var_os("HOME").map(PathBuf::from)
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

pub(super) fn general_thread_project_path_like_web(
    active_project: Option<&DesktopProjectLocation>,
    fallback_path: String,
) -> String {
    active_project.map_or(fallback_path, |project| project.path.clone())
}

pub(super) fn general_thread_project_scoped_like_web(
    active_project: Option<&DesktopProjectLocation>,
) -> bool {
    active_project.is_some()
}

pub(super) fn project_picker_directory_error_like_web(_error: &str) -> String {
    PROJECT_PICKER_LIST_ERROR.to_string()
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
        match listing {
            Ok(listing) => {
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
        let thread = Self::create_thread(
            project.name.clone(),
            project.path.clone(),
            self.current_model.clone(),
            self.permission_mode,
            window,
            cx,
        );
        self.stamp_chat_run_prefs(&thread.chat, cx);
        thread.chat.update(cx, |chat, cx| {
            chat.tasks_panel_visible = self.tasks_panel_visible;
            chat.diff_panel_visible = self.diff_panel_visible;
            cx.notify();
        });
        self.active_thread_id = Some(thread.id.clone());
        self.threads.push(thread);
        (self.active_view, self.sidebar_tab) =
            project_session_surface_after_create_like_web(self.active_view.clone());
        self.sidebar_filter = sidebar_ui::SidebarFilter::All;
        self.close_transient_surfaces();
        self.active_project = Some(project.clone());
        push_recent_project(&mut self.recent_projects, project);
        self.persist_and_notify(cx);
    }

    pub(super) fn select_project_location(
        &mut self,
        path: String,
        fallback_name: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let resolve =
            cx.background_spawn(async move { canonical_project_location(&path, &fallback_name) });
        self.project_picker_task = Some(cx.spawn_in(window, async move |this, cx| {
            let location = resolve.await;
            // The workspace may have closed meanwhile; nothing to open then.
            let _closed = this.update_in(cx, |this, window, cx| match location {
                Ok(project) if this.project_picker_for_terminal() => {
                    this.open_terminal_in_project(project, window, cx)
                }
                Ok(project) => this.create_project_thread(project, window, cx),
                Err(error) => {
                    this.project_picker_error = Some(error);
                    cx.notify();
                }
            });
        }));
    }

    pub(super) fn open_project_picker(
        &mut self,
        _: &OpenProjectPicker,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.open_top_layer(TopLayer::ProjectPicker {
            for_terminal: false,
        });
        self.focus_first_modal_tab_stop(window, cx);
        if self.project_picker_current_path.is_empty() {
            self.load_project_directory(None, cx);
        } else {
            cx.notify();
        }
    }

    pub(super) fn render_project_picker(&self, cx: &Context<'_, Self>) -> AnyElement {
        let current_path = self.project_picker_current_path.clone();
        let current_name = self.project_picker_current_name.clone();
        let has_current_path = !current_path.is_empty();
        let header_path = current_path.clone();
        let select_current_path = current_path;
        let select_current_name = current_name;
        let select_current_path_key = select_current_path.clone();
        let select_current_name_key = select_current_name.clone();

        let mut recent_rows = Vec::new();
        for project in &self.recent_projects {
            let path = project.path.clone();
            let name = project.name.clone();
            let path_for_key = project.path.clone();
            let name_for_key = project.name.clone();
            let debug_key = element_key(&project.path);
            recent_rows.push(
                div()
                    .id(("desktop-project-recent", element_key(&project.path)))
                    .debug_selector(move || format!("desktop-project-recent-{debug_key}"))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(PROJECT_PICKER_RECENT_ROW_GAP_PX))
                    .rounded(px(PROJECT_PICKER_ROW_RADIUS_PX))
                    .px(px(PROJECT_PICKER_RECENT_ROW_X_PADDING_PX))
                    .py(px(PROJECT_PICKER_RECENT_ROW_Y_PADDING_PX))
                    .font_family(PROJECT_PICKER_LABEL_FONT_FAMILY)
                    .text_size(px(PROJECT_PICKER_LABEL_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_400())
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label(format!("Open recent project {}", project.path))
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| {
                        el.bg(
                            theme::longan_800().opacity(PROJECT_PICKER_RECENT_ROW_HOVER_BG_OPACITY)
                        )
                        .text_color(theme::rambutan_200())
                    })
                    .on_click(cx.listener(move |this, _ev, window, cx| {
                        this.select_project_location(path.clone(), name.clone(), window, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.select_project_location(
                                path_for_key.clone(),
                                name_for_key.clone(),
                                window,
                                cx,
                            );
                        }
                    }))
                    .child(
                        svg()
                            .path(assets::FOLDER_ICON_PATH)
                            .size(px(PROJECT_PICKER_ICON_SIZE_PX))
                            .text_color(theme::rambutan_500())
                            .flex_shrink_0(),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .child(project.path.clone()),
                    )
                    .into_any_element(),
            );
        }
        let show_recents = !has_current_path && !recent_rows.is_empty();

        let mut directory_rows = Vec::new();
        if let Some(parent_path) = &self.project_picker_parent_path {
            let parent_path = parent_path.clone();
            let parent_path_for_key = parent_path.clone();
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
                    .font_family(PROJECT_PICKER_LABEL_FONT_FAMILY)
                    .text_size(px(PROJECT_PICKER_LABEL_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_500())
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label("Go to parent folder")
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| {
                        el.bg(theme::longan_800().opacity(PROJECT_PICKER_ROW_HOVER_BG_OPACITY))
                    })
                    .on_click(cx.listener(move |this, _ev, _window, cx| {
                        this.load_project_directory(Some(parent_path.clone()), cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.load_project_directory(Some(parent_path_for_key.clone()), cx);
                        }
                    }))
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
                    .font_family(PROJECT_PICKER_LABEL_FONT_FAMILY)
                    .text_size(px(PROJECT_PICKER_LABEL_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_400())
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label(format!("Open folder {}", dir.name))
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| {
                        el.bg(theme::longan_800().opacity(PROJECT_PICKER_ROW_HOVER_BG_OPACITY))
                            .text_color(theme::rambutan_200())
                    })
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
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
                    .font_family(PROJECT_PICKER_LABEL_FONT_FAMILY)
                    .text_size(px(PROJECT_PICKER_LABEL_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_500())
                    .child("No subdirectories")
                    .into_any_element(),
            );
        }

        div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .occlude()
            .id("desktop-project-picker-overlay")
            .debug_selector(|| "desktop-project-picker-overlay".to_string())
            .bg(theme::longan_950().opacity(PROJECT_PICKER_OVERLAY_OPACITY))
            .flex()
            .items_start()
            .justify_center()
            .pt(px(DESKTOP_DIALOG_TOP_PADDING_PX))
            .on_click(cx.listener(|this, _ev, _window, cx| {
                this.close_transient_surfaces();
                cx.notify();
            }))
            .child(
                div()
                    .id("desktop-project-picker-dialog")
                    .role(Role::Dialog)
                    .aria_label("Open project")
                    .child(self.blur_scrim.backdrop())
                    .debug_selector(|| "desktop-project-picker-dialog".to_string())
                    .w(px(DESKTOP_DIALOG_MD_WIDTH_PX))
                    .max_w(relative(DESKTOP_DIALOG_MAX_WIDTH_RATIO))
                    .rounded(px(DESKTOP_DIALOG_RADIUS_PX))
                    .border_1()
                    .border_color(theme::ring_modal())
                    .bg(theme::longan_800())
                    .text_color(cx.theme().popover_foreground)
                    .font_family(PROJECT_PICKER_BODY_FONT_FAMILY)
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .track_focus(&self.modal_start_focus_handle)
                    .tab_group()
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        this.trap_modal_tab_focus(event, window, cx);
                    }))
                    .on_click(cx.listener(|_this, _ev, _window, cx| {
                        cx.stop_propagation();
                    }))
                    .child(
                        div()
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
                                            .font_family(PROJECT_PICKER_LABEL_FONT_FAMILY)
                                            .text_size(px(PROJECT_PICKER_LABEL_TEXT_SIZE_PX))
                                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme::rambutan_200())
                                            .child("Open project"),
                                    )
                                    .when(has_current_path, |el| {
                                        el.child(
                                            div()
                                                .mt(px(PROJECT_PICKER_PATH_TOP_MARGIN_PX))
                                                .truncate()
                                                .text_size(px(PROJECT_PICKER_LABEL_TEXT_SIZE_PX))
                                                .font_family(theme::UI_SMALL_FONT_FAMILY)
                                                .font_family(PROJECT_PICKER_PATH_FONT_FAMILY)
                                                .text_color(theme::rambutan_500())
                                                .child(header_path.clone()),
                                        )
                                    }),
                            )
                            .when(has_current_path, |el| {
                                el.child(
                                    div()
                                        .id("desktop-project-select-current")
                                        .debug_selector(|| {
                                            "desktop-project-select-current".to_string()
                                        })
                                        .ml(px(PROJECT_PICKER_SELECT_MARGIN_LEFT_PX))
                                        .flex_shrink_0()
                                        .rounded(px(PROJECT_PICKER_ROW_RADIUS_PX))
                                        .bg(theme::rambutan_500()
                                            .opacity(PROJECT_PICKER_SELECT_BG_OPACITY))
                                        .px(px(PROJECT_PICKER_SELECT_X_PADDING_PX))
                                        .py(px(PROJECT_PICKER_SELECT_Y_PADDING_PX))
                                        .font_family(PROJECT_PICKER_LABEL_FONT_FAMILY)
                                        .text_size(px(PROJECT_PICKER_LABEL_TEXT_SIZE_PX))
                                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(theme::rambutan_300())
                                        .tab_index(0)
                                        .role(Role::Button)
                                        .aria_label("Select this folder")
                                        .focus_visible(|el| {
                                            el.border_2().border_color(theme::persimmon_400())
                                        })
                                        .cursor_pointer()
                                        .hover(|el| {
                                            el.bg(theme::rambutan_500()
                                                .opacity(PROJECT_PICKER_SELECT_HOVER_BG_OPACITY))
                                        })
                                        .on_click(cx.listener(move |this, _ev, window, cx| {
                                            this.select_project_location(
                                                select_current_path.clone(),
                                                select_current_name.clone(),
                                                window,
                                                cx,
                                            );
                                        }))
                                        .on_key_down(cx.listener(
                                            move |this, event: &KeyDownEvent, window, cx| {
                                                if is_web_button_activation_key(
                                                    event.keystroke.key.as_str(),
                                                ) {
                                                    window.prevent_default();
                                                    cx.stop_propagation();
                                                    this.select_project_location(
                                                        select_current_path_key.clone(),
                                                        select_current_name_key.clone(),
                                                        window,
                                                        cx,
                                                    );
                                                }
                                            },
                                        ))
                                        .child("Select this folder"),
                                )
                            }),
                    )
                    .when(show_recents, |el| {
                        el.child(
                            div()
                                .border_b_1()
                                .border_color(
                                    theme::longan_800()
                                        .opacity(PROJECT_PICKER_RECENTS_BORDER_OPACITY),
                                )
                                .px(px(PROJECT_PICKER_RECENTS_X_PADDING_PX))
                                .py(px(PROJECT_PICKER_RECENTS_Y_PADDING_PX))
                                .flex()
                                .flex_col()
                                .gap(px(PROJECT_PICKER_RECENTS_GAP_PX))
                                .child(
                                    div()
                                        .mb(px(PROJECT_PICKER_RECENTS_LABEL_BOTTOM_MARGIN_PX))
                                        .font_family(PROJECT_PICKER_BODY_FONT_FAMILY)
                                        .text_size(px(PROJECT_PICKER_SECTION_TEXT_SIZE_PX))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(theme::rambutan_500())
                                        .child("RECENT"),
                                )
                                .children(recent_rows),
                        )
                    })
                    .child(
                        div()
                            .max_h(px(PROJECT_PICKER_CONTENT_MAX_HEIGHT_PX))
                            .overflow_y_scrollbar()
                            .when_some(self.project_picker_error.clone(), |el, error| {
                                el.child(
                                    div()
                                        .px(px(PROJECT_PICKER_EMPTY_X_PADDING_PX))
                                        .py(px(PROJECT_PICKER_EMPTY_Y_PADDING_PX))
                                        .text_center()
                                        .font_family(PROJECT_PICKER_LABEL_FONT_FAMILY)
                                        .text_size(px(PROJECT_PICKER_LABEL_TEXT_SIZE_PX))
                                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                                        .text_color(theme::lychee_400())
                                        .child(error),
                                )
                            })
                            .when(self.project_picker_error.is_none(), |el| {
                                el.child(
                                    div()
                                        .py(px(PROJECT_PICKER_LIST_Y_PADDING_PX))
                                        .children(directory_rows),
                                )
                            }),
                    )
                    .child(
                        div()
                            .border_t_1()
                            .border_color(
                                theme::longan_800().opacity(PROJECT_PICKER_FOOTER_BORDER_OPACITY),
                            )
                            .px(px(PROJECT_PICKER_FOOTER_X_PADDING_PX))
                            .py(px(PROJECT_PICKER_FOOTER_Y_PADDING_PX))
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(PROJECT_PICKER_FOOTER_GAP_PX))
                            .font_family(PROJECT_PICKER_BODY_FONT_FAMILY)
                            .text_size(px(PROJECT_PICKER_FOOTER_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_500())
                            .child("click to browse, double-click to select")
                            .child(
                                div()
                                    .id("desktop-project-cancel")
                                    .debug_selector(|| "desktop-project-cancel".to_string())
                                    .flex_shrink_0()
                                    .text_color(theme::rambutan_500())
                                    .tab_index(0)
                                    .role(Role::Button)
                                    .aria_label("Cancel")
                                    .focus_visible(|el| {
                                        el.border_2().border_color(theme::persimmon_400())
                                    })
                                    .cursor_pointer()
                                    .hover(|el| el.text_color(theme::rambutan_300()))
                                    .on_click(cx.listener(|this, _ev, _window, cx| {
                                        this.close_transient_surfaces();
                                        cx.notify();
                                    }))
                                    .on_key_down(cx.listener(
                                        |this, event: &KeyDownEvent, window, cx| {
                                            if is_web_button_activation_key(
                                                event.keystroke.key.as_str(),
                                            ) {
                                                window.prevent_default();
                                                cx.stop_propagation();
                                                this.close_transient_surfaces();
                                                cx.notify();
                                            }
                                        },
                                    ))
                                    .child("Cancel"),
                            ),
                    )
                    .child(self.render_modal_focus_sentinel()),
            )
            .into_any_element()
    }
}
