//! Native Life context surface: manual finance, health, privacy, and evidence.

use super::*;
use bruhs_models::num::ToIntSaturating;
use bruhs_models::{
    finance_provider_adapters, health_provider_adapters, life_agent_prompt, FinanceAccount,
    FinanceBudget, FinanceTransaction, Habit, Holding, IncomeStream, LifeInsightDomain,
    ProviderStatus, RecoveryLog, SleepLog, WorkoutLog,
};

#[derive(Clone, Copy)]
enum FinanceEntryKind {
    Account,
    Spend,
    Budget,
    Income,
    Holding,
}

#[derive(Clone, Copy)]
enum HealthEntryKind {
    Workout,
    Sleep,
    Recovery,
    Habit,
}

fn parse_amount_minor(value: &str) -> Option<i64> {
    bruhs_models::parse_amount_minor(value).ok()
}

fn money(minor: i64, currency: &str) -> String {
    bruhs_models::format_money(i128::from(minor), currency)
}

fn provider_label(status: ProviderStatus) -> &'static str {
    match status {
        ProviderStatus::Local => "Local",
        ProviderStatus::Available => "Available",
        ProviderStatus::Disconnected => "Disconnected",
        ProviderStatus::Connected => "Connected",
        ProviderStatus::Error => "Needs attention",
    }
}

fn provider_color(status: ProviderStatus) -> Hsla {
    match status {
        ProviderStatus::Local | ProviderStatus::Connected => theme::pandan_300(),
        ProviderStatus::Available => theme::blueberry_400(),
        ProviderStatus::Disconnected => theme::rambutan_500(),
        ProviderStatus::Error => theme::lychee_300(),
    }
}

/// Web `text-label`, each card's title.
const CARD_TITLE_PX: f32 = 13.0;
/// Web `text-body-sm`.
const BODY_SM_PX: f32 = 13.0;
/// Web `text-label-sm`.
const LABEL_SM_PX: f32 = 12.0;
/// Web's `lg` breakpoint: from here the record cards sit two to a row.
const RECORD_GRID_TWO_COLUMNS_PX: f32 = 1024.0;
/// Web's `md` breakpoint: from here the finance sources sit three to a row.
const SOURCES_THREE_COLUMNS_PX: f32 = 768.0;
/// Transactions a card lists, newest first (web `take(8)`).
const TRANSACTION_ROWS: usize = 8;
/// Workouts a card lists, newest first (web `take(6)`).
const WORKOUT_ROWS: usize = 6;

/// Web `CARD_CLASS` with its `h2` title.
fn life_card(id: &'static str, title: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || format!("desktop-{id}"))
        .min_w_0()
        .rounded(px(6.0))
        .bg(theme::longan_900().opacity(0.65))
        .border_1()
        .border_color(theme::longan_800())
        .p_3()
        .child(card_title(title))
}

fn card_title(title: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(CARD_TITLE_PX))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::rambutan_200())
        .child(title.into())
}

/// Web `ROW`: one record on the tone below its card.
fn life_row(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .min_w_0()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_2()
        .rounded(px(4.0))
        .bg(theme::longan_950().opacity(0.45))
        .px(px(10.0))
        .py_2()
}

/// A row's name over its secondary label.
fn row_label(name: String, detail: String) -> Div {
    div()
        .min_w_0()
        .child(
            div()
                .text_size(px(BODY_SM_PX))
                .text_color(theme::rambutan_300())
                .child(name),
        )
        .child(
            div()
                .text_size(px(LABEL_SM_PX))
                .text_color(theme::rambutan_400())
                .child(detail),
        )
}

fn row_amount(amount: String) -> Div {
    div()
        .min_w_0()
        .font_family(theme::CODE_FONT_FAMILY)
        .text_size(px(BODY_SM_PX))
        .text_color(theme::rambutan_200())
        .child(amount)
}

/// The record cards' grid: two to a row on wide windows, else one.
fn record_grid(two_columns: bool, cards: Vec<AnyElement>) -> Div {
    div()
        .grid()
        .grid_cols(if two_columns { 2 } else { 1 })
        .gap_3()
        .children(cards)
}

