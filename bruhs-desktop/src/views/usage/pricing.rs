use super::*;
use bruhs_core::agent_extensions::{directory, edit_usage_model_prices_at, usage_model_pricing_at};
use bruhs_core::delivery::writer::desktop_state_file;
use bruhs_core::usage_hubs::linked::{self, PriceDestination};
use bruhs_models::agent_extensions::{
    valid_usage_model_id, TokenPrice, UsageModelPricing, UsageModelPricingChange,
};
use bruhs_ui::checkbox::Checkbox;
use bruhs_ui::input::{Input, InputEvent, InputState};
use futures_util::{stream, StreamExt};
use std::{cell::Cell, collections::BTreeSet};

mod batch;
use batch::{number, rates, Attempt, Batch, PriceEdit, PriceRows, RateEdit, RateValue};

const RATE_LABELS: [&str; 4] = ["Input", "Output", "Cache read", "Cache write"];

#[derive(Clone)]
enum PriceTarget {
    Local,
    Linked(PriceDestination),
}

impl PriceTarget {
    fn name(&self) -> &str {
        match self {
            Self::Local => LOCAL,
            Self::Linked(destination) => &destination.name,
        }
    }

    async fn discover() -> (Vec<Self>, Option<String>) {
        let mut targets = vec![Self::Local];
        match linked::price_destinations(desktop_state_file().as_deref()).await {
            Ok(destinations) => {
                let mut seen = BTreeSet::new();
                targets.extend(
                    destinations
                        .into_iter()
                        .filter(|d| seen.insert(d.id))
                        .map(Self::Linked),
                );
                (targets, None)
            }
            Err(_) => (targets, Some("Could not read linked environments.".into())),
        }
    }

    async fn tunnels(
        destination: &PriceDestination,
    ) -> Option<bruhs_models::remote_hosts::RemoteHostSnapshot> {
        if destination.needs_tunnel() {
            ServerClient::workbench_client()
                .remote_hosts(None)
                .await
                .ok()
        } else {
            None
        }
    }

    async fn load(&self) -> Result<PriceRows, String> {
        match self {
            Self::Local => tokio::task::spawn_blocking(|| {
                directory()
                    .and_then(|root| usage_model_pricing_at(&root))
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())?,
            Self::Linked(target) => {
                let path = desktop_state_file().ok_or("The environments store is unavailable.")?;
                let tunnels = Self::tunnels(target).await;
                linked::read_model_prices(&path, target, tunnels.as_ref()).await
            }
        }
    }

    async fn save(&self, changes: Vec<UsageModelPricingChange>) -> Result<(), String> {
        match self {
            Self::Local => tokio::task::spawn_blocking(move || {
                directory()
                    .and_then(|root| edit_usage_model_prices_at(&root, changes))
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())?,
            Self::Linked(target) => {
                let path = desktop_state_file().ok_or("The environments store is unavailable.")?;
                let tunnels = Self::tunnels(target).await;
                linked::save_model_prices(&path, target, tunnels.as_ref(), &changes).await
            }
        }
    }

    async fn apply(
        &self,
        attempt: Attempt,
    ) -> (
        Option<PriceRows>,
        Result<Vec<UsageModelPricingChange>, String>,
    ) {
        let fetched;
        let rows = if let Some(rows) = &attempt.rows {
            rows
        } else {
            fetched = match self.load().await {
                Ok(rows) => rows,
                Err(error) => return (None, Err(error)),
            };
            &fetched
        };
        let result = async {
            let changes = attempt.changes(rows)?;
            if !changes.is_empty() {
                self.save(changes.clone()).await?;
            }
            Ok(changes)
        }
        .await;
        (attempt.rows.is_none().then(|| rows.clone()), result)
    }
}

fn initial_selection(
    targets: &[PriceTarget],
    scope: Option<&str>,
) -> (BTreeSet<usize>, Option<String>) {
    if let Some(name) = scope {
        let mut matches = targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.name() == name);
        if let Some((index, _)) = matches.next() {
            if name == LOCAL || matches.next().is_none() {
                return (BTreeSet::from([index]), None);
            }
            return (
                BTreeSet::new(),
                Some(
                    "More than one environment has this name. Choose where to apply changes."
                        .into(),
                ),
            );
        }
        return (BTreeSet::new(), Some("Link an environment grant that allows editing model prices, or choose another destination.".into()));
    }
    ((0..targets.len()).collect(), None)
}

fn price_summary(row: &UsageModelPricing) -> String {
    if let Some(alias) = &row.alias {
        format!("Maps to {alias}")
    } else if let Some(price) = &row.price {
        format!(
            "${} input / ${} output",
            price.input_per_million, price.output_per_million
        )
    } else {
        "Automatic".into()
    }
}

fn saved_price_summary(row: &UsageModelPricing) -> String {
    let Some(price) = row.price.as_ref().filter(|_| row.alias.is_none()) else {
        return price_summary(row);
    };
    let rates = |rates: TokenPrice| {
        format!(
            "${} input / ${} output / ${} cache read / ${} cache write",
            rates.input_per_million,
            rates.output_per_million,
            rates.cache_read_per_million,
            rates.cache_write_per_million
        )
    };
    let standard = rates(price.standard());
    match price.fast {
        Some(fast) => format!("Standard {standard}. Fast {}.", rates(fast)),
        None => standard,
    }
}

pub(super) struct PricePage {
    targets: Vec<PriceTarget>,
    batch: Batch,
    back_focus: FocusHandle,
    target_focus: FocusHandle,
    scroll: ScrollHandle,
    request: Option<Task<()>>,
    form: Option<PriceForm>,
    choosing_targets: bool,
    loading: bool,
    saving: bool,
    error: Option<String>,
}

impl PricePage {
    pub(super) fn saving(&self) -> bool {
        self.saving
    }

