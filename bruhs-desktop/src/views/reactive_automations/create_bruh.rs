//! Make a new bruh: its name, job, working folder and face, optionally
//! starting from a role (web `crew/create.rs`). A role is copied once, as a
//! snapshot: later edits to the role never reach the bruh. The workspace
//! draws the dialog's shell and footer; this module owns its fields.
use super::*;
use bruhs_models::agent::DEFAULT_AGENT_MODEL;
use bruhs_models::crew::{create_command, draft_from_role, BruhDraft, BruhLook};
use bruhs_models::roles::ResolvedRole;

/// The service made the bruh the dialog asked for; open its page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReactiveBruhCreated {
    pub agent_id: String,
}

const NO_ROLE: &str = "";
const NO_ROLE_LABEL: &str = "None";
const ROLE_LABEL: &str = "Start from role";
/// Web `Textarea rows=2`: two 21px lines, `py-1.5` and the 1px ring.
const JOB_FIELD_PX: f32 = 55.0;
const LABEL_TEXT_PX: f32 = 12.0;
pub(super) const FIELD_TEXT_PX: f32 = theme::APP_INPUT_TEXT_SIZE_PX;
pub(super) const FIELD_RADIUS_PX: f32 = 8.0;
pub(super) const FIELD_X_PADDING_PX: f32 = 10.0;
const ROLE_LIST_PADDING_PX: f32 = 4.0;
const ROLE_OPTION_RADIUS_PX: f32 = 6.0;
/// Web `bg-white/5 inset-ring-white/10` on fields.
const FIELD_FILL_OPACITY: f32 = 0.05;
const FIELD_RING_OPACITY: f32 = 0.10;
const OPTION_HOVER_OPACITY: f32 = 0.60;
pub(super) const ERROR_FILL_OPACITY: f32 = 0.10;

/// The dialog's state while it is open.
pub(super) struct CreateBruh {
    name: Entity<InputState>,
    job: Entity<TextareaState>,
    folder: Entity<InputState>,
    roles: Vec<ResolvedRole>,
    role: String,
    role_open: bool,
    model: String,
    effort: Option<String>,
    pub(super) look: BruhLook,
    invalid: Option<String>,
    /// The bruh this dialog asked the service to make, until it answers.
    pending: Option<String>,
    _subscriptions: Vec<gpui::Subscription>,
}

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

pub(super) fn label(text: &'static str) -> Div {
    div()
        .text_size(px(LABEL_TEXT_PX))
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .text_color(theme::rambutan_300())
        .child(text)
}

pub(super) fn hint(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .text_size(px(LABEL_TEXT_PX))
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .text_color(theme::rambutan_500())
        .child(text.into())
}

pub(super) fn labelled(text: &'static str, control: impl IntoElement) -> Div {
    div()
        .min_w_0()
        .w_full()
        .flex()
        .flex_col()
        .gap_1()
        .child(label(text))
        .child(control)
}

pub(super) fn text_field(
    input: &Entity<InputState>,
    aria: &'static str,
    disabled: bool,
) -> impl IntoElement {
    Styled::h(
        Input::new(input).aria_label(aria).disabled(disabled),
        px(theme::CONTROL_PX),
    )
    .px(px(FIELD_X_PADDING_PX))
    .font_family(theme::CODE_FONT_FAMILY)
    .text_size(px(FIELD_TEXT_PX))
}

