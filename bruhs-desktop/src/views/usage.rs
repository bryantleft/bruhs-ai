//! The native Usage window, shared by the desktop entry points.
use super::web_button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use crate::{app_state::DesktopState, server_client::ServerClient, theme};
use bruhs_models::{
    agent::AgentProvider,
    agent_extensions::{QuotaWindow, SharedUsageHistory, UsageHistoryPeriod, UsageLimitSnapshot},
};
use bruhs_ui::dropdown::{
    Dropdown, DropdownGroup, DropdownOption, DropdownTriggerVariant, Listbox,
};
use bruhs_ui::{press::on_press_view, zoom::px};
use gpui::{prelude::*, *};
use std::rc::Rc;

mod history;
mod limits;
mod pools;
mod pricing;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum View {
    Cost,
    Tokens,
    #[default]
    Limits,
}

impl View {
    fn label(self) -> &'static str {
        match self {
            Self::Cost => "Cost",
            Self::Tokens => "Tokens",
            Self::Limits => "Limits",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    Environment,
    Provider,
}

impl Filter {
    fn id(self) -> &'static str {
        match self {
            Self::Environment => "usage-environment",
            Self::Provider => "usage-provider",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Environment => "All environments",
            Self::Provider => "All providers",
        }
    }
}

const ALL: &str = "all";
const LOCAL: &str = "This computer";
const MODEL_PRICES: &str = "usage-model-prices";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Period {
    Last24h,
    Days(u16),
}

pub(super) enum UsageShortcut {
    View(View),
    Period(Period),
}

impl UsageShortcut {
    pub(super) fn from_command(command: &str) -> Option<Self> {
        Some(match command {
            "usage.cost" => Self::View(View::Cost),
            "usage.tokens" => Self::View(View::Tokens),
            "usage.limits" => Self::View(View::Limits),
            "usage.period.day" => Self::Period(Period::Last24h),
            "usage.period.week" => Self::Period(Period::Days(7)),
            "usage.period.month" => Self::Period(Period::Days(30)),
            "usage.period.quarter" => Self::Period(Period::Days(90)),
            _ => return None,
        })
    }
}

#[derive(Default)]
struct HistoryData {
    report: SharedUsageHistory,
    local_unpriced_models: std::collections::BTreeSet<String>,
    imported_records: usize,
    environments: Vec<String>,
    notices: Vec<String>,
    has_source: bool,
    last_24h_ending_at: Option<i64>,
}

impl HistoryData {
    async fn load(environment: Option<String>, period: UsageHistoryPeriod) -> Self {
        use bruhs_core::{
            agent_extensions::{directory, snapshot_for_period_at, UsageHistoryRange},
            delivery::writer::desktop_state_file,
            usage_hubs::linked::history_reports,
        };

        let selected = environment.as_deref();
        let local = async {
            if selected.is_none_or(|name| name == LOCAL) {
                Some(
                    tokio::task::spawn_blocking(move || {
                        let range = UsageHistoryRange::from_period(period).ok_or_else(|| {
                            std::io::Error::other("Usage history period is invalid.")
                        })?;
                        snapshot_for_period_at(&directory()?, range, period.provider)
                    })
                    .await
                    .map_err(std::io::Error::other)
                    .and_then(|result| result),
                )
            } else {
                None
            }
        };
        let linked = async {
            history_reports(desktop_state_file(), selected, period, async {
                ServerClient::workbench_client()
                    .remote_hosts(None)
                    .await
                    .ok()
            })
            .await
        };
        let (local, linked) = tokio::join!(local, linked);
        let mut data = Self {
            last_24h_ending_at: period.last_24h_ending_at,
            ..Self::default()
        };
        match local {
            Some(Ok(snapshot)) => {
                data.has_source = true;
                data.imported_records = snapshot.imported.records;
                data.local_unpriced_models.extend(
                    snapshot
                        .by_model
                        .iter()
                        .filter(|(_, usage)| usage.unpriced_turns > 0)
                        .map(|(model, _)| model.clone()),
                );
                data.report = snapshot.into();
            }
            Some(Err(error)) => data.notices.push(format!("{LOCAL} · {error}")),
            None => {}
        }
        match linked {
            Ok(linked) => {
                data.environments = linked.environments;
                if let Some(name) =
                    selected.filter(|name| *name != LOCAL && linked.reports.is_empty())
                {
                    data.notices.push(format!(
                        "{name} · Cost and token history isn’t shared with this computer."
                    ));
                }
                for (name, report) in linked.reports {
                    match report {
                        Ok(report) => {
                            data.has_source = true;
                            data.report.merge(report);
                        }
                        Err(error) => data.notices.push(format!("{name} · {error}")),
                    }
                }
            }
            Err(_) => data
                .notices
                .push("Could not read linked environments for usage history.".into()),
        }
        data.environments.sort();
        data.environments.dedup();
        data.environments.retain(|name| name != LOCAL);
        data.environments.insert(0, LOCAL.into());
        data
    }
}