impl Workspace {
    fn clear_life_inputs(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        for input in [
            &self.life_name_input,
            &self.life_value_input,
            &self.life_category_input,
        ] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    /// Web: a transaction row's ×.
    pub(super) fn delete_life_transaction(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let transactions = &mut self.life_context.finance.transactions;
        let before = transactions.len();
        transactions.retain(|transaction| transaction.id != id);
        if transactions.len() != before {
            self.life_context.finance.updated_at_ms = EpochMs::now().as_millis();
            self.persist_and_notify(cx);
        }
    }

    fn add_finance_entry(
        &mut self,
        kind: FinanceEntryKind,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let name = self.life_name_input.read(cx).value().trim().to_string();
        let category = self.life_category_input.read(cx).value().trim().to_string();
        let value = self.life_value_input.read(cx).value().to_string();
        if name.is_empty() {
            self.life_error = Some("Name the finance record first.".to_string());
            cx.notify();
            return;
        }
        let Some(amount) = parse_amount_minor(&value) else {
            self.life_error = Some(bruhs_models::MoneyError::InvalidAmount.to_string());
            cx.notify();
            return;
        };
        let now = EpochMs::now().as_millis();
        match kind {
            FinanceEntryKind::Account => self
                .life_context
                .finance
                .accounts
                .insert(0, FinanceAccount::manual(name, amount, now)),
            FinanceEntryKind::Spend => self.life_context.finance.transactions.insert(
                0,
                FinanceTransaction::manual(
                    name,
                    if category.is_empty() {
                        "Other"
                    } else {
                        &category
                    },
                    if amount < 0 { amount } else { -amount },
                    now,
                )
                .into(),
            ),
            FinanceEntryKind::Budget => self.life_context.finance.budgets.insert(
                0,
                FinanceBudget::monthly(if category.is_empty() { name } else { category }, amount),
            ),
            FinanceEntryKind::Income => self
                .life_context
                .finance
                .income_streams
                .insert(0, IncomeStream::monthly(name, amount)),
            FinanceEntryKind::Holding => self
                .life_context
                .finance
                .holdings
                .insert(0, Holding::manual(name, amount, now)),
        }
        self.life_context.finance.updated_at_ms = now;
        self.life_error = None;
        self.clear_life_inputs(window, cx);
        self.persist_and_notify(cx);
    }

    fn add_health_entry(
        &mut self,
        kind: HealthEntryKind,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let name = self.life_name_input.read(cx).value().trim().to_string();
        let value = self.life_value_input.read(cx).value().trim().to_string();
        let now = EpochMs::now().as_millis();
        match kind {
            HealthEntryKind::Workout => {
                let Ok(minutes) = value.parse::<u16>() else {
                    self.life_error = Some("Enter workout minutes from 1 to 1440.".into());
                    cx.notify();
                    return;
                };
                if !(1..=1440).contains(&minutes) {
                    self.life_error = Some("Enter workout minutes from 1 to 1440.".into());
                    cx.notify();
                    return;
                }
                if name.is_empty() {
                    self.life_error = Some("Name the workout first.".to_string());
                    cx.notify();
                    return;
                }
                self.life_context
                    .health
                    .workouts
                    .insert(0, WorkoutLog::manual(name, minutes, now));
            }
            HealthEntryKind::Sleep => {
                let Ok(hours) = value.parse::<f64>() else {
                    self.life_error = Some("Enter sleep hours.".to_string());
                    cx.notify();
                    return;
                };
                if !hours.is_finite() || !(1.0 / 60.0..=24.0).contains(&hours) {
                    self.life_error = Some("Enter sleep hours from one minute to 24 hours.".into());
                    cx.notify();
                    return;
                }
                let Ok(quality) = name.parse::<u8>() else {
                    self.life_error =
                        Some("Enter sleep quality from 0 to 100 in the name field.".into());
                    cx.notify();
                    return;
                };
                if quality > 100 {
                    self.life_error =
                        Some("Enter sleep quality from 0 to 100 in the name field.".into());
                    cx.notify();
                    return;
                }
                self.life_context.health.sleep.insert(
                    0,
                    SleepLog::manual((hours * 60.0).round().to_u16_saturating(), quality, now),
                );
            }
            HealthEntryKind::Recovery => {
                let Ok(score) = value.parse::<u8>() else {
                    self.life_error = Some("Enter a recovery score from 0 to 100.".into());
                    cx.notify();
                    return;
                };
                if score > 100 {
                    self.life_error = Some("Enter a recovery score from 0 to 100.".into());
                    cx.notify();
                    return;
                }
                self.life_context
                    .health
                    .recovery
                    .insert(0, RecoveryLog::manual(score, now));
            }
            HealthEntryKind::Habit => {
                if name.is_empty() {
                    self.life_error = Some("Name the habit first.".to_string());
                    cx.notify();
                    return;
                }
                self.life_context
                    .health
                    .habits
                    .insert(0, Habit::weekly(name, 5));
            }
        }
        self.life_context.health.updated_at_ms = now;
        self.life_error = None;
        self.clear_life_inputs(window, cx);
        self.persist_and_notify(cx);
    }

    fn ask_agent_about_life(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let prompt = match life_agent_prompt(
            &self.life_context,
            &self.planning_work_items(),
            EpochMs::now().as_millis(),
        ) {
            Ok(prompt) => prompt,
            Err(message) => {
                self.life_error = Some(message.to_string());
                cx.notify();
                return;
            }
        };
        let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) else {
            self.life_error = Some("Start a chat before sharing life context.".to_string());
            cx.notify();
            return;
        };
        self.active_view = AppView::Chat;
        chat.update(cx, |chat, cx| chat.set_composer_draft(prompt, window, cx));
        self.persist_and_notify(cx);
    }

