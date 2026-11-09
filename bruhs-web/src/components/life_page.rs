//! Private, manual-first finance and health context.

mod entries;
mod record_detail;

use bruhs_models::{
    finance_provider_adapters, health_provider_adapters, life_agent_prompt, LifeContext,
    LifeInsightDomain, LifeProviderAdapter, LifeSection, ProviderStatus,
};
use leptos::prelude::*;

use crate::durable_metadata::DurableMetadata;
use crate::state::{
    AppView, ComposerDraft, LifeContextSignal, LifeSectionSignal, SetActiveView, WorkItemsSignal,
};

fn money(minor: i64, currency: &str) -> String {
    bruhs_models::format_money(i128::from(minor), currency)
}

fn now_ms() -> i64 {
    crate::clock::now_ms()
}

fn provider_tone(status: ProviderStatus) -> &'static str {
    match status {
        ProviderStatus::Local | ProviderStatus::Connected => "text-pandan-300",
        ProviderStatus::Available => "text-blueberry-300",
        ProviderStatus::Disconnected => "text-rambutan-500",
        ProviderStatus::Error => "text-lychee-300",
    }
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

const INPUT_CLASS: &str = "mt-1 h-9 min-h-[44px] min-w-0 w-full sm:min-h-9 rounded-grape bg-longan-950/70 px-2.5 text-body-sm text-rambutan-200 outline-none inset-ring inset-ring-longan-700 placeholder:text-rambutan-500 focus:inset-ring-persimmon-400";
const CARD_CLASS: &str =
    "min-w-0 rounded-grape bg-longan-900/65 p-3 inset-ring inset-ring-longan-800";
const TAB_ON: &str = "min-h-[44px] sm:min-h-6 rounded-seed bg-longan-700 px-3 py-1.5 text-label-sm text-rambutan-100 cursor-pointer";
const TAB_OFF: &str = "min-h-[44px] sm:min-h-6 rounded-seed px-3 py-1.5 text-label-sm text-rambutan-500 hover:text-rambutan-300 cursor-pointer";
const SMALL_ACTION: &str = "mt-1 inline-flex min-h-[44px] min-w-[44px] shrink-0 items-center justify-center rounded-grape bg-longan-800 px-3 text-label-sm text-rambutan-200 transition-colors hover:bg-longan-700 cursor-pointer sm:h-9 sm:min-h-9";
const ADD_BUTTON: &str = "mt-2 inline-flex min-h-[44px] min-w-[44px] items-center justify-center rounded-grape bg-longan-800 px-3 text-label-sm text-rambutan-200 transition-colors hover:bg-longan-700 cursor-pointer sm:h-9 sm:min-h-9";
const ROW: &str =
    "flex min-w-0 flex-wrap items-center justify-between gap-2 rounded-seed bg-longan-950/45 px-2.5 py-2";
const ROW_AMOUNT: &str =
    "min-w-0 [overflow-wrap:anywhere] font-mono text-body-sm text-rambutan-200";

/// The text the forms hold. Owned by the page, so a half-typed entry
/// survives switching between Finance and Health.
#[derive(Clone, Copy)]
struct Inputs {
    account_name: RwSignal<String>,
    account_balance: RwSignal<String>,
    merchant: RwSignal<String>,
    spend_amount: RwSignal<String>,
    spend_category: RwSignal<String>,
    budget_category: RwSignal<String>,
    budget_limit: RwSignal<String>,
    income_name: RwSignal<String>,
    income_amount: RwSignal<String>,
    holding_symbol: RwSignal<String>,
    holding_value: RwSignal<String>,
    csv: RwSignal<String>,
    workout_activity: RwSignal<String>,
    workout_minutes: RwSignal<String>,
    sleep_hours: RwSignal<String>,
    sleep_quality: RwSignal<String>,
    recovery_score: RwSignal<String>,
    habit_title: RwSignal<String>,
}

impl Inputs {
    fn new() -> Self {
        let field = || RwSignal::new(String::new());
        Self {
            account_name: field(),
            account_balance: field(),
            merchant: field(),
            spend_amount: field(),
            spend_category: field(),
            budget_category: field(),
            budget_limit: field(),
            income_name: field(),
            income_amount: field(),
            holding_symbol: field(),
            holding_value: field(),
            csv: field(),
            workout_activity: field(),
            workout_minutes: field(),
            sleep_hours: field(),
            sleep_quality: field(),
            recovery_score: field(),
            habit_title: field(),
        }
    }
}

fn clear(fields: &[RwSignal<String>]) {
    for field in fields {
        field.set(String::new());
    }
}