impl ReactiveAutomations {
    /// Open the dialog on a blank bruh. `roles` are the ones it can start
    /// from.
    pub(crate) fn open_create_bruh(
        &mut self,
        roles: Vec<ResolvedRole>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<'_, Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let name = input("", window, cx);
        let job = cx.new(|cx| TextareaState::new(window, cx).placeholder("What this bruh does"));
        let folder = input("Optional", window, cx);
        // Enter in a one-line field creates, like the web form's submit.
        let submit = |this: &mut Self,
                      _: &Entity<InputState>,
                      event: &InputEvent,
                      _: &mut Window,
                      cx: &mut Context<'_, Self>| {
            match event {
                InputEvent::PressEnter {
                    secondary: false, ..
                } => this.submit_create_bruh(cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(&name, window, submit),
            cx.subscribe_in(&folder, window, submit),
        ];
        self.create = Some(CreateBruh {
            name,
            job,
            folder,
            roles,
            role: NO_ROLE.to_string(),
            role_open: false,
            model: DEFAULT_AGENT_MODEL.to_string(),
            effort: None,
            look: BruhLook::default(),
            invalid: None,
            pending: None,
            _subscriptions: subscriptions,
        });
        self.clear_create_error();
        cx.notify();
    }

    pub(crate) fn create_bruh_open(&self) -> bool {
        self.create.is_some()
    }

    /// Close the dialog. A bruh it asked for still gets made, but nothing
    /// opens when the service answers.
    pub(crate) fn close_create_bruh(&mut self, cx: &mut Context<'_, Self>) {
        if self.create.take().is_some() {
            self.clear_create_error();
            cx.notify();
        }
    }

    /// Create is off while a bruh is on its way or the service is older.
    pub(crate) fn create_bruh_disabled(&self) -> bool {
        self.busy
            || !self.crew_ready()
            || self
                .create
                .as_ref()
                .is_none_or(|create| create.pending.is_some())
    }

    pub(crate) fn submit_create_bruh(&mut self, cx: &mut Context<'_, Self>) {
        if self.create_bruh_disabled() {
            return;
        }
        let Some(create) = self.create.as_mut() else {
            return;
        };
        let draft = BruhDraft {
            name: create.name.read(cx).value().trim().to_string(),
            instructions: create.job.read(cx).value().trim().to_string(),
            model: create.model.clone(),
            effort: create.effort.clone(),
        };
        if draft.name.is_empty() {
            create.invalid = Some("Give the bruh a name.".into());
            cx.notify();
            return;
        }
        create.invalid = None;
        let id = uuid::Uuid::new_v4().to_string();
        let folder = folder_choice(&create.folder.read(cx).value());
        let command = create_command(id.clone(), &draft, create.look.clone(), folder);
        create.pending = Some(id);
        self.send_in(command, Scope::CreateBruh, cx);
        cx.notify();
    }

    /// The service made a bruh: open it if this dialog is still waiting.
    pub(super) fn created_bruh(&mut self, agent_id: String, cx: &mut Context<'_, Self>) {
        if self
            .create
            .as_ref()
            .is_some_and(|create| create.pending.as_deref() == Some(agent_id.as_str()))
        {
            self.create = None;
            cx.emit(ReactiveBruhCreated { agent_id });
        }
    }

    /// The service refused: keep the dialog open with its reason.
    pub(super) fn create_failed(&mut self) {
        if let Some(create) = self.create.as_mut() {
            create.pending = None;
        }
    }

    fn clear_create_error(&mut self) {
        if self
            .scoped_error
            .as_ref()
            .is_some_and(|(scope, _)| *scope == Scope::CreateBruh)
        {
            self.scoped_error = None;
        }
    }

    fn choose_role(&mut self, id: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(create) = self.create.as_mut() else {
            return;
        };
        match role_draft(&create.roles, id) {
            Some(draft) => {
                create
                    .name
                    .update(cx, |input, cx| input.set_value(&draft.name, window, cx));
                create.job.update(cx, |input, cx| {
                    input.set_value(&draft.instructions, window, cx)
                });
                create.model = draft.model;
                create.effort = draft.effort;
            }
            None => {
                create.model = DEFAULT_AGENT_MODEL.to_string();
                create.effort = None;
            }
        }
        create.role = id.to_string();
        create.role_open = false;
        cx.notify();
    }

    /// The role picker: a trigger and, while open, the roles below it.
    fn render_role_picker(&self, create: &CreateBruh, cx: &Context<'_, Self>) -> Div {
        let current = create
            .roles
            .iter()
            .find(|role| role.id == create.role)
            .map_or(NO_ROLE_LABEL.to_string(), |role| role.name.clone());
        let open = create.role_open;
        let trigger = div()
            .id("desktop-create-bruh-role")
            .debug_selector(|| "desktop-create-bruh-role".to_string())
            .role(Role::ComboBox)
            .aria_label(ROLE_LABEL)
            .aria_expanded(open)
            .tab_index(0)
            .h(px(theme::CONTROL_PX))
            .w_full()
            .px(px(FIELD_X_PADDING_PX))
            .rounded(px(FIELD_RADIUS_PX))
            .bg(gpui::white().opacity(FIELD_FILL_OPACITY))
            .border_1()
            .border_color(gpui::white().opacity(FIELD_RING_OPACITY))
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
            .text_color(theme::rambutan_100())
            .child(div().flex_1().min_w_0().truncate().child(current))
            .child(
                svg()
                    .path(crate::assets::CHEVRON_DOWN_ICON_PATH)
                    .size(px(theme::ICON_PX))
                    .text_color(theme::rambutan_400()),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(create) = this.create.as_mut() {
                    create.role_open = !create.role_open;
                }
                cx.notify();
            }));
        let options = std::iter::once((NO_ROLE.to_string(), NO_ROLE_LABEL.to_string())).chain(
            create
                .roles
                .iter()
                .map(|role| (role.id.clone(), role.name.clone())),
        );
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_1()
            .child(trigger)
            .when(open, |el| {
                el.child(
                    div()
                        .id("desktop-create-bruh-role-list")
                        .debug_selector(|| "desktop-create-bruh-role-list".to_string())
                        .role(Role::ListBox)
                        .aria_label(ROLE_LABEL)
                        .p(px(ROLE_LIST_PADDING_PX))
                        .rounded(px(FIELD_RADIUS_PX))
                        .bg(theme::longan_800())
                        .border_1()
                        .border_color(theme::ring_modal())
                        .flex()
                        .flex_col()
                        .children(options.enumerate().map(|(index, (id, name))| {
                            let chosen = id == create.role;
                            div()
                                .id(("desktop-create-bruh-role-option", index))
                                .debug_selector({
                                    let name = name.clone();
                                    move || format!("desktop-create-bruh-role-option-{name}")
                                })
                                .role(Role::ListBoxOption)
                                .aria_label(name.clone())
                                .aria_selected(chosen)
                                .tab_index(0)
                                .min_h(px(theme::CONTROL_PX))
                                .px_2()
                                .rounded(px(ROLE_OPTION_RADIUS_PX))
                                .flex()
                                .items_center()
                                .cursor_pointer()
                                .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
                                .text_color(if chosen {
                                    theme::rambutan_100()
                                } else {
                                    theme::rambutan_300()
                                })
                                .when(chosen, |el| el.bg(theme::longan_700()))
                                .hover(|el| {
                                    el.bg(theme::longan_700().opacity(OPTION_HOVER_OPACITY))
                                })
                                .border_1()
                                .border_color(gpui::transparent_black())
                                .focus_visible(|el| el.border_color(theme::persimmon_400()))
                                .child(name)
                                .on_activate(cx, move |this, window, cx| {
                                    this.choose_role(&id, window, cx)
                                })
                        })),
                )
            })
    }

    /// The dialog's fields, in the web's order.
    pub(crate) fn render_create_bruh(&self, cx: &Context<'_, Self>) -> Option<Div> {
        let create = self.create.as_ref()?;
        let disabled = create.pending.is_some();
        let folder_blank = folder_choice(&create.folder.read(cx).value()).is_none();
        let error = create.invalid.clone().or_else(|| {
            self.scoped_error
                .as_ref()
                .filter(|(scope, _)| *scope == Scope::CreateBruh)
                .map(|(_, message)| message.clone())
        });
        let reason = self
            .crew
            .protocol
            .filter(|_| !self.crew_ready())
            .and_then(|protocol| service_notice(protocol, None))
            .map(ServiceNotice::message);
        Some(
            div()
                .debug_selector(|| "desktop-create-bruh".to_string())
                .w_full()
                .min_w_0()
                .px_4()
                .py_3()
                .flex()
                .flex_col()
                .gap_3()
                .child(labelled(
                    "Name",
                    div()
                        .debug_selector(|| "desktop-create-bruh-name".to_string())
                        .child(text_field(&create.name, "Name", disabled)),
                ))
                .child(labelled(
                    "Job",
                    Textarea::new(&create.job)
                        .aria_label("Job")
                        .disabled(disabled)
                        .h(px(JOB_FIELD_PX))
                        .px(px(FIELD_X_PADDING_PX))
                        .font_family(theme::CODE_FONT_FAMILY)
                        .text_size(px(FIELD_TEXT_PX)),
                ))
                .child(
                    labelled(
                        "Working folder",
                        text_field(&create.folder, "Working folder", disabled),
                    )
                    .when(folder_blank, |el| {
                        el.child(hint("Bruhs makes a private folder."))
                    }),
                )
                .child(
                    labelled(ROLE_LABEL, self.render_role_picker(create, cx)).child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_x_3()
                            .child(hint(format!("Model: {}", create.model)))
                            .when_some(create.effort.as_ref(), |el, effort| {
                                el.child(hint(format!("Effort: {effort}")))
                            }),
                    ),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(label("Face"))
                        .child(self.render_face_editor(&create.look, cx)),
                )
                .when_some(error, |el, message| {
                    el.child(
                        div()
                            .id("desktop-create-bruh-error")
                            .debug_selector(|| "desktop-create-bruh-error".to_string())
                            .role(Role::Alert)
                            .p_3()
                            .rounded(px(FIELD_RADIUS_PX))
                            .bg(theme::lychee_500().opacity(ERROR_FILL_OPACITY))
                            .text_size(px(theme::APP_BODY_TEXT_SIZE_PX))
                            .text_color(theme::lychee_200())
                            .child(message),
                    )
                })
                .when_some(reason, |el, reason| {
                    el.child(hint(reason).text_color(theme::rambutan_400()))
                }),
        )
    }
}