    fn life_summary_card(&self, label: &str, value: String) -> AnyElement {
        div()
            .id(SharedString::from(format!("life-summary-{label}")))
            .role(Role::Group)
            .aria_label(format!("{label}: {value}"))
            .flex_1()
            .min_w(px(150.0))
            .rounded(px(6.0))
            .bg(theme::longan_900().opacity(0.65))
            .border_1()
            .border_color(theme::longan_800())
            .p_3()
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_500())
                    .child(label.to_string()),
            )
            .child(
                div()
                    .mt_1()
                    .font_family(theme::CODE_FONT_FAMILY)
                    .text_size(px(18.0))
                    .text_color(theme::rambutan_100())
                    .child(value),
            )
            .into_any_element()
    }

    fn render_life_privacy(&self, cx: &Context<'_, Self>) -> AnyElement {
        let finance = self.life_tab == DesktopLifeTab::Finance;
        let enabled = if finance {
            self.life_context.finance.privacy.allow_agent_context
        } else {
            self.life_context.health.privacy.allow_agent_context
        };
        div()
            .debug_selector(|| "desktop-life-privacy".into())
            .rounded(px(6.0))
            .bg(theme::longan_900().opacity(0.65))
            .border_1()
            .border_color(theme::longan_800())
            .p_3()
            .flex()
            .flex_wrap()
            .gap_3()
            .items_center()
            .justify_between()
            .child(
                div()
                    .child(card_title(if finance {
                        "Finance privacy"
                    } else {
                        "Health privacy"
                    }))
                    .child(
                        div()
                            .mt_1()
                            .text_size(px(11.0))
                            .text_color(theme::rambutan_500())
                            .child(if finance {
                                "Local records stay out of agent context until enabled."
                            } else {
                                "Local logs stay out of agent context; bruhs never diagnoses."
                            }),
                    ),
            )
            .child(
                web_button(
                    "life-agent-access",
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .debug_selector(|| "desktop-life-agent-access".into())
                .aria_label(if enabled {
                    "Disable agent access"
                } else {
                    "Enable agent access"
                })
                .child(if enabled {
                    "Agent access on"
                } else {
                    "Agent access off"
                })
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    let now = EpochMs::now().as_millis();
                    if finance {
                        this.life_context.finance.privacy.allow_agent_context =
                            !this.life_context.finance.privacy.allow_agent_context;
                        if !this.life_context.finance.privacy.allow_agent_context {
                            this.life_context
                                .finance
                                .privacy
                                .reveal_exact_amounts_to_agent = false;
                        }
                        this.life_context.finance.updated_at_ms = now;
                    } else {
                        this.life_context.health.privacy.allow_agent_context =
                            !this.life_context.health.privacy.allow_agent_context;
                        this.life_context.health.updated_at_ms = now;
                    }
                    this.persist_and_notify(cx);
                })),
            )
            .when(finance, |panel| {
                panel.child(
                    web_button(
                        "life-exact-amounts",
                        WebButtonVariant::Outline,
                        WebButtonSize::Small,
                        ButtonState::disabled_if(!enabled),
                    )
                    .debug_selector(|| "desktop-life-exact-amounts".into())
                    .aria_label(
                        if self
                            .life_context
                            .finance
                            .privacy
                            .reveal_exact_amounts_to_agent
                        {
                            "Hide exact amounts from agents"
                        } else {
                            "Share exact amounts with agents"
                        },
                    )
                    .child(
                        if self
                            .life_context
                            .finance
                            .privacy
                            .reveal_exact_amounts_to_agent
                        {
                            "Exact amounts shared"
                        } else {
                            "Amounts redacted"
                        },
                    )
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        if this.life_context.finance.privacy.allow_agent_context {
                            this.life_context
                                .finance
                                .privacy
                                .reveal_exact_amounts_to_agent = !this
                                .life_context
                                .finance
                                .privacy
                                .reveal_exact_amounts_to_agent;
                            this.life_context.finance.updated_at_ms = EpochMs::now().as_millis();
                            this.persist_and_notify(cx);
                        }
                    })),
                )
            })
            .into_any_element()
    }

    fn render_life_quick_add(&self, compact: bool, cx: &Context<'_, Self>) -> AnyElement {
        let finance = self.life_tab == DesktopLifeTab::Finance;
        let finance_buttons =
            [
                ("Account", FinanceEntryKind::Account),
                ("Spend", FinanceEntryKind::Spend),
                ("Budget", FinanceEntryKind::Budget),
                ("Income", FinanceEntryKind::Income),
                ("Holding", FinanceEntryKind::Holding),
            ]
            .into_iter()
            .enumerate()
            .map(|(index, (label, kind))| {
                web_button(
                    ("finance-quick-add", index),
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .aria_label(label)
                .child(label)
                .on_click(cx.listener(move |this, _event, window, cx| {
                    this.add_finance_entry(kind, window, cx)
                }))
                .into_any_element()
            })
            .collect::<Vec<_>>();
        let health_buttons =
            [
                ("Workout", HealthEntryKind::Workout),
                ("Sleep", HealthEntryKind::Sleep),
                ("Recovery", HealthEntryKind::Recovery),
                ("Habit", HealthEntryKind::Habit),
            ]
            .into_iter()
            .enumerate()
            .map(|(index, (label, kind))| {
                web_button(
                    ("health-quick-add", index),
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .aria_label(label)
                .child(label)
                .on_click(cx.listener(move |this, _event, window, cx| {
                    this.add_health_entry(kind, window, cx)
                }))
                .into_any_element()
            })
            .collect::<Vec<_>>();
        div()
            .rounded(px(6.0))
            .bg(theme::longan_900().opacity(0.65))
            .border_1()
            .border_color(theme::longan_800())
            .p_3()
            .child(card_title(if finance {
                "Quick add finance"
            } else {
                "Quick add health"
            }))
            .child(
                div()
                    .mt_2()
                    .grid()
                    .grid_cols(if compact { 1 } else if finance { 3 } else { 2 })
                    .gap_2()
                    .child(Input::new(&self.life_name_input).aria_label(if finance { "Finance record name" } else { "Workout or habit name, or sleep quality percent" }))
                    .child(Input::new(&self.life_value_input).aria_label(if finance { "Amount in USD" } else { "Workout minutes, sleep hours, or recovery percent" }))
                    .when(finance, |form| form.child(Input::new(&self.life_category_input).aria_label("Category, optional"))),
            )
            .child(
                div()
                    .mt_2()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(if finance {
                        finance_buttons
                    } else {
                        health_buttons
                    }),
            )
            .child(
                div()
                    .mt_2()
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_400())
                    .child(if finance {
                        "Name, amount, optional category. Spend is recorded as an outflow."
                    } else {
                        "Workout: name + minutes. Sleep: quality % + hours. Recovery: score. Habit: name."
                    }),
            )
            .when_some(self.life_error.clone(), |panel, error| panel.child(
                div().id("life-entry-error").role(Role::Alert).text_size(px(11.0)).text_color(theme::lychee_300()).child(error)
            ))
            .into_any_element()
    }

    /// Web `finance_tab`'s grid: Accounts, Transactions, Budgets & income
    /// and Holdings.
    fn render_finance_cards(&self, two_columns: bool, cx: &Context<'_, Self>) -> AnyElement {
        let finance = &self.life_context.finance;
        let accounts = finance
            .accounts
            .iter()
            .enumerate()
            .map(|(index, account)| {
                let id = account.id.clone();
                life_row(("life-account", index))
                    .child(row_label(
                        account.name.clone(),
                        account.kind.label().to_string(),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(row_amount(money(account.balance_minor, &account.currency)))
                            .child(
                                web_button(
                                    ("delete-life-account", index),
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .aria_label("Delete account")
                                .child(
                                    svg()
                                        .path(assets::CLOSE_ICON_PATH)
                                        .size(px(14.0))
                                        .text_color(theme::rambutan_400()),
                                )
                                .on_click(cx.listener(
                                    move |this, _event, _window, cx| {
                                        this.life_context
                                            .finance
                                            .accounts
                                            .retain(|item| item.id != id);
                                        this.life_context.finance.updated_at_ms =
                                            EpochMs::now().as_millis();
                                        this.persist_and_notify(cx);
                                    },
                                )),
                            ),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let transactions = finance
            .transactions
            .iter()
            .take(TRANSACTION_ROWS)
            .enumerate()
            .map(|(index, transaction)| {
                let id = transaction.id.clone();
                life_row(("life-transaction", index))
                    .debug_selector(move || format!("desktop-life-transaction-{index}"))
                    .child(row_label(
                        transaction.merchant.clone(),
                        transaction.category.clone(),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(row_amount(money(
                                transaction.amount_minor,
                                &transaction.currency,
                            )))
                            .child(
                                web_button(
                                    ("delete-life-transaction", index),
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .debug_selector(move || {
                                    format!("desktop-life-delete-transaction-{index}")
                                })
                                .aria_label("Delete transaction")
                                .child(
                                    svg()
                                        .path(assets::CLOSE_ICON_PATH)
                                        .size(px(14.0))
                                        .text_color(theme::rambutan_400()),
                                )
                                .on_click(cx.listener(
                                    move |this, _event, _window, cx| {
                                        this.delete_life_transaction(&id, cx);
                                    },
                                )),
                            ),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let holdings = finance
            .holdings
            .iter()
            .enumerate()
            .map(|(index, holding)| {
                div()
                    .id(("life-holding", index))
                    .debug_selector(move || format!("desktop-life-holding-{index}"))
                    .max_w_full()
                    .rounded(px(4.0))
                    .bg(theme::longan_950().opacity(0.45))
                    .px(px(10.0))
                    .py(px(6.0))
                    .font_family(theme::CODE_FONT_FAMILY)
                    .text_size(px(BODY_SM_PX))
                    .text_color(theme::rambutan_300())
                    .child(holding.label())
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        record_grid(
            two_columns,
            vec![
                life_card("life-accounts", "Accounts")
                    .child(div().mt_2().flex().flex_col().gap_1().children(accounts))
                    .into_any_element(),
                life_card("life-transactions", "Transactions")
                    .child(
                        div()
                            .mt_2()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(transactions),
                    )
                    .into_any_element(),
                life_card("life-budgets", "Budgets & income")
                    .child(
                        div()
                            .mt_2()
                            .text_size(px(BODY_SM_PX))
                            .text_color(theme::rambutan_500())
                            .child(finance.budgets_and_income_summary()),
                    )
                    .into_any_element(),
                life_card("life-holdings", "Holdings")
                    .child(div().mt_2().flex().flex_wrap().gap_2().children(holdings))
                    .into_any_element(),
            ],
        )
        .into_any_element()
    }

    /// Web `health_tab`'s grid: Workout, Sleep & recovery, Habits and the
    /// health sources.
    fn render_health_cards(&self, two_columns: bool, cx: &Context<'_, Self>) -> AnyElement {
        let health = &self.life_context.health;
        let today = EpochMs::now().as_millis().div_euclid(86_400_000);
        let workouts = health
            .workouts
            .iter()
            .take(WORKOUT_ROWS)
            .enumerate()
            .map(|(index, workout)| {
                div()
                    .id(("life-workout", index))
                    .rounded(px(4.0))
                    .bg(theme::longan_950().opacity(0.45))
                    .px(px(10.0))
                    .py_2()
                    .text_size(px(BODY_SM_PX))
                    .text_color(theme::rambutan_300())
                    .child(format!(
                        "{}, {} min, effort {}/10",
                        workout.activity, workout.duration_minutes, workout.perceived_exertion
                    ))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let habits = health
            .habits
            .iter()
            .filter(|habit| !habit.archived)
            .enumerate()
            .map(|(index, habit)| {
                let id = habit.id.clone();
                let done = habit.completion_days.contains(&today);
                web_button(
                    ("life-habit", index),
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .w_full()
                .justify_between()
                .px(px(10.0))
                .rounded(px(4.0))
                .bg(if done {
                    theme::pandan_500().opacity(0.1)
                } else {
                    theme::longan_950().opacity(0.45)
                })
                .text_size(px(BODY_SM_PX))
                .text_color(if done {
                    theme::pandan_300()
                } else {
                    theme::rambutan_300()
                })
                .aria_label(format!(
                    "{} {}",
                    if done { "Unmark" } else { "Complete" },
                    habit.title
                ))
                .aria_toggled(if done { Toggled::True } else { Toggled::False })
                .child(div().min_w_0().child(habit.title.clone()))
                .child(if done { "Done today" } else { "Mark done" })
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    if let Some(habit) = this
                        .life_context
                        .health
                        .habits
                        .iter_mut()
                        .find(|habit| habit.id == id)
                    {
                        habit.toggle_day(EpochMs::now().as_millis());
                        this.life_context.health.updated_at_ms = EpochMs::now().as_millis();
                        this.persist_and_notify(cx);
                    }
                }))
                .into_any_element()
            })
            .collect::<Vec<_>>();
        record_grid(
            two_columns,
            vec![
                life_card("life-workouts", "Workout")
                    .child(div().mt_2().flex().flex_col().gap_1().children(workouts))
                    .into_any_element(),
                life_card("life-sleep", "Sleep & recovery")
                    .child(
                        div()
                            .debug_selector(|| "desktop-life-sleep-summary".into())
                            .mt_2()
                            .text_size(px(BODY_SM_PX))
                            .text_color(theme::rambutan_500())
                            .child(health.sleep_and_recovery_summary()),
                    )
                    .into_any_element(),
                life_card("life-habits", "Habits")
                    .child(div().mt_2().flex().flex_col().gap_1().children(habits))
                    .into_any_element(),
                self.render_life_providers(false),
            ],
        )
        .into_any_element()
    }

    fn render_life_insights(&self, cx: &Context<'_, Self>) -> AnyElement {
        let insights = self
            .life_context
            .insights(&self.planning_work_items(), EpochMs::now().as_millis())
            .into_iter()
            .enumerate()
            .map(|(index, insight)| {
                let id = insight.id.clone();
                let color = match insight.domain {
                    LifeInsightDomain::Finance => theme::durian_300(),
                    LifeInsightDomain::Health => theme::pandan_300(),
                    LifeInsightDomain::Productivity => theme::blueberry_400(),
                };
                div()
                    .id(("life-insight", index))
                    .rounded(px(4.0))
                    .bg(theme::longan_950().opacity(0.45))
                    .p_3()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(color)
                                    .child(insight.title),
                            )
                            .child(
                                web_button(
                                    ("dismiss-life-insight", index),
                                    WebButtonVariant::Ghost,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .aria_label("Dismiss observation")
                                .child(
                                    svg()
                                        .path(assets::CLOSE_ICON_PATH)
                                        .size(px(14.0))
                                        .text_color(theme::rambutan_400()),
                                )
                                .on_click(cx.listener(
                                    move |this, _event, _window, cx| {
                                        this.life_context.dismiss_insight(id.clone());
                                        this.persist_and_notify(cx);
                                    },
                                )),
                            ),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_size(px(11.0))
                            .text_color(theme::rambutan_300())
                            .child(insight.suggestion),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_size(px(12.0))
                            .text_color(theme::rambutan_500())
                            .child(format!("Evidence: {}", insight.evidence)),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_size(px(12.0))
                            .text_color(theme::rambutan_400())
                            .child(insight.disclaimer),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        div()
            .rounded(px(6.0))
            .bg(theme::longan_900().opacity(0.65))
            .border_1()
            .border_color(theme::longan_800())
            .p_3()
            .child(card_title("Observations with evidence"))
            .child(
                div()
                    .mt_1()
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_400())
                    .child("Local comparisons do not provide diagnoses, causal claims, or autonomous transactions."),
            )
            .when(insights.is_empty(), |panel| {
                panel.child(
                    div()
                        .mt_2()
                        .text_size(px(11.0))
                        .text_color(theme::rambutan_400())
                        .child("Add consistent records to reveal observations supported by evidence."),
                )
            })
            .children(insights)
            .into_any_element()
    }

    /// Web "Finance sources" (three to a row from `md`) and "Health
    /// sources" (one column, in the health grid).
    fn render_life_providers(&self, three_columns: bool) -> AnyElement {
        let finance = self.life_tab == DesktopLifeTab::Finance;
        let providers = if finance {
            finance_provider_adapters()
        } else {
            health_provider_adapters()
        };
        let rows = providers
            .into_iter()
            .enumerate()
            .map(|(index, provider)| {
                div()
                    .id(("life-provider", index))
                    .rounded(px(4.0))
                    .bg(theme::longan_950().opacity(0.45))
                    .p_2()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(theme::rambutan_300())
                                    .child(provider.name),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(provider_color(provider.status))
                                    .child(provider_label(provider.status)),
                            ),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_size(px(12.0))
                            .text_color(theme::rambutan_400())
                            .child(provider.privacy_note),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        life_card(
            "life-sources",
            if finance {
                "Finance sources"
            } else {
                "Health sources"
            },
        )
        .child(
            div()
                .mt_2()
                .grid()
                .grid_cols(if three_columns { 3 } else { 1 })
                .gap_2()
                .children(rows),
        )
        .into_any_element()
    }

    pub(super) fn render_life_page(&self, window: &Window, cx: &Context<'_, Self>) -> Div {
        let compact = Self::compact_layout(window);
        let width = f32::from(window.viewport_size().width);
        let two_columns = width >= RECORD_GRID_TWO_COLUMNS_PX;
        let finance = self.life_tab == DesktopLifeTab::Finance;
        let summary = if finance {
            vec![
                self.life_summary_card(
                    "Manual net worth",
                    bruhs_models::format_money_totals(self.life_context.finance.net_worth_totals()),
                ),
                self.life_summary_card(
                    "Spend, trailing 30 days",
                    bruhs_models::format_money_totals(
                        self.life_context
                            .finance
                            .trailing_spend_totals(EpochMs::now().as_millis(), 30),
                    ),
                ),
                self.life_summary_card(
                    "Expected monthly income",
                    bruhs_models::format_money_totals(
                        self.life_context.finance.monthly_income_totals(),
                    ),
                ),
            ]
        } else {
            vec![
                self.life_summary_card(
                    "Workouts logged",
                    self.life_context.health.workouts.len().to_string(),
                ),
                self.life_summary_card(
                    "Latest recovery",
                    self.life_context
                        .health
                        .recovery
                        .first()
                        .map_or("Not logged".to_string(), |log| {
                            format!("{}%", log.score_percent)
                        }),
                ),
                self.life_summary_card(
                    "Active habits",
                    self.life_context
                        .health
                        .habits
                        .iter()
                        .filter(|habit| !habit.archived)
                        .count()
                        .to_string(),
                ),
            ]
        };
        div()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme::longan_950())
            .child(
                div()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(theme::longan_800().opacity(0.5))
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .child(
                                div()
                                    .font_family(theme::HEADING_FONT_FAMILY)
                                    .text_size(px(18.0))
                                    .text_color(theme::rambutan_100())
                                    .child("Life context"),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .text_size(px(11.0))
                                    .text_color(theme::rambutan_500())
                                    .child("Manual first, private by default, useful only with your consent."),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                web_button(
                                    "life-tab-finance",
                                    if finance {
                                        WebButtonVariant::Outline
                                    } else {
                                        WebButtonVariant::Ghost
                                    },
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .aria_label("Finance")
                                .child("Finance")
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.life_tab = DesktopLifeTab::Finance;
                                    cx.notify();
                                })),
                            )
                            .child(
                                web_button(
                                    "life-tab-health",
                                    if finance {
                                        WebButtonVariant::Ghost
                                    } else {
                                        WebButtonVariant::Outline
                                    },
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .aria_label("Health")
                                .child("Health")
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.life_tab = DesktopLifeTab::Health;
                                    cx.notify();
                                })),
                            )
                            .child(
                                web_button(
                                    "ask-agent-about-life",
                                    WebButtonVariant::Primary,
                                    WebButtonSize::Small,
                                    ButtonState::Enabled,
                                )
                                .aria_label("Ask agent")
                                .child("Ask agent")
                                .on_click(cx.listener(|this, _event, window, cx| {
                                    this.ask_agent_about_life(window, cx)
                                })),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .when_some(self.render_search_record_detail(cx), |page, detail| page.child(detail))
                    .when(finance, |el| el.child(div().text_size(px(11.0)).text_color(theme::rambutan_500())
                        .child("Manual amounts are USD. Other currencies stay separate in minor units; no exchange rate is applied.")))
                    .child(div().flex().flex_wrap().gap_2().children(summary))
                    .child(self.render_life_privacy(cx))
                    .child(self.render_life_quick_add(compact, cx))
                    // Web order: the record cards, then the finance sources;
                    // the health sources share its grid.
                    .child(if finance {
                        self.render_finance_cards(two_columns, cx)
                    } else {
                        self.render_health_cards(two_columns, cx)
                    })
                    .when(finance, |page| {
                        page.child(self.render_life_providers(width >= SOURCES_THREE_COLUMNS_PX))
                    })
                    .child(self.render_life_insights(cx)),
            )
    }
}

/// The manual Finance / Health entry fields.
pub(super) struct LifeInputs {
    pub(super) life_name_input: Entity<InputState>,
    pub(super) life_value_input: Entity<InputState>,
    pub(super) life_category_input: Entity<InputState>,
}

impl Workspace {
    /// The manual Finance / Health entry fields; edits redraw.
    pub(super) fn new_life_inputs(
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> LifeInputs {
        let life_name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Name"));
        subscriptions.push(redraw_on_edit(&life_name_input, window, cx));
        let life_value_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Amount or score"));
        subscriptions.push(redraw_on_edit(&life_value_input, window, cx));
        let life_category_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Category (optional)"));
        subscriptions.push(redraw_on_edit(&life_category_input, window, cx));
        LifeInputs {
            life_name_input,
            life_value_input,
            life_category_input,
        }
    }
}

#[cfg(test)]
mod parity_tests;

#[cfg(test)]
mod money_tests {
    use super::super::load_desktop_state_from_path;
    use super::{theme, DesktopState, FinanceEntryKind, Workspace};
    use gpui::AppContext;
    use std::{cell::RefCell, fs, rc::Rc};

    #[gpui::test]
    async fn native_finance_forms_preserve_exact_amounts_and_invalid_drafts(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_component::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().unwrap());
        });
        let cell = Rc::new(RefCell::new(None));
        let capture = cell.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let workspace = cx.new(|cx| Workspace::new(window, cx));
            *capture.borrow_mut() = Some(workspace.clone());
            gpui_component::Root::new(workspace, window, cx)
        });
        let workspace = cell.borrow().clone().unwrap();
        let dir = std::env::temp_dir().join(format!("bruhs-native-money-{}", uuid::Uuid::new_v4()));
        let path = dir.join("state.json");
        workspace.update_in(cx, |w, _, _| {
            w.persistence_test_path = Some(path.clone());
            w.life_context = bruhs_models::LifeContext::default();
        });
        for kind in [
            FinanceEntryKind::Account,
            FinanceEntryKind::Spend,
            FinanceEntryKind::Budget,
            FinanceEntryKind::Income,
            FinanceEntryKind::Holding,
        ] {
            workspace.update_in(cx, |w, window, cx| {
                w.life_name_input
                    .update(cx, |input, cx| input.set_value("Exact", window, cx));
                w.life_value_input.update(cx, |input, cx| {
                    input.set_value("90071992547409.93", window, cx)
                });
                w.add_finance_entry(kind, window, cx);
                assert!(w.life_error.is_none());
            });
            cx.condition(&workspace, |w, _| w.save_status.is_saved())
                .await;
        }
        let finance = load_desktop_state_from_path(&path)
            .unwrap()
            .unwrap()
            .life_context
            .finance;
        assert_eq!(finance.accounts[0].balance_minor, 9_007_199_254_740_993);
        assert_eq!(finance.transactions[0].amount_minor, -9_007_199_254_740_993);
        assert_eq!(
            finance.budgets[0].monthly_limit_minor,
            9_007_199_254_740_993
        );
        assert_eq!(
            finance.income_streams[0].expected_monthly_minor,
            9_007_199_254_740_993
        );
        assert_eq!(
            finance.holdings[0].current_value_minor,
            9_007_199_254_740_993
        );
        let before = fs::read(&path).unwrap();
        workspace.update_in(cx, |w, window, cx| {
            w.life_name_input
                .update(cx, |input, cx| input.set_value("Retain", window, cx));
            w.life_value_input
                .update(cx, |input, cx| input.set_value("1e30", window, cx));
            w.persistence_error = Some("Existing disk failure".into());
            w.add_finance_entry(FinanceEntryKind::Account, window, cx);
            assert!(w.life_error.is_some());
            assert_eq!(
                w.persistence_error.as_deref(),
                Some("Existing disk failure")
            );
            assert_eq!(w.life_value_input.read(cx).value().as_str(), "1e30");
            assert_eq!(w.life_context.finance.accounts.len(), 1);
        });
        cx.run_until_parked();
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod health_audit_tests {
    use super::{theme, DesktopState, HealthEntryKind, Workspace};
    use gpui::TestAppContext;

    #[gpui::test]
    fn health_forms_reject_normalized_values_and_preserve_invalid_drafts(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().unwrap());
        });
        let (workspace, cx) = cx.add_window_view(Workspace::new);
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.life_context = bruhs_models::LifeContext::default();
            workspace.persistence_error = Some("Existing disk failure".into());
            let before_saves = workspace.persist_requests;
            for (kind, name, value) in [
                (HealthEntryKind::Workout, "Walk", "0"),
                (HealthEntryKind::Workout, "Walk", "1441"),
                (HealthEntryKind::Workout, "Walk", "abc"),
                (HealthEntryKind::Sleep, "Quality", "8"),
                (HealthEntryKind::Sleep, "101", "8"),
                (HealthEntryKind::Sleep, "75", "0"),
                (HealthEntryKind::Sleep, "75", "0.001"),
                (HealthEntryKind::Sleep, "75", "24.1"),
                (HealthEntryKind::Sleep, "75", "NaN"),
                (HealthEntryKind::Recovery, "", "101"),
                (HealthEntryKind::Recovery, "", "abc"),
            ] {
                workspace
                    .life_name_input
                    .update(cx, |input, cx| input.set_value(name, window, cx));
                workspace
                    .life_value_input
                    .update(cx, |input, cx| input.set_value(value, window, cx));
                workspace.add_health_entry(kind, window, cx);
                assert!(workspace.life_error.is_some(), "{name} / {value}");
                assert_eq!(
                    workspace.persistence_error.as_deref(),
                    Some("Existing disk failure")
                );
                assert_eq!(workspace.life_name_input.read(cx).value().as_str(), name);
                assert_eq!(workspace.life_value_input.read(cx).value().as_str(), value);
                assert!(workspace.life_context.health.workouts.is_empty());
                assert!(workspace.life_context.health.sleep.is_empty());
                assert!(workspace.life_context.health.recovery.is_empty());
                assert_eq!(workspace.persist_requests, before_saves);
            }
            for (kind, name, value) in [
                (HealthEntryKind::Workout, "Walk", "1"),
                (HealthEntryKind::Workout, "Walk", "1440"),
                (HealthEntryKind::Sleep, "75", "8.5"),
                (HealthEntryKind::Sleep, "100", "24"),
                (HealthEntryKind::Recovery, "", "0"),
                (HealthEntryKind::Recovery, "", "100"),
            ] {
                workspace
                    .life_name_input
                    .update(cx, |input, cx| input.set_value(name, window, cx));
                workspace
                    .life_value_input
                    .update(cx, |input, cx| input.set_value(value, window, cx));
                workspace.add_health_entry(kind, window, cx);
                assert!(workspace.life_error.is_none());
                assert_eq!(
                    workspace.persistence_error.as_deref(),
                    Some("Existing disk failure")
                );
                assert!(workspace.life_value_input.read(cx).value().is_empty());
            }
            assert_eq!(
                workspace
                    .life_context
                    .health
                    .workouts
                    .iter()
                    .map(|log| log.duration_minutes)
                    .collect::<Vec<_>>(),
                [1440, 1]
            );
            assert_eq!(
                workspace.life_context.health.sleep[0].duration_minutes,
                1440
            );
            assert_eq!(workspace.life_context.health.sleep[1].duration_minutes, 510);
            assert_eq!(workspace.life_context.health.sleep[1].quality_percent, 75);
            assert_eq!(workspace.life_context.health.recovery[0].score_percent, 100);
            assert_eq!(workspace.life_context.health.recovery[1].score_percent, 0);
        });
    }
}