/// Everything the page's handlers touch.
#[derive(Clone, Copy)]
struct Page {
    life: DurableMetadata<LifeContext>,
    notice: RwSignal<Option<String>>,
    inputs: Inputs,
}

impl Page {
    /// Apply a finance entry: on success add it and clear `fields`, else
    /// show why not.
    fn finance<T>(
        self,
        entry: Result<T, entries::EntryError>,
        fields: &[RwSignal<String>],
        add: impl FnOnce(&mut bruhs_models::FinanceLedger, T),
    ) {
        match entry {
            Ok(record) => {
                let now = now_ms();
                self.life.update(|context| {
                    add(&mut context.finance, record);
                    context.finance.updated_at_ms = now;
                });
                clear(fields);
                self.notice.set(None);
            }
            Err(error) => self.notice.set(Some(error.to_string())),
        }
    }

    /// Apply a health entry when the form holds one; otherwise do nothing.
    fn health<T>(
        self,
        entry: Option<T>,
        fields: &[RwSignal<String>],
        add: impl FnOnce(&mut bruhs_models::HealthJournal, T),
    ) {
        if let Some(record) = entry {
            let now = now_ms();
            self.life.update(|context| {
                add(&mut context.health, record);
                context.health.updated_at_ms = now;
            });
            clear(fields);
        }
    }

    fn add_account(self) {
        let i = self.inputs;
        let entry = entries::account(
            i.account_name.get_untracked(),
            &i.account_balance.get_untracked(),
            now_ms(),
        );
        self.finance(entry, &[i.account_name, i.account_balance], |f, a| {
            f.accounts.insert(0, a)
        });
    }

    fn add_spend(self) {
        let i = self.inputs;
        let entry = entries::spend(
            i.merchant.get_untracked(),
            &i.spend_amount.get_untracked(),
            i.spend_category.get_untracked(),
            now_ms(),
        );
        self.finance(
            entry,
            &[i.merchant, i.spend_amount, i.spend_category],
            |f, t| f.transactions.insert(0, t),
        );
    }

    fn add_budget(self) {
        let i = self.inputs;
        let entry = entries::budget(
            i.budget_category.get_untracked(),
            &i.budget_limit.get_untracked(),
        );
        self.finance(entry, &[i.budget_category, i.budget_limit], |f, b| {
            f.budgets.insert(0, b)
        });
    }

    fn add_income(self) {
        let i = self.inputs;
        let entry = entries::income(
            i.income_name.get_untracked(),
            &i.income_amount.get_untracked(),
        );
        self.finance(entry, &[i.income_name, i.income_amount], |f, s| {
            f.income_streams.insert(0, s)
        });
    }

    fn add_holding(self) {
        let i = self.inputs;
        let entry = entries::holding(
            i.holding_symbol.get_untracked(),
            &i.holding_value.get_untracked(),
            now_ms(),
        );
        self.finance(entry, &[i.holding_symbol, i.holding_value], |f, h| {
            f.holdings.insert(0, h)
        });
    }

    fn import_csv(self) {
        let now = now_ms();
        let imported = entries::csv_transactions(&self.inputs.csv.get_untracked(), now);
        let count = imported.len();
        if count == 0 {
            self.notice.set(Some("No valid CSV rows found.".into()));
            return;
        }
        self.life.update(|context| {
            context.finance.transactions.splice(0..0, imported);
            context.finance.updated_at_ms = now;
        });
        self.inputs.csv.set(String::new());
        self.notice
            .set(Some(format!("Imported {count} transactions locally.")));
    }

    fn log_workout(self) {
        let i = self.inputs;
        let entry = entries::workout(
            i.workout_activity.get_untracked(),
            &i.workout_minutes.get_untracked(),
            now_ms(),
        );
        self.health(entry, &[i.workout_activity, i.workout_minutes], |h, w| {
            h.workouts.insert(0, w)
        });
    }

    fn log_sleep(self) {
        let i = self.inputs;
        let entry = entries::sleep(
            &i.sleep_hours.get_untracked(),
            &i.sleep_quality.get_untracked(),
            now_ms(),
        );
        self.health(entry, &[i.sleep_hours, i.sleep_quality], |h, s| {
            h.sleep.insert(0, s)
        });
    }

    fn log_recovery(self) {
        let i = self.inputs;
        let entry = entries::recovery(&i.recovery_score.get_untracked(), now_ms());
        self.health(entry, &[i.recovery_score], |h, r| h.recovery.insert(0, r));
    }

