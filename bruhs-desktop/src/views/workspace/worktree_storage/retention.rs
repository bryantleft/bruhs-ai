use super::*;
use bruhs_models::num::ToIntSaturating;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DaysRule {
    Inactive,
    Settled,
}

impl DaysRule {
    fn value(self, prefs: WorktreeRetention) -> Option<u16> {
        match self {
            Self::Inactive => prefs.inactive_days,
            Self::Settled => prefs.settled_days,
        }
    }
    fn set(self, prefs: &mut WorktreeRetention, days: Option<u16>) {
        match self {
            Self::Inactive => prefs.inactive_days = days,
            Self::Settled => prefs.settled_days = days,
        }
    }
    fn minimum(self) -> u16 {
        u16::from(self == Self::Inactive)
    }
    fn label(self) -> &'static str {
        match self {
            Self::Inactive => "Delete inactive worktrees",
            Self::Settled => "Delete settled worktrees",
        }
    }
    fn id(self) -> &'static str {
        match self {
            Self::Inactive => "settings-cleanup-inactive",
            Self::Settled => "settings-cleanup-settled",
        }
    }
}

pub(in crate::views::workspace) struct DaysEditor {
    rule: DaysRule,
    scope: String,
    field: Entity<InputState>,
    _events: Subscription,
}

