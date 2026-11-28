//! Focus navigation for the current native workspace layout.

use gpui::{
    div, App, Context, Div, FocusHandle, Focusable, InteractiveElement, ParentElement, Styled,
    Subscription, WeakFocusHandle, Window,
};

use super::{AppView, SettingsSection, Workspace};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Area {
    Sidebar,
    Content,
    Composer,
    Details,
    Panel,
    Terminal,
    SettingsNav,
    SettingsContent,
}

impl Area {
    const ALL: [Self; 8] = [
        Self::Sidebar,
        Self::Content,
        Self::Composer,
        Self::Details,
        Self::Panel,
        Self::Terminal,
        Self::SettingsNav,
        Self::SettingsContent,
    ];
}

pub(super) struct AreaFocus {
    handles: [FocusHandle; Area::ALL.len()],
    previous: [Option<WeakFocusHandle>; Area::ALL.len()],
    _subscriptions: Vec<Subscription>,
}

impl AreaFocus {
    pub(super) fn new(window: &mut Window, cx: &mut Context<'_, Workspace>) -> Self {
        let handles = std::array::from_fn(|_| cx.focus_handle().tab_stop(false));
        let subscriptions = Area::ALL
            .into_iter()
            .filter(|area| !matches!(area, Area::Composer | Area::Details))
            .map(|area| {
                cx.on_focus_out(
                    &handles[area as usize],
                    window,
                    move |workspace, event, window, _| {
                        if let Some(blurred) = event.blurred.upgrade() {
                            if &blurred == workspace.area_focus.handle(area) {
                                return;
                            }
                            // The main region also contains the composer and terminal.
                            // Keep its last control local to the conversation/page.
                            if workspace.area_focus.handles.iter().enumerate().any(
                                |(index, region)| {
                                    index != area as usize
                                        && workspace
                                            .area_focus
                                            .handle(area)
                                            .contains(region, window)
                                        && region.contains(&blurred, window)
                                },
                            ) {
                                return;
                            }
                        }
                        workspace.area_focus.previous[area as usize] = Some(event.blurred);
                    },
                )
            })
            .collect();
        Self {
            handles,
            previous: Default::default(),
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn handle(&self, area: Area) -> &FocusHandle {
        &self.handles[area as usize]
    }
}

impl Workspace {
    fn focus_regions(&self, window: &Window, cx: &App) -> Vec<(Area, FocusHandle, FocusHandle)> {
        let root = |area| self.area_focus.handle(area).clone();
        let mut regions = Vec::new();
        let mut push = |area, target: Option<FocusHandle>| {
            let root = root(area);
            regions.push((area, root.clone(), target.unwrap_or(root)));
        };
        if self.settings_visible() {
            let compact_search = self.settings_search_open
                && bruhs_ui::zoom::css_px(window.viewport_size().width) < 768.0;
            let nav_target =
                if compact_search || !self.settings_search.read(cx).value().trim().is_empty() {
                    Some(self.settings_search.focus_handle(cx))
                } else {
                    SettingsSection::ALL
                        .iter()
                        .position(|section| *section == self.settings_section)
                        .map(|index| self.settings_nav_focus[index].clone())
                };
            push(Area::SettingsNav, nav_target);
            if !compact_search {
                push(Area::SettingsContent, None);
            }
            return regions;
        }
        let booking = matches!(self.active_view, AppView::Booking { .. });
        let sidebar_open = if Self::sidebar_drawer_layout(window) {
            self.compact_sidebar_open
        } else {
            self.sidebar_visible
        };
        if sidebar_open && !booking {
            push(Area::Sidebar, Some(self.sidebar_search.focus_handle(cx)));
        }
        let active_chat = (self.active_view == AppView::Chat)
            .then(|| self.active_thread())
            .flatten();
        let chat = active_chat.map(|thread| thread.chat.read(cx));
        let transcript = chat
            .filter(|chat| !chat.messages.is_empty())
            .map(|chat| chat.messages_focus.clone());
        push(Area::Content, transcript);
        if let Some(chat) = chat {
            regions.push((
                Area::Composer,
                chat.composer_area_focus.clone(),
                chat.input_state.focus_handle(cx),
            ));
        }
        if active_chat.is_some_and(|thread| self.details.is_open(&thread.id)) {
            regions.push((
                Area::Details,
                self.details.card.clone(),
                self.details.card.clone(),
            ));
        }
        if self.active_view == AppView::Chat && self.right_panel_open() {
            let panel = root(Area::Panel);
            regions.push((Area::Panel, panel.clone(), panel));
        }
        if self.active_view == AppView::Terminal
            || (self.active_view == AppView::Chat && self.terminal_drawer_open)
        {
            let terminal = root(Area::Terminal);
            let target = self
                .terminal_panel
                .read(cx)
                .active_session(cx)
                .map_or_else(|| terminal.clone(), |session| session.focus_handle(cx));
            regions.push((Area::Terminal, terminal, target));
        }
        regions
    }

    pub(super) fn step_workspace_focus(
        &mut self,
        backwards: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let settings_only = self.settings_visible()
            && self.top_layer.is_none()
            && self.task_detail_menu_open.is_none()
            && self.date_picker.is_none();
        let model_picker_open = self.active_view == AppView::Chat
            && self
                .active_thread()
                .is_some_and(|thread| thread.chat.read(cx).model_picker_open);
        if !settings_only && (self.overlay_open() || model_picker_open) {
            return false;
        }
        self.close_thread_menu(window, cx);
        self.close_sidebar_apps_menu(window, cx);
        let regions = self.focus_regions(window, cx);
        let focused = window.focused(cx);
        let current = focused.as_ref().and_then(|focused| {
            regions.iter().rposition(|(_, root, default)| {
                default == focused || root.contains(focused, window)
            })
        });
        if let Some((index, focused)) = current.zip(focused) {
            let (area, root, _) = &regions[index];
            if &focused != root {
                self.area_focus.previous[*area as usize] = Some(focused.downgrade());
            }
        }
        let next = match (current, backwards) {
            (Some(index), false) => (index + 1) % regions.len(),
            (Some(index), true) => (index + regions.len() - 1) % regions.len(),
            (None, false) => 0,
            (None, true) => regions.len() - 1,
        };
        let (area, root, default) = &regions[next];
        let target = self.area_focus.previous[*area as usize]
            .as_ref()
            .and_then(WeakFocusHandle::upgrade)
            .filter(|previous| {
                previous != root
                    && root.contains(previous, window)
                    && !regions
                        .iter()
                        .skip(next + 1)
                        .any(|(_, child, _)| child.contains(previous, window))
            })
            .unwrap_or_else(|| default.clone());
        target.focus(window, cx);
        cx.notify();
        true
    }

    pub(super) fn render_focusable_terminal(&self) -> Div {
        div()
            .track_focus(self.area_focus.handle(Area::Terminal))
            .flex()
            .flex_col()
            .flex_1()
            .size_full()
            .min_h_0()
            .child(self.terminal_panel.clone())
    }
}