    fn add_habit(self) {
        let i = self.inputs;
        let entry = entries::habit(i.habit_title.get_untracked());
        self.health(entry, &[i.habit_title], |h, habit| {
            h.habits.insert(0, habit)
        });
    }

    fn toggle_finance_agent_access(self) {
        self.life.update(|context| {
            let privacy = &mut context.finance.privacy;
            privacy.allow_agent_context = !privacy.allow_agent_context;
            if !privacy.allow_agent_context {
                privacy.reveal_exact_amounts_to_agent = false;
            }
            context.finance.updated_at_ms = now_ms();
        });
    }

    fn toggle_exact_amounts(self) {
        self.life.update(|context| {
            let privacy = &mut context.finance.privacy;
            privacy.reveal_exact_amounts_to_agent = !privacy.reveal_exact_amounts_to_agent;
            context.finance.updated_at_ms = now_ms();
        });
    }

    fn toggle_health_agent_access(self) {
        self.life.update(|context| {
            let privacy = &mut context.health.privacy;
            privacy.allow_agent_context = !privacy.allow_agent_context;
            context.health.updated_at_ms = now_ms();
        });
    }
}

/// A text field bound to `value`.
fn field(
    label: &'static str,
    placeholder: &'static str,
    value: RwSignal<String>,
    inputmode: Option<&'static str>,
) -> impl IntoView {
    view! {
        <input
            aria-label=label
            inputmode=inputmode
            class=INPUT_CLASS
            placeholder=placeholder
            prop:value=move || value.get()
            on:input=move |event| value.set(event_target_value(&event))
        />
    }
}

/// [`field`] under a visible caption.
fn labelled_field(
    caption: &'static str,
    label: &'static str,
    placeholder: &'static str,
    value: RwSignal<String>,
    inputmode: Option<&'static str>,
    class: &'static str,
) -> impl IntoView {
    view! {
        <label class=class>{caption}{field(label, placeholder, value, inputmode)}</label>
    }
}

fn stat_card(caption: &'static str, value: impl IntoView + 'static) -> impl IntoView {
    view! {
        <article class=CARD_CLASS>
            <p class="text-label-sm text-rambutan-500">{caption}</p>
            <p class="mt-1 [overflow-wrap:anywhere] font-mono text-h3 text-rambutan-100">{value}</p>
        </article>
    }
}

fn health_stat_card(caption: &'static str, value: impl IntoView + 'static) -> impl IntoView {
    view! {
        <article class=CARD_CLASS>
            <p class="text-label-sm text-rambutan-500">{caption}</p>
            <p class="mt-1 text-h3 text-rambutan-100">{value}</p>
        </article>
    }
}

fn sources(adapters: Vec<LifeProviderAdapter>, header_gap: &'static str) -> impl IntoView {
    adapters
        .into_iter()
        .map(|provider| {
            view! {
                <div class="rounded-seed bg-longan-950/45 p-2.5">
                    <div class=header_gap>
                        <p class="text-body-sm text-rambutan-300">{provider.name}</p>
                        <span class=format!("text-label-sm {}", provider_tone(provider.status))>
                            {provider_label(provider.status)}
                        </span>
                    </div>
                    <p class="mt-1 text-label-sm text-rambutan-400">{provider.privacy_note}</p>
                </div>
            }
        })
        .collect_view()
}