/// Draws the open dialog's fields by themselves; the workspace otherwise
/// puts them in its modal.
#[cfg(test)]
pub(super) struct CreateBruhHost(pub(super) Entity<ReactiveAutomations>);

#[cfg(test)]
impl Render for CreateBruhHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let body = self.0.update(cx, |panel, cx| panel.render_create_bruh(cx));
        div().children(body)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        folder_choice, role_draft, CreateBruhHost, ReactiveAutomations, ReactiveBruhCreated,
        DEFAULT_AGENT_MODEL, NO_ROLE,
    };
    use crate::theme;
    use bruhs_models::crew::{BruhDraft, CREW_PROTOCOL};
    use bruhs_models::reactive::ReactiveCommand;
    use bruhs_models::roles::ResolvedRole;
    use gpui::{AppContext as _, Entity, VisualTestContext};
    use std::{cell::RefCell, rc::Rc};

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

    #[test]
    fn a_blank_folder_lets_the_server_make_one() {
        assert_eq!(folder_choice("  "), None);
        assert_eq!(folder_choice(""), None);
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
    }

    fn open_panel(
        cx: &mut gpui::TestAppContext,
    ) -> (
        Entity<ReactiveAutomations>,
        &mut VisualTestContext,
        Rc<RefCell<Vec<ReactiveBruhCreated>>>,
    ) {
        cx.update(|cx| {
            gpui_component::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = Rc::new(RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            let host = cx.new(|_| CreateBruhHost(panel));
            gpui_component::Root::new(host, window, cx)
        });
        let panel = holder.borrow().clone().unwrap();
        let created = Rc::new(RefCell::new(Vec::new()));
        let seen = created.clone();
        cx.update(|_, cx| {
            cx.subscribe(&panel, move |_, event: &ReactiveBruhCreated, _| {
                seen.borrow_mut().push(event.clone());
            })
            .detach();
        });
        panel.update_in(cx, |panel, window, cx| {
            panel.show_crew_for_test(Vec::new(), CREW_PROTOCOL, cx);
            panel.open_create_bruh(vec![role("a", Some("gpt-5.1-codex"))], window, cx);
        });
        (panel, cx, created)
    }

    fn created_id(panel: &ReactiveAutomations) -> String {
        match panel.sent_for_test().last() {
            Some(ReactiveCommand::CreateBruh { agent, .. }) => agent.id.clone(),
            other => panic!("expected CreateBruh, got {other:?}"),
        }
    }

    #[gpui::test]
    fn enter_in_the_name_creates_and_the_answer_opens_the_bruh(cx: &mut gpui::TestAppContext) {
        let (panel, cx, created) = open_panel(cx);
        panel.update_in(cx, |panel, window, cx| {
            let name = panel.create.as_ref().unwrap().name.clone();
            name.update(cx, |input, cx| input.set_value("Ada", window, cx));
            // What the field emits for Enter. (The test platform would then
            // type the newline into a one-line field, which a real one never
            // receives.)
            name.update(cx, |_, cx| {
                cx.emit(gpui_component::input::InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                });
            });
        });
        cx.run_until_parked();
        let id = panel.update(cx, |panel, _| {
            assert_eq!(panel.sent_for_test().len(), 1, "one CreateBruh");
            created_id(panel)
        });
        panel.update(cx, |panel, cx| panel.created_bruh(id.clone(), cx));
        cx.run_until_parked();
        assert_eq!(
            created.borrow().as_slice(),
            [ReactiveBruhCreated { agent_id: id }]
        );
        panel.update(cx, |panel, _| assert!(!panel.create_bruh_open()));
    }

    #[gpui::test]
    fn a_nameless_bruh_is_never_sent(cx: &mut gpui::TestAppContext) {
        let (panel, cx, _) = open_panel(cx);
        panel.update(cx, |panel, cx| {
            panel.submit_create_bruh(cx);
            assert!(panel.sent_for_test().is_empty());
            assert_eq!(
                panel.create.as_ref().unwrap().invalid.as_deref(),
                Some("Give the bruh a name.")
            );
        });
    }

    #[gpui::test]
    fn cancel_while_pending_never_opens_the_bruh_later(cx: &mut gpui::TestAppContext) {
        let (panel, cx, created) = open_panel(cx);
        panel.update_in(cx, |panel, window, cx| {
            let name = panel.create.as_ref().unwrap().name.clone();
            name.update(cx, |input, cx| input.set_value("Ada", window, cx));
            panel.submit_create_bruh(cx);
            assert!(panel.create_bruh_disabled(), "one bruh at a time");
            let id = created_id(panel);
            panel.close_create_bruh(cx);
            panel.created_bruh(id, cx);
        });
        cx.run_until_parked();
        assert!(created.borrow().is_empty());
    }

    #[gpui::test]
    fn a_role_fills_name_job_model_and_effort(cx: &mut gpui::TestAppContext) {
        let (panel, cx, _) = open_panel(cx);
        panel.update_in(cx, |panel, window, cx| {
            panel.choose_role("a", window, cx);
            let create = panel.create.as_ref().unwrap();
            assert_eq!(create.name.read(cx).value().as_ref(), "Role a");
            assert_eq!(create.job.read(cx).value().as_ref(), "Do a work.");
            assert_eq!(create.model, "gpt-5.1-codex");
            assert_eq!(create.effort.as_deref(), Some("high"));
            panel.choose_role(NO_ROLE, window, cx);
            let create = panel.create.as_ref().unwrap();
            assert_eq!(create.model, DEFAULT_AGENT_MODEL);
            assert_eq!(create.effort, None);
        });
    }

    #[gpui::test]
    fn an_older_service_keeps_create_off(cx: &mut gpui::TestAppContext) {
        let (panel, cx, _) = open_panel(cx);
        panel.update_in(cx, |panel, window, cx| {
            panel.show_crew_for_test(Vec::new(), 0, cx);
            let name = panel.create.as_ref().unwrap().name.clone();
            name.update(cx, |input, cx| input.set_value("Ada", window, cx));
            assert!(panel.create_bruh_disabled());
            panel.submit_create_bruh(cx);
            assert!(panel.sent_for_test().is_empty());
        });
    }
}
