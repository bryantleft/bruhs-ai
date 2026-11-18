//! Choose how a bruh looks (web `crew/face_editor.rs`): which way it faces,
//! its face and mouth colours, with a live preview and a still preview of
//! every state. The create dialog and a bruh's settings share it.
use super::*;
use crate::views::braille::braille_face;
use bruhs_models::crew::{
    face_view, BruhColor, BruhLook, BruhStatus, FaceView, Look, MouthColor, EDITOR_FACE_PX,
    STATE_PREVIEW_PX,
};

/// Every state in the order the previews show them.
const STATES: [BruhStatus; 5] = [
    BruhStatus::Asleep,
    BruhStatus::Working,
    BruhStatus::NeedsYou,
    BruhStatus::Paused,
    BruhStatus::Off,
];
const LABEL_TEXT_PX: f32 = 12.0;
const SEGMENT_RADIUS_PX: f32 = 4.0;
const SEGMENT_GROUP_RADIUS_PX: f32 = 8.0;
/// Web `p-0.5` around the segments.
const SEGMENT_GROUP_PADDING_PX: f32 = 2.0;
const EDITOR_GAP_PX: f32 = 16.0;
const SWATCH_GAP_PX: f32 = 8.0;
/// Web `ring-2` on the chosen swatch.
const SWATCH_CHOSEN_RING_PX: f32 = 2.0;

/// One row of colour swatches: its group name, each swatch's selector
/// prefix (`{prefix}-{label}`), and how a choice reads and applies.
struct SwatchRow<T: 'static> {
    group: &'static str,
    prefix: &'static str,
    options: &'static [T],
    name: fn(T) -> &'static str,
    rgb: fn(T) -> u32,
    choose: fn(&mut BruhLook, T),
}

const FACE_SWATCHES: SwatchRow<BruhColor> = SwatchRow {
    group: "Face colour",
    prefix: "desktop-bruh-face-swatch",
    options: &BruhColor::ALL,
    name: BruhColor::label,
    rgb: BruhColor::rgb,
    choose: |look, face| look.face = face,
};

const MOUTH_SWATCHES: SwatchRow<MouthColor> = SwatchRow {
    group: "Mouth colour",
    prefix: "desktop-bruh-mouth-swatch",
    options: &MouthColor::ALL,
    name: MouthColor::label,
    rgb: MouthColor::rgb,
    choose: |look, mouth| look.mouth = mouth,
};

/// A face drawn still: the look as chosen, or the state's expression without
/// its motion.
pub(super) fn still_view(look: &BruhLook, status: BruhStatus) -> FaceView {
    face_view(look, status, 0, true)
}

fn small_label(text: &'static str) -> Div {
    div()
        .text_size(px(LABEL_TEXT_PX))
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .text_color(theme::rambutan_300())
        .child(text)
}

impl ReactiveAutomations {
    /// Change the look of the editor that is open: the create dialog's,
    /// which sits above everything, else the bruh page's settings.
    fn edit_look(&mut self, change: impl FnOnce(&mut BruhLook), cx: &mut Context<'_, Self>) {
        let look = match self.create.as_mut() {
            Some(create) => Some(&mut create.look),
            None => self.bruh_settings_look_mut(),
        };
        if let Some(look) = look {
            change(look);
            cx.notify();
        }
    }