    fn selection_label(&self) -> String {
        let count = self.batch.selected.len();
        match count {
            0 => "Choose environments".into(),
            1 => self.targets[*self
                .batch
                .selected
                .first()
                .expect("one selected environment")]
            .name()
            .into(),
            _ if count == self.targets.len() => "All environments".into(),
            _ => format!("{count} environments"),
        }
    }

    fn targets_locked(&self) -> bool {
        self.loading || self.saving || !self.batch.drafts.is_empty()
    }
}

#[derive(Default)]
struct PriceFields {
    alias: Option<String>,
    standard: [Option<String>; 4],
    fast: [Option<String>; 4],
    has_settings: bool,
    has_fast: bool,
}

impl PriceFields {
    fn from_rows(rows: &[UsageModelPricing]) -> Self {
        let common = |value: &dyn Fn(&UsageModelPricing) -> String| {
            let mut values = rows.iter().map(value);
            let first = values.next().unwrap_or_default();
            values.all(|cell| cell == first).then_some(first)
        };
        let tier = |fast: bool| {
            std::array::from_fn(|index| {
                common(&|row| {
                    row.price
                        .as_ref()
                        .and_then(|price| {
                            if fast {
                                price.fast
                            } else {
                                Some(price.standard())
                            }
                        })
                        .map_or_else(String::new, |price| rates(price)[index].to_string())
                })
            })
        };
        Self {
            alias: common(&|row| row.alias.clone().unwrap_or_default()),
            standard: tier(false),
            fast: tier(true),
            has_settings: rows.iter().any(|row| *row != UsageModelPricing::default()),
            has_fast: rows
                .iter()
                .any(|row| row.price.as_ref().is_some_and(|price| price.fast.is_some())),
        }
    }
}

struct PriceForm {
    model: Entity<InputState>,
    existing_model: Option<String>,
    alias: Entity<InputState>,
    standard: [Entity<InputState>; 4],
    fast: [Entity<InputState>; 4],
    initial: PriceFields,
    reset: bool,
    clear_alias: bool,
    remove_fast: bool,
    touched_rates: Rc<Cell<u8>>,
    _input_observers: Vec<Subscription>,
}

fn input(
    value: Option<String>,
    window: &mut Window,
    cx: &mut Context<'_, UsageWindow>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let input = InputState::new(window, cx);
        match value {
            Some(value) => input.default_value(value),
            None => input.placeholder("Mixed"),
        }
    })
}

fn edit_rates(
    fields: &[Entity<InputState>; 4],
    initial: &[Option<String>; 4],
    reset: bool,
    touched: u8,
    cx: &App,
) -> Result<RateEdit, &'static str> {
    let mut edit = RateEdit {
        reset,
        ..Default::default()
    };
    for (index, field) in fields.iter().enumerate() {
        let value = field.read(cx).value();
        let value = value.trim();
        let original = if reset {
            ""
        } else {
            initial[index].as_deref().unwrap_or_default()
        };
        let cleared_mixed_cache =
            index >= 2 && initial[index].is_none() && touched & (1 << index) != 0;
        if value == original && !(value.is_empty() && cleared_mixed_cache) {
            continue;
        }
        edit.fields[index] = Some(if index >= 2 && value.is_empty() {
            RateValue::Input
        } else {
            RateValue::Number(number(value)?)
        });
    }
    Ok(edit)
}

impl PriceForm {
    fn new(
        model: Option<String>,
        initial: PriceFields,
        scroll: &ScrollHandle,
        window: &mut Window,
        cx: &mut Context<'_, UsageWindow>,
    ) -> Self {
        let standard =
            std::array::from_fn(|index| input(initial.standard[index].clone(), window, cx));
        let fast = std::array::from_fn(|index| input(initial.fast[index].clone(), window, cx));
        let alias = input(initial.alias.clone(), window, cx);
        let model_input = input(Some(model.clone().unwrap_or_default()), window, cx);
        // The scroll container's children are the ID, mapping, help, and two rate sections.
        let mut input_observers = [(&model_input, 0), (&alias, 1)]
            .into_iter()
            .chain(standard.iter().map(|field| (field, 3)))
            .chain(fast.iter().map(|field| (field, 4)))
            .map(|(field, row)| {
                let scroll = scroll.clone();
                let focus = field.focus_handle(cx);
                cx.on_focus(&focus, window, move |_, _, cx| {
                    let scroll = scroll.clone();
                    let view = cx.entity_id();
                    cx.defer(move |cx| {
                        scroll.scroll_to_item(row);
                        cx.notify(view);
                    });
                })
            })
            .collect::<Vec<_>>();
        input_observers.push(cx.subscribe(&alias, |_, _, event: &InputEvent, cx| {
            if *event == InputEvent::Change {
                cx.notify();
            }
        }));
        let touched_rates = Rc::new(Cell::new(0));
        input_observers.extend(
            standard
                .iter()
                .chain(&fast)
                .enumerate()
                .map(|(index, field)| {
                    let touched = touched_rates.clone();
                    cx.subscribe(field, move |_, _, event: &InputEvent, _| {
                        if *event == InputEvent::Change {
                            touched.set(touched.get() | (1 << index));
                        }
                    })
                }),
        );
        Self {
            model: model_input,
            existing_model: model,
            alias,
            standard,
            fast,
            initial,
            reset: false,
            clear_alias: false,
            remove_fast: false,
            touched_rates,
            _input_observers: input_observers,
        }
    }