#[component]
pub fn LifePage() -> impl IntoView {
    let LifeContextSignal(life) = expect_context::<LifeContextSignal>();
    let WorkItemsSignal(tasks) = expect_context::<WorkItemsSignal>();
    let ComposerDraft(composer_draft) = expect_context::<ComposerDraft>();
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let LifeSectionSignal(tab) = expect_context::<LifeSectionSignal>();
    let notice = RwSignal::new(Option::<String>::None);
    let page = Page {
        life,
        notice,
        inputs: Inputs::new(),
    };

    let ask_agent = move |_| {
        let context = life.get_untracked();
        let prompt = tasks.with_untracked(|tasks| life_agent_prompt(&context, tasks, now_ms()));
        match prompt {
            Ok(prompt) => {
                composer_draft.set(prompt);
                set_active_view.set(AppView::Chat);
            }
            Err(message) => notice.set(Some(message.to_string())),
        }
    };
    let tab_button = move |section: LifeSection, name: &'static str| {
        view! {
            <button
                type="button"
                role="tab"
                aria-selected=move || (tab.get() == section).to_string()
                class=move || if tab.get() == section { TAB_ON } else { TAB_OFF }
                on:click=move |_| tab.set(section)
            >
                {name}
            </button>
        }
    };

    view! {
        <section class="flex min-h-0 flex-1 flex-col bg-longan-950 text-rambutan-300" aria-labelledby="life-title">
            <header class="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b border-longan-800/50 px-4 py-3 sm:px-6">
                <div>
                    <h1 id="life-title" tabindex="-1" class="text-h3 text-rambutan-100">"Life context"</h1>
                    <p class="mt-0.5 text-body-sm text-rambutan-500">
                        "Manual first, private by default, useful only with your consent."
                    </p>
                </div>
                <div class="flex min-w-0 max-w-full flex-wrap items-center gap-2">
                    <div class="flex rounded-grape bg-longan-900 p-0.5 inset-ring inset-ring-longan-800" role="tablist" aria-label="Life context section">
                        {tab_button(LifeSection::Finance, "Finance")}
                        {tab_button(LifeSection::Health, "Health")}
                    </div>
                    <crate::ui::Button
                        size=crate::ui::ButtonSize::Small
                        disabled=Signal::derive(move || !life.ready())
                        on_click=Callback::new(ask_agent)
                    >
                        "Ask agent"
                    </crate::ui::Button>
                </div>
            </header>
            <super::metadata_status::MetadataStatus
                status=life.status
                retry=Callback::new(move |_| life.retry())
                recovery=Signal::derive(move || life.recovery_json())
            />
            <div class="flex-1 overflow-y-auto px-4 py-4 sm:px-6">
                <record_detail::LifeRecordDetail />
                <fieldset disabled=move || !life.editable() class="m-0 min-w-0 border-0 p-0">
                    <div class="mx-auto max-w-6xl space-y-4">
                        <Show when=move || notice.get().is_some()>
                            <div role="status" class="flex min-w-0 flex-wrap items-center justify-between gap-2 rounded-seed bg-blueberry-500/10 px-3 py-2 text-body-sm text-blueberry-200">
                                <span>{move || notice.get().unwrap_or_default()}</span>
                                <button
                                    type="button"
                                    class="min-h-[44px] min-w-[44px] sm:min-h-6 sm:min-w-6 text-label-sm cursor-pointer"
                                    on:click=move |_| notice.set(None)
                                >
                                    "Dismiss"
                                </button>
                            </div>
                        </Show>
                        <Show when=move || tab.get() == LifeSection::Finance>{finance_tab(page)}</Show>
                        <Show when=move || tab.get() == LifeSection::Health>{health_tab(page)}</Show>
                        {insights(life, tasks)}
                    </div>
                </fieldset>
            </div>
        </section>
    }
}

fn finance_tab(page: Page) -> impl IntoView {
    let life = page.life;
    let totals = move |f: fn(&bruhs_models::FinanceLedger) -> String| {
        move || life.with(|context| f(&context.finance))
    };
    view! {
        <p class="text-body-sm text-rambutan-500">
            "Manual amounts are USD. Other currencies are listed separately in minor units; no exchange rate is applied."
        </p>
        <div class="grid gap-3 md:grid-cols-3">
            {stat_card(
                "Manual net worth",
                totals(|f| bruhs_models::format_money_totals(f.net_worth_totals())),
            )}
            {stat_card(
                "Spend · trailing 30 days",
                totals(|f| bruhs_models::format_money_totals(f.trailing_spend_totals(now_ms(), 30))),
            )}
            {stat_card(
                "Expected monthly income",
                totals(|f| bruhs_models::format_money_totals(f.monthly_income_totals())),
            )}
        </div>
        {finance_privacy(page)}
        <div class="grid gap-3 lg:grid-cols-2">
            {accounts(page)}
            {transactions(page)}
            {budgets_and_income(page)}
            {holdings(page)}
        </div>
        {csv_import(page)}
        <section class=CARD_CLASS>
            <h2 class="text-label text-rambutan-200">"Finance sources"</h2>
            <div class="mt-2 grid gap-2 md:grid-cols-3">
                {sources(finance_provider_adapters(), "flex justify-between gap-2")}
            </div>
        </section>
    }
}

