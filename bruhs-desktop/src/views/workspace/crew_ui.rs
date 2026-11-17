//! The crew rail, 1:1 with web `crew/rail.rs`: every bruh above the thread
//! list with its face, status and unread count, and the New bruh dialog. It
//! sits outside the thread sections, so a bruh never mixes into thread
//! sorting, folds or keyboard rows.

use super::*;
use crate::views::crew_face::bruh_face;
use crate::views::reactive_automations::local_time;
use bruhs_models::crew::{
    rail_order, status_line, BruhState, BruhStatus, ServiceNotice, RAIL_FACE_PX,
};
use bruhs_models::reactive::ReactiveCommand;

const CREW_TITLE: &str = "Crew";
const NEW_BRUH_LABEL: &str = "New bruh";
/// Web `px-2 pb-2` around the rail, `px-1` inside it.
const CREW_X_PADDING_PX: f32 = 8.0;
const CREW_BOTTOM_PADDING_PX: f32 = 8.0;
const CREW_INNER_X_PADDING_PX: f32 = 4.0;
const CREW_GAP_PX: f32 = 4.0;
/// Web `space-y-0.5` between rows.
const CREW_ROW_GAP_PX: f32 = 2.0;
/// A row's transparent focus border. The web draws an outline instead, so
/// the border comes out of the row's padding.
const CREW_ROW_BORDER_PX: f32 = 1.0;
/// Web `px-2 py-1` on a row, less the focus border.
const CREW_ROW_X_PADDING_PX: f32 = 8.0 - CREW_ROW_BORDER_PX;
const CREW_ROW_Y_PADDING_PX: f32 = 4.0 - CREW_ROW_BORDER_PX;
/// Web `max-h-[40vh]` on the list: the rail takes at most this share of the
/// sidebar, and its list scrolls.
const CREW_MAX_SHARE: f32 = 0.4;
const CREW_RADIUS_GRAPE_PX: f32 = 8.0;
const CREW_RADIUS_SEED_PX: f32 = 4.0;
const CREW_LABEL_TEXT_PX: f32 = 12.0;
const CREW_NAME_TEXT_PX: f32 = theme::APP_UI_TEXT_SIZE_PX;
const CREW_UNREAD_X_PADDING_PX: f32 = 6.0;
const CREW_HOVER_OPACITY: f32 = 0.40;
const CREW_CURRENT_OPACITY: f32 = 0.60;
const CREW_ACTION_HOVER_OPACITY: f32 = 0.50;
const CREW_HAIRLINE_OPACITY: f32 = 0.80;
const CREW_NOTICE_FILL_OPACITY: f32 = 0.60;
const CREW_NOTICE_RING_OPACITY: f32 = 0.50;
const CREW_DISABLED_OPACITY: f32 = 0.50;

/// The bruhs in rail order.
fn rail_rows(bruhs: &[BruhState]) -> Vec<BruhState> {
    let mut rows = bruhs.to_vec();
    rail_order(&mut rows);
    rows
}

/// The line under a bruh's name, with its next wake in local time.
fn row_status(state: &BruhState) -> String {
    let wake = state.next_wake_ms.map(local_time);
    status_line(state.status, wake.as_deref())
}

fn unread_text(unread: usize) -> String {
    if unread > 99 {
        "99+".into()
    } else {
        unread.to_string()
    }
}

/// What a row reads as: its name and line, then any unread count.
fn row_label(state: &BruhState) -> String {
    let line = format!("{} {}", state.name, row_status(state));
    if state.unread > 0 {
        format!("{line}, {} unread", state.unread)
    } else {
        line
    }
}

fn crew_text(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .text_size(px(CREW_LABEL_TEXT_PX))
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .text_color(theme::rambutan_500())
        .child(text.into())
}

/// Web `SMALL_ACTION`: a quiet blueberry text button.
fn small_action(id: &'static str, label: &'static str, disabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .role(Role::Button)
        .aria_label(label)
        .min_h(px(theme::CONTROL_XS_PX))
        .px_2()
        .flex()
        .items_center()
        .flex_shrink_0()
        .rounded(px(CREW_RADIUS_SEED_PX))
        .text_size(px(CREW_LABEL_TEXT_PX))
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .text_color(theme::blueberry_300())
        .border_1()
        .border_color(gpui::transparent_black())
        .child(label)
        .map(|el| {
            if disabled {
                el.opacity(CREW_DISABLED_OPACITY)
            } else {
                el.tab_index(0)
                    .cursor_pointer()
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .hover(|el| {
                        el.bg(theme::longan_800().opacity(CREW_ACTION_HOVER_OPACITY))
                            .text_color(theme::blueberry_200())
                    })
            }
        })
}