    fn edit(&self, cx: &App) -> Result<(String, PriceEdit), &'static str> {
        let model = self.model.read(cx).value();
        let model = model.trim();
        if !valid_usage_model_id(model) {
            return Err("Enter a model ID.");
        }
        let alias = self.alias.read(cx).value();
        let alias = alias.trim();
        if !alias.is_empty() && !valid_usage_model_id(alias) {
            return Err("Enter a valid model ID to map to.");
        }
        let mapping_changed =
            self.clear_alias || alias != self.initial.alias.as_deref().unwrap_or_default();
        let mut edit = PriceEdit {
            reset: self.reset,
            alias: mapping_changed.then(|| (!alias.is_empty()).then(|| alias.to_owned())),
            ..Default::default()
        };
        if alias.is_empty() {
            edit.standard = edit_rates(
                &self.standard,
                &self.initial.standard,
                self.reset,
                self.touched_rates.get() & 0x0f,
                cx,
            )?;
            edit.fast = edit_rates(
                &self.fast,
                &self.initial.fast,
                self.reset || self.remove_fast,
                self.touched_rates.get() >> 4,
                cx,
            )?;
        }
        Ok((model.to_owned(), edit))
    }
}

impl UsageWindow {
    fn pricing_scope(&self, local_model: bool) -> Option<&str> {
        if local_model {
            return Some(LOCAL);
        }
        if self.view == View::Limits {
            self.environment.as_deref()
        } else {
            self.history_environment.as_deref()
        }
    }

