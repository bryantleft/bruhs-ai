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
            self.persistence_error = Some("Name the finance record first.".to_string());
            cx.notify();
            return;
        }
        let Some(amount) = parse_amount_minor(&value) else {
            self.persistence_error = Some(bruhs_models::MoneyError::InvalidAmount.to_string());
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
                ),
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
        self.persistence_error = None;
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
                    self.persistence_error = Some("Enter workout minutes.".to_string());
                    cx.notify();
                    return;
                };
                if name.is_empty() {
                    self.persistence_error = Some("Name the workout first.".to_string());
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
                    self.persistence_error = Some("Enter sleep hours.".to_string());
                    cx.notify();
                    return;
                };
                if !hours.is_finite() || hours <= 0.0 {
                    self.persistence_error = Some("Enter sleep hours above zero.".to_string());
                    cx.notify();
                    return;
                }
                let quality = name.parse::<u8>().unwrap_or(0);
                self.life_context.health.sleep.insert(
                    0,
                    SleepLog::manual((hours * 60.0).round().to_u16_saturating(), quality, now),
                );
            }
            HealthEntryKind::Recovery => {
                let Ok(score) = value.parse::<u8>() else {
                    self.persistence_error = Some("Enter a recovery score.".to_string());
                    cx.notify();
                    return;
                };
                self.life_context
                    .health
                    .recovery
                    .insert(0, RecoveryLog::manual(score, now));
            }
            HealthEntryKind::Habit => {
                if name.is_empty() {
                    self.persistence_error = Some("Name the habit first.".to_string());
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
        self.persistence_error = None;
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
                self.persistence_error = Some(message.to_string());
                cx.notify();
                return;
            }
        };
        let Some(chat) = self.active_thread().map(|thread| thread.chat.clone()) else {
            self.persistence_error = Some("Start a chat before sharing life context.".to_string());
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
            .rounded(px(6.0))
            .bg(theme::longan_900().opacity(0.65))
            .border_1()
            .border_color(theme::longan_800())
            .p_3()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme::rambutan_200())
                            .child(if finance {
                                "Finance privacy"
                            } else {
                                "Health privacy"
                            }),
                    )
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

    fn render_life_quick_add(&self, cx: &Context<'_, Self>) -> AnyElement {
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
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_200())
                    .child(if finance {
                        "Quick add finance"
                    } else {
                        "Quick add health"
                    }),
            )
            .child(
                div()
                    .mt_2()
                    .grid()
                    .grid_cols(3)
                    .gap_2()
                    .child(Input::new(&self.life_name_input))
                    .child(Input::new(&self.life_value_input))
                    .child(Input::new(&self.life_category_input)),
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
                        "Name · amount · optional category. Spend is recorded as an outflow."
                    } else {
                        "Workout: name + minutes. Sleep: quality % + hours. Recovery: score. Habit: name."
                    }),
            )
            .into_any_element()
    }

    fn render_finance_records(&self, cx: &Context<'_, Self>) -> AnyElement {
        let mut rows = Vec::new();
        rows.extend(
            self.life_context
                .finance
                .accounts
                .iter()
                .take(8)
                .enumerate()
                .map(|(index, account)| {
                    let id = account.id.clone();
                    div()
                        .id(("life-account", index))
                        .rounded(px(4.0))
                        .bg(theme::longan_950().opacity(0.45))
                        .px_3()
                        .py_2()
                        .flex()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(theme::rambutan_300())
                                .child(account.name.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    div()
                                        .font_family(theme::CODE_FONT_FAMILY)
                                        .text_size(px(11.0))
                                        .text_color(theme::rambutan_200())
                                        .child(money(account.balance_minor, &account.currency)),
                                )
                                .child(
                                    web_button(
                                        ("delete-life-account", index),
                                        WebButtonVariant::Ghost,
                                        WebButtonSize::Small,
                                        ButtonState::Enabled,
                                    )
                                    .aria_label("Delete account")
                                    .child("×")
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
                }),
        );
        rows.extend(
            self.life_context
                .finance
                .transactions
                .iter()
                .take(8)
                .enumerate()
                .map(|(index, transaction)| {
                    div()
                        .id(("life-transaction", index))
                        .rounded(px(4.0))
                        .bg(theme::longan_950().opacity(0.45))
                        .px_3()
                        .py_2()
                        .flex()
                        .justify_between()
                        .child(
                            div()
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(theme::rambutan_300())
                                        .child(transaction.merchant.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(theme::rambutan_400())
                                        .child(transaction.category.clone()),
                                ),
                        )
                        .child(
                            div()
                                .font_family(theme::CODE_FONT_FAMILY)
                                .text_size(px(11.0))
                                .text_color(theme::rambutan_200())
                                .child(money(transaction.amount_minor, &transaction.currency)),
                        )
                        .into_any_element()
                }),
        );
        div()
            .rounded(px(6.0))
            .bg(theme::longan_900().opacity(0.65))
            .border_1()
            .border_color(theme::longan_800())
            .p_3()
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_200())
                    .child("Accounts & recent transactions"),
            )
            .when(rows.is_empty(), |panel| {
                panel.child(
                    div()
                        .mt_2()
                        .text_size(px(11.0))
                        .text_color(theme::rambutan_400())
                        .child("No finance records yet."),
                )
            })
            .children(rows)
            .into_any_element()
    }

    fn render_health_records(&self, cx: &Context<'_, Self>) -> AnyElement {
        let today = EpochMs::now().as_millis().div_euclid(86_400_000);
        let habits = self
            .life_context
            .health
            .habits
            .iter()
            .filter(|habit| !habit.archived)
            .enumerate()
            .map(|(index, habit)| {
                let id = habit.id.clone();
                let done = habit.completion_days.contains(&today);
                web_button(
                    ("life-habit", index),
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .aria_label(format!(
                    "{} {}",
                    if done { "Unmark" } else { "Complete" },
                    habit.title
                ))
                .child(format!(
                    "{} · {}",
                    habit.title,
                    if done { "Done today" } else { "Mark done" }
                ))
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
        let workouts = self
            .life_context
            .health
            .workouts
            .iter()
            .take(8)
            .enumerate()
            .map(|(index, workout)| {
                div()
                    .id(("life-workout", index))
                    .rounded(px(4.0))
                    .bg(theme::longan_950().opacity(0.45))
                    .px_3()
                    .py_2()
                    .text_size(px(11.0))
                    .text_color(theme::rambutan_300())
                    .child(format!(
                        "{} · {} min · effort {}/10",
                        workout.activity, workout.duration_minutes, workout.perceived_exertion
                    ))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        div()
            .rounded(px(6.0))
            .bg(theme::longan_900().opacity(0.65))
            .border_1()
            .border_color(theme::longan_800())
            .p_3()
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_200())
                    .child("Habits & workouts"),
            )
            .child(div().mt_2().flex().flex_wrap().gap_1().children(habits))
            .children(workouts)
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
                                .child("×")
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
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_200())
                    .child("Evidence-labelled observations"),
            )
            .child(
                div()
                    .mt_1()
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_400())
                    .child("Local comparisons only—never diagnoses, causal claims, or autonomous transactions."),
            )
            .when(insights.is_empty(), |panel| {
                panel.child(
                    div()
                        .mt_2()
                        .text_size(px(11.0))
                        .text_color(theme::rambutan_400())
                        .child("Add consistent records to reveal evidence-backed observations."),
                )
            })
            .children(insights)
            .into_any_element()
    }

    fn render_life_providers(&self) -> AnyElement {
        let providers = if self.life_tab == DesktopLifeTab::Finance {
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
        div()
            .rounded(px(6.0))
            .bg(theme::longan_900().opacity(0.65))
            .border_1()
            .border_color(theme::longan_800())
            .p_3()
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(theme::rambutan_200())
                    .child("Sources"),
            )
            .children(rows)
            .into_any_element()
    }

    pub(super) fn render_life_page(&self, _window: &Window, cx: &Context<'_, Self>) -> Div {
        let finance = self.life_tab == DesktopLifeTab::Finance;
        let summary = if finance {
            vec![
                self.life_summary_card(
                    "Manual net worth",
                    bruhs_models::format_money_totals(self.life_context.finance.net_worth_totals()),
                ),
                self.life_summary_card(
                    "Spend · trailing 30 days",
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
                        .map_or("—".to_string(), |log| format!("{}%", log.score_percent)),
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
                    .child(self.render_life_quick_add(cx))
                    .child(if finance {
                        self.render_finance_records(cx)
                    } else {
                        self.render_health_records(cx)
                    })
                    .child(self.render_life_insights(cx))
                    .child(self.render_life_providers())
                    .when_some(self.persistence_error.clone(), |page, error| {
                        page.child(
                            div()
                                .text_size(px(11.0))
                                .text_color(theme::lychee_300())
                                .child(error),
                        )
                    }),
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
                assert!(w.persistence_error.is_none());
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
            w.add_finance_entry(FinanceEntryKind::Account, window, cx);
            assert!(w.persistence_error.is_some());
            assert_eq!(w.life_value_input.read(cx).value().as_str(), "1e30");
            assert_eq!(w.life_context.finance.accounts.len(), 1);
        });
        cx.run_until_parked();
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::remove_dir_all(dir).unwrap();
    }
}