struct Report<T> {
    data: Option<T>,
    error: Option<String>,
    loading: bool,
}
impl<T> Default for Report<T> {
    fn default() -> Self {
        Self {
            data: None,
            error: None,
            loading: false,
        }
    }
}
impl<T> Report<T> {
    fn finish(&mut self, result: Result<T, String>) {
        self.loading = false;
        match result {
            Ok(data) => {
                self.data = Some(data);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn notice(&self) -> Option<&str> {
        if self.loading {
            Some("Refreshing…")
        } else {
            self.error.as_deref()
        }
    }
}

#[derive(Clone, PartialEq)]
struct Selection {
    provider: String,
    row: usize,
    account: usize,
}

#[derive(Default)]
struct Credit {
    confirming: bool,
    pending: bool,
    message: Option<String>,
}

pub(super) struct UsageWindow {
    scroll: ScrollHandle,
    view: View,
    breakdown: history::Breakdown,
    period: Period,
    environment: Option<String>,
    history_environment: Option<String>,
    provider: Option<String>,
    environments: Vec<String>,
    history_environments: Vec<String>,
    providers: Vec<String>,
    open_filter: Option<Filter>,
    limit_revision: u64,
    history_revision: u64,
    limits: Report<UsageLimitSnapshot>,
    recorded: Report<Vec<QuotaWindow>>,
    history: Report<HistoryData>,
    history_total: bruhs_ui::animated_number::AnimatedNumber,
    selection: Option<Selection>,
    reveal_email: bool,
    credit: Credit,
    show_recorded: bool,
    prices: Option<pricing::PricePage>,
    usage_focus: FocusHandle,
}

pub(super) struct CloseUsage;
impl EventEmitter<CloseUsage> for UsageWindow {}

impl UsageWindow {
    pub(super) fn new(cx: &mut Context<'_, Self>) -> Self {
        let mut view = Self {
            scroll: ScrollHandle::new(),
            view: View::Limits,
            breakdown: history::Breakdown::default(),
            period: Period::Days(30),
            environment: None,
            history_environment: None,
            provider: None,
            environments: vec![LOCAL.into()],
            history_environments: vec![LOCAL.into()],
            providers: Vec::new(),
            open_filter: None,
            limit_revision: 0,
            history_revision: 0,
            limits: Report::default(),
            recorded: Report::default(),
            history: Report::default(),
            history_total: Default::default(),
            selection: None,
            reveal_email: false,
            credit: Credit::default(),
            show_recorded: false,
            prices: None,
            usage_focus: cx.focus_handle(),
        };
        view.refresh_limits(cx);
        view
    }

    pub(super) fn reopen(&mut self, cx: &mut Context<'_, Self>) {
        self.selection = None;
        self.open_filter = None;
        self.prices = None;
        self.reveal_email = false;
        self.credit.confirming = false;
        if self.view == View::Limits {
            self.refresh_limits(cx);
        } else {
            self.refresh_history(cx);
        }
    }

    pub(super) fn escape(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.credit.pending {
            return;
        }
        if self.prices.is_some() {
            self.back_from_model_prices(window, cx);
            return;
        }
        if self.open_filter.take().is_some() {
            cx.notify();
        } else if self.view == View::Limits && self.selection.take().is_some() {
            self.credit = Credit::default();
            cx.notify();
        } else {
            cx.emit(CloseUsage);
        }
    }

    pub(super) fn can_close(&self) -> bool {
        !self.credit.pending && self.prices.as_ref().is_none_or(|page| !page.saving())
    }

    fn refresh_limits(&mut self, cx: &mut Context<'_, Self>) {
        if self.limits.loading || self.credit.pending {
            return;
        }
        let Some(runtime) = cx
            .try_global::<DesktopState>()
            .map(|state| state.runtime.clone())
        else {
            self.limits.error = Some("The desktop usage service is unavailable.".into());
            cx.notify();
            return;
        };
        self.limits.loading = true;
        let revision = self.limit_revision;
        let environment = self.environment.clone();
        let provider = self.provider.clone();
        let request = runtime.spawn(async move {
            ServerClient::workbench_client()
                .usage_limits(environment.as_deref(), provider.as_deref())
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = match request.await {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                if this.limit_revision != revision {
                    return;
                }
                if let Ok(snapshot) = &result {
                    if !snapshot.environments.is_empty() {
                        this.environments.clone_from(&snapshot.environments);
                    }
                    if !snapshot.providers.is_empty() {
                        this.providers.clone_from(&snapshot.providers);
                    }
                    this.selection = None;
                    this.reveal_email = false;
                    this.credit.confirming = false;
                }
                this.limits.finish(result);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn refresh_history(&mut self, cx: &mut Context<'_, Self>) {
        if self.history.loading {
            return;
        }
        let Some(runtime) = cx
            .try_global::<DesktopState>()
            .map(|state| state.runtime.clone())
        else {
            self.history.error = Some("The desktop usage service is unavailable.".into());
            cx.notify();
            return;
        };
        self.history.loading = true;
        let revision = self.history_revision;
        let provider = self.provider.as_deref().and_then(|provider| {
            AgentProvider::ALL.into_iter().find(|choice| {
                choice.as_str().eq_ignore_ascii_case(provider)
                    || choice.label().eq_ignore_ascii_case(provider)
            })
        });
        let (days, last_24h_ending_at) = match self.period {
            Period::Last24h => (None, Some(chrono::Utc::now().timestamp())),
            Period::Days(days) => (Some(days), None),
        };
        let period = UsageHistoryPeriod {
            days,
            last_24h_ending_at,
            provider,
        };
        let request = runtime.spawn(HistoryData::load(self.history_environment.clone(), period));
        cx.spawn(async move |this, cx| {
            let result = request.await.map_err(|error| error.to_string());
            let _ = this.update(cx, |this, cx| {
                if this.history_revision == revision {
                    if let Ok(data) = &result {
                        this.history_environments.clone_from(&data.environments);
                    }
                    this.history.finish(result);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn switch_view(&mut self, view: View, cx: &mut Context<'_, Self>) {
        if self.view == view {
            return;
        }
        self.view = view;
        self.history_total = Default::default();
        self.open_filter = None;
        self.scroll.set_offset(Point::default());
        if view != View::Limits && self.history.data.is_none() {
            self.refresh_history(cx);
        } else if view == View::Limits && self.limits.data.is_none() {
            self.refresh_limits(cx);
        }
        cx.notify();
    }

    fn set_period(&mut self, period: Period, cx: &mut Context<'_, Self>) {
        if self.period != period && self.view != View::Limits {
            self.period = period;
            self.reset_history();
            self.refresh_history(cx);
        }
    }

    pub(super) fn apply_shortcut(
        &mut self,
        shortcut: UsageShortcut,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if self.open_filter.is_some()
            || self.prices.is_some()
            || self.credit.confirming
            || self.credit.pending
        {
            return false;
        }
        match shortcut {
            UsageShortcut::View(view) => self.switch_view(view, cx),
            UsageShortcut::Period(period) => self.set_period(period, cx),
        }
        true
    }

    pub(super) fn select_environment(&mut self, environment: String, cx: &mut Context<'_, Self>) {
        self.change_filter(Filter::Environment, Some(environment), cx);
    }

    fn change_filter(&mut self, filter: Filter, value: Option<String>, cx: &mut Context<'_, Self>) {
        self.open_filter = None;
        let field = match filter {
            Filter::Environment if self.view != View::Limits => &mut self.history_environment,
            Filter::Environment => &mut self.environment,
            Filter::Provider => &mut self.provider,
        };
        if *field != value && !self.credit.pending {
            *field = value;
            if self.view == View::Limits || filter == Filter::Provider {
                self.limit_revision = self.limit_revision.wrapping_add(1);
                self.limits = Report::default();
                self.selection = None;
                self.reveal_email = false;
                self.credit = Credit::default();
            }
            if self.view != View::Limits || filter == Filter::Provider {
                self.reset_history();
            }
            if self.view == View::Limits {
                self.refresh_limits(cx);
            } else {
                self.refresh_history(cx);
            }
        }
        cx.notify();
    }

    fn reset_history(&mut self) {
        self.history_revision = self.history_revision.wrapping_add(1);
        self.history = Report::default();
        self.history_total = Default::default();
    }

    fn render_filter(&self, filter: Filter, cx: &Context<'_, Self>) -> AnyElement {
        let provider_choices;
        let (selected, choices) = match filter {
            Filter::Environment if self.view != View::Limits => (
                &self.history_environment,
                self.history_environments.as_slice(),
            ),
            Filter::Environment => (&self.environment, self.environments.as_slice()),
            Filter::Provider if self.view == View::Limits => {
                (&self.provider, self.providers.as_slice())
            }
            Filter::Provider => {
                provider_choices = AgentProvider::ALL
                    .into_iter()
                    .map(|provider| {
                        if provider == AgentProvider::Claude {
                            "Claude"
                        } else {
                            provider.label()
                        }
                        .to_owned()
                    })
                    .collect::<Vec<_>>();
                (&self.provider, provider_choices.as_slice())
            }
        };
        let toggle = cx.entity().downgrade();
        let select = toggle.clone();
        let options = std::iter::once(DropdownOption {
            value: ALL.into(),
            label: filter.label().into(),
            description: None,
        })
        .chain(
            choices
                .iter()
                .chain(selected.iter().filter(|name| !choices.contains(name)))
                .map(|name| DropdownOption {
                    value: format!("{}:{name}", filter.id()).into(),
                    label: name.clone(),
                    description: None,
                }),
        )
        .chain((filter == Filter::Environment).then(|| DropdownOption {
            value: MODEL_PRICES.into(),
            label: "Model prices".into(),
            description: None,
        }))
        .collect();
        let selected_value = selected
            .as_ref()
            .map(|name| format!("{}:{name}", filter.id()));
        Dropdown::new(
            filter.id(),
            filter.label(),
            selected_value.as_deref().unwrap_or(ALL),
            vec![DropdownGroup {
                label: None,
                mark: None,
                options,
            }],
            if self.open_filter == Some(filter) {
                Listbox::Popover
            } else {
                Listbox::Closed
            },
            Rc::new(move |_, cx| {
                let _ = toggle.update(cx, |this, cx| {
                    this.open_filter = (this.open_filter != Some(filter)).then_some(filter);
                    cx.notify();
                });
            }),
            Rc::new(move |value, window, cx| {
                let value = value.to_owned();
                let _ = select.update(cx, |this, cx| {
                    if value == MODEL_PRICES {
                        this.open_model_prices(None, window, cx);
                        return;
                    }
                    let value = value.split_once(':').map(|(_, name)| name.to_owned());
                    this.change_filter(filter, value, cx);
                });
            }),
        )
        .compact()
        .trigger_variant(DropdownTriggerVariant::Toolbar)
        .into_any_element()
    }

    fn button(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        variant: WebButtonVariant,
        disabled: bool,
        cx: &Context<'_, Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<'_, Self>) + 'static,
    ) -> Stateful<Div> {
        let label = label.into();
        on_press_view(
            web_button(
                id,
                variant,
                WebButtonSize::Small,
                ButtonState::disabled_if(disabled),
            )
            .aria_label(label.clone())
            .child(label),
            cx,
            move |this, window, cx| {
                if !disabled {
                    action(this, window, cx);
                }
            },
        )
    }

    fn control_group() -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(8.))
            .bg(theme::longan_900())
    }

    fn header(&self, cx: &Context<'_, Self>) -> Div {
        // Match the compact dropdown's horizontal padding and border.
        let filter_inset = px(9.);
        let limits = self.view == View::Limits;
        let views =
            Self::control_group().children([View::Cost, View::Tokens, View::Limits].map(|view| {
                let selected = view == self.view;
                Self::button(
                    view.label(),
                    view.label(),
                    WebButtonVariant::Ghost,
                    false,
                    cx,
                    move |this, _, cx| this.switch_view(view, cx),
                )
                .min_h(px(24.))
                .px(px(11.))
                .py_0()
                .text_size(px(14.))
                .text_color(if selected {
                    theme::rambutan_100()
                } else {
                    theme::rambutan_400()
                })
                .when(selected, |el| el.bg(theme::longan_700()))
                .aria_selected(selected)
                .debug_selector(move || format!("usage-view-{}", view.label()))
            }));
        let actions = div()
            .flex()
            .flex_wrap()
            .gap_1()
            .items_center()
            .when(!limits, |el| {
                let periods = [
                    Period::Last24h,
                    Period::Days(7),
                    Period::Days(30),
                    Period::Days(90),
                ];
                el.child(
                    Self::control_group().children(periods.into_iter().enumerate().map(
                        |(index, period)| {
                            let (label, description) = match period {
                                Period::Last24h => ("Past 24h".into(), "Past 24 hours".into()),
                                Period::Days(days) => (format!("{days}d"), format!("{days} days")),
                            };
                            let variant = if self.period == period {
                                WebButtonVariant::Outline
                            } else {
                                WebButtonVariant::Ghost
                            };
                            Self::button(
                                ("usage-period", index),
                                label,
                                variant,
                                false,
                                cx,
                                move |this, _, cx| this.set_period(period, cx),
                            )
                            .aria_label(description)
                            .aria_selected(self.period == period)
                            .when(self.period == period, |el| {
                                el.debug_selector(move || format!("usage-selected-period-{index}"))
                            })
                        },
                    )),
                )
            })
            .child(views)
            .child(on_press_view(
                web_button(
                    "usage-refresh",
                    WebButtonVariant::Ghost,
                    WebButtonSize::IconSmall,
                    ButtonState::disabled_if(if limits {
                        self.limits.loading || self.credit.pending
                    } else {
                        self.history.loading
                    }),
                )
                .aria_label(if limits {
                    "Refresh limits"
                } else {
                    "Refresh usage"
                })
                .tooltip(move |window, cx| {
                    bruhs_ui::tooltip::Tooltip::new(if limits {
                        "Refresh limits"
                    } else {
                        "Refresh usage"
                    })
                    .build(window, cx)
                })
                .child(
                    svg()
                        .path(crate::assets::REFRESH_ICON_PATH)
                        .size(px(14.))
                        .text_color(theme::rambutan_400()),
                ),
                cx,
                move |this, _, cx| {
                    if limits {
                        this.refresh_limits(cx);
                    } else {
                        this.refresh_history(cx);
                    }
                },
            ))
            .child(on_press_view(
                web_button(
                    "usage-close",
                    WebButtonVariant::Ghost,
                    WebButtonSize::IconSmall,
                    ButtonState::disabled_if(self.credit.pending),
                )
                .aria_label("Close usage")
                .child(
                    svg()
                        .path(crate::assets::CLOSE_ICON_PATH)
                        .size(px(14.))
                        .text_color(theme::rambutan_400()),
                ),
                cx,
                |this, _, cx| {
                    if this.can_close() {
                        cx.emit(CloseUsage);
                    }
                },
            ));
        div()
            .flex_none()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .min_h(px(48.))
            .pl(px(20.))
            .pr(px(8.))
            .py(px(6.))
            .border_b_1()
            .border_color(theme::longan_800())
            .bg(theme::longan_900())
            .child(
                bruhs_ui::breadcrumb::Breadcrumb::new(
                    "usage-breadcrumb",
                    vec![
                        bruhs_ui::breadcrumb::BreadcrumbItem::custom(
                            div()
                                .pr(filter_inset)
                                .text_size(px(14.))
                                .text_color(theme::rambutan_400())
                                .child("Usage"),
                        ),
                        bruhs_ui::breadcrumb::BreadcrumbItem::custom(
                            div()
                                .mr(px(-2.))
                                .child(self.render_filter(Filter::Environment, cx)),
                        ),
                        bruhs_ui::breadcrumb::BreadcrumbItem::custom(
                            self.render_filter(Filter::Provider, cx),
                        ),
                    ],
                )
                .gap_px(5.0),
            )
            .child(actions)
    }
}

fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(theme::rambutan_400())
        .child(text.into())
}

fn title(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(14.))
        .line_height(px(20.))
        .font_weight(FontWeight::MEDIUM)
        .child(text.into())
}

fn figure(content: impl IntoElement) -> Div {
    div()
        .text_size(px(30.))
        .line_height(px(36.))
        .font_features(theme::tabular_nums())
        .font_weight(FontWeight::SEMIBOLD)
        .child(content)
}

impl Render for UsageWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let narrow = bruhs_ui::zoom::css_px(window.viewport_size().width) < 740.;
        if let Some(prices) = &self.prices {
            return self
                .render_model_prices(prices, window, cx)
                .into_any_element();
        }
        let view = self.view.label();
        let content = match self.view {
            View::Limits => self.render_limits(narrow, cx),
            View::Tokens | View::Cost => self.render_history(narrow, window, cx),
        };
        div()
            .id("usage-window")
            .debug_selector(move || format!("usage-mode-{view}"))
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.usage_focus)
            .text_color(theme::rambutan_100())
            .font_family(theme::UI_FONT_FAMILY)
            .child(self.header(cx))
            .child(
                div()
                    .id("usage-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        div()
                            .w_full()
                            .max_w(px(1024.))
                            .mx_auto()
                            .p(px(if narrow { 16. } else { 24. }))
                            .child(content),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