fn finance_privacy(page: Page) -> impl IntoView {
    let life = page.life;
    let agent_access = move || life.with(|context| context.finance.privacy.allow_agent_context);
    let exact = move || life.with(|context| context.finance.privacy.reveal_exact_amounts_to_agent);
    view! {
        <section class=CARD_CLASS aria-labelledby="finance-privacy-title">
            <div class="flex flex-wrap items-center justify-between gap-3">
                <div>
                    <h2 id="finance-privacy-title" class="text-label text-rambutan-200">"Finance privacy"</h2>
                    <p class="mt-0.5 text-body-sm text-rambutan-500">
                        "Records remain local. Agent access is off until you enable it."
                    </p>
                </div>
                <div class="flex flex-wrap gap-2">
                    <button
                        type="button"
                        aria-pressed=move || agent_access().to_string()
                        class="min-h-[44px] min-w-[44px] sm:min-h-6 sm:min-w-6 rounded-seed px-2.5 py-1.5 text-label-sm text-blueberry-300 inset-ring inset-ring-blueberry-500/25 cursor-pointer"
                        on:click=move |_| page.toggle_finance_agent_access()
                    >
                        {move || if agent_access() { "Agent access on" } else { "Agent access off" }}
                    </button>
                    <button
                        type="button"
                        disabled=move || !agent_access()
                        aria-pressed=move || exact().to_string()
                        class="min-h-[44px] min-w-[44px] sm:min-h-6 sm:min-w-6 rounded-seed px-2.5 py-1.5 text-label-sm text-rambutan-400 inset-ring inset-ring-longan-700 disabled:opacity-40 cursor-pointer"
                        on:click=move |_| page.toggle_exact_amounts()
                    >
                        {move || if exact() { "Exact amounts shared" } else { "Amounts redacted" }}
                    </button>
                </div>
            </div>
        </section>
    }
}

fn accounts(page: Page) -> impl IntoView {
    let (life, i) = (page.life, page.inputs);
    let rows = move || {
        life.with(|context| context.finance.accounts.clone())
            .into_iter()
            .map(|account| {
                let id = account.id.clone();
                let delete = move |_| {
                    life.update(|context| {
                        context.finance.accounts.retain(|item| item.id != id);
                        context.finance.updated_at_ms = now_ms();
                    })
                };
                view! {
                    <div class=ROW>
                        <div class="min-w-0 [overflow-wrap:anywhere]">
                            <p class="text-body-sm text-rambutan-300">{account.name}</p>
                            <p class="text-label-sm text-rambutan-400">{account.kind.label()}</p>
                        </div>
                        <div class="flex min-w-0 max-w-full flex-wrap items-center gap-2">
                            <span class=ROW_AMOUNT>{money(account.balance_minor, &account.currency)}</span>
                            <button
                                type="button"
                                aria-label="Delete account"
                                class="min-h-[44px] min-w-[44px] sm:min-h-6 sm:min-w-6 text-label-sm text-rambutan-400 hover:text-lychee-300 cursor-pointer"
                                on:click=delete
                            >
                                "×"
                            </button>
                        </div>
                    </div>
                }
            })
            .collect_view()
    };
    let caption = "text-label-sm text-rambutan-500";
    view! {
        <section class=CARD_CLASS>
            <h2 class="text-label text-rambutan-200">"Accounts"</h2>
            <div class="mt-2 grid grid-cols-1 sm:grid-cols-[minmax(0,1fr)_8rem_auto] gap-2">
                {labelled_field("Name", "Account name", "Checking", i.account_name, None, caption)}
                {labelled_field("Balance", "Account balance", "2500.00", i.account_balance, Some("decimal"), caption)}
                <button
                    type="button"
                    class="mt-5 inline-flex min-h-[44px] min-w-[44px] shrink-0 items-center justify-center rounded-grape bg-longan-800 px-3 text-label-sm text-rambutan-200 transition-colors hover:bg-longan-700 cursor-pointer sm:h-9 sm:min-h-9"
                    on:click=move |_| page.add_account()
                >
                    "Add"
                </button>
            </div>
            <div class="mt-3 space-y-1">{rows}</div>
        </section>
    }
}