/// Run `action` on a click or on Enter or Space, once per press.
fn on_press(
    el: Stateful<Div>,
    cx: &Context<'_, Workspace>,
    action: impl Fn(&mut Workspace, &mut Window, &mut Context<'_, Workspace>) + 'static,
) -> Stateful<Div> {
    let action = Rc::new(action);
    let key = Rc::clone(&action);
    el.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
        if !event.is_keyboard() {
            cx.stop_propagation();
            action(this, window, cx);
        }
    }))
    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
        if is_web_button_activation_key(event.keystroke.key.as_str()) {
            window.prevent_default();
            cx.stop_propagation();
            key(this, window, cx);
        }
    }))
}

impl Workspace {
    /// The rail between the sidebar controls and the thread list.
    pub(super) fn render_crew_rail(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let panel = self.reactive_automations.read(cx);
        let open = self.crew_rail_open;
        let ready = panel.crew_ready();
        let rows = rail_rows(panel.crew());
        let any_paused = rows.iter().any(|bruh| bruh.status == BruhStatus::Paused);
        let header = div()
            .px(px(CREW_INNER_X_PADDING_PX))
            .flex()
            .items_center()
            .gap_1()
            .flex_shrink_0()
            .child(on_press(
                div()
                    .id("desktop-crew-toggle")
                    .debug_selector(|| "desktop-crew-toggle".to_string())
                    .role(Role::Button)
                    .aria_label(CREW_TITLE)
                    .aria_expanded(open)
                    .tab_index(0)
                    .min_h(px(theme::CONTROL_PX))
                    .min_w_0()
                    .flex_1()
                    .py_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .text_size(px(CREW_LABEL_TEXT_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_500())
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .hover(|el| el.text_color(theme::rambutan_300()))
                    .child(div().flex_shrink_0().child(CREW_TITLE))
                    .child(
                        div()
                            .flex_1()
                            .h(px(1.0))
                            .bg(theme::longan_800().opacity(CREW_HAIRLINE_OPACITY)),
                    )
                    .child(
                        svg()
                            .path(assets::CHEVRON_DOWN_ICON_PATH)
                            .size(px(theme::ICON_PX))
                            .flex_shrink_0()
                            .text_color(theme::rambutan_500())
                            .when(open, |el| {
                                el.with_transformation(Transformation::rotate(Radians(
                                    std::f32::consts::PI,
                                )))
                            }),
                    ),
                cx,
                |this, _window, cx| this.toggle_crew_rail(cx),
            ))
            .child(
                div()
                    .id("desktop-crew-new")
                    .debug_selector(|| "desktop-crew-new".to_string())
                    .role(Role::Button)
                    .aria_label(NEW_BRUH_LABEL)
                    .size(px(theme::CONTROL_SM_PX))
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_shrink_0()
                    .rounded(px(CREW_RADIUS_GRAPE_PX))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .child(
                        svg()
                            .path(assets::PLUS_ICON_PATH)
                            .size(px(theme::ICON_PX))
                            .text_color(theme::rambutan_400()),
                    )
                    .map(|el| {
                        if ready {
                            on_press(
                                el.track_focus(&self.crew_new_focus)
                                    .tab_index(0)
                                    .cursor_pointer()
                                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                                    .hover(|el| {
                                        el.bg(theme::longan_800()
                                            .opacity(CREW_ACTION_HOVER_OPACITY))
                                    }),
                                cx,
                                |this, window, cx| this.open_create_bruh_dialog(window, cx),
                            )
                        } else {
                            el.opacity(CREW_DISABLED_OPACITY)
                        }
                    }),
            );
        let body = open.then(|| {
            let mut body = div()
                .id("desktop-crew-body")
                .min_h_0()
                .flex()
                .flex_col()
                .gap(px(CREW_GAP_PX));
            if let Some(notice) = panel.crew_notice().filter(|_| !panel.service_updating()) {
                body = body.child(self.render_crew_notice(notice, cx));
            }
            if any_paused {
                body = body.child(div().px(px(CREW_INNER_X_PADDING_PX)).flex().child(
                    if ready && !panel.crew_busy() {
                        on_press(
                            small_action("desktop-crew-resume-all", "Resume all", false),
                            cx,
                            |this, _window, cx| {
                                this.reactive_automations.update(cx, |panel, cx| {
                                    panel.send_crew(ReactiveCommand::ResumeAll, cx);
                                });
                            },
                        )
                    } else {
                        small_action("desktop-crew-resume-all", "Resume all", true)
                    },
                ));
            }
            if panel.crew_protocol().is_none() {
                if panel.crew_load_error().is_some() {
                    body = body.child(
                        div()
                            .px(px(CREW_INNER_X_PADDING_PX))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(crew_text("Couldn't load the crew."))
                            .child(on_press(
                                small_action("desktop-crew-retry", "Retry", false),
                                cx,
                                |this, _window, cx| {
                                    this.reactive_automations
                                        .update(cx, |panel, cx| panel.reload_crew(cx));
                                },
                            )),
                    );
                }
            } else if rows.is_empty() {
                body = body.child(crew_text("No bruhs yet.").px(px(CREW_INNER_X_PADDING_PX)));
            }
            if let Some(message) = panel.crew_error() {
                body = body.child(
                    div()
                        .id("desktop-crew-error")
                        .role(Role::Alert)
                        .mx(px(CREW_INNER_X_PADDING_PX))
                        .text_size(px(CREW_LABEL_TEXT_PX))
                        .text_color(theme::lychee_200())
                        .child(message.to_string()),
                );
            }
            body.child(
                div()
                    .id("desktop-crew-list")
                    .debug_selector(|| "desktop-crew-list".to_string())
                    .role(Role::List)
                    .aria_label("Bruhs")
                    .min_h_0()
                    .flex_shrink(1.0)
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(CREW_ROW_GAP_PX))
                    .children(
                        rows.iter()
                            .enumerate()
                            .map(|(index, bruh)| self.render_crew_row(bruh, index == 0, cx)),
                    ),
            )
        });
        Some(
            div()
                .id("desktop-crew-rail")
                .debug_selector(|| "desktop-crew-rail".to_string())
                .role(Role::Region)
                .aria_label(CREW_TITLE)
                .px(px(CREW_X_PADDING_PX))
                .pb(px(CREW_BOTTOM_PADDING_PX))
                .max_h(relative(CREW_MAX_SHARE))
                .min_h_0()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .child(header)
                .children(body)
                .into_any_element(),
        )
    }

