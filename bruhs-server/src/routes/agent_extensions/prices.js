(() => {
  const form = $('price'), fields = form.elements;
  const picker = $('price-target-picker'), chooser = $('price-target-label');
  const targetList = $('price-target-list'), saveButton = $('price-save');
  const local = { name: 'This computer', key: null, selected: true };
  const targets = [local], drafts = new Map();
  const automatic = { price: null, alias: null };
  const rateFields = {
    input: 'input_per_million', output: 'output_per_million',
    read: 'cache_read_per_million', write: 'cache_write_per_million',
  };
  const rateKeys = Object.values(rateFields);
  const modes = ['standard', 'fast'];
  let busy = true, listingError = false, original, typed;
  let options = [];

  const selected = () => targets.filter(target => target.selected);
  const pending = () => selected().filter(target => target.status !== 'saved');
  const failed = () => pending().filter(target => target.status === 'failed');
  const partial = () => failed().length > 0 && selected().some(target => target.status === 'saved');
  const path = target => '/usage/model-prices' +
    (target.key === null ? '' : '/' + encodeURIComponent(target.key));
  const saved = (target, model) => target.rows?.get(model) ?? automatic;
  const sameRates = (a, b) => (a ?? null) === (b ?? null) ||
    Boolean(a && b && rateKeys.every(key => (a[key] ?? 0) === (b[key] ?? 0)));
  const same = (a, b) => a.alias === b.alias && sameRates(a.price, b.price) &&
    sameRates(a.price?.fast, b.price?.fast);

  function rates(previous, patch) {
    if (!patch) return previous ?? null;
    const { reset, ...values } = patch;
    const base = reset ? null : previous;
    if (!Object.keys(values).length) return base ?? null;
    const [inputKey, outputKey, readKey, writeKey] = rateKeys;
    const input = values[inputKey] ?? base?.[inputKey];
    const output = values[outputKey] ?? base?.[outputKey];
    if (input == null || output == null) throw Error('Enter both Input and Output for a new rate.');
    const cache = key => Object.hasOwn(values, key) ? values[key] ?? input : base?.[key] ?? input;
    return { [inputKey]: input, [outputKey]: output, [readKey]: cache(readKey), [writeKey]: cache(writeKey) };
  }

  function apply(base, patch) {
    const previous = patch.reset ? automatic : base;
    const alias = Object.hasOwn(patch, 'alias') ? patch.alias : previous.alias;
    if (alias) {
      if (patch.alias === undefined && (patch.standard || patch.fast)) {
        throw Error('Clear the mapping before changing this environment’s rates.');
      }
      return { alias, price: previous.price };
    }
    const standard = rates(previous.price, patch.standard);
    const fast = rates(previous.price?.fast, patch.fast);
    if (fast && !standard) throw Error('Set Standard rates before adding Fast rates.');
    const price = standard && Object.fromEntries(rateKeys.map(key => [key, standard[key] ?? 0]));
    if (fast) price.fast = fast;
    return { alias, price };
  }

  function merge(old, edit) {
    if (edit.reset) return edit;
    const result = { ...old, ...edit };
    for (const mode of modes) {
      if (old?.[mode] && edit[mode] && !edit[mode].reset) result[mode] = { ...old[mode], ...edit[mode] };
    }
    return result;
  }

  function effective(target, model) {
    const row = saved(target, model);
    if (target.status === 'saved' || !drafts.has(model)) return row;
    try { return apply(row, drafts.get(model).patch); }
    catch { return row; }
  }

  function conflicts(draft) {
    return pending().filter(target => {
      if (!target.rows || !draft.expected.has(target)) return false;
      const expected = draft.expected.get(target), current = saved(target, draft.model);
      if (same(expected, current)) return false;
      try { return !same(apply(expected, draft.patch), current); }
      catch { return true; }
    });
  }

  function change(draft, target) {
    const expected = draft.expected.get(target) ?? saved(target, draft.model);
    const edit = apply(expected, draft.patch);
    return same(expected, edit) ? null : { model: draft.model, expected, edit };
  }

  function summarize(row) {
    if (row.alias) return 'Maps to ' + row.alias;
    if (!row.price) return 'Automatic';
    const show = rate => rateKeys.map(key => '$' + (rate[key] ?? 0)).join(' / ');
    return show(row.price) + ' per million' + (row.price.fast ? '. Fast ' + show(row.price.fast) : '');
  }

  function common(values) {
    return values.every(value => value === values[0]) ? values[0] ?? '' : null;
  }

  function showRates() {
    const mode = fields.tier.value;
    for (const [name, key] of Object.entries(rateFields)) {
      const base = original[mode][key];
      fields[name].placeholder = base === null ? 'Mixed' : (name === 'read' || name === 'write' ? 'Input rate' : '');
      fields[name].value = typed[mode][key] ?? base ?? '';
    }
  }

  function showModel() {
    const model = fields.model.value.trim();
    const rows = selected().filter(target => target.rows).map(target => effective(target, model));
    original = { alias: common(rows.map(row => row.alias ?? '')), standard: {}, fast: {} };
    for (const mode of modes) for (const key of rateKeys) {
      original[mode][key] = common(rows.map(row => {
        const rate = mode === 'fast' ? row.price?.fast : row.price;
        return String(rate?.[key] ?? '');
      }));
    }
    typed = { standard: {}, fast: {} };
    fields.alias.value = original.alias ?? '';
    fields.alias.placeholder = original.alias === null ? 'Mixed' : 'Optional model ID';
    showRates();
    syncControls();
  }

  const editing = () => fields.alias.value.trim() !== (original.alias ?? '') ||
    modes.some(mode => Object.entries(typed[mode]).some(([key, value]) => value !== original[mode][key]));
  const scopeLocked = () => busy || drafts.size > 0 || editing();

  function syncControls() {
    const mapped = Boolean(fields.alias.value.trim());
    const disabled = busy || !selected().length || partial();
    fields.model.disabled = fields.alias.disabled = disabled;
    for (const field of [fields.tier, ...Object.keys(rateFields).map(name => fields[name])]) {
      field.disabled = mapped || disabled;
      field.closest('label').hidden = mapped;
    }
    for (const button of form.querySelectorAll('button')) {
      button.disabled = disabled;
      if (button.value === 'remove-fast') button.hidden = mapped;
      if (button.value === 'clear-mapping') button.hidden = original.alias !== null;
    }
    $('price-cancel-edit').hidden = !editing();
    chooser.setAttribute('aria-disabled', String(scopeLocked()));
    for (const { input } of options) input.disabled = scopeLocked();
  }

  function renderTargets() {
    const active = selected(), count = active.length;
    const all = count === targets.length;
    chooser.textContent = count === 0 ? 'Choose environments' : count === 1 ?
      active[0].name : all ? 'All environments' : count + ' environments';
    chooser.setAttribute('aria-disabled', String(scopeLocked()));
    const countOfChoices = targets.length + Number(targets.length > 1);
    if (options.length !== countOfChoices) {
      options = [];
      const choice = (name, target) => {
        const label = el('label'), input = el('input');
        input.type = 'checkbox';
        input.onchange = () => {
          if (scopeLocked()) return;
          if (target) target.selected = input.checked;
          else for (const destination of targets) destination.selected = input.checked;
          clearOutcomes();
          void action(() => Promise.all(selected().filter(item => !item.rows && !item.issue).map(item => read(item))));
        };
        label.append(input, document.createTextNode(name));
        options.push({ input, target });
        return label;
      };
      targetList.replaceChildren(
        ...(targets.length > 1 ? [choice('All environments')] : []),
        ...targets.map(target => choice(target.name, target)),
      );
    }
    for (const { input, target } of options) {
      input.checked = target?.selected ?? all;
      input.disabled = scopeLocked();
    }
    $('price-read').hidden = !listingError && (!active.some(target => !target.rows) || drafts.size > 0);
    $('price-read').disabled = busy;
  }

  function renderResults() {
    const labels = { saved: 'Saved', failed: 'Not saved', saving: 'Saving…', loading: 'Reading saved prices…' };
    const rows = selected().flatMap(target => {
      const label = labels[target.status] ?? (target.issue ? 'Unavailable' : '');
      if (!label) return [];
      const row = el('div'), status = el('span', label);
      row.className = 'price-result';
      if (target.issue) status.className = 'price-error';
      row.append(el('strong', target.name), status);
      if (target.issue) row.append(el('small', target.issue));
      return [row];
    });
    if (listingError) rows.push(el('small', 'Could not read linked environments.'));
    $('price-results').replaceChildren(...rows);
  }

  function clearOutcomes() {
    for (const target of targets) {
      if (target.status === 'saved' || target.status === 'failed') target.status = null;
    }
  }

  function dropDraft(model) {
    drafts.delete(model);
    if (!drafts.size && failed().length) clearOutcomes();
    showModel();
    render();
  }

  function keepDraft(draft) {
    try {
      const expected = new Map(draft.expected);
      for (const target of pending().filter(target => target.rows)) {
        const current = saved(target, draft.model);
        apply(current, draft.patch);
        expected.set(target, current);
      }
      draft.expected = expected;
      if (pending().every(target => target.rows && !change(draft, target))) dropDraft(draft.model);
      else { showModel(); render(); }
    } catch (error) { say(error.message); }
  }

  function renderDrafts() {
    $('price-draft-panel').hidden = !drafts.size;
    $('price-draft-count').textContent = drafts.size + ' pending';
    saveButton.textContent = failed().length ? 'Retry failed saves' : 'Save changes';
    saveButton.disabled = busy || !drafts.size || [...drafts.values()].some(draft => conflicts(draft).length);
    $('price-refresh').disabled = $('price-discard').disabled = busy;
    $('price-drafts').replaceChildren(...Array.from(drafts.values(), draft => {
      const conflict = conflicts(draft);
      const row = el('div'), buttons = el('div');
      buttons.className = 'row';
      const variants = new Set(selected().filter(target => target.rows)
        .map(target => summarize(effective(target, draft.model))));
      row.append(el('strong', draft.model), el('p', variants.size > 1 ? 'Mixed values' :
        variants.values().next().value ?? 'Pending'));
      const button = (label, action, disabled = false) => {
        const control = el('button', label);
        control.disabled = busy || disabled;
        control.setAttribute('aria-label', label + ' for ' + draft.model);
        control.onclick = action;
        buttons.append(control);
      };
      for (const target of conflict) {
        row.append(el('p', 'Saved elsewhere on ' + target.name + '. ' + summarize(saved(target, draft.model))));
      }
      if (conflict.length) button('Keep draft', () => keepDraft(draft));
      button('Edit draft', () => {
        fields.model.value = draft.model;
        showModel();
        fields.alias.focus();
      }, Boolean(conflict.length || partial()));
      button('Discard draft', () => dropDraft(draft.model));
      row.append(buttons);
      return row;
    }));
  }

  function render() { renderTargets(); renderResults(); renderDrafts(); syncControls(); }

  form.oninput = event => {
    const name = event.target.name;
    if (Object.hasOwn(rateFields, name)) typed[fields.tier.value][rateFields[name]] = event.target.value;
    if (name === 'alias' || Object.hasOwn(rateFields, name)) syncControls();
  };
  fields.model.onchange = showModel;
  fields.tier.onchange = showRates;

  function formPatch(command, alias) {
    if (command === 'reset') return { reset: true };
    if (command === 'clear-mapping') return { alias: null };
    if (command === 'remove-fast') return { fast: { reset: true } };
    const patch = {};
    if (alias !== (original.alias ?? '')) patch.alias = alias || null;
    if (alias) return patch;
    for (const mode of modes) {
      const edits = {};
      for (const [key, value] of Object.entries(typed[mode])) {
        if (value === original[mode][key]) continue;
        if (value === '') {
          if (rateKeys.indexOf(key) < 2) throw Error('Enter both Input and Output for a new rate.');
          edits[key] = null;
        } else {
          const rate = Number(value);
          if (!Number.isFinite(rate) || rate < 0) throw Error('Enter a non-negative number for each rate you change.');
          edits[key] = rate;
        }
      }
      if (Object.keys(edits).length) patch[mode] = edits;
    }
    return patch;
  }

  form.onsubmit = event => {
    event.preventDefault();
    if (busy || !selected().length || partial()) return;
    const model = fields.model.value.trim(), alias = fields.alias.value.trim();
    try {
      if (!model || /\p{Cc}/u.test(model)) throw Error('Enter a model ID.');
      if (/\p{Cc}/u.test(alias)) throw Error('Enter a valid model ID to map to.');
      const previous = drafts.get(model);
      if (previous && conflicts(previous).length) throw Error('Review the saved values for this model first.');
      const edit = formPatch(event.submitter?.value, alias);
      if (!Object.keys(edit).length) { say('No changes to stage.'); return; }
      const draft = { model, patch: merge(previous?.patch, edit), expected: new Map(previous?.expected) };
      const active = selected();
      for (const target of active.filter(target => target.rows)) {
        if (!draft.expected.has(target)) draft.expected.set(target, saved(target, model));
      }
      // Validate every online destination before retaining the patch.
      const changes = active.map(target => !target.rows || change(draft, target));
      if (changes.some(Boolean)) drafts.set(model, draft);
      else drafts.delete(model);
      clearOutcomes();
      showModel();
      render();
      say(drafts.size ? 'Ready to save.' : 'No changes to save.');
    } catch (error) { say(error.message); }
  };

  async function read(target, duringSave = false) {
    const status = target.status;
    if (!duringSave) target.status = 'loading';
    target.issue = null;
    renderResults();
    try {
      target.rows = new Map(Object.entries(await api(path(target))));
      for (const draft of drafts.values()) {
        if (!draft.expected.has(target)) draft.expected.set(target, saved(target, draft.model));
      }
    } catch (error) {
      target.issue = error.message;
      if (duringSave) throw error;
    } finally {
      target.status = status;
      renderResults();
    }
  }

  async function discover() {
    try {
      const links = await api('/usage/price-destinations');
      const known = new Set(targets.map(target => target.key));
      for (const link of links) {
        if (known.has(link.key)) continue;
        known.add(link.key);
        targets.push({ ...link, selected: true });
      }
      listingError = false;
    } catch { listingError = true; }
  }

  async function action(run) {
    if (busy) return;
    busy = true;
    render();
    try { await run(); }
    catch (error) { say(error.message); }
    finally { busy = false; showModel(); render(); }
  }

  async function save(target) {
    try {
      if (!target.rows) await read(target, true);
      const changes = [...drafts.values()].map(draft => change(draft, target)).filter(Boolean);
      if (changes.length) {
        await api(path(target), changes);
        for (const { model, edit } of changes) {
          if (same(edit, automatic)) target.rows.delete(model);
          else target.rows.set(model, edit);
        }
      }
      target.status = 'saved';
      return target === local && changes.length > 0;
    } catch (error) {
      target.status = 'failed';
      target.issue = error.message;
      return false;
    } finally { renderResults(); }
  }

  saveButton.onclick = () => action(async () => {
    if (!drafts.size || [...drafts.values()].some(draft => conflicts(draft).length)) return;
    picker.open = false;
    const work = pending();
    for (const target of work) { target.status = 'saving'; target.issue = null; }
    renderResults();
    const wrote = await Promise.all(work.map(save));
    if (!pending().length) {
      drafts.clear();
      say('Saved.');
    } else say(failed()[0]?.issue ?? '');
    if (wrote.includes(true)) {
      try { await loadUsage(); }
      catch { say('Prices saved. Usage totals could not be refreshed.'); }
    }
  });

  $('price-refresh').onclick = () => action(async () => {
    await Promise.all(pending().map(target => read(target)));
    for (const draft of drafts.values()) {
      const settled = pending().every(target => {
        if (!target.rows) return false;
        try { return same(apply(draft.expected.get(target), draft.patch), saved(target, draft.model)); }
        catch { return false; }
      });
      if (settled) drafts.delete(draft.model);
    }
    if (!drafts.size && failed().length) clearOutcomes();
    say('Saved values refreshed.');
  });

  $('price-read').onclick = () => action(async () => {
    if (listingError) await discover();
    await Promise.all(selected().filter(target => !target.rows).map(target => read(target)));
  });

  $('price-discard').onclick = () => {
    if (busy) return;
    drafts.clear();
    clearOutcomes();
    showModel();
    render();
    fields.model.focus();
    say('Drafts discarded.');
  };
  $('price-cancel-edit').onclick = () => { showModel(); fields.model.focus(); };

  chooser.onclick = event => { if (scopeLocked()) event.preventDefault(); };
  picker.onkeydown = event => {
    if (event.key === 'Escape' && picker.open) {
      event.stopPropagation();
      picker.open = false;
      chooser.focus();
    }
  };
  document.addEventListener('pointerdown', event => {
    if (!picker.contains(event.target)) picker.open = false;
  });

  showModel();
  render();
  void (async () => {
    await workbenchLoaded;
    await discover();
    renderTargets();
    await Promise.all(selected().map(target => read(target)));
    busy = false;
    showModel();
    render();
  })();
})();