fn transactions(page: Page) -> impl IntoView {
    let (life, i) = (page.life, page.inputs);
    let rows = move || {
        life.with(|context| context.finance.transactions.iter().take(8).cloned().collect::<Vec<_>>())
            .into_iter()
            .map(|transaction| {
                let id = transaction.id.clone();
                let delete = move |_| {
                    life.update(|context| {
                        context.finance.transactions.retain(|item| item.id != id);
                        context.finance.updated_at_ms = now_ms();
                    })
                };
                view! {
                    <div class=ROW>
                        <div class="min-w-0 [overflow-wrap:anywhere]">
                            <p class="text-body-sm text-rambutan-300">{transaction.merchant}</p>
                            <p class="text-label-sm text-rambutan-400">{transaction.category}</p>
                        </div>
                        <div class="flex min-w-0 max-w-full flex-wrap items-center gap-2">
                            <span class=ROW_AMOUNT>{money(transaction.amount_minor, &transaction.currency)}</span>
                            <button
                                type="button"
                                aria-label="Delete transaction"
                                class="min-h-[44px] min-w-[44px] sm:min-h-6 sm:min-w-6 text-rambutan-400 hover:text-lychee-300 cursor-pointer"
                                on:click=delete
                            >
                                "×"
                            </button>
                        </div>
                    </div>
                }
            })
            .collect_view()
    };
    let caption = "text-label-sm text-rambutan-500";
    view! {
        <section class=CARD_CLASS>
            <h2 class="text-label text-rambutan-200">"Transactions"</h2>
            <div class="mt-2 grid grid-cols-2 gap-2">
                {labelled_field("Merchant", "Transaction merchant", "Groceries", i.merchant, None, caption)}
                {labelled_field("Spend amount", "Transaction spend amount", "42.00", i.spend_amount, Some("decimal"), caption)}
                {labelled_field("Category", "Transaction category", "Food", i.spend_category, None, "col-span-2 text-label-sm text-rambutan-500")}
            </div>
            <button type="button" class=ADD_BUTTON on:click=move |_| page.add_spend()>
                "Add spend"
            </button>
            <div class="mt-3 space-y-1">{rows}</div>
        </section>
    }
}

fn budgets_and_income(page: Page) -> impl IntoView {
    let (life, i) = (page.life, page.inputs);
    let summary = move || {
        format!(
            "{} budgets · {} income streams",
            life.with(|context| context.finance.budgets.len()),
            life.with(|context| context.finance.income_streams.len())
        )
    };
    view! {
        <section class=CARD_CLASS>
            <h2 class="text-label text-rambutan-200">"Budgets & income"</h2>
            <div class="mt-2 grid grid-cols-1 sm:grid-cols-[minmax(0,1fr)_8rem_auto] gap-2">
                {field("Budget category", "Food", i.budget_category, None)}
                {field("Monthly budget limit", "500", i.budget_limit, None)}
                <button type="button" class=SMALL_ACTION on:click=move |_| page.add_budget()>"Budget"</button>
                {field("Income stream name", "Salary", i.income_name, None)}
                {field("Expected monthly income", "5000", i.income_amount, None)}
                <button type="button" class=SMALL_ACTION on:click=move |_| page.add_income()>"Income"</button>
            </div>
            <div class="mt-3 text-body-sm text-rambutan-500">{summary}</div>
        </section>
    }
}

fn holdings(page: Page) -> impl IntoView {
    let (life, i) = (page.life, page.inputs);
    let chips = move || {
        life.with(|context| context.finance.holdings.clone())
            .into_iter()
            .map(|holding| {
                view! {
                    <span class="max-w-full [overflow-wrap:anywhere] rounded-seed bg-longan-950/45 px-2.5 py-1.5 font-mono text-body-sm text-rambutan-300">
                        {format!("{} · {}", holding.symbol, money(holding.current_value_minor, &holding.currency))}
                    </span>
                }
            })
            .collect_view()
    };
    view! {
        <section class=CARD_CLASS>
            <h2 class="text-label text-rambutan-200">"Holdings"</h2>
            <div class="mt-2 grid grid-cols-1 sm:grid-cols-[minmax(0,1fr)_8rem_auto] gap-2">
                {field("Holding symbol", "Symbol", i.holding_symbol, None)}
                {field("Holding current value", "Value", i.holding_value, None)}
                <button type="button" class=SMALL_ACTION on:click=move |_| page.add_holding()>"Add"</button>
            </div>
            <div class="mt-3 flex flex-wrap gap-2">{chips}</div>
        </section>
    }
}

fn csv_import(page: Page) -> impl IntoView {
    let csv = page.inputs.csv;
    view! {
        <section class=CARD_CLASS>
            <h2 class="text-label text-rambutan-200">"CSV import"</h2>
            <p class="mt-1 text-body-sm text-rambutan-500">
                "Paste local rows as merchant, category, signed amount, optional note. Nothing is uploaded."
            </p>
            <textarea
                aria-label="Finance CSV rows"
                class="mt-2 min-h-24 w-full resize-y rounded-grape bg-longan-950/70 p-2.5 font-mono text-body-sm text-rambutan-300 outline-none inset-ring inset-ring-longan-700 focus:inset-ring-persimmon-400"
                placeholder="Coffee, Food, -5.25, client meeting"
                prop:value=move || csv.get()
                on:input=move |event| csv.set(event_target_value(&event))
            ></textarea>
            <button type="button" class=ADD_BUTTON on:click=move |_| page.import_csv()>
                "Import rows"
            </button>
        </section>
    }
}

