//! Device-wide visibility preferences shared by Appearance and the thread details card.
use super::*;
use bruhs_models::num::ToFloatLossy as _;
use bruhs_ui::menu_popover::menu_popover;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Item {
    Environment,
    Resources,
    OpenIn,
    Scripts,
    Branch,
    GitActions,
    Changes,
    Servers,
    Automations,
    Lineage,
    SideChats,
    Usage,
    ContextWindow,
}
use Item::*;

const MENU_WIDTH: f32 = 256.;

const ITEMS: [(Item, &str, &str); 13] = [
    (Environment, "environment", "Environment and worktree"),
    (Resources, "resources", "Computer and disk usage"),
    (OpenIn, "open-in", "Open in editor"),
    (Scripts, "scripts", "Project scripts"),
    (Branch, "branch", "Branch and pull request"),
    (GitActions, "git-actions", "Git actions"),
    (Changes, "changes", "Changes"),
    (Servers, "servers", "Servers"),
    (Automations, "automations", "Automations"),
    (Lineage, "lineage", "Lineage"),
    (SideChats, "side-chats", "Side chats"),
    (Usage, "usage", "Usage limits"),
    (ContextWindow, "context", "Context window"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    environment: bool,
    resources: bool,
    open_in: bool,
    scripts: bool,
    branch: bool,
    git_actions: bool,
    changes: bool,
    servers: bool,
    automations: bool,
    lineage: bool,
    side_chats: bool,
    usage: bool,
    context: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            environment: true,
            resources: true,
            open_in: true,
            scripts: true,
            branch: true,
            git_actions: true,
            changes: true,
            servers: true,
            automations: true,
            lineage: true,
            side_chats: true,
            usage: false,
            context: false,
        }
    }
}
impl Preferences {
    fn setting(&mut self, item: Item) -> &mut bool {
        match item {
            Resources => &mut self.resources,
            Environment => &mut self.environment,
            OpenIn => &mut self.open_in,
            Scripts => &mut self.scripts,
            Branch => &mut self.branch,
            GitActions => &mut self.git_actions,
            Changes => &mut self.changes,
            Servers => &mut self.servers,
            Automations => &mut self.automations,
            Lineage => &mut self.lineage,
            SideChats => &mut self.side_chats,
            Usage => &mut self.usage,
            ContextWindow => &mut self.context,
        }
    }
    fn visible(mut self, item: Item) -> bool {
        *self.setting(item)
    }
    fn count(self) -> usize {
        ITEMS.iter().filter(|(i, _, _)| self.visible(*i)).count()
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuAt {
    Settings,
    Card,
}

pub(super) struct DetailsUi {
    measurement: Option<(String, Option<String>, String)>,
    measuring: bool,
    open_thread: Option<String>,
    inline: bool,
    pub(super) hidden_inline: std::collections::BTreeSet<String>,
    menu: Option<MenuAt>,
    context_position: Option<Point<Pixels>>,
    active: usize,
    options: [FocusHandle; ITEMS.len() + 1],
    trigger: FocusHandle,
    card: FocusHandle,
    menu_scroll: ScrollHandle,
}
impl DetailsUi {
    #[cfg(test)]
    pub(super) fn last_measurement(&self) -> Option<&(String, Option<String>, String)> {
        self.measurement.as_ref()
    }

    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            measuring: false,
            measurement: None,
            open_thread: None,
            inline: false,
            hidden_inline: Default::default(),
            menu: None,
            context_position: None,
            active: 0,
            options: std::array::from_fn(|_| cx.focus_handle()),
            trigger: cx.focus_handle().tab_stop(true),
            card: cx.focus_handle(),
            menu_scroll: ScrollHandle::new(),
        }
    }
    pub(super) fn close(&mut self) {
        self.open_thread = None;
        self.menu = None;
    }
    pub(super) fn is_open(&self, id: &str) -> bool {
        if self.inline {
            !self.hidden_inline.contains(id)
        } else {
            self.open_thread.as_deref() == Some(id)
        }
    }
    pub(super) fn open_for_active_thread(&self, id: Option<&str>) -> bool {
        id.is_some_and(|id| self.is_open(id))
    }
    pub(super) fn close_menu(&mut self) {
        self.menu = None;
    }
}

