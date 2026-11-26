use bruhs_models::agent_extensions::{
    ModelPrice, TokenPrice, UsageModelPricing, UsageModelPricingChange,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) type PriceRows = BTreeMap<String, UsageModelPricing>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum RateValue {
    Number(f64),
    Input,
}

pub(super) fn number(value: &str) -> Result<f64, &'static str> {
    value
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.)
        .ok_or("Enter a non-negative number for each rate you change.")
}

pub(super) fn rates(price: TokenPrice) -> [f64; 4] {
    [
        price.input_per_million,
        price.output_per_million,
        price.cache_read_per_million,
        price.cache_write_per_million,
    ]
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct RateEdit {
    pub reset: bool,
    pub fields: [Option<RateValue>; 4],
}

impl RateEdit {
    fn changed(&self) -> bool {
        self.fields.iter().any(Option::is_some)
    }

    fn merge(&mut self, next: Self) {
        if next.reset {
            *self = next;
            return;
        }
        for (field, change) in self.fields.iter_mut().zip(next.fields) {
            if change.is_some() {
                *field = change;
            }
        }
    }

    fn apply(&self, saved: Option<TokenPrice>) -> Result<Option<TokenPrice>, &'static str> {
        let saved = saved.filter(|_| !self.reset);
        if !self.changed() {
            return Ok(saved);
        }
        let saved = saved.map(rates);
        let required = |index| match self.fields[index] {
            Some(RateValue::Number(number)) => Some(number),
            _ => saved.map(|rates| rates[index]),
        };
        let input = required(0).ok_or("Enter both Input and Output for a new rate.")?;
        let output = required(1).ok_or("Enter both Input and Output for a new rate.")?;
        let cache = |index| match self.fields[index] {
            Some(RateValue::Number(number)) => number,
            Some(RateValue::Input) => input,
            None => saved.map_or(input, |rates| rates[index]),
        };
        Ok(Some(TokenPrice {
            input_per_million: input,
            output_per_million: output,
            cache_read_per_million: cache(2),
            cache_write_per_million: cache(3),
        }))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct PriceEdit {
    pub reset: bool,
    pub alias: Option<Option<String>>,
    pub standard: RateEdit,
    pub fast: RateEdit,
}

impl PriceEdit {
    pub fn merge(&mut self, next: Self) {
        if next.reset {
            *self = next;
            return;
        }
        if next.alias.is_some() {
            self.alias = next.alias;
        }
        self.standard.merge(next.standard);
        self.fast.merge(next.fast);
    }

    pub fn apply(&self, mut saved: UsageModelPricing) -> Result<UsageModelPricing, &'static str> {
        if self.reset {
            saved = UsageModelPricing::default();
        }
        if let Some(alias) = &self.alias {
            saved.alias.clone_from(alias);
        }
        if saved.alias.is_some() {
            // An explicit mapping hides this model's own rates until the mapping is cleared.
            if self.alias.is_none() && (self.standard.changed() || self.fast.changed()) {
                return Err("Clear the mapping before changing this environment’s rates.");
            }
            return Ok(saved);
        }
        let standard = self
            .standard
            .apply(saved.price.as_ref().map(ModelPrice::standard))?;
        let fast = self.fast.apply(saved.price.and_then(|price| price.fast))?;
        if fast.is_some() && standard.is_none() {
            return Err("Set Standard rates before adding Fast rates.");
        }
        saved.price = standard.map(|rates| ModelPrice {
            input_per_million: rates.input_per_million,
            output_per_million: rates.output_per_million,
            cache_read_per_million: rates.cache_read_per_million,
            cache_write_per_million: rates.cache_write_per_million,
            fast,
        });
        Ok(saved)
    }
}

fn saved_row(rows: &PriceRows, model: &str) -> UsageModelPricing {
    rows.get(model).cloned().unwrap_or_default()
}

#[derive(Clone, Debug, Default)]
pub(super) struct Draft {
    expected: BTreeMap<usize, UsageModelPricing>,
    patch: PriceEdit,
}

impl Draft {
    fn change(
        &self,
        target: usize,
        model: &str,
        rows: &PriceRows,
    ) -> Result<Option<UsageModelPricingChange>, String> {
        let expected = self
            .expected
            .get(&target)
            .cloned()
            .unwrap_or_else(|| saved_row(rows, model));
        let edit = self
            .patch
            .apply(expected.clone())
            .map_err(|reason| format!("{model}. {reason}"))?;
        Ok((expected != edit).then(|| UsageModelPricingChange {
            model: model.to_owned(),
            expected,
            edit,
        }))
    }
}

pub(super) struct Attempt {
    pub index: usize,
    pub draft: BTreeMap<String, Draft>,
    pub rows: Option<PriceRows>,
}

impl Attempt {
    pub fn changes(&self, rows: &PriceRows) -> Result<Vec<UsageModelPricingChange>, String> {
        self.draft
            .iter()
            .filter_map(|(model, draft)| draft.change(self.index, model, rows).transpose())
            .collect()
    }
}

#[derive(Debug, Default)]
pub(super) struct Batch {
    pub selected: BTreeSet<usize>,
    pub rows: BTreeMap<usize, PriceRows>,
    pub unavailable: BTreeMap<usize, String>,
    pub drafts: BTreeMap<String, Draft>,
    pub outcomes: BTreeMap<usize, Result<(), String>>,
}

impl Batch {
    pub fn pending_targets(&self) -> impl Iterator<Item = usize> + '_ {
        self.selected
            .iter()
            .copied()
            .filter(|index| !self.outcomes.get(index).is_some_and(Result::is_ok))
    }

    pub fn is_retry(&self) -> bool {
        self.outcomes.values().any(Result::is_err)
    }

    pub fn attempts(&self) -> impl Iterator<Item = Attempt> + '_ {
        self.pending_targets().map(|index| Attempt {
            index,
            draft: self.drafts.clone(),
            rows: self.rows.get(&index).cloned(),
        })
    }

    pub fn model_ids(&self) -> BTreeSet<&str> {
        self.selected
            .iter()
            .filter_map(|index| self.rows.get(index))
            .flat_map(BTreeMap::keys)
            .chain(self.drafts.keys())
            .map(String::as_str)
            .collect()
    }

    pub fn effective_rows(&self, model: &str) -> Vec<UsageModelPricing> {
        self.selected
            .iter()
            .filter_map(|index| {
                let saved = saved_row(self.rows.get(index)?, model);
                if self.outcomes.get(index).is_some_and(Result::is_ok) {
                    return Some(saved);
                }
                Some(
                    self.drafts
                        .get(model)
                        .and_then(|draft| draft.patch.apply(saved.clone()).ok())
                        .unwrap_or(saved),
                )
            })
            .collect()
    }

    pub fn stage(&mut self, model: String, edit: PriceEdit) -> Result<(), (usize, String)> {
        let mut draft = self.drafts.get(&model).cloned().unwrap_or_default();
        draft.patch.merge(edit);
        if draft.patch == PriceEdit::default() {
            return Ok(());
        }
        for index in self.selected.iter().copied() {
            if let Some(rows) = self.rows.get(&index) {
                draft
                    .expected
                    .entry(index)
                    .or_insert_with(|| saved_row(rows, &model));
                draft
                    .change(index, &model, rows)
                    .map_err(|error| (index, error))?;
            }
        }
        self.outcomes.clear();
        self.drafts.insert(model.clone(), draft);
        self.remove_unchanged(&model);
        Ok(())
    }

    pub fn rebase(&mut self, model: &str) {
        let expected = self
            .pending_targets()
            .filter_map(|index| Some((index, saved_row(self.rows.get(&index)?, model))))
            .collect();
        if let Some(draft) = self.drafts.get_mut(model) {
            draft.expected = expected;
        }
        self.remove_unchanged(model);
    }

    pub fn conflict(&self, model: &str) -> Option<(usize, UsageModelPricing)> {
        let draft = self.drafts.get(model)?;
        self.pending_targets().find_map(|index| {
            let saved = saved_row(self.rows.get(&index)?, model);
            let expected = draft.expected.get(&index)?;
            (saved != *expected
                && draft.patch.apply(expected.clone()).ok().as_ref() != Some(&saved))
            .then_some((index, saved))
        })
    }

    pub fn loaded(&mut self, index: usize, rows: PriceRows) {
        for (model, draft) in &mut self.drafts {
            draft
                .expected
                .entry(index)
                .or_insert_with(|| saved_row(&rows, model));
        }
        self.rows.insert(index, rows);
        self.unavailable.remove(&index);
    }

    pub fn settled(
        &mut self,
        index: usize,
        baseline: Option<PriceRows>,
        result: Result<Vec<UsageModelPricingChange>, String>,
    ) {
        if let Some(rows) = baseline {
            self.loaded(index, rows);
        }
        self.outcomes.insert(
            index,
            result.map(|changes| {
                if let Some(rows) = self.rows.get_mut(&index) {
                    for change in changes {
                        if change.edit == UsageModelPricing::default() {
                            rows.remove(&change.model);
                        } else {
                            rows.insert(change.model, change.edit);
                        }
                    }
                }
            }),
        );
        if !self.is_retry() && self.pending_targets().next().is_none() {
            self.drafts.clear();
        }
    }

    fn remove_unchanged(&mut self, model: &str) {
        if self.drafts.get(model).is_some_and(|draft| {
            self.pending_targets().all(|index| {
                self.rows.get(&index).is_some_and(|rows| {
                    let saved = saved_row(rows, model);
                    draft.patch.apply(saved.clone()).ok().as_ref() == Some(&saved)
                })
            })
        }) {
            self.drafts.remove(model);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::core::prelude::v1::test;
    use bruhs_core::agent_extensions::{edit_usage_model_prices_at, usage_model_pricing_at};

    const MODEL: &str = "codex/canonical";

    fn price(input: f64, output: f64, cache: f64) -> UsageModelPricing {
        UsageModelPricing {
            price: Some(ModelPrice {
                input_per_million: input,
                output_per_million: output,
                cache_read_per_million: cache,
                cache_write_per_million: 0.,
                fast: None,
            }),
            alias: None,
        }
    }

    #[test]
    fn a_new_rate_uses_input_for_a_blank_cache_and_preserves_zero() {
        let edit = RateEdit {
            fields: [
                Some(RateValue::Number(number("1,25").unwrap())),
                Some(RateValue::Number(number("3.50").unwrap())),
                None,
                Some(RateValue::Number(0.)),
            ],
            ..Default::default()
        };
        let result = edit.apply(None).unwrap().unwrap();
        assert_eq!(rates(result), [1.25, 3.5, 1.25, 0.]);
        assert!(number("Infinity").is_err());
        assert!(number("-1").is_err());
        let mut partial = edit;
        partial.fields[1] = None;
        assert!(partial.apply(None).is_err());
    }

    #[test]
    fn a_batch_keeps_mixed_rates_and_only_retries_the_failed_destination() {
        let roots = std::array::from_fn::<_, 3, _>(|_| tempfile::tempdir().unwrap());
        for (index, original) in [price(2., 4., 0.5), price(3., 6., 1.), price(5., 8., 2.)]
            .into_iter()
            .enumerate()
        {
            edit_usage_model_prices_at(
                roots[index].path(),
                vec![UsageModelPricingChange {
                    model: MODEL.into(),
                    expected: UsageModelPricing::default(),
                    edit: original,
                }],
            )
            .unwrap();
        }
        let mut batch = Batch {
            selected: BTreeSet::from([0, 1, 2]),
            ..Default::default()
        };
        for index in [0, 1] {
            batch.loaded(index, usage_model_pricing_at(roots[index].path()).unwrap());
        }
        batch.unavailable.insert(2, "Offline".into());
        batch
            .stage(
                MODEL.into(),
                PriceEdit {
                    standard: RateEdit {
                        fields: [Some(RateValue::Number(7.)), None, None, None],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .unwrap();
        batch
            .stage(
                "codex/preview".into(),
                PriceEdit {
                    alias: Some(Some(MODEL.into())),
                    ..Default::default()
                },
            )
            .unwrap();

        let mut writes = Vec::new();
        let attempts = batch.attempts().collect::<Vec<_>>();
        for attempt in attempts {
            let index = attempt.index;
            if let Some(rows) = attempt.rows.as_ref() {
                let changes = attempt.changes(rows).unwrap();
                assert_eq!(changes.len(), 2);
                edit_usage_model_prices_at(roots[index].path(), changes.clone()).unwrap();
                writes.push(index);
                batch.settled(index, None, Ok(changes));
            } else {
                batch.settled(index, None, Err("Offline".into()));
            }
        }
        assert_eq!(writes, [0, 1]);
        assert!(batch.outcomes[&0].is_ok());
        assert!(batch.outcomes[&2].is_err());
        assert_eq!(batch.drafts.len(), 2);

        let retries = batch.attempts().collect::<Vec<_>>();
        assert_eq!(retries.len(), 1);
        let retry = &retries[0];
        assert_eq!(retry.index, 2);
        let rows = usage_model_pricing_at(roots[2].path()).unwrap();
        let changes = retry.changes(&rows).unwrap();
        edit_usage_model_prices_at(roots[2].path(), changes.clone()).unwrap();
        batch.settled(2, Some(rows), Ok(changes));

        assert!(batch.drafts.is_empty());
        assert!(batch.outcomes.values().all(Result::is_ok));
        for (index, expected) in [[7., 4., 0.5, 0.], [7., 6., 1., 0.], [7., 8., 2., 0.]]
            .into_iter()
            .enumerate()
        {
            let rows = usage_model_pricing_at(roots[index].path()).unwrap();
            assert_eq!(
                rates(rows[MODEL].price.as_ref().unwrap().standard()),
                expected
            );
            assert_eq!(rows["codex/preview"].alias.as_deref(), Some(MODEL));
        }
    }

    #[test]
    fn keeping_a_stale_draft_reapplies_only_its_edited_fields() {
        let mut batch = Batch {
            selected: BTreeSet::from([0]),
            ..Default::default()
        };
        batch.loaded(0, BTreeMap::from([(MODEL.into(), price(1., 2., 3.))]));
        batch
            .stage(
                MODEL.into(),
                PriceEdit {
                    standard: RateEdit {
                        fields: [
                            Some(RateValue::Number(4.)),
                            None,
                            Some(RateValue::Input),
                            None,
                        ],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .unwrap();
        batch.loaded(0, BTreeMap::from([(MODEL.into(), price(1., 6., 7.))]));
        assert!(batch.conflict(MODEL).is_some());
        batch.rebase(MODEL);
        assert!(batch.conflict(MODEL).is_none());
        let attempt = batch.attempts().next().unwrap();
        let change = &attempt.changes(&batch.rows[&0]).unwrap()[0];
        assert_eq!(
            rates(change.edit.price.as_ref().unwrap().standard()),
            [4., 6., 4., 0.]
        );
    }
}