impl Workspace {
    pub(super) fn retention(&self) -> WorktreeRetention {
        let project = &self.settings_project_scope;
        match self.project_cleanup_mode(project) {
            Some(CleanupMode::Off) => WorktreeRetention::default(),
            Some(CleanupMode::Custom) => self.project_defaults[project]
                .worktree_retention
                .unwrap_or(self.worktree_retention),
            None => self.worktree_retention,
        }
    }
    fn set_retention(
        &mut self,
        change: impl FnOnce(&mut WorktreeRetention),
        cx: &mut Context<'_, Self>,
    ) {
        let mut rules = self.retention();
        change(&mut rules);
        if self.settings_project_scope.is_empty() {
            self.worktree_retention = rules;
        } else if self.project_cleanup_mode(&self.settings_project_scope)
            == Some(CleanupMode::Custom)
        {
            self.project_defaults
                .entry(self.settings_project_scope.clone())
                .or_default()
                .worktree_retention = Some(rules);
        }
        self.persist_and_notify(cx);
    }
    fn edit_retention_days(&mut self, rule: DaysRule, w: &mut Window, cx: &mut Context<'_, Self>) {
        let days = rule.value(self.retention()).unwrap_or(8);
        let field = cx.new(|cx| InputState::new(w, cx).default_value(days.to_string()));
        let events = cx.subscribe(&field, move |this, field, event: &InputEvent, cx| {
            let Some(editor) = &this.retention_editor else {
                return;
            };
            if editor.field != field || editor.scope != this.settings_project_scope {
                return;
            }
            let enter = matches!(event, InputEvent::PressEnter { .. });
            if !enter && !matches!(event, InputEvent::Blur) {
                return;
            }
            let value = field
                .read(cx)
                .value()
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite());
            this.retention_editor = None;
            if let Some(value) = value {
                let days = value
                    .round()
                    .clamp(f64::from(rule.minimum()), 3650.)
                    .to_u16_saturating();
                if enter || days != rule.value(this.retention()).unwrap_or(8) {
                    this.set_retention(|r| rule.set(r, Some(days)), cx);
                }
            }
            cx.notify();
        });
        self.retention_editor = Some(DaysEditor {
            rule,
            scope: self.settings_project_scope.clone(),
            field: field.clone(),
            _events: events,
        });
        field.focus_handle(cx).focus(w, cx);
        cx.notify();
    }
    fn step_days(
        &mut self,
        rule: DaysRule,
        delta: i32,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let field = self
            .retention_editor
            .as_ref()
            .filter(|editor| editor.rule == rule && editor.scope == self.settings_project_scope)
            .map(|editor| editor.field.clone());
        let value = field
            .as_ref()
            .and_then(|field| field.read(cx).value().trim().parse().ok())
            .or(rule.value(self.retention()))
            .unwrap_or(8);
        let days = u16::try_from((i32::from(value) + delta).clamp(i32::from(rule.minimum()), 3650))
            .unwrap_or(rule.minimum());
        if let Some(field) = field {
            field.update(cx, |field, cx| field.set_value(days.to_string(), w, cx));
            if rule.value(self.retention()).is_none() {
                return;
            }
        }
        self.set_retention(|r| rule.set(r, Some(days)), cx);
    }
    fn days_value(
        &self,
        rule: DaysRule,
        value: u16,
        editor: Option<&DaysEditor>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let label = rule.label();
        let id = rule.id();
        let key_down = cx.listener(move |this, event: &KeyDownEvent, w, cx| {
            let delta = match event.keystroke.key.as_str() {
                "up" => 1,
                "down" => -1,
                _ => return,
            };
            this.step_days(rule, delta, w, cx);
            w.prevent_default();
            cx.stop_propagation();
        });
        if let Some(editor) = editor {
            return div()
                .w(px(56.))
                .debug_selector(move || format!("{id}-input"))
                .capture_key_down(key_down)
                .child(Input::new(&editor.field).aria_label(format!("{label} in days")))
                .into_any_element();
        }
        let key = format!("{id}-value");
        let value_button = web_button(
            SharedString::from(key.clone()),
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .debug_selector(move || key)
        .role(Role::SpinButton)
        .aria_label(format!("{label} in days"))
        .aria_numeric_value(f64::from(value))
        .aria_min_numeric_value(f64::from(rule.minimum()))
        .aria_max_numeric_value(3650.)
        .on_key_down(key_down)
        .child(value.to_string());
        on_press_view(value_button, cx, move |this, w, cx| {
            this.edit_retention_days(rule, w, cx)
        })
        .into_any_element()
    }
    fn days_row(&self, rule: DaysRule, cx: &Context<'_, Self>) -> AnyElement {
        let value = rule.value(self.retention());
        let editor = self
            .retention_editor
            .as_ref()
            .filter(|e| e.scope == self.settings_project_scope && e.rule == rule);
        let enabled = value.is_some() || editor.is_some();
        let label = rule.label();
        let id = rule.id();
        let mut control = div().flex().items_center().gap_2();
        if enabled {
            let step = |delta, sign: &'static str, action: &'static str| {
                let key = format!("{id}-{}", action.to_lowercase());
                let button = web_button(
                    SharedString::from(key.clone()),
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .aria_label(format!("{action} {label}"))
                .debug_selector(move || key)
                .child(sign);
                on_press_view(button, cx, move |this, w, cx| {
                    this.step_days(rule, delta, w, cx)
                })
            };
            control = control
                .child(step(-1, "−", "Decrease"))
                .child(self.days_value(rule, value.unwrap_or(8), editor, cx))
                .child(div().text_color(theme::rambutan_400()).child("days"))
                .child(step(1, "+", "Increase"));
        } else {
            control = control.child(div().text_color(theme::rambutan_400()).child("Off"));
        }
        control = control.child(settings_ui::switch(
            id,
            enabled,
            false,
            label,
            cx,
            move |this, w, cx| {
                if enabled {
                    this.retention_editor = None;
                    this.set_retention(|r| rule.set(r, None), cx);
                } else {
                    this.edit_retention_days(rule, w, cx);
                }
            },
        ));
        let (row, help) = match rule {
            DaysRule::Inactive => (
                "settings-row-cleanup-inactive",
                "Remove worktrees and scratch folders after this many days without thread activity. Thread history is kept.",
            ),
            DaysRule::Settled => (
                "settings-row-cleanup-settled",
                "Remove worktrees and scratch folders this many days after the thread settles. Set 0 for the next cleanup.",
            ),
        };
        self.settings_row(row, label, help, control.into_any_element())
    }
    pub(super) fn retention_rows(&self, advanced: bool, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let rules = self.retention();
        if advanced {
            let label = "Delete unchanged worktrees";
            vec![
                self.days_row(DaysRule::Settled, cx),
                self.settings_row(
                    "settings-row-cleanup-unchanged", label,
                    "Remove settled or archived worktrees with no commits beyond the default branch.",
                    settings_ui::switch("settings-cleanup-unchanged", rules.unchanged, false, label, cx,
                        |this, _, cx| this.set_retention(|r| r.unchanged = !r.unchanged, cx)),
                ),
            ]
        } else {
            let label = "Delete merged worktrees";
            vec![
                self.settings_row(
                    "settings-row-cleanup-merged",
                    label,
                    "Remove worktrees whose pull request has merged into the default branch.",
                    settings_ui::switch(
                        "settings-cleanup-merged",
                        rules.merged,
                        false,
                        label,
                        cx,
                        |this, _, cx| this.set_retention(|r| r.merged = !r.merged, cx),
                    ),
                ),
                self.days_row(DaysRule::Inactive, cx),
            ]
        }
    }
}