impl Workspace {
    fn focus_detail_option(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.details.active = index;
        self.details.menu_scroll.scroll_to_item(index + 1);
        self.details.options[index].focus(window, cx);
        cx.notify();
    }
    fn open_detail_menu(&mut self, at: MenuAt, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.details.context_position = None;
        self.details.menu = Some(at);
        self.focus_detail_option(0, window, cx);
    }
    fn close_detail_menu(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let Some(at) = self.details.menu.take() else {
            return false;
        };
        match at {
            MenuAt::Settings => &self.details.trigger,
            MenuAt::Card => &self.details.card,
        }
        .focus(window, cx);
        cx.notify();
        true
    }
    pub(super) fn dismiss_thread_details(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if self.close_detail_menu(window, cx) {
            return true;
        }
        if self.details.open_thread.is_none() {
            return false;
        }
        self.details.close();
        if let Some(t) = self.active_thread() {
            t.chat.read(cx).details_trigger.clone().focus(window, cx);
        }
        cx.notify();
        true
    }

    fn change_detail(&mut self, index: usize, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some((item, _, _)) = ITEMS.get(index) {
            let setting = self.appearance.thread_details.setting(*item);
            *setting = !*setting;
            self.focus_detail_option(index, window, cx);
        } else {
            self.appearance.thread_details = Preferences::default();
            self.close_detail_menu(window, cx);
        }
        self.persist_and_notify(cx);
    }
    fn detail_menu(&self, cx: &Context<'_, Self>) -> AnyElement {
        let prefs = self.appearance.thread_details;
        let changed = prefs != Preferences::default();
        let mut menu = div()
            .id("thread-details-menu")
            .debug_selector(|| "thread-details-menu".into())
            .role(Role::Menu)
            .aria_label("Show in thread details")
            .occlude()
            .w(px(MENU_WIDTH))
            .max_h(px(438.))
            .overflow_y_scroll()
            .track_scroll(&self.details.menu_scroll)
            .rounded(px(8.))
            .shadow_md()
            .border_1()
            .border_color(theme::ring_floating())
            .p_1()
            .bg(theme::longan_900())
            .flex()
            .flex_col()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(theme::rambutan_400())
                    .child("Show in thread details"),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.details.close_menu();
                cx.notify();
            }))
            .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, win, cx| {
                let count = ITEMS.len() + usize::from(changed);
                match event.keystroke.key.as_str() {
                    "escape" => {
                        this.close_detail_menu(win, cx);
                    }
                    "tab" => {
                        this.close_detail_menu(win, cx);
                        return;
                    }
                    key @ ("up" | "down" | "home" | "end") => {
                        if let Some(next) = bruhs_models::logic::menu::menu_step(
                            Some(this.details.active),
                            count,
                            key,
                        ) {
                            this.focus_detail_option(next, win, cx);
                        }
                    }
                    "space" | "enter" => this.change_detail(this.details.active, win, cx),
                    _ => return,
                }
                win.prevent_default();
                cx.stop_propagation();
            }));
        for (index, (item, id, label)) in ITEMS
            .iter()
            .map(|(i, id, label)| (Some(*i), *id, *label))
            .chain(std::iter::once((None, "reset", "Reset to default")))
            .enumerate()
        {
            let checked = item.is_some_and(|i| prefs.visible(i));
            let enabled = item.is_some() || changed;
            let key = format!("thread-details-option-{id}");
            let row = div()
                .id(SharedString::from(key.clone()))
                .debug_selector(move || key.clone())
                .h(px(28.))
                .flex_shrink_0()
                .role(if item.is_some() {
                    Role::MenuItemCheckBox
                } else {
                    Role::MenuItem
                })
                .aria_label(label)
                .when(item.is_some(), |el| {
                    el.aria_toggled(if checked {
                        gpui::Toggled::True
                    } else {
                        gpui::Toggled::False
                    })
                })
                .when(item.is_none(), |el| {
                    el.mt_1().border_t_1().border_color(theme::longan_700())
                })
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .rounded(px(4.))
                .text_size(px(13.))
                .when(checked, |el| {
                    el.child(
                        svg()
                            .path(assets::CHECK_ICON_PATH)
                            .size(px(14.))
                            .flex_shrink_0()
                            .text_color(theme::rambutan_100()),
                    )
                })
                .when(!checked && item.is_some(), |el| {
                    el.child(div().w(px(14.)).flex_shrink_0())
                })
                .when(!enabled, |el| el.opacity(0.5))
                .when(enabled, |el| {
                    el.track_focus(&self.details.options[index])
                        .cursor_pointer()
                        .focus(|el| el.bg(theme::longan_800()))
                        .hover(|el| el.bg(theme::longan_800()))
                        .map(|el| {
                            on_press_view(el, cx, move |this, win, cx| {
                                this.change_detail(index, win, cx)
                            })
                        })
                })
                .child(label);
            menu = menu.child(row);
        }
        menu.into_any_element()
    }

    pub(super) fn thread_detail_settings_row(&self, cx: &Context<'_, Self>) -> AnyElement {
        let prefs = self.appearance.thread_details;
        let open = self.details.menu == Some(MenuAt::Settings);
        let count = format!("{} of {} on", prefs.count(), ITEMS.len());
        let trigger = web_button(
            "thread-details-settings",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .track_focus(&self.details.trigger)
        .debug_selector(|| "thread-details-settings".into())
        .aria_label(format!("Thread details: {count}"))
        .aria_expanded(open)
        .w_full()
        .justify_between()
        .child(count)
        .child(
            svg()
                .path(assets::CHEVRON_DOWN_ICON_PATH)
                .size(px(14.))
                .text_color(theme::rambutan_400()),
        );
        let trigger = on_press_view(trigger, cx, move |this, win, cx| {
            if open {
                this.close_detail_menu(win, cx);
            } else {
                this.open_detail_menu(MenuAt::Settings, win, cx);
            }
        });
        let controls = div()
            .flex()
            .items_center()
            .gap_2()
            .when(prefs != Preferences::default(), |el| {
                let reset = web_button(
                    "reset-thread-details",
                    WebButtonVariant::Ghost,
                    WebButtonSize::IconSmall,
                    ButtonState::Enabled,
                )
                .debug_selector(|| "reset-thread-details".into())
                .aria_label("Reset thread details to default")
                .child(
                    svg()
                        .path(assets::ROTATE_CCW_ICON_PATH)
                        .size(px(14.))
                        .text_color(theme::rambutan_400()),
                );
                el.child(on_press_view(reset, cx, |this, _, cx| {
                    this.appearance.thread_details = Default::default();
                    this.persist_and_notify(cx);
                }))
            })
            .child(
                div()
                    .relative()
                    .w(px(160.))
                    .max_w_full()
                    .child(trigger)
                    .when(open, |el| {
                        el.child(div().absolute().top_0().right(px(MENU_WIDTH)).child(
                            menu_popover(size(px(MENU_WIDTH), px(32.)), self.detail_menu(cx)),
                        ))
                    }),
            );
        self.settings_row(
            "desktop-settings-row-thread-details",
            "Thread details",
            "Choose what the thread details card shows. You can also right-click the card.",
            controls.into_any_element(),
        )
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub(super) enum Origin {
    Handoff(String),
    SideChat(String),
}
impl Origin {
    pub(super) fn parent(&self) -> &str {
        match self {
            Self::Handoff(id) | Self::SideChat(id) => id,
        }
    }
}

impl Workspace {
    pub(super) fn conversation_with_details(
        &self,
        thread: &DesktopThread,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let workspace = cx.weak_entity();
        let inline = self.details.inline;
        let open = self.details.is_open(&thread.id) && !self.overlay_open();
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .relative()
            .flex()
            .flex_col()
            .child(
                canvas(
                    move |bounds, _, cx| {
                        // Reserve 280px for the card beside a minimum 640px conversation,
                        // with the reference's outer padding and gap. Measure the pane
                        // itself so sidebar, zoom and split panel changes all participate.
                        let fits = bruhs_ui::zoom::css_px(bounds.size.width) >= 1012.
                            && bruhs_ui::zoom::css_px(bounds.size.height) >= 228.;
                        if fits != inline {
                            let workspace = workspace.clone();
                            cx.defer(move |cx| {
                                let _ = workspace.update(cx, |ws, cx| {
                                    if ws.details.inline != fits {
                                        ws.details.inline = fits;
                                        ws.details.close();
                                        if fits && ws.appearance.thread_details.automations {
                                            ws.reactive_automations
                                                .update(cx, |panel, cx| panel.refresh_snapshot(cx));
                                        }
                                        cx.notify();
                                    }
                                });
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(
                div()
                    .min_w_0()
                    .min_h_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .when(inline && open, |lane| lane.pr(px(312.)))
                    .child(thread.chat.clone()),
            )
            .when(open, |main| {
                main.child(self.thread_details_card(thread, window, cx))
            })
            .into_any_element()
    }

    pub(super) fn toggle_thread_details(&mut self, win: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(thread) = self.active_thread() else {
            return;
        };
        let id = thread.id.clone();
        if self.details.inline {
            if self.details.hidden_inline.remove(&id) {
                self.details.card.focus(win, cx);
            } else {
                self.details.hidden_inline.insert(id);
                self.details.close_menu();
            }
            self.persist_and_notify(cx);
            return;
        }
        if self.details.is_open(&id) {
            self.dismiss_thread_details(win, cx);
            return;
        }
        self.details.open_thread = Some(id);
        self.details.card.focus(win, cx);
        if self.appearance.thread_details.automations {
            self.reactive_automations
                .update(cx, |panel, cx| panel.refresh_snapshot(cx));
        }
        cx.notify();
    }

    fn new_side_chat(&mut self, win: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(thread) = self.active_thread() else {
            return;
        };
        let parent = thread.id.clone();
        let view = thread.chat.read(cx);
        let context = view
            .conversation_context(&view.messages)
            .or_else(|| view.import_context.clone());
        let project = view.project_scoped.then(|| {
            (
                thread.project_name.clone(),
                view.settings_project_path().to_owned(),
            )
        });
        let worktree = view.worktree_path.clone();
        let branch = thread.branch.clone();
        self.details.close();
        if let Some((name, path)) = project {
            self.new_thread_in_project(name, path, win, cx);
        } else {
            self.new_general_thread(win, cx);
        }
        if let Some(thread) = self.active_thread() {
            let child = thread.id.clone();
            thread.chat.update(cx, |chat, cx| {
                chat.import_context = context;
                chat.lock_place(worktree, branch, cx);
                cx.notify();
            });
            self.thread_origins.insert(child, Origin::SideChat(parent));
            self.persist_and_notify(cx);
        }
    }

    fn detail_action(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        cx: &Context<'_, Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<'_, Self>) + 'static,
    ) -> AnyElement {
        let id = id.into();
        let selector = id.to_string();
        let label = label.into();
        let button = web_button(
            id,
            WebButtonVariant::Ghost,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .debug_selector(move || selector.clone())
        .aria_label(label.clone())
        .w_full()
        .min_w_0()
        .justify_start()
        .child(div().min_w_0().truncate().child(label));
        on_press_view(button, cx, move |this, win, cx| {
            this.details.close();
            action(this, win, cx);
            cx.notify();
        })
        .into_any_element()
    }

    fn measure_thread_usage(
        &mut self,
        id: String,
        path: Option<String>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.details.measuring {
            return;
        }
        self.details.measuring = true;
        self.details.measurement = Some((id.clone(), path.clone(), "Measuring…".into()));
        let app = cx.global::<DesktopState>().app.clone();
        let folder = path.clone();
        let thread = bruhs_models::ids::ThreadId::try_from(id.as_str()).ok();
        let request = cx.background_spawn(async move {
            let process = thread
                .as_ref()
                .and_then(|thread| app.thread_process(thread));
            let usage = process.and_then(|(_, pid)| diagnostics::process_snapshot(pid));
            let mut text = match (process, usage) {
                (None, _) => "No active local CLI".to_owned(),
                (Some(_), Some(usage))
                    if thread
                        .as_ref()
                        .and_then(|thread| app.thread_process(thread))
                        == process =>
                {
                    usage
                }
                _ => "CLI sample unavailable".to_owned(),
            };
            if let Some(folder) = folder {
                // du reports allocated storage and accounts for hard links and sparse files.
                let kib = std::process::Command::new("/usr/bin/du")
                    .args(["-sk", "."])
                    .current_dir(folder)
                    .output()
                    .ok()
                    .filter(|r| r.status.success())
                    .and_then(|r| String::from_utf8(r.stdout).ok())
                    .and_then(|s| s.split_whitespace().next()?.parse::<u64>().ok());
                text.push_str(&kib.map_or_else(
                    || "\nCheckout size unavailable".to_owned(),
                    |n| {
                        if n < 1024 {
                            format!("\nCheckout size · {n} KiB")
                        } else {
                            format!("\nCheckout size · {:.1} MiB", n.to_f64_lossy() / 1024.)
                        }
                    },
                ));
            }
            text
        });
        cx.spawn(async move |this, cx| {
            let text = request.await;
            let _ = this.update(cx, |this, cx| {
                this.details.measurement = Some((id, path, format!("Last measurement\n{text}")));
                this.details.measuring = false;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn thread_details_card(
        &self,
        thread: &DesktopThread,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let prefs = self.appearance.thread_details;
        let chat = thread.chat.read(cx);
        let mut rows = Vec::new();
        let cwd = chat.worktree_path.as_ref().unwrap_or(&chat.project_path);
        let scoped = chat.project_scoped && !cwd.is_empty();
        if prefs.environment {
            let place = if chat.worktree_path.is_some() {
                "Worktree"
            } else if scoped {
                "Local checkout"
            } else {
                "Local environment"
            };
            let label = if scoped {
                format!("{place} · {}", thread.project_name)
            } else {
                place.to_owned()
            };
            let entity = thread.chat.clone();
            if scoped {
                rows.push(
                    self.detail_action("detail-environment", label, cx, move |_, w, cx| {
                        entity.update(cx, |c, cx| c.toggle_workspace_menu(w, cx))
                    }),
                );
            } else {
                rows.push(detail_text("detail-environment", label));
            }
        }
        if prefs.resources {
            let id = thread.id.clone();
            let path = scoped.then(|| cwd.clone());
            let matching = self
                .details
                .measurement
                .as_ref()
                .filter(|(key, folder, _)| key == &id && folder == &path);
            if let Some((_, _, text)) = matching {
                rows.push(detail_text("detail-resource-sample", text.clone()));
            }
            let button = web_button(
                "measure-thread-usage",
                WebButtonVariant::Ghost,
                WebButtonSize::Small,
                ButtonState::disabled_if(self.details.measuring),
            )
            .debug_selector(|| "measure-thread-usage".into())
            .child("Measure computer and disk usage");
            rows.push(
                on_press_view(button, cx, move |this, _, cx| {
                    this.measure_thread_usage(id.clone(), path.clone(), cx);
                })
                .into_any_element(),
            );
        }
        if scoped && prefs.open_in {
            let path = cwd.clone();
            rows.push(self.detail_action(
                "detail-open-editor",
                "Open in editor",
                cx,
                move |this, _, cx| this.open_path_in_editor(&path, None, cx),
            ));
        }
        if scoped && prefs.scripts {
            let scope = chat.settings_project_path();
            for action in self.actions_for_project(scope) {
                let action = action.clone();
                let path = cwd.clone();
                rows.push(self.detail_action(
                    format!("detail-script-{}", action.id),
                    format!("Run {}", action.name),
                    cx,
                    move |this, w, cx| this.run_action_in(&action, path.clone(), w, cx),
                ));
            }
            let scope = scope.to_owned();
            rows.push(self.detail_action(
                "detail-manage-scripts",
                "Manage project scripts",
                cx,
                move |this, w, cx| {
                    this.open_settings(&OpenSettings, w, cx);
                    this.settings_project_scope = scope.clone();
                    this.set_settings_section(SettingsSection::Projects, cx);
                },
            ));
        }
        let branch = chat
            .git_current_branch
            .as_deref()
            .or(thread.branch.as_deref());
        if scoped && prefs.branch {
            if let Some(branch) = branch {
                rows.push(detail_text("detail-branch", format!("Branch · {branch}")));
            }
            if let Some(pull) = self.displayed_pull() {
                rows.push(self.detail_action(
                    "detail-pull",
                    format!("#{} · {}", pull.number, pull.title),
                    cx,
                    |this, _, cx| this.show_pull_surface(cx),
                ));
            }
        }
        if scoped && prefs.git_actions && branch.is_some() {
            let entity = thread.chat.clone();
            rows.push(
                self.detail_action("detail-git", "Switch branch…", cx, move |_, w, cx| {
                    entity.update(cx, |c, cx| c.toggle_branch_menu(w, cx))
                }),
            );
        }
        if scoped && prefs.changes && (branch.is_some() || chat.git_dirty_count > 0) {
            let count = chat.git_dirty_count;
            rows.push(self.detail_action(
                "detail-changes",
                format!(
                    "{count} changed {}",
                    if count == 1 { "file" } else { "files" }
                ),
                cx,
                |this, _, cx| this.set_diff_panel_visible(true, cx),
            ));
        }
        if prefs.servers {
            for (id, name, url) in self
                .terminal_panel
                .read(cx)
                .thread_server_previews(&thread.id, cx)
            {
                rows.push(self.detail_action(
                    format!("detail-server-{id}"),
                    format!("Preview · {name}"),
                    cx,
                    move |this, w, cx| this.show_action_preview(&url, w, cx),
                ));
            }
        }
        if prefs.automations {
            for (id, title) in self
                .reactive_automations
                .read(cx)
                .tasks_for_thread(&thread.id)
            {
                rows.push(self.detail_action(
                    format!("detail-automation-{id}"),
                    title,
                    cx,
                    move |this, w, cx| {
                        this.open_settings(&OpenSettings, w, cx);
                        this.set_settings_section(SettingsSection::ScheduledTasks, cx);
                        this.reactive_automations
                            .update(cx, |p, cx| p.open_task_by_id(&id, w, cx));
                    },
                ));
            }
        }
        if prefs.lineage {
            let parent = chat
                .fork
                .as_ref()
                .map(|fork| fork.parent.as_str())
                .or_else(|| self.thread_origins.get(&thread.id).map(Origin::parent));
            if let Some(parent) = parent.and_then(|id| self.threads.iter().find(|t| t.id == id)) {
                let id = parent.id.clone();
                let title = parent.chat.read(cx).thread_title.clone();
                rows.push(self.detail_action(
                    "detail-parent",
                    format!("Parent · {title}"),
                    cx,
                    move |this, _, cx| this.select_thread_by_id(&id, cx),
                ));
            }
        }
        if prefs.lineage || prefs.side_chats {
            for child in &self.threads {
                let view = child.chat.read(cx);
                let origin = self
                    .thread_origins
                    .get(&child.id)
                    .filter(|o| o.parent() == thread.id);
                let label = match origin {
                    Some(Origin::SideChat(_)) if prefs.side_chats => "Side chat",
                    Some(Origin::Handoff(_)) if prefs.lineage => "Handoff",
                    _ if prefs.lineage
                        && view.fork.as_ref().is_some_and(|f| f.parent == thread.id) =>
                    {
                        "Fork"
                    }
                    _ => continue,
                };
                let id = child.id.clone();
                let title = &child.chat.read(cx).thread_title;
                rows.push(self.detail_action(
                    format!("detail-child-{id}"),
                    format!("{label} · {title}"),
                    cx,
                    move |this, _, cx| this.select_thread_by_id(&id, cx),
                ));
            }
        }
        if prefs.side_chats {
            rows.push(self.detail_action(
                "detail-new-side-chat",
                "New side chat",
                cx,
                |this, w, cx| this.new_side_chat(w, cx),
            ));
        }
        if prefs.usage {
            let limit = chat
                .reported_limit
                .filter(|(generation, _)| chat.is_quota_turn(*generation));
            let text = match limit {
                Some((_, Some(ms))) if ms > EpochMs::now().as_millis() => {
                    chrono::DateTime::from_timestamp_millis(ms)
                        .map(|t| {
                            format!(
                                "Usage limit resets {}",
                                t.with_timezone(&chrono::Local)
                                    .format(crate::clock_format::pattern())
                            )
                        })
                        .unwrap_or_else(|| "Usage limit reached".into())
                }
                Some((_, None)) => "Usage limit reached".into(),
                _ => "No usage limit reported".into(),
            };
            rows.push(detail_text("detail-usage", text));
        }
        if prefs.context {
            if let Some(reading) = chat.latest_context_reading() {
                rows.push(detail_text(
                    "detail-context",
                    format!(
                        "Context · {:.0}% ({} / {} tokens)",
                        reading.percent(),
                        reading.used,
                        reading.window
                    ),
                ));
            }
        }
        let menu_open = self.details.menu == Some(MenuAt::Card);
        let configure = web_button(
            "detail-configure",
            WebButtonVariant::Ghost,
            WebButtonSize::IconSmall,
            ButtonState::Enabled,
        )
        .debug_selector(|| "detail-configure".into())
        .aria_label("Customize thread details")
        .aria_expanded(menu_open)
        .child(
            svg()
                .path(assets::ELLIPSIS_ICON_PATH)
                .size(px(16.))
                .text_color(theme::rambutan_400()),
        );
        let configure = on_press_view(configure, cx, |this, w, cx| {
            this.open_detail_menu(MenuAt::Card, w, cx)
        });
        div()
            .absolute()
            .top(px(46.))
            .right(px(12.))
            .w(px(280.))
            .max_w_full()
            .child(
                div()
                    .id("thread-details-card")
                    .debug_selector(|| "thread-details-card".into())
                    .track_focus(&self.details.card)
                    .role(if self.details.inline {
                        Role::Group
                    } else {
                        Role::Dialog
                    })
                    .aria_label("Thread details")
                    .occlude()
                    .rounded(px(8.))
                    .shadow_md()
                    .border_1()
                    .border_color(theme::ring_floating())
                    .bg(theme::longan_900())
                    .p_2()
                    .flex()
                    .flex_col()
                    .on_mouse_down_out(cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        if this.details.inline || this.details.menu.is_some() {
                            return;
                        }
                        if let Some(t) = this.active_thread() {
                            if t.chat
                                .read(cx)
                                .details_trigger_bounds
                                .get()
                                .contains(&event.position)
                            {
                                return;
                            }
                        }
                        this.details.close();
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event: &MouseDownEvent, w, cx| {
                            this.open_detail_menu(MenuAt::Card, w, cx);
                            this.details.context_position = Some(event.position);
                            cx.stop_propagation();
                        }),
                    )
                    .child(
                        div()
                            .relative()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .px_2()
                                    .text_sm()
                                    .text_color(theme::rambutan_200())
                                    .child("Thread details"),
                            )
                            .child(configure)
                            .when(menu_open, |el| {
                                if let Some(position) = self.details.context_position {
                                    el.child(
                                        deferred(
                                            anchored()
                                                .position(position)
                                                .snap_to_window_with_margin(px(6.))
                                                .child(self.detail_menu(cx)),
                                        )
                                        .with_priority(1),
                                    )
                                } else {
                                    el.child(div().absolute().top_0().right(px(MENU_WIDTH)).child(
                                        menu_popover(
                                            size(px(MENU_WIDTH), px(28.)),
                                            self.detail_menu(cx),
                                        ),
                                    ))
                                }
                            }),
                    )
                    .child(
                        div()
                            .id("thread-details-items")
                            .overflow_y_scroll()
                            .max_h((window.viewport_size().height - px(142.)).max(px(70.)))
                            .children(rows),
                    ),
            )
            .into_any_element()
    }
}

fn detail_text(id: &'static str, text: String) -> AnyElement {
    div()
        .debug_selector(move || id.into())
        .px_2()
        .py_1()
        .text_size(px(13.))
        .text_color(theme::rambutan_300())
        .child(text)
        .into_any_element()
}