fn health_tab(page: Page) -> impl IntoView {
    let life = page.life;
    let agent_access = move || life.with(|context| context.health.privacy.allow_agent_context);
    let latest_recovery = move || {
        life.with(|context| {
            context
                .health
                .recovery
                .first()
                .map_or_else(|| "—".into(), |log| format!("{}%", log.score_percent))
        })
    };
    let active_habits = move || {
        life.with(|context| {
            context
                .health
                .habits
                .iter()
                .filter(|habit| !habit.archived)
                .count()
        })
    };
    view! {
        <div class="grid gap-3 md:grid-cols-3">
            {health_stat_card("Workouts logged", move || life.with(|context| context.health.workouts.len()))}
            {health_stat_card("Latest recovery", latest_recovery)}
            {health_stat_card("Active habits", active_habits)}
        </div>
        <section class=CARD_CLASS>
            <div class="flex flex-wrap items-center justify-between gap-3">
                <div>
                    <h2 class="text-label text-rambutan-200">"Health privacy"</h2>
                    <p class="mt-0.5 text-body-sm text-rambutan-500">
                        "Local logs are excluded from agent context by default. This surface never diagnoses."
                    </p>
                </div>
                <button
                    type="button"
                    aria-pressed=move || agent_access().to_string()
                    class="min-h-[44px] min-w-[44px] sm:min-h-6 sm:min-w-6 rounded-seed px-2.5 py-1.5 text-label-sm text-blueberry-300 inset-ring inset-ring-blueberry-500/25 cursor-pointer"
                    on:click=move |_| page.toggle_health_agent_access()
                >
                    {move || if agent_access() { "Agent access on" } else { "Agent access off" }}
                </button>
            </div>
        </section>
        <div class="grid gap-3 lg:grid-cols-2">
            {workouts(page)}
            {sleep_and_recovery(page)}
            {habits(page)}
            <section class=CARD_CLASS>
                <h2 class="text-label text-rambutan-200">"Health sources"</h2>
                <div class="mt-2 space-y-2">{sources(health_provider_adapters(), "flex justify-between")}</div>
            </section>
        </div>
    }
}

fn workouts(page: Page) -> impl IntoView {
    let (life, i) = (page.life, page.inputs);
    let rows = move || {
        life.with(|context| context.health.workouts.iter().take(6).cloned().collect::<Vec<_>>())
            .into_iter()
            .map(|workout| {
                view! {
                    <div class="rounded-seed bg-longan-950/45 px-2.5 py-2 text-body-sm text-rambutan-300">
                        {format!(
                            "{} · {} min · effort {}/10",
                            workout.activity,
                            workout.duration_minutes,
                            workout.perceived_exertion
                        )}
                    </div>
                }
            })
            .collect_view()
    };
    view! {
        <section class=CARD_CLASS>
            <h2 class="text-label text-rambutan-200">"Workout"</h2>
            <div class="mt-2 grid grid-cols-[1fr_7rem_auto] gap-2">
                {field("Workout activity", "Run", i.workout_activity, None)}
                {field("Workout duration minutes", "Minutes", i.workout_minutes, Some("numeric"))}
                <button
                    type="button"
                    class="min-h-[44px] min-w-[44px] sm:min-h-6 sm:min-w-6 mt-1 rounded-seed bg-pandan-500/10 px-2 text-label-sm text-pandan-300 cursor-pointer"
                    on:click=move |_| page.log_workout()
                >
                    "Log"
                </button>
            </div>
            <div class="mt-3 space-y-1">{rows}</div>
        </section>
    }
}

fn sleep_and_recovery(page: Page) -> impl IntoView {
    let (life, i) = (page.life, page.inputs);
    let summary = move || {
        format!(
            "{} sleep logs · {} recovery logs",
            life.with(|context| context.health.sleep.len()),
            life.with(|context| context.health.recovery.len())
        )
    };
    view! {
        <section class=CARD_CLASS>
            <h2 class="text-label text-rambutan-200">"Sleep & recovery"</h2>
            <div class="mt-2 grid grid-cols-1 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] gap-2">
                {field("Sleep hours", "Hours", i.sleep_hours, Some("decimal"))}
                {field("Sleep quality percent", "Quality %", i.sleep_quality, Some("numeric"))}
                <button type="button" class=SMALL_ACTION on:click=move |_| page.log_sleep()>"Sleep"</button>
                {field("Recovery score percent", "Recovery score %", i.recovery_score, Some("numeric"))}
                <span></span>
                <button type="button" class=SMALL_ACTION on:click=move |_| page.log_recovery()>"Recovery"</button>
            </div>
            <p class="mt-3 text-body-sm text-rambutan-500">{summary}</p>
        </section>
    }
}

