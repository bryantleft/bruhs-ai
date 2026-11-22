// Subscription capacity is independent of the historical token ledger.
(() => {
  const kindOrder = ['Session', 'Weekly', 'Monthly', 'Other'];
  const host = document.getElementById('pool-results');
  const refresh = document.getElementById('pool-refresh');
  const status = document.getElementById('pool-status');
  const dialog = document.getElementById('pool-dialog');
  const confirm = document.getElementById('pool-confirm');
  const cancel = document.getElementById('pool-cancel');
  let pending = null;
  let lastCheck = 0;
  let chosen = null;
  let opener = null;
  const accountPopup = createPoolAccountPopover(showCredit, resetTime);

  function windowKind(w) {
    const name = w.name.toLowerCase();
    if (w.provider.toLowerCase() === 'cursor') return 'Monthly';
    if (w.provider.toLowerCase() === 'grok') {
      if (name === 'weekly') return 'Weekly';
      if (name === 'monthly') return 'Monthly';
      return 'Other';
    }
    if (w.provider.toLowerCase() === 'codex') {
      if (name === 'individual') return 'Monthly';
      if (w.duration_minutes >= 43200) return 'Monthly';
      if (w.duration_minutes >= 10080) return 'Weekly';
      return 'Session';
    }
    if (w.duration_minutes === 300 || ['session', '5-hour', 'five_hour'].includes(name))
      return 'Session';
    if (w.duration_minutes === 10080 || name.startsWith('weekly') || name.startsWith('seven_day'))
      return 'Weekly';
    if (w.duration_minutes === 43200 || name.startsWith('monthly') || name === 'go · monthly')
      return 'Monthly';
    return 'Other';
  }
  function windowLabel(w, kind) {
    if (w.provider.toLowerCase() === 'codex')
      return w.name === 'individual' ? 'Monthly credit limit' : kind;
    const name = w.name.replace(/^seven_day_/, 'Weekly · ').replaceAll('_', ' ');
    if (
      [
        'primary',
        'secondary',
        '5-hour',
        'five hour',
        'seven day',
        'weekly',
        'session',
        'monthly',
      ].includes(name.toLowerCase()) &&
      kind !== 'Other'
    )
      return kind;
    return name.charAt(0).toUpperCase() + name.slice(1);
  }
  const codexWeights = new Map([
    ['plus', 1],
    ['prolite', 5],
    ['pro', 20],
  ]);
  const claudeWeights = new Map([
    ['pro', 1],
    ['max', 5],
    ['ultra', 20],
  ]);
  function weight(account, kind) {
    const plan = account.plan?.toLowerCase();
    if (account.provider === 'Codex' && ['Session', 'Weekly'].includes(kind)) {
      return codexWeights.get(plan);
    }
    if (account.provider === 'Claude' && kind === 'Session') return claudeWeights.get(plan);
  }
  function groupKey(account, w) {
    const kind = windowKind(w);
    return account.provider === 'Codex' ? kind : kind + '/' + (w.window_id ?? windowLabel(w, kind));
  }
  function member(account, w, now) {
    const kind = windowKind(w);
    const blocked =
      kind === 'Session' &&
      account.windows.find(
        (week) =>
          week.used_percent >= 100 &&
          ((account.provider === 'Codex' && windowKind(week) === 'Weekly') ||
            (account.provider === 'Claude' &&
              ['weekly', 'seven_day'].includes(week.name.toLowerCase()))),
      );
    const original = {
      account,
      used: w.used_percent,
      reset: w.resets_at,
      duration: w.duration_minutes,
      refill: w.used_percent,
    };
    if (!blocked) return original;
    const current = Math.max(now, blocked.observed_at, w.observed_at);
    const unavailable = { account, used: 100, reset: null, duration: null, refill: 0 };
    if (blocked.resets_at > current && w.resets_at > current) {
      if (w.resets_at <= blocked.resets_at)
        return { ...unavailable, reset: blocked.resets_at, refill: 100 };
      if (w.used_percent >= 100) return { ...unavailable, reset: w.resets_at, refill: 100 };
      return { ...unavailable, reset: blocked.resets_at, refill: 100 - w.used_percent };
    }
    return unavailable;
  }
  function providerPools(accounts, now) {
    const first = accounts
      .flatMap((a) => a.windows)
      .sort((a, b) => kindOrder.indexOf(windowKind(a)) - kindOrder.indexOf(windowKind(b)))[0];
    const firstKey = groupKey(accounts[0], first);
    const resetOrder = (account) => {
      const w = account.windows.find((w) => groupKey(account, w) === firstKey);
      return w ? (member(account, w, now).reset ?? Infinity) : Infinity;
    };
    const ordered = [...accounts].sort(
      (a, b) =>
        resetOrder(a) - resetOrder(b) ||
        a.label.localeCompare(b.label, undefined, { sensitivity: 'base' }) ||
        a.sources.join().localeCompare(b.sources.join()),
    );
    const rows = new Map();
    for (const account of ordered) {
      for (const w of account.windows) {
        const kind = windowKind(w);
        let key = groupKey(account, w);
        if (rows.get(key)?.members.has(account)) key += '/' + w.name;
        if (!rows.has(key))
          rows.set(key, { id: w.window_id, kind, label: windowLabel(w, kind), members: new Map() });
        rows.get(key).members.set(account, member(account, w, now));
      }
    }
    const pools = [...rows.values()].sort(
      (a, b) => kindOrder.indexOf(a.kind) - kindOrder.indexOf(b.kind),
    );
    for (const pool of pools) {
      const members = [...pool.members.values()];
      const weighted = members.every((m) => weight(m.account, pool.kind) !== undefined);
      const factor = (m) => (weighted ? weight(m.account, pool.kind) : 1);
      const total = members.reduce((sum, m) => sum + factor(m), 0);
      pool.remaining = Math.round(
        100 - members.reduce((sum, m) => sum + m.used * factor(m), 0) / total,
      );
      let paceSum = 0,
        timedWeight = 0;
      for (const m of members) {
        m.remaining = Math.round(100 - m.used);
        m.restores = Math.round((m.refill * factor(m)) / total);
        if (m.duration > 0 && m.reset != null) {
          const elapsed = Math.min(1, Math.max(0, 1 - (m.reset - now) / (60 * m.duration)));
          m.timeRemaining = (1 - elapsed) * 100;
          paceSum += factor(m) * (m.used - elapsed * 100);
          timedWeight += factor(m);
        }
      }
      if (timedWeight) {
        const pace = paceSum / timedWeight;
        pool.pace = pace > 5 ? 'Ahead of pace' : pace < -5 ? 'Under pace' : 'On pace';
      }
      pool.next = members
        .filter((m) => m.reset != null && m.restores > 0)
        .sort((a, b) => a.reset - b.reset)[0];
    }
    return { ordered, pools };
  }
  // Cursor can report one shared allowance or two independent model allowances.
  function cursorCards(pools) {
    const details = new Map([
      ['totalPercentUsed', 'Total usage for the current billing cycle.'],
      ['autoPercentUsed', 'Grok, Composer, and Auto draw from this allowance.'],
      ['apiPercentUsed', 'Third-party models such as Claude, GPT, and Gemini draw from this allowance.'],
    ]);
    const byId = new Map(pools.map((pool) => [pool.id, pool]));
    const total = byId.get('totalPercentUsed');
    const auto = byId.get('autoPercentUsed');
    const other = byId.get('apiPercentUsed');
    const shared = pools.length === 2 && total && auto &&
      total.members.size === auto.members.size &&
      [...total.members].every(([account, m]) => {
        const peer = auto.members.get(account);
        return peer && peer.used === m.used && peer.reset === m.reset;
      });
    return [...details].flatMap(([id, description]) => {
      const pool = byId.get(id);
      if (!pool || (pool === total && auto && (other || shared))) return [];
      if (shared || (pool === total && pools.length === 1))
        return [{ ...pool, label: null }];
      return [{ ...pool, description }];
    });
  }
  function showCredit(account, trigger) {
    chosen = account.reset;
    opener = trigger;
    document.getElementById('pool-choice').textContent = chosen.hub + ' · ' + account.label;
    document.getElementById('pool-action-status').textContent = '';
    confirm.textContent = 'Use credit';
    dialog.showModal();
    cancel.focus();
  }
  function shortLabel(label) {
    const at = label.indexOf('@');
    return at > 0
      ? (label[0] + (label[at + 1] || '')).toUpperCase()
      : label.replace(/\.json$/i, '');
  }
  function accountLabel(account) {
    const label = el('span', shortLabel(account.email || account.label));
    if (account.email) {
      let hash = 0;
      for (const char of account.email) hash = Math.imul(hash, 31) + char.charCodeAt(0) | 0;
      label.className = 'pool-avatar';
      label.style.setProperty('--hue', Math.abs(hash) % 360);
    }
    return label;
  }
  function drawPool(pool, ordered) {
    const card = el('article');
    card.className = 'pool-card';
    const summary = el('div');
    summary.className = 'pool-summary';
    if (pool.label) summary.append(el('strong', pool.label));
    const amount = el('div');
    amount.append(el('b', pool.remaining + '%'), el('small', ' left'));
    if (pool.pace) {
      const symbol = { 'Ahead of pace': '↗', 'Under pace': '↘', 'On pace': '→' }[pool.pace];
      const pace = el('small', symbol);
      pace.className = 'pool-pace';
      pace.title = pool.pace;
      pace.setAttribute('role', 'img');
      pace.setAttribute('aria-label', pool.pace);
      amount.append(pace);
    }
    summary.append(amount);
    if (pool.next && ordered.length > 1)
      summary.append(el('small', '+' + pool.next.restores + '% ' + resetTime(pool.next.reset)));
    const column = el('div');
    column.className = 'pool-column';
    const track = el('div');
    track.className = 'pool-track';

    track.style.setProperty('--pool-columns', ordered.length);
    ordered.forEach((account, i) => {
      const m = pool.members.get(account);
      const segment = el(m ? 'button' : 'span');
      segment.className = 'pool-segment';
      segment.style.gridColumn = String(i + 1);
      track.append(segment);
      if (!m) {
        if (pool.kind === 'Monthly' && account.zero_spend_cap) {
          segment.classList.add('pool-zero-cap');
          segment.textContent = '$0 cap';
          segment.setAttribute('role', 'img');
          segment.setAttribute('aria-label', shortLabel(account.label) + ', $0 monthly spend cap');
        } else {
          segment.classList.add('pool-missing');
          segment.setAttribute('aria-hidden', 'true');
        }
        return;
      }
      const fill = el('span');
      fill.style.width = m.remaining + '%';
      segment.append(fill);
      if (m.reset != null && m.refill > 0 && m.remaining < 100) {
        const refill = el('span');
        refill.className = 'pool-refill';
        refill.style.width = Math.min(100 - m.remaining, m.refill) + '%';
        segment.append(refill);
      }
      if (m.timeRemaining != null) {
        const tick = el('i');
        tick.style.left = Math.min(99, m.timeRemaining) + '%';
        segment.append(tick);
      }
      const number = el('b', String(i + 1));
      number.className = 'pool-number';
      segment.append(number);
      const name = shortLabel(account.email || account.label);
      const left = m.remaining + '%';
      const reset = m.reset == null ? '' : '↻ ' + resetTime(m.reset).replace(/^in /, '');
      const credit = account.credits ? account.credits + (account.credits === 1 ? ' credit' : ' credits') : '';
      const meta = [reset, credit].filter(Boolean).join(' · ');
      const label = [name, left + ' left', reset && 'Resets ' + resetTime(m.reset),
        m.timeRemaining != null && Math.round(m.timeRemaining) + '% of the window left', credit].filter(Boolean).join(', ');
      const inline = el('span');
      inline.className = 'pool-inline';
      if (ordered.length > 1) inline.append(accountLabel(account), el('b', left));
      if (meta) inline.append(el('small', meta));
      segment.append(inline);
      segment.type = 'button';
      segment.setAttribute('aria-label', label);
      accountPopup.bind(segment, account, m);

      const head = el('button');
      head.className = 'pool-account';
      head.style.gridRow = String(i + 2);
      head.type = 'button';
      const index = el('span', String(i + 1));
      index.className = 'pool-index';
      head.append(index, accountLabel(account), el('b', left));
      if (meta) head.append(el('small', meta));
      head.setAttribute('aria-label', 'Segment ' + (i + 1) + ', ' + label);
      accountPopup.bind(head, account, m);
      track.append(head);
    });
    column.append(track);
    card.append(summary, column);
    if (pool.description) {
      const description = el('small', pool.description);
      description.className = 'pool-description';
      card.append(description);
    }
    return card;
  }
  function resetTime(at) {
    const minutes = Math.max(0, Math.floor((at - Date.now() / 1000) / 60));
    if (!minutes) return 'now';
    if (minutes < 60) return 'in ' + minutes + 'm';
    if (minutes < 1440) return 'in ' + Math.floor(minutes / 60) + 'h ' + (minutes % 60) + 'm';
    return 'in ' + Math.floor(minutes / 1440) + 'd ' + Math.floor((minutes % 1440) / 60) + 'h';
  }
  let desktopLinks = false;
  window.addEventListener('message', (event) => {
    if (
      event.source === window.parent &&
      event.origin === location.origin &&
      event.data?.type === 'bruhs:usage-links-ready'
    )
      desktopLinks = true;
  });
  function usagePage(page) {
    let url;
    try {
      url = new URL(page.url);
    } catch {
      return null;
    }
    if (!['https:', 'http:'].includes(url.protocol) || url.username || url.password) return null;
    const section = el('section');
    section.className = 'external-usage';
    const details = el('div');
    details.append(el('h4', page.label), el('small', page.sources.join(', ')));
    const link = el('a', 'Manage usage ↗');
    link.href = url.href;
    link.target = '_blank';
    link.rel = 'noopener noreferrer';
    link.setAttribute('aria-label', 'Manage usage for ' + page.label);
    link.onclick = (event) => {
      if (!desktopLinks) return;
      event.preventDefault();
      window.parent.postMessage({ type: 'bruhs:open-usage-page', url: link.href }, location.origin);
    };
    section.append(details, link);
    return section;
  }
  function draw(snapshot) {
    const byProvider = new Map();
    for (const a of snapshot.accounts) {
      if (!a.windows.length) continue;
      if (!byProvider.has(a.provider)) byProvider.set(a.provider, []);
      byProvider.get(a.provider).push(a);
    }
    const content = document.createDocumentFragment();
    for (const [name, accounts] of byProvider) {
      const { ordered, pools } = providerPools(accounts, Date.now() / 1000);
      const section = el('section');
      section.className = 'pool-provider';
      section.dataset.provider = name;
      const visible = name === 'Cursor' ? cursorCards(pools) : pools;
      section.append(el('h4', name), ...visible.map((pool) => drawPool(pool, ordered)));
      content.append(section);
    }
    const pages = (snapshot.external || []).map(usagePage).filter(Boolean);
    content.append(...pages);
    if (!byProvider.size && !pages.length && !snapshot.notices.length)
      content.append(el('p', 'No subscription limits reported.'));
    const notices = document.getElementById('pool-notices');
    const list = notices.querySelector('ul');
    const messages = snapshot.notices;
    if (messages.length !== list.children.length ||
        messages.some((message, i) => message !== list.children[i].textContent)) {
      notices.hidden = !messages.length;
      list.replaceChildren(...messages.map((message) => el('li', message)));
    }
    accountPopup.close();
    host.replaceChildren(content);
  }
  function reload() {
    if (pending) return pending;
    lastCheck = Date.now();
    refresh.disabled = true;
    status.textContent = 'Checking subscription limits…';
    pending = api('/usage/pools')
      .then((data) => {
        draw(data);
        status.textContent = '';
      })
      .catch((error) => {
        status.textContent = error.message;
      })
      .finally(() => {
        pending = null;
        refresh.disabled = false;
      });
    return pending;
  }
  function checkVisible() {
    if (
      (lastCheck > 0 && document.hidden) ||
      location.hash === '#providers' ||
      dialog.open ||
      Date.now() - lastCheck < 60000
    )
      return;
    // Preserve focus and expanded account details while the user is reading them.
    if (host.contains(document.activeElement) || accountPopup.isOpen()) return;
    reload();
  }
  refresh.onclick = reload;
  window.addEventListener('hashchange', checkVisible);
  document.addEventListener('visibilitychange', checkVisible);
  setInterval(checkVisible, 60000);
  checkVisible();
  cancel.onclick = () => dialog.close();
  dialog.oncancel = (event) => {
    if (confirm.disabled) event.preventDefault();
  };
  dialog.onclose = () => {
    if (opener?.isConnected) opener.focus();
    else refresh.focus();
  };
  confirm.onclick = async () => {
    if (!chosen || confirm.disabled) return;
    confirm.disabled = true;
    cancel.disabled = true;
    const actionStatus = document.getElementById('pool-action-status');
    actionStatus.textContent = 'Using credit…';
    try {
      const message = await api('/usage/pools/reset/' + encodeURIComponent(chosen.handle), null);
      chosen = null;
      dialog.close();
      if (pending) await pending;
      await reload();
      status.textContent = [message, status.textContent].filter(Boolean).join(' ');
      refresh.focus();
    } catch (error) {
      if (error.status === 400) {
        chosen = null;
        dialog.close();
        await reload();
        status.textContent = [error.message, status.textContent].filter(Boolean).join(' ');
        refresh.focus();
      } else {
        actionStatus.textContent = error.message;
        confirm.textContent = 'Retry same credit';
      }
    } finally {
      confirm.disabled = false;
      cancel.disabled = false;
    }
  };
})();