    /// One swatch row, a radio group.
    fn swatches<T: Copy + PartialEq + 'static>(
        &self,
        row: SwatchRow<T>,
        selected: T,
        cx: &Context<'_, Self>,
    ) -> Div {
        let SwatchRow {
            group,
            prefix,
            options,
            name,
            rgb,
            choose,
        } = row;
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(small_label(group))
            .child(
                div()
                    .id(group)
                    .role(Role::RadioGroup)
                    .aria_label(group)
                    .min_w_0()
                    .flex()
                    .flex_wrap()
                    .gap(px(SWATCH_GAP_PX))
                    .children(options.iter().copied().map(|option| {
                        let label = name(option);
                        let chosen = option == selected;
                        div()
                            .id(SharedString::from(format!("{prefix}-{label}")))
                            .debug_selector(move || format!("{prefix}-{label}"))
                            .role(Role::RadioButton)
                            .aria_label(label)
                            .aria_toggled(chosen.into())
                            .tab_index(0)
                            .size(px(theme::CONTROL_PX))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .map(|el| {
                                if chosen {
                                    el.border(px(SWATCH_CHOSEN_RING_PX))
                                        .border_color(theme::rambutan_100())
                                } else {
                                    el.border_1()
                                        .border_color(theme::longan_700())
                                        .hover(|el| el.border_color(theme::rambutan_400()))
                                }
                            })
                            .focus_visible(|el| el.border_color(theme::persimmon_400()))
                            .child(
                                div()
                                    .size(px(theme::CONTROL_XS_PX))
                                    .rounded_full()
                                    .bg(gpui::rgb(rgb(option))),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.edit_look(|look| choose(look, option), cx);
                            }))
                    })),
            )
    }

    /// The face editor for `look`.
    pub(super) fn render_face_editor(&self, look: &BruhLook, cx: &Context<'_, Self>) -> Div {
        let looks = div()
            .id("desktop-bruh-look")
            .role(Role::RadioGroup)
            .aria_label("Look")
            .min_w_0()
            .flex()
            .gap_1()
            .p(px(SEGMENT_GROUP_PADDING_PX))
            .rounded(px(SEGMENT_GROUP_RADIUS_PX))
            .bg(theme::longan_900())
            .border_1()
            .border_color(theme::longan_800())
            .children(Look::ALL.into_iter().map(|option| {
                let chosen = look.look == option;
                let key = option.label().to_lowercase();
                div()
                    .id(SharedString::from(format!("desktop-bruh-look-{key}")))
                    .debug_selector(move || format!("desktop-bruh-look-{key}"))
                    .role(Role::RadioButton)
                    .aria_label(option.label())
                    .aria_toggled(chosen.into())
                    .tab_index(0)
                    .flex_1()
                    .min_h(px(theme::CONTROL_PX))
                    .px_3()
                    .rounded(px(SEGMENT_RADIUS_PX))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_size(px(LABEL_TEXT_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .map(|el| {
                        if chosen {
                            el.bg(theme::longan_700()).text_color(theme::rambutan_100())
                        } else {
                            el.text_color(theme::rambutan_400())
                                .hover(|el| el.text_color(theme::rambutan_200()))
                        }
                    })
                    .child(option.label())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit_look(|look| look.look = option, cx);
                    }))
            }));
        let previews = div()
            .id("desktop-bruh-state-previews")
            .role(Role::List)
            .aria_label("How it looks in each state")
            .min_w_0()
            .flex()
            .flex_wrap()
            .gap_3()
            .children(STATES.into_iter().map(|status| {
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .child(braille_face(STATE_PREVIEW_PX, &still_view(look, status)))
                    .child(
                        div()
                            .text_size(px(LABEL_TEXT_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_color(theme::rambutan_500())
                            .child(status.label()),
                    )
            }));
        div()
            .debug_selector(|| "desktop-bruh-face-editor".to_string())
            .min_w_0()
            .flex()
            .gap(px(EDITOR_GAP_PX))
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .justify_center()
                    .child(braille_face(
                        EDITOR_FACE_PX,
                        &still_view(look, BruhStatus::Working),
                    )),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(small_label("Look"))
                            .child(looks),
                    )
                    .child(self.swatches(FACE_SWATCHES, look.face, cx))
                    .child(self.swatches(MOUTH_SWATCHES, look.mouth, cx))
                    .child(previews),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::super::create_bruh::CreateBruhHost;
    use super::{still_view, ReactiveAutomations, STATES};
    use crate::theme;
    use bruhs_models::crew::{BruhColor, BruhLook, BruhStatus, Eyes, Look, MouthColor};
    use gpui::AppContext as _;

    #[test]
    fn the_preview_shows_the_chosen_look_awake_and_still() {
        let look = BruhLook {
            look: Look::Right,
            face: BruhColor::Pandan,
            mouth: MouthColor::Longan,
        };
        let view = still_view(&look, BruhStatus::Working);
        assert!(!view.animates);
        assert_eq!(view.style.look_x, Look::Right.look_x());
        assert_eq!(view.style.face_rgb, BruhColor::Pandan.rgb());
        assert_eq!(view.style.eyes, Eyes::Open);
        assert!(STATES
            .iter()
            .all(|status| !still_view(&look, *status).animates));
    }

    /// Clicking a look, a face swatch and a mouth swatch changes the
    /// dialog's look.
    #[gpui::test]
    fn the_editor_changes_the_look(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = std::rc::Rc::new(std::cell::RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            let host = cx.new(|_| CreateBruhHost(panel));
            bruhs_ui::Root::new(host, window, cx)
        });
        let panel = holder.borrow().clone().unwrap();
        panel.update_in(cx, |panel, window, cx| {
            panel.open_create_bruh(Vec::new(), window, cx);
        });
        cx.run_until_parked();
        for selector in [
            "desktop-bruh-look-right",
            "desktop-bruh-face-swatch-Pandan",
            "desktop-bruh-mouth-swatch-Longan",
        ] {
            let bounds = cx.debug_bounds(selector).expect(selector);
            cx.simulate_click(bounds.center(), gpui::Modifiers::default());
            cx.run_until_parked();
        }
        panel.update(cx, |panel, _| {
            assert_eq!(
                panel.create.as_ref().map(|create| create.look.clone()),
                Some(BruhLook {
                    look: Look::Right,
                    face: BruhColor::Pandan,
                    mouth: MouthColor::Longan,
                })
            );
        });
    }
}