    /// The older-service notice, with its update button or what to run.
    fn render_crew_notice(&self, notice: ServiceNotice, cx: &Context<'_, Self>) -> Div {
        let panel = self.reactive_automations.read(cx);
        let action = if panel.can_update_service() {
            on_press(
                small_action("desktop-crew-update-service", "Update service", false),
                cx,
                |this, _window, cx| {
                    this.reactive_automations
                        .update(cx, |panel, cx| panel.update_service(cx));
                },
            )
            .into_any_element()
        } else {
            crew_text(crate::helpers::SERVICE_INSTALL_HINT)
                .text_color(theme::rambutan_400())
                .into_any_element()
        };
        div()
            .debug_selector(|| "desktop-crew-notice".to_string())
            .mx(px(CREW_INNER_X_PADDING_PX))
            .p_2()
            .rounded(px(CREW_RADIUS_GRAPE_PX))
            .bg(theme::longan_950().opacity(CREW_NOTICE_FILL_OPACITY))
            .border_1()
            .border_color(theme::longan_700().opacity(CREW_NOTICE_RING_OPACITY))
            .flex()
            .flex_col()
            .gap_1()
            .text_size(px(CREW_LABEL_TEXT_PX))
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_color(theme::rambutan_300())
            .child(notice.message())
            .child(div().flex().child(action))
            .when_some(panel.service_update_error(), |el, error| {
                el.child(
                    div()
                        .text_color(theme::lychee_200())
                        .child(error.to_string()),
                )
            })
    }