    pub(super) fn open_model_prices(
        &mut self,
        model: Option<String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.credit.pending {
            return;
        }
        self.open_filter = None;
        let scope = self.pricing_scope(model.is_some()).map(str::to_owned);
        let back_focus = cx.focus_handle().tab_stop(true);
        window.focus(&back_focus, cx);
        let mut page = PricePage {
            targets: Vec::new(),
            batch: Batch::default(),
            back_focus,
            target_focus: cx.focus_handle().tab_stop(true),
            scroll: ScrollHandle::new(),
            request: None,
            form: None,
            choosing_targets: false,
            loading: false,
            saving: false,
            error: None,
        };
        if cx.try_global::<DesktopState>().is_none() {
            page.error = Some("The desktop pricing service is unavailable.".into());
            self.prices = Some(page);
            cx.notify();
            return;
        }
        page.loading = true;
        let task = crate::app_state::spawn_tokio(cx, PriceTarget::discover());
        page.request = Some(cx.spawn_in(window, async move |this, cx| {
            let (targets, problem) = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(page) = &mut this.prices else { return };
                let (selected, hint) = initial_selection(&targets, scope.as_deref());
                page.targets = targets;
                page.batch.selected = selected;
                page.error = hint.or(problem);
                page.loading = false;
                this.refresh_price_rows(model, window, cx);
                cx.notify();
            });
        }));
        self.prices = Some(page);
        cx.notify();
    }

    fn refresh_price_rows(
        &mut self,
        model: Option<String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(page) = &mut self.prices else { return };
        if page.loading || page.saving || page.batch.selected.is_empty() {
            return;
        }
        let targets = page
            .batch
            .pending_targets()
            .map(|index| (index, page.targets[index].clone()))
            .collect::<Vec<_>>();
        // A completed batch can still be refreshed to inspect current values.
        let targets = if targets.is_empty() {
            page.batch
                .selected
                .iter()
                .map(|&index| (index, page.targets[index].clone()))
                .collect()
        } else {
            targets
        };
        page.loading = true;
        let (send, receive) = async_channel::unbounded();
        let task = crate::app_state::spawn_tokio(cx, async move {
            let mut reads = stream::iter(
                targets
                    .into_iter()
                    .map(|(index, target)| async move { (index, target.load().await) }),
            )
            .buffer_unordered(4);
            while let Some(result) = reads.next().await {
                if send.send(result).await.is_err() {
                    break;
                }
            }
        });
        page.request = Some(cx.spawn_in(window, async move |this, cx| {
            while let Ok((index, result)) = receive.recv().await {
                let _ = this.update_in(cx, |this, _, cx| {
                    let Some(page) = &mut this.prices else { return };
                    match result {
                        Ok(rows) => page.batch.loaded(index, rows),
                        Err(error) => {
                            page.batch.unavailable.insert(index, error);
                        }
                    }
                    cx.notify();
                });
            }
            task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(page) = &mut this.prices else { return };
                page.loading = false;
                if model.is_some() {
                    this.edit_price_row(model, window, cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn change_price_targets(
        &mut self,
        index: Option<usize>,
        checked: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(page) = &mut self.prices else { return };
        if page.targets_locked() {
            return;
        }
        match (index, checked) {
            (Some(index), true) => {
                page.batch.selected.insert(index);
            }
            (Some(index), false) => {
                page.batch.selected.remove(&index);
            }
            (None, true) => {
                page.batch.selected = (0..page.targets.len()).collect();
            }
            (None, false) => {
                page.batch.selected.clear();
            }
        }
        page.batch.outcomes.clear();
        page.error = None;
        if page.batch.selected.iter().any(|index| {
            !page.batch.rows.contains_key(index) && !page.batch.unavailable.contains_key(index)
        }) {
            self.refresh_price_rows(None, window, cx);
        }
        cx.notify();
    }

    pub(super) fn back_from_model_prices(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(page) = &mut self.prices else { return };
        if page.saving {
            return;
        }
        if page.choosing_targets {
            page.choosing_targets = false;
            window.focus(&page.target_focus, cx);
            cx.notify();
            return;
        }
        if page.form.take().is_some() {
            page.error = None;
            page.scroll.set_offset(Point::default());
            window.focus(&page.back_focus, cx);
        } else {
            self.prices = None;
            window.focus(&self.usage_focus, cx);
        }
        cx.notify();
    }

    fn edit_price_row(
        &mut self,
        model: Option<String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(page) = &mut self.prices else { return };
        if page.batch.selected.is_empty() || page.loading || page.saving || page.batch.is_retry() {
            return;
        }
        let initial = PriceFields::from_rows(
            &model
                .as_ref()
                .map_or_else(Vec::new, |id| page.batch.effective_rows(id)),
        );
        let form = PriceForm::new(model, initial, &page.scroll, window, cx);
        let focus = if form
            .initial
            .alias
            .as_ref()
            .is_some_and(|alias| !alias.is_empty())
        {
            &form.alias
        } else if form.existing_model.is_some() {
            &form.standard[0]
        } else {
            &form.model
        };
        focus.update(cx, |field, cx| field.focus(window, cx));
        page.choosing_targets = false;
        page.form = Some(form);
        page.scroll.set_offset(Point::default());
        page.error = None;
        cx.notify();
    }

    fn clear_price_form(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(form) = self.prices.as_mut().and_then(|page| page.form.as_mut()) else {
            return;
        };
        form.reset = true;
        form.touched_rates.set(0);
        for field in form.standard.iter().chain(&form.fast).chain([&form.alias]) {
            field.update(cx, |state, cx| {
                state.set_value("", window, cx);
                state.set_placeholder("", window, cx);
            });
        }
        cx.notify();
    }

    fn stage_price_row(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(page) = &mut self.prices else { return };
        let Some(form) = &page.form else { return };
        if page.saving || page.loading || page.batch.is_retry() {
            return;
        }
        let result = form
            .edit(cx)
            .map_err(str::to_owned)
            .and_then(|(model, edit)| {
                if form.existing_model.is_none() && page.batch.model_ids().contains(model.as_str())
                {
                    return Err("This model already has settings. Open its row to edit it.".into());
                }
                page.batch.stage(model, edit).map_err(|(index, error)| {
                    format!("Check {}. {error}", page.targets[index].name())
                })
            });
        if let Err(error) = result {
            page.error = Some(error);
            cx.notify();
            return;
        }
        self.back_from_model_prices(window, cx);
    }

    fn save_price_rows(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(page) = &mut self.prices else { return };
        if page.saving
            || page.loading
            || page.form.is_some()
            || page.batch.drafts.is_empty()
            || page
                .batch
                .drafts
                .keys()
                .any(|id| page.batch.conflict(id).is_some())
        {
            return;
        }
        let work = page
            .batch
            .attempts()
            .map(|attempt| (page.targets[attempt.index].clone(), attempt))
            .collect::<Vec<_>>();
        if work.is_empty() {
            return;
        }
        page.saving = true;
        page.choosing_targets = false;
        page.error = None;
        let (send, receive) = async_channel::unbounded();
        let task = crate::app_state::spawn_tokio(cx, async move {
            let mut writes = stream::iter(work.into_iter().map(|(target, attempt)| async move {
                let index = attempt.index;
                let (rows, result) = target.apply(attempt).await;
                (index, rows, result)
            }))
            .buffer_unordered(4);
            while let Some(result) = writes.next().await {
                if send.send(result).await.is_err() {
                    break;
                }
            }
        });
        page.request = Some(cx.spawn_in(window, async move |this, cx| {
            let mut wrote = false;
            while let Ok((index, rows, result)) = receive.recv().await {
                wrote |= result.as_ref().is_ok_and(|changes| !changes.is_empty());
                let _ = this.update_in(cx, |this, _, cx| {
                    if let Some(page) = &mut this.prices {
                        page.batch.settled(index, rows, result);
                        cx.notify();
                    }
                });
            }
            task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(page) = &mut this.prices else { return };
                page.saving = false;
                window.focus(&page.back_focus, cx);
                if wrote {
                    // Keep the settled chart visible until the replacement arrives.
                    this.history_revision = this.history_revision.wrapping_add(1);
                    this.history.loading = false;
                    this.refresh_history(cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn resolve_price_draft(&mut self, model: &str, keep: bool, cx: &mut Context<'_, Self>) {
        let Some(page) = &mut self.prices else { return };
        if page.saving || page.loading {
            return;
        }
        if keep {
            page.batch.rebase(model);
        } else {
            page.batch.drafts.remove(model);
        }
        if page.batch.drafts.is_empty() && page.batch.is_retry() {
            page.batch.outcomes.clear();
        }
        page.error = None;
        cx.notify();
    }

    pub(super) fn render_model_prices(
        &self,
        page: &PricePage,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> Div {
        let content = if let Some(form) = &page.form {
            self.render_price_form(form, page.saving, window, cx)
        } else {
            self.render_price_rows(page, cx)
        };
        let pending = page.batch.drafts.len();
        let back_label = if page.form.is_some() {
            "Back to model prices"
        } else {
            "Back to Usage"
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .text_color(theme::rambutan_100())
            .font_family(theme::UI_FONT_FAMILY)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_3()
                    .border_b_1()
                    .border_color(theme::longan_800())
                    .child(
                        Self::button(
                            "usage-prices-back",
                            "Back",
                            WebButtonVariant::Ghost,
                            page.saving,
                            cx,
                            |this, window, cx| this.back_from_model_prices(window, cx),
                        )
                        .aria_label(back_label)
                        .track_focus(&page.back_focus),
                    )
                    .child(title("Model prices").flex_1())
                    .when(page.form.is_some(), |el| {
                        el.child(muted(page.selection_label()).truncate())
                    }),
            )
            .child(
                content
                    .id("usage-prices-scroll")
                    .track_scroll(&page.scroll)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .w_full()
                    .max_w(px(780.))
                    .mx_auto()
                    .p_4(),
            )
            .when_some(page.error.as_ref(), |el, error| {
                el.child(
                    muted(error.clone())
                        .id("usage-price-error")
                        .role(Role::Alert)
                        .aria_label(error.clone())
                        .text_color(theme::lychee_400())
                        .px_4()
                        .py_2(),
                )
            })
            .when(page.form.is_some() || pending > 0, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .p_3()
                        .border_t_1()
                        .border_color(theme::longan_800())
                        .when(page.form.is_some(), |el| {
                            el.child(Self::button(
                                "usage-price-cancel",
                                "Cancel",
                                WebButtonVariant::Ghost,
                                page.saving,
                                cx,
                                |this, window, cx| this.back_from_model_prices(window, cx),
                            ))
                        })
                        .when(page.form.is_none(), |el| {
                            el.child(muted(format!("{pending} pending")).flex_1())
                                .child(Self::button(
                                    "usage-prices-refresh",
                                    "Refresh saved values",
                                    WebButtonVariant::Ghost,
                                    page.loading || page.saving,
                                    cx,
                                    |this, window, cx| this.refresh_price_rows(None, window, cx),
                                ))
                                .child(Self::button(
                                    "usage-prices-discard",
                                    "Discard pending changes",
                                    WebButtonVariant::Ghost,
                                    page.loading || page.saving,
                                    cx,
                                    |this, window, cx| {
                                        let Some(page) = &mut this.prices else { return };
                                        if page.saving || page.loading {
                                            return;
                                        }
                                        page.batch.drafts.clear();
                                        page.batch.outcomes.clear();
                                        page.error = None;
                                        window.focus(&page.back_focus, cx);
                                        cx.notify();
                                    },
                                ))
                        })
                        .child(if page.form.is_some() {
                            Self::button(
                                "usage-price-stage",
                                "Stage change",
                                WebButtonVariant::Primary,
                                false,
                                cx,
                                |this, window, cx| this.stage_price_row(window, cx),
                            )
                            .debug_selector(|| "usage-price-stage".into())
                        } else {
                            Self::button(
                                "usage-price-save",
                                if page.saving {
                                    "Saving…"
                                } else if page.batch.is_retry() {
                                    "Retry failed saves"
                                } else {
                                    "Save changes"
                                },
                                WebButtonVariant::Primary,
                                page.saving
                                    || page.loading
                                    || page
                                        .batch
                                        .drafts
                                        .keys()
                                        .any(|id| page.batch.conflict(id).is_some()),
                                cx,
                                |this, window, cx| this.save_price_rows(window, cx),
                            )
                            .debug_selector(|| "usage-price-save".into())
                        }),
                )
            })
    }

    fn render_price_targets(&self, page: &PricePage, cx: &Context<'_, Self>) -> Div {
        let selection = page.selection_label();
        let selectable = page.targets.len() > 1 || page.batch.selected.is_empty();
        let target_checkbox = |index: Option<usize>, name: String, checked: bool| {
            let id = index.map_or_else(
                || "usage-price-all-targets".into(),
                |index| format!("usage-price-target-{index}"),
            );
            let view = cx.entity().downgrade();
            Checkbox::new(SharedString::from(id.clone()))
                .debug_selector(id)
                .label(name)
                .checked(checked)
                .disabled(page.targets_locked())
                .on_click(move |checked, window, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.change_price_targets(index, *checked, window, cx)
                    });
                })
        };
        let bounds = Rc::new(Cell::new(Bounds::default()));
        let observed = bounds.clone();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(muted("Apply to"))
                    .when_else(
                        selectable,
                        |el| {
                            let available = !page.loading && !page.saving;
                            let toggle = Self::button(
                                "usage-price-targets",
                                selection.clone(),
                                WebButtonVariant::Outline,
                                !available,
                                cx,
                                |this, _, cx| {
                                    if let Some(page) = &mut this.prices {
                                        page.choosing_targets = !page.choosing_targets;
                                        cx.notify();
                                    }
                                },
                            )
                            .max_w(px(260.))
                            .aria_label(format!("Apply to {selection}"))
                            .aria_expanded(page.choosing_targets)
                            .track_focus(&page.target_focus)
                            .debug_selector(|| "usage-price-targets".into())
                            .child(
                                svg()
                                    .path(bruhs_ui::icons::CHEVRON_DOWN)
                                    .size(px(12.))
                                    .text_color(theme::rambutan_400()),
                            );
                            let chooser = div()
                                .relative()
                                .min_w_0()
                                .on_children_prepainted(move |children, _, _| {
                                    if let Some(first) = children.first() {
                                        observed.set(*first);
                                    }
                                })
                                .child(toggle)
                                .when(page.choosing_targets, |el| {
                                    let menu =
                                        bruhs_ui::card::popover_card("usage-price-target-menu")
                                            .role(Role::Group)
                                            .aria_label("Pricing environments")
                                            .min_w(px(184.))
                                            .max_w(px(328.))
                                            .max_h(px(220.))
                                            .p_2()
                                            .gap_2()
                                            .overflow_y_scroll()
                                            .when(page.targets.len() > 1, |el| {
                                                el.child(
                                                    div()
                                                        .pb_2()
                                                        .border_b_1()
                                                        .border_color(theme::longan_800())
                                                        .child(target_checkbox(
                                                            None,
                                                            "All environments".into(),
                                                            page.batch.selected.len()
                                                                == page.targets.len(),
                                                        )),
                                                )
                                            })
                                            .children(page.targets.iter().enumerate().map(
                                                |(index, target)| {
                                                    target_checkbox(
                                                        Some(index),
                                                        target.name().into(),
                                                        page.batch.selected.contains(&index),
                                                    )
                                                },
                                            ))
                                            .on_mouse_down_out(cx.listener(
                                                move |this, event: &MouseDownEvent, _, cx| {
                                                    if !bounds.get().contains(&event.position) {
                                                        if let Some(page) = &mut this.prices {
                                                            page.choosing_targets = false;
                                                            cx.notify();
                                                        }
                                                    }
                                                },
                                            ));
                                    el.child(div().absolute().top_0().left_0().child(
                                        bruhs_ui::menu_popover::menu_popover(
                                            size(px(184.), px(theme::CONTROL_SM_PX)),
                                            menu.into_any_element(),
                                        ),
                                    ))
                                });
                            el.child(chooser)
                        },
                        |el| el.child(title(selection.clone())),
                    ),
            )
            .children(page.batch.selected.iter().filter_map(|index| {
                let outcome = page.batch.outcomes.get(index);
                let (status, error) = match outcome {
                    Some(Ok(())) => ("Saved", None),
                    _ if page.saving => ("Saving…", None),
                    Some(Err(error)) => ("Not saved", Some(error.as_str())),
                    None => (
                        "Unavailable",
                        Some(page.batch.unavailable.get(index)?.as_str()),
                    ),
                };
                let name = page.targets[*index].name();
                Some(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(title(name.to_owned()))
                        .child(
                            muted(status)
                                .when(error.is_some(), |el| el.text_color(theme::lychee_400())),
                        )
                        .when_some(error, |el, error| el.child(muted(error.to_owned()))),
                )
            }))
    }

    fn render_price_rows(&self, page: &PricePage, cx: &Context<'_, Self>) -> Div {
        let ids = page.batch.model_ids();
        let busy = page.loading || page.saving;
        div()
            .flex()
            .flex_col()
            .gap_3()
            .when(!page.targets.is_empty(), |el| {
                el.child(self.render_price_targets(page, cx))
            })
            .when(page.loading, |el| el.child(muted("Loading model prices…")))
            .when(!page.batch.selected.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(muted("Overrides affect estimated usage costs."))
                        .when(
                            page.batch.drafts.is_empty() && !page.batch.unavailable.is_empty(),
                            |el| {
                                el.child(Self::button(
                                    "usage-prices-retry",
                                    "Refresh saved values",
                                    WebButtonVariant::Ghost,
                                    busy,
                                    cx,
                                    |this, window, cx| this.refresh_price_rows(None, window, cx),
                                ))
                            },
                        )
                        .child(
                            Self::button(
                                "usage-price-add",
                                "Add model",
                                WebButtonVariant::Outline,
                                busy || page.batch.is_retry(),
                                cx,
                                |this, window, cx| this.edit_price_row(None, window, cx),
                            )
                            .debug_selector(|| "usage-price-add".into()),
                        ),
                )
            })
            .when(
                ids.is_empty() && !busy && !page.batch.rows.is_empty(),
                |el| el.child(muted("No model prices or mappings set.")),
            )
            .children(ids.into_iter().map(|id| {
                let pending = page.batch.drafts.contains_key(id);
                let conflict = page.batch.conflict(id);
                let rows = page.batch.effective_rows(id);
                let summary = rows.first().map_or_else(String::new, |first| {
                    if rows.iter().all(|row| row == first) {
                        price_summary(first)
                    } else {
                        "Mixed values".into()
                    }
                });
                let model = id.to_owned();
                let discard = model.clone();
                let keep = model.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .rounded(px(8.))
                    .bg(theme::longan_900())
                    .child(
                        Self::button(
                            SharedString::from(format!("usage-price-row-{id}")),
                            id.to_owned(),
                            WebButtonVariant::Ghost,
                            busy || conflict.is_some() || page.batch.is_retry(),
                            cx,
                            move |this, window, cx| {
                                this.edit_price_row(Some(model.clone()), window, cx)
                            },
                        )
                        .aria_label(format!("Edit model price for {id}"))
                        .w_full()
                        .justify_between()
                        .gap_3()
                        .child(muted(summary).truncate()),
                    )
                    .when_some(conflict.as_ref(), |el, (index, saved)| {
                        el.child(
                            muted(format!(
                                "Saved on {}. {}",
                                page.targets[*index].name(),
                                saved_price_summary(saved)
                            ))
                            .text_color(theme::lychee_400()),
                        )
                    })
                    .when(pending, |el| {
                        el.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_2()
                                .child(muted("Pending").flex_1())
                                .when(conflict.is_some(), |el| {
                                    el.child(
                                        Self::button(
                                            SharedString::from(format!("usage-price-keep-{id}")),
                                            "Keep draft",
                                            WebButtonVariant::Outline,
                                            busy,
                                            cx,
                                            move |this, _, cx| {
                                                this.resolve_price_draft(&keep, true, cx)
                                            },
                                        )
                                        .aria_label(format!("Keep draft for {id}")),
                                    )
                                })
                                .child(
                                    Self::button(
                                        SharedString::from(format!("usage-price-discard-{id}")),
                                        "Discard draft",
                                        WebButtonVariant::Ghost,
                                        busy,
                                        cx,
                                        move |this, _, cx| {
                                            this.resolve_price_draft(&discard, false, cx)
                                        },
                                    )
                                    .aria_label(format!("Discard draft for {id}")),
                                ),
                        )
                    })
            }))
    }

    fn render_price_form(
        &self,
        form: &PriceForm,
        saving: bool,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> Div {
        let field = |label: &str, aria_label: &str, state: &Entity<InputState>, read_only: bool| {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(muted(label.to_owned()))
                .child(
                    Input::new(state)
                        .aria_label(aria_label.to_owned())
                        .readonly(read_only)
                        .disabled(saving),
                )
        };
        let compact = bruhs_ui::zoom::css_px(window.viewport_size().width) < 760.;
        let rates = |name: &str, states: &[Entity<InputState>; 4], removable: bool| {
            let pair = |start: usize| {
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .gap_3()
                    .children((start..start + 2).map(|index| {
                        field(
                            RATE_LABELS[index],
                            &format!("{name} {}", RATE_LABELS[index]),
                            &states[index],
                            false,
                        )
                    }))
            };
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(title(name.to_owned()))
                        .when(removable, |el| {
                            el.child(Self::button(
                                "usage-price-remove-fast",
                                "Remove Fast rates",
                                WebButtonVariant::Ghost,
                                saving,
                                cx,
                                |this, window, cx| {
                                    let Some(form) =
                                        this.prices.as_mut().and_then(|page| page.form.as_mut())
                                    else {
                                        return;
                                    };
                                    form.remove_fast = true;
                                    form.touched_rates.set(form.touched_rates.get() & 0x0f);
                                    for field in &form.fast {
                                        field.update(cx, |state, cx| {
                                            state.set_value("", window, cx);
                                            state.set_placeholder("", window, cx);
                                        });
                                    }
                                    cx.notify();
                                },
                            ))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .gap_3()
                        .when(compact, |el| el.flex_col())
                        .child(pair(0))
                        .child(pair(2)),
                )
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                field(
                    "Model ID",
                    "Model ID",
                    &form.model,
                    form.existing_model.is_some(),
                )
                .flex_none(),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .flex_none()
                    .child(field("Map to", "Map to", &form.alias, false))
                    .when(!form.clear_alias && !form.reset && form.initial.alias.as_deref() != Some(""), |el| {
                        el.child(Self::button(
                            "usage-price-clear-mapping",
                            "Clear mapping",
                            WebButtonVariant::Ghost,
                            saving,
                            cx,
                            |this, window, cx| {
                                let Some(form) = this.prices.as_mut().and_then(|page| page.form.as_mut()) else { return };
                                form.clear_alias = true;
                                form.alias.update(cx, |state, cx| {
                                    state.set_value("", window, cx);
                                    state.set_placeholder("", window, cx);
                                    state.focus(window, cx);
                                });
                                cx.notify();
                            },
                        ))
                    }),
            )
            .when_else(
                form.alias.read(cx).value().trim().is_empty(),
                |el| {
                    let help = if form.reset {
                        "Automatic pricing will apply where no new rates are entered."
                    } else {
                        "USD per million tokens. Mixed values stay as saved until edited. A cleared cache rate uses the input rate."
                    };
                    el.child(muted(help))
                    .child(rates("Standard", &form.standard, false))
                    .child(rates("Fast", &form.fast, form.initial.has_fast && !form.remove_fast && !form.reset))
                },
                |el| {
                    el.child(muted(
                        "Usage counts these tokens and costs under the mapped model.",
                    ))
                },
            )
            .when(form.initial.has_settings && !form.reset, |el| {
                el.child(Self::button(
                    "usage-price-reset",
                    "Reset to automatic",
                    WebButtonVariant::Ghost,
                    saving,
                    cx,
                    |this, window, cx| this.clear_price_form(window, cx),
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::core::prelude::v1::test;

    fn with_empty_prices(_: &mut Window, cx: &mut Context<'_, UsageWindow>) -> UsageWindow {
        let mut usage = UsageWindow::new(cx);
        let mut batch = Batch {
            selected: BTreeSet::from([0]),
            ..Default::default()
        };
        batch.loaded(0, PriceRows::new());
        usage.prices = Some(PricePage {
            targets: vec![PriceTarget::Local],
            batch,
            back_focus: cx.focus_handle().tab_stop(true),
            target_focus: cx.focus_handle().tab_stop(true),
            scroll: ScrollHandle::new(),
            request: None,
            form: None,
            choosing_targets: false,
            loading: false,
            saving: false,
            error: None,
        });
        usage
    }

    #[gpui::test]
    fn pricing_follows_the_environment_filter_and_keeps_set_price_local(cx: &mut TestAppContext) {
        let usage = cx.update(|cx| {
            cx.new(|cx| {
                let mut view = UsageWindow::new(cx);
                view.view = View::Cost;
                view.history_environment = Some("Fixture environment".into());
                view
            })
        });
        usage.read_with(cx, |view, _| {
            assert_eq!(view.pricing_scope(false), Some("Fixture environment"));
            assert_eq!(view.pricing_scope(true), Some(LOCAL));
        });
        let (selection, error) =
            initial_selection(&[PriceTarget::Local], Some("Fixture environment"));
        assert!(selection.is_empty());
        assert!(error.is_some());
    }

    #[gpui::test]
    fn pricing_reveals_a_focused_rate_in_a_short_window(cx: &mut TestAppContext) {
        cx.update(bruhs_ui::init);
        let (usage, visual) = cx.add_window_view(with_empty_prices);
        visual.simulate_resize(size(px(640.), px(480.)));
        visual.update(|window, _| window.activate_window());
        visual.run_until_parked();
        usage.update_in(visual, |view, window, cx| {
            view.edit_price_row(Some("codex/fixture".into()), window, cx);
        });
        visual.run_until_parked();
        usage.read_with(visual, |view, _| {
            let scroll = &view.prices.as_ref().unwrap().scroll;
            let section = scroll.bounds_for_item(4).expect("Fast rate section");
            assert!(
                section.bottom() > scroll.bounds().bottom(),
                "the fixture should overflow: {section:?} in {:?}",
                scroll.bounds()
            );
        });
        usage.update_in(visual, |view, window, cx| {
            let page = view.prices.as_ref().unwrap();
            let field = &page.form.as_ref().unwrap().fast[3];
            field.update(cx, |state, cx| state.focus(window, cx));
        });
        visual.run_until_parked();
        assert!(usage.read_with(visual, |view, _| {
            view.prices.as_ref().unwrap().scroll.offset().y < Pixels::ZERO
        }));
    }

    #[gpui::test]
    fn pricing_keeps_multiple_staged_models_separate_from_saved_rows(cx: &mut TestAppContext) {
        cx.update(bruhs_ui::init);
        let (usage, visual) = cx.add_window_view(with_empty_prices);
        visual.update(|window, _| window.activate_window());
        visual.run_until_parked();
        for (id, alias) in [
            ("codex/canonical", ""),
            ("codex/preview", "codex/canonical"),
        ] {
            let add = visual.debug_bounds("usage-price-add").unwrap().center();
            visual.simulate_click(add, Modifiers::default());
            usage.update_in(visual, |view, window, cx| {
                let form = view.prices.as_ref().unwrap().form.as_ref().unwrap();
                for (field, value) in [
                    (&form.model, id),
                    (&form.standard[0], "3"),
                    (&form.standard[1], "4"),
                    (&form.alias, alias),
                ] {
                    field.update(cx, |state, cx| state.set_value(value, window, cx));
                }
            });
            visual.run_until_parked();
            let stage = visual.debug_bounds("usage-price-stage").unwrap().center();
            visual.simulate_click(stage, Modifiers::default());
            visual.run_until_parked();
        }
        usage.read_with(visual, |view, _| {
            let page = view.prices.as_ref().unwrap();
            assert!(page.form.is_none());
            assert!(page.batch.rows[&0].is_empty());
            assert_eq!(page.batch.drafts.len(), 2);
            let attempt = page.batch.attempts().next().unwrap();
            let changes = attempt.changes(&page.batch.rows[&0]).unwrap();
            assert!(changes[0].edit.price.is_some());
            assert_eq!(changes[1].edit.alias.as_deref(), Some("codex/canonical"));
        });
        assert!(visual.debug_bounds("usage-price-save").is_some());
    }

    #[gpui::test]
    fn mixed_cache_can_be_cleared_to_each_destination_input(cx: &mut TestAppContext) {
        use bruhs_models::agent_extensions::ModelPrice;

        cx.update(bruhs_ui::init);
        let (usage, visual) = cx.add_window_view(with_empty_prices);
        visual.update(|window, _| window.activate_window());
        let model = "codex/fixture";
        usage.update_in(visual, |view, _, cx| {
            let page = view.prices.as_mut().unwrap();
            page.targets.push(PriceTarget::Local);
            for (index, input, cache) in [(0, 2., 0.5), (1, 3., 1.)] {
                page.batch.loaded(
                    index,
                    PriceRows::from([(
                        model.into(),
                        UsageModelPricing {
                            price: Some(ModelPrice {
                                input_per_million: input,
                                output_per_million: 4.,
                                cache_read_per_million: cache,
                                ..Default::default()
                            }),
                            alias: None,
                        },
                    )]),
                );
            }
            cx.notify();
        });
        visual.run_until_parked();
        for selector in ["usage-price-targets", "usage-price-target-1"] {
            let control = visual.debug_bounds(selector).unwrap().center();
            visual.simulate_click(control, Modifiers::default());
            visual.run_until_parked();
        }
        usage.update_in(visual, |view, window, cx| {
            assert_eq!(view.prices.as_ref().unwrap().batch.selected.len(), 2);
            view.edit_price_row(Some(model.into()), window, cx);
            let form = view.prices.as_ref().unwrap().form.as_ref().unwrap();
            assert!(form.initial.standard[2].is_none());
            assert_eq!(form.edit(cx).unwrap().1, PriceEdit::default());
            form.standard[2].update(cx, |field, cx| field.focus(window, cx));
        });
        visual.run_until_parked();
        visual.simulate_keystrokes("9 backspace");
        visual.run_until_parked();
        usage.update_in(visual, |view, window, cx| {
            view.stage_price_row(window, cx);
            let page = view.prices.as_ref().unwrap();
            assert_eq!(page.batch.drafts.len(), 1);
            for attempt in page.batch.attempts() {
                let rows = attempt.rows.as_ref().unwrap();
                let changes = attempt.changes(rows).unwrap();
                assert_eq!(changes.len(), 1);
                let price = changes[0].edit.price.as_ref().unwrap();
                assert_eq!(price.cache_read_per_million, price.input_per_million);
            }
        });
    }
}