fn habits(page: Page) -> impl IntoView {
    let (life, i) = (page.life, page.inputs);
    let rows = move || {
        let today = now_ms().div_euclid(86_400_000);
        life.with(|context| {
            context
                .health
                .habits
                .iter()
                .filter(|habit| !habit.archived)
                .cloned()
                .collect::<Vec<_>>()
        })
        .into_iter()
        .map(|habit| {
            let id = habit.id.clone();
            let done = habit.completion_days.contains(&today);
            let toggle = move |_| {
                life.update(|context| {
                    if let Some(habit) = context.health.habits.iter_mut().find(|habit| habit.id == id) {
                        habit.toggle_day(now_ms());
                        context.health.updated_at_ms = now_ms();
                    }
                })
            };
            view! {
                <button
                    type="button"
                    aria-pressed=done.to_string()
                    class=if done {
                        "flex w-full items-center justify-between rounded-seed bg-pandan-500/10 px-2.5 py-2 text-left text-body-sm text-pandan-300 cursor-pointer"
                    } else {
                        "flex w-full items-center justify-between rounded-seed bg-longan-950/45 px-2.5 py-2 text-left text-body-sm text-rambutan-300 cursor-pointer"
                    }
                    on:click=toggle
                >
                    <span>{habit.title}</span>
                    <span>{if done { "Done today" } else { "Mark done" }}</span>
                </button>
            }
        })
        .collect_view()
    };
    view! {
        <section class=CARD_CLASS>
            <h2 class="text-label text-rambutan-200">"Habits"</h2>
            <div class="mt-2 grid grid-cols-[1fr_auto] gap-2">
                {field("New habit title", "Walk after lunch", i.habit_title, None)}
                <button
                    type="button"
                    class=SMALL_ACTION
                    on:click=move |_| page.add_habit()
                >
                    "Add"
                </button>
            </div>
            <div class="mt-3 space-y-1">{rows}</div>
        </section>
    }
}

fn insights(
    life: DurableMetadata<LifeContext>,
    tasks: ReadSignal<Vec<bruhs_models::WorkItem>>,
) -> impl IntoView {
    let cards = move || {
        let now = now_ms();
        let insights = life.with(|context| tasks.with(|tasks| context.insights(tasks, now)));
        if insights.is_empty() {
            return view! {
                <p class="text-body-sm text-rambutan-400">
                    "Add a few consistent records and bruhs will show exactly what evidence supports each observation."
                </p>
            }
            .into_any();
        }
        insights
            .into_iter()
            .map(|insight| {
                let id = insight.id.clone();
                let tone = match insight.domain {
                    LifeInsightDomain::Finance => "text-durian-300",
                    LifeInsightDomain::Health => "text-pandan-300",
                    LifeInsightDomain::Productivity => "text-blueberry-300",
                };
                view! {
                    <article class="rounded-seed bg-longan-950/45 p-3">
                        <div class="flex items-start justify-between gap-2">
                            <h3 class=format!("text-label {}", tone)>{insight.title}</h3>
                            <button
                                type="button"
                                aria-label="Dismiss observation"
                                class="min-h-[44px] min-w-[44px] sm:min-h-6 sm:min-w-6 text-rambutan-400 hover:text-rambutan-300 cursor-pointer"
                                on:click=move |_| life.update(|context| context.dismiss_insight(id.clone()))
                            >
                                "×"
                            </button>
                        </div>
                        <p class="mt-2 text-body-sm text-rambutan-300">{insight.suggestion}</p>
                        <p class="mt-2 text-label-sm text-rambutan-500">{format!("Evidence: {}", insight.evidence)}</p>
                        <p class="mt-1 text-label-sm text-rambutan-400">{insight.disclaimer}</p>
                    </article>
                }
            })
            .collect_view()
            .into_any()
    };
    view! {
        <section class=CARD_CLASS aria-labelledby="insights-title">
            <div class="flex items-start justify-between gap-3">
                <div>
                    <h2 id="insights-title" class="text-label text-rambutan-200">"Evidence-labelled observations"</h2>
                    <p class="mt-1 text-body-sm text-rambutan-500">
                        "Local comparisons only. Never diagnoses, causal claims, or autonomous financial actions."
                    </p>
                </div>
            </div>
            <div class="mt-3 grid gap-2 lg:grid-cols-2">{cards}</div>
        </section>
    }
}