    fn render_crew_row(&self, bruh: &BruhState, first: bool, cx: &Context<'_, Self>) -> AnyElement {
        let agent_id = bruh.agent_id.clone();
        let current = matches!(
            &self.active_view,
            AppView::Bruh { agent_id: open } if *open == agent_id
        );
        let selector = format!("desktop-crew-row-{agent_id}");
        on_press(
            div()
                // Keyed by the bruh, so focus stays on it when the rail
                // re-sorts.
                .id(SharedString::from(selector.clone()))
                .debug_selector(move || selector)
                .role(Role::Button)
                .aria_label(row_label(bruh))
                .when(first, |el| el.track_focus(&self.crew_row_focus))
                .tab_index(0)
                .w_full()
                .min_h(px(theme::CONTROL_PX))
                .min_w_0()
                .px(px(CREW_ROW_X_PADDING_PX))
                .py(px(CREW_ROW_Y_PADDING_PX))
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .rounded(px(CREW_RADIUS_GRAPE_PX))
                .border(px(CREW_ROW_BORDER_PX))
                .border_color(gpui::transparent_black())
                .focus_visible(|el| el.border_color(theme::persimmon_400()))
                .map(|el| {
                    if current {
                        el.bg(theme::longan_800().opacity(CREW_CURRENT_OPACITY))
                    } else {
                        el.hover(|el| el.bg(theme::longan_800().opacity(CREW_HOVER_OPACITY)))
                    }
                })
                .child(bruh_face(
                    SharedString::from(format!("crew-{agent_id}")),
                    RAIL_FACE_PX,
                    bruh.look.clone(),
                    bruh.status,
                    cx,
                ))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .truncate()
                                .text_size(px(CREW_NAME_TEXT_PX))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::rambutan_100())
                                .child(bruh.name.clone()),
                        )
                        .child(crew_text(row_status(bruh)).truncate()),
                )
                .when(bruh.unread > 0, |el| {
                    el.child(
                        div()
                            .flex_shrink_0()
                            .px(px(CREW_UNREAD_X_PADDING_PX))
                            .rounded_full()
                            .bg(theme::blueberry_400())
                            .text_size(px(CREW_LABEL_TEXT_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_color(theme::longan_950())
                            .child(unread_text(bruh.unread)),
                    )
                }),
            cx,
            move |this, window, cx| {
                this.activate_sidebar_view(
                    AppView::Bruh {
                        agent_id: agent_id.clone(),
                    },
                    window,
                    cx,
                );
            },
        )
        .into_any_element()
    }

    /// ⌘⇧B: open the rail if it is folded and focus its first bruh, or the
    /// New bruh button when there is none.
    pub(super) fn focus_crew(
        &mut self,
        _: &FocusCrew,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.crew_rail_open {
            self.focus_crew_target(window, cx);
            return;
        }
        self.crew_rail_open = true;
        self.persist_and_notify(cx);
        // The rows draw next frame; focus them once they have.
        cx.defer_in(window, |this, window, cx| {
            this.focus_crew_target(window, cx)
        });
    }

    fn focus_crew_target(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.reactive_automations.read(cx).crew().is_empty() {
            self.crew_new_focus.focus(window, cx);
        } else {
            self.crew_row_focus.focus(window, cx);
        }
    }

    /// ⌘.: stop the thread on screen as its Interrupt does, then every bruh.
    pub(super) fn stop_all(
        &mut self,
        _: &StopAll,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) {
            chat.update(cx, |chat, cx| chat.stop_streaming(cx));
        }
        let stopping = self
            .reactive_automations
            .update(cx, |panel, cx| panel.stop_all_crew(cx));
        // The notice waits for the sends, so it never claims a failed stop.
        let hub = self.meetings_hub.clone();
        cx.spawn(async move |_, cx| {
            let outcome = stopping.await;
            hub.update(cx, |hub, cx| match outcome {
                Ok(message) => hub.show_notice(message, false, cx),
                Err(error) => hub.show_notice(error, true, cx),
            });
        })
        .detach();
    }

    fn toggle_crew_rail(&mut self, cx: &mut Context<'_, Self>) {
        self.crew_rail_open = !self.crew_rail_open;
        self.persist_and_notify(cx);
    }

    /// The rail's +: a blank New bruh dialog, once the service speaks crew.
    pub(super) fn open_create_bruh_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.reactive_automations.read(cx).crew_ready() {
            return;
        }
        let roles = self.roles.resolved("");
        self.reactive_automations
            .update(cx, |panel, cx| panel.open_create_bruh(roles, window, cx));
        self.open_overlay(Overlay::CreateBruh);
        self.focus_first_modal_tab_stop(window, cx);
        cx.notify();
    }

    /// The New bruh dialog: the panel's fields in the shared modal shell.
    pub(super) fn render_create_bruh(
        &self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let (fields, disabled) = self.reactive_automations.update(cx, |panel, cx| {
            (panel.render_create_bruh(cx), panel.create_bruh_disabled())
        });
        let cancel = on_press(
            web_button(
                "desktop-create-bruh-cancel",
                WebButtonVariant::Ghost,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .debug_selector(|| "desktop-create-bruh-cancel".to_string())
            .aria_label("Cancel")
            .role(Role::Button)
            .tab_index(0)
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .child("Cancel"),
            cx,
            |this, _window, cx| {
                this.close_transient_surfaces();
                cx.notify();
            },
        );
        let create = web_button(
            "desktop-create-bruh-submit",
            WebButtonVariant::Primary,
            WebButtonSize::Small,
            ButtonState::disabled_if(disabled),
        )
        .debug_selector(|| "desktop-create-bruh-submit".to_string())
        .aria_label("Create")
        .role(Role::Button)
        .child("Create");
        let create = if disabled {
            create
        } else {
            on_press(
                create
                    .tab_index(0)
                    .focus_visible(|el| el.border_color(theme::persimmon_400())),
                cx,
                |this, _window, cx| {
                    this.reactive_automations
                        .update(cx, |panel, cx| panel.submit_create_bruh(cx));
                },
            )
        };
        let footer = div()
            .pt_3()
            .flex()
            .justify_end()
            .gap_2()
            .child(cancel)
            .child(create);
        self.render_modal_shell_sized(
            NEW_BRUH_LABEL,
            fields.unwrap_or_else(div),
            Some(footer),
            DESKTOP_DIALOG_2XL_WIDTH_PX,
            window,
            cx,
        )
    }

    /// A dialog closed by Escape, the backdrop or another surface forgets its
    /// bruh, so a late answer never opens it.
    pub(super) fn sync_create_bruh(&mut self, cx: &mut Context<'_, Self>) {
        if !matches!(self.overlay, Some(Overlay::CreateBruh))
            && self.reactive_automations.read(cx).create_bruh_open()
        {
            self.reactive_automations
                .update(cx, |panel, cx| panel.close_create_bruh(cx));
        }
    }

    /// The service made the dialog's bruh: open its page.
    pub(super) fn open_created_bruh(
        &mut self,
        agent_id: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.activate_sidebar_view(AppView::Bruh { agent_id }, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{performance_tests::fixture, Workspace};
    use super::{rail_rows, row_status, unread_text};
    use bruhs_models::crew::{BruhAccess, BruhLook, BruhState, BruhStatus, CREW_PROTOCOL};
    use bruhs_models::reactive::ReactiveCommand;
    use bruhs_models::EpochMs;
    use gpui::{Entity, VisualTestContext};

    fn bruh(name: &str, status: BruhStatus) -> BruhState {
        BruhState {
            agent_id: format!("id-{name}"),
            name: name.into(),
            look: BruhLook::default(),
            access: BruhAccess::default(),
            effort: None,
            status,
            next_wake_ms: None,
            unread: 0,
        }
    }

    fn show(
        workspace: &Entity<Workspace>,
        cx: &mut VisualTestContext,
        states: Vec<BruhState>,
        protocol: u32,
    ) {
        workspace.update_in(cx, |workspace, _, cx| {
            workspace.reactive_automations.update(cx, |panel, cx| {
                panel.show_crew_for_test(states, protocol, cx);
            });
            cx.notify();
        });
        cx.run_until_parked();
    }

    fn leak(text: String) -> &'static str {
        Box::leak(text.into_boxed_str())
    }

    #[gpui::test]
    fn crew_rows_are_as_tall_as_the_web_rows(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = fixture(cx);
        cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
        show(
            &workspace,
            cx,
            vec![bruh("amy", BruhStatus::Asleep)],
            CREW_PROTOCOL,
        );
        let row = cx.debug_bounds("desktop-crew-row-id-amy").expect("amy");
        // Web: py-1 around a 13px name and a 12px status line at 1.5 line
        // height. The 1px focus border sits inside that padding.
        let web = 4.0 + 13.0 * 1.5 + 12.0 * 1.5 + 4.0;
        assert!(
            (f32::from(row.size.height) - web).abs() < 0.5,
            "row {:?}, web {web}",
            row.size.height
        );
    }

    #[gpui::test]
    fn the_crew_rail_sits_above_threads_outside_their_sections(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = fixture(cx);
        cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.new_thread_in_project("proj".into(), "/tmp/bruhs-crew".into(), window, cx);
        });
        show(
            &workspace,
            cx,
            vec![
                bruh("amy", BruhStatus::Asleep),
                bruh("zed", BruhStatus::NeedsYou),
            ],
            CREW_PROTOCOL,
        );
        let rail = cx.debug_bounds("desktop-crew-rail").expect("the crew rail");
        let threads = cx
            .debug_bounds("desktop-sidebar-threads")
            .expect("the thread list");
        assert!(
            rail.bottom() <= threads.top(),
            "rail {rail:?} sits above threads {threads:?}"
        );
        let zed = cx.debug_bounds("desktop-crew-row-id-zed").expect("zed");
        let amy = cx.debug_bounds("desktop-crew-row-id-amy").expect("amy");
        assert!(zed.top() < amy.top(), "whoever needs you comes first");
        assert!(rail.contains(&zed.center()) && rail.contains(&amy.center()));
        assert!(!threads.contains(&zed.center()), "no bruh is a thread row");
        // A row is a control: at least 32px tall, with a 20px face.
        assert!(zed.size.height >= gpui::px(crate::theme::CONTROL_PX));
        let face = cx
            .debug_bounds("desktop-bruh-face-crew-id-zed")
            .expect("zed's face");
        assert_eq!(face.size.width, gpui::px(20.0));
        // Clicking a row opens the bruh.
        cx.simulate_click(amy.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        workspace.update(cx, |workspace, cx| {
            assert_eq!(
                workspace.active_view,
                bruhs_models::AppView::Bruh {
                    agent_id: "id-amy".into()
                }
            );
            assert_eq!(
                workspace.reactive_automations.read(cx).bruh_page_agent(),
                Some("id-amy")
            );
        });
        assert!(
            cx.debug_bounds("desktop-bruh-page").is_some(),
            "its page shows"
        );
        // Leaving the bruh closes its page; Automations draws the panel again.
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.activate_sidebar_view(bruhs_models::AppView::Automations, window, cx);
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-bruh-page").is_none());
        workspace.update(cx, |workspace, cx| {
            assert_eq!(
                workspace.reactive_automations.read(cx).bruh_page_agent(),
                None
            );
        });
    }

    #[gpui::test]
    fn crew_faces_sweep_only_with_motion(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = fixture(cx);
        cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
        show(
            &workspace,
            cx,
            vec![bruh("bo", BruhStatus::Working)],
            CREW_PROTOCOL,
        );
        let face = leak("desktop-bruh-face-crew-id-bo".to_string());
        let sweep = leak("desktop-bruh-face-sweep-crew-id-bo".to_string());
        for reduced in [false, true] {
            cx.update(|_, cx| crate::motion::set_reduced_motion(reduced, cx));
            workspace.update(cx, |_, cx| cx.notify());
            cx.run_until_parked();
            assert!(cx.debug_bounds(face).is_some(), "the face shows");
            assert_eq!(
                cx.debug_bounds(sweep).is_some(),
                !reduced,
                "reduced: {reduced}"
            );
        }
    }

    #[gpui::test]
    fn an_older_service_shows_a_notice(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = fixture(cx);
        cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
        show(&workspace, cx, Vec::new(), CREW_PROTOCOL);
        assert!(cx.debug_bounds("desktop-crew-notice").is_none());
        show(&workspace, cx, Vec::new(), 0);
        assert!(cx.debug_bounds("desktop-crew-notice").is_some());
        // New bruh needs the newer service.
        let new = cx.debug_bounds("desktop-crew-new").expect("new bruh");
        cx.simulate_click(new.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        workspace.update(cx, |workspace, _| assert!(workspace.overlay.is_none()));
    }

    #[gpui::test]
    fn resume_all_sends_one_command(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = fixture(cx);
        cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
        show(
            &workspace,
            cx,
            vec![bruh("cy", BruhStatus::Paused)],
            CREW_PROTOCOL,
        );
        let resume = cx
            .debug_bounds("desktop-crew-resume-all")
            .expect("resume all");
        cx.simulate_click(resume.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        workspace.update(cx, |workspace, cx| {
            assert_eq!(
                workspace.reactive_automations.read(cx).sent_for_test(),
                [ReactiveCommand::ResumeAll]
            );
        });
    }

    fn queued_run(id: &str) -> bruhs_models::reactive::ReactiveRun {
        serde_json::from_value(serde_json::json!({
            "id": id, "agent_id": "id-amy", "source_id": null, "subscription_id": null,
            "event_id": null, "status": "queued", "prompt": id, "thread_id": "t",
            "session_id": null, "created_at": 1, "updated_at": 1,
            "not_before": 1, "result": null, "error": null,
        }))
        .unwrap()
    }

    fn show_runs(
        workspace: &Entity<Workspace>,
        cx: &mut VisualTestContext,
        protocol: u32,
        runs: Vec<bruhs_models::reactive::ReactiveRun>,
    ) {
        workspace.update_in(cx, |workspace, _, cx| {
            workspace.reactive_automations.update(cx, |panel, cx| {
                panel.show_snapshot_for_test(
                    bruhs_models::reactive::ReactiveSnapshot {
                        protocol,
                        runs,
                        ..Default::default()
                    },
                    cx,
                );
            });
        });
        cx.run_until_parked();
    }

    fn sent(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<ReactiveCommand> {
        workspace.update(cx, |workspace, cx| {
            workspace
                .reactive_automations
                .read(cx)
                .sent_for_test()
                .to_vec()
        })
    }

    fn dispatch(
        workspace: &Entity<Workspace>,
        cx: &mut VisualTestContext,
        action: Box<dyn gpui::Action>,
    ) {
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.workspace_focus_handle.focus(window, cx);
            window.dispatch_action(action, cx);
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn stop_all_stops_the_streaming_thread_and_every_bruh(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = fixture(cx);
        cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.new_thread_in_project("proj".into(), "/tmp/bruhs-crew".into(), window, cx);
            let chat = workspace.active_thread().unwrap().chat.clone();
            chat.update(cx, |chat, _| chat.is_streaming = true);
        });
        show_runs(&workspace, cx, CREW_PROTOCOL, vec![queued_run("q")]);
        dispatch(&workspace, cx, Box::new(super::StopAll));
        workspace.update(cx, |workspace, cx| {
            let chat = workspace.active_thread().unwrap().chat.read(cx);
            assert!(!chat.is_streaming, "the thread on screen stopped");
        });
        assert_eq!(sent(&workspace, cx), [ReactiveCommand::StopAll]);
        workspace.update(cx, |workspace, cx| {
            let notice = workspace.meetings_hub.read(cx).notice.as_ref();
            assert_eq!(
                notice.map(|n| n.message.as_str()),
                Some("Stopped all bruhs.")
            );
        });
    }

    /// ⌘. never waits behind a command in flight, and says when it failed.
    #[gpui::test]
    fn stop_all_skips_the_queue_and_reports_a_failure(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = fixture(cx);
        cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
        show_runs(&workspace, cx, CREW_PROTOCOL, vec![queued_run("q")]);
        workspace.update(cx, |workspace, cx| {
            workspace.reactive_automations.update(cx, |panel, _| {
                panel.set_busy_for_test(true);
                panel.fail_stops_for_test("offline");
            });
        });
        dispatch(&workspace, cx, Box::new(super::StopAll));
        assert_eq!(sent(&workspace, cx), [ReactiveCommand::StopAll]);
        workspace.update(cx, |workspace, cx| {
            let notice = workspace.meetings_hub.read(cx).notice.as_ref();
            assert_eq!(
                notice.map(|n| n.message.as_str()),
                Some("Couldn't stop every bruh: offline")
            );
        });
    }

    #[gpui::test]
    fn stop_all_cancels_each_live_run_on_an_old_service(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = fixture(cx);
        cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
        show_runs(&workspace, cx, 0, vec![queued_run("q")]);
        dispatch(&workspace, cx, Box::new(super::StopAll));
        assert_eq!(
            sent(&workspace, cx),
            [ReactiveCommand::CancelRun { id: "q".into() }]
        );
    }

    #[gpui::test]
    fn cmd_shift_b_focuses_the_first_crew_row(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = fixture(cx);
        cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
        show(
            &workspace,
            cx,
            vec![
                bruh("amy", BruhStatus::Asleep),
                bruh("zed", BruhStatus::NeedsYou),
            ],
            CREW_PROTOCOL,
        );
        // Fold the rail first: the shortcut opens it.
        workspace.update_in(cx, |workspace, _, cx| workspace.toggle_crew_rail(cx));
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-crew-row-id-zed").is_none());
        dispatch(&workspace, cx, Box::new(super::FocusCrew));
        cx.run_until_parked();
        workspace.update_in(cx, |workspace, window, _| {
            assert!(workspace.crew_rail_open, "the rail opened");
            assert!(
                workspace.crew_row_focus.is_focused(window),
                "the first row (whoever needs you) has focus"
            );
        });
        assert!(cx.debug_bounds("desktop-crew-row-id-zed").is_some());
    }

    #[gpui::test]
    fn cmd_shift_b_with_no_bruhs_focuses_the_new_button(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = fixture(cx);
        cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
        show(&workspace, cx, Vec::new(), CREW_PROTOCOL);
        dispatch(&workspace, cx, Box::new(super::FocusCrew));
        workspace.update_in(cx, |workspace, window, _| {
            assert!(workspace.crew_new_focus.is_focused(window));
        });
    }

    #[gpui::test]
    fn the_rail_folds_and_new_bruh_opens_the_dialog(cx: &mut gpui::TestAppContext) {
        let (workspace, cx) = fixture(cx);
        cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
        show(
            &workspace,
            cx,
            vec![bruh("dee", BruhStatus::Asleep)],
            CREW_PROTOCOL,
        );
        let toggle = cx.debug_bounds("desktop-crew-toggle").expect("toggle");
        cx.simulate_click(toggle.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-crew-row-id-dee").is_none());
        workspace.update(cx, |workspace, cx| {
            assert!(!workspace.crew_rail_open);
            assert_eq!(workspace.snapshot(cx).crew_rail_open, Some(false), "saved");
        });
        let new = cx.debug_bounds("desktop-crew-new").expect("new bruh");
        cx.simulate_click(new.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-create-bruh").is_some());
        assert!(cx.debug_bounds("desktop-bruh-face-editor").is_some());
        // Web `max-w-2xl`.
        let dialog = cx.debug_bounds("desktop-modal-dialog").expect("dialog");
        assert_eq!(dialog.size.width, gpui::px(672.0));
        // Cancel closes it, and the panel forgets its bruh.
        let cancel = cx
            .debug_bounds("desktop-create-bruh-cancel")
            .expect("cancel");
        cx.simulate_click(cancel.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-create-bruh").is_none());
        workspace.update(cx, |workspace, cx| {
            assert!(!workspace.reactive_automations.read(cx).create_bruh_open());
        });
    }

    #[test]
    fn rows_put_whoever_needs_you_first_and_say_when_a_sleeper_wakes() {
        let rows = rail_rows(&[
            bruh("Off", BruhStatus::Off),
            bruh("Zed", BruhStatus::Asleep),
            bruh("Ann", BruhStatus::NeedsYou),
            bruh("Bo", BruhStatus::Paused),
        ]);
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["Ann", "Zed", "Bo", "Off"]);
        let mut sleeper = bruh("Zed", BruhStatus::Asleep);
        assert_eq!(row_status(&sleeper), "Asleep");
        sleeper.next_wake_ms = Some(EpochMs::saturating_from_i64(1_000));
        assert!(row_status(&sleeper).starts_with("Wakes "));
    }

    #[test]
    fn the_unread_pill_caps_at_ninety_nine() {
        assert_eq!(unread_text(3), "3");
        assert_eq!(unread_text(99), "99");
        assert_eq!(unread_text(100), "99+");
    }
}
