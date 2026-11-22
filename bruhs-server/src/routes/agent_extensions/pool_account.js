// The graph and its compact legend open the same nonmodal account dialog.
function createPoolAccountPopover(showCredit, resetTime) {
  const panel = el('div');
  panel.id = 'pool-account-popup';
  panel.className = 'pool-info';
  panel.setAttribute('role', 'dialog');
  panel.setAttribute('aria-labelledby', 'pool-account-name');
  panel.tabIndex = -1;
  panel.setAttribute('popover', 'manual');
  panel.hidden = true;
  document.body.append(panel);
  let anchor = null;
  let pinned = false;
  let enterTimer;
  let leaveTimer;

  function clearTimers() {
    clearTimeout(enterTimer);
    clearTimeout(leaveTimer);
  }
  function close(restoreFocus = false) {
    clearTimers();
    if (!anchor) return;
    const trigger = anchor;
    anchor = null;
    pinned = false;
    trigger.setAttribute('aria-expanded', 'false');
    panel.hidePopover?.();
    panel.hidden = true;
    panel.replaceChildren();
    document.body.append(panel);
    if (restoreFocus && trigger.isConnected) trigger.focus({ preventScroll: true });
  }
  function position() {
    if (!anchor) return;
    const bounds = anchor.getBoundingClientRect();
    const width = document.documentElement.clientWidth;
    const height = document.documentElement.clientHeight;
    if (!bounds.width || bounds.bottom <= 0 || bounds.top >= height) {
      close();
      return;
    }
    const box = panel.getBoundingClientRect();
    const left = bounds.left + (bounds.width - box.width) / 2;
    const below = bounds.bottom + 5;
    const top = below + box.height <= height - 6 || below < height - bounds.top
      ? below : bounds.top - box.height - 5;
    panel.style.left = Math.max(6, Math.min(left, width - box.width - 6)) + 'px';
    panel.style.top = Math.max(6, Math.min(top, height - box.height - 6)) + 'px';
  }
  function emailSwitch(email) {
    const button = el('button');
    button.type = 'button';
    button.className = 'pool-email';
    button.setAttribute('aria-label', 'Toggle account email visibility');
    const update = (visible) => {
      button.setAttribute('aria-pressed', String(visible));
      button.textContent = visible ? email : email.replace(/[^@._-]/gu, '•');
      if (visible) button.setAttribute('aria-description', email);
      else button.removeAttribute('aria-description');
      button.title = visible ? 'Hide email' : 'Reveal email';
    };
    update(false);
    button.onclick = () => update(button.getAttribute('aria-pressed') !== 'true');
    return button;
  }
  function detailList(rows) {
    const list = el('dl');
    for (const [label, value] of rows) {
      if (value != null && value !== '') list.append(el('dt', label), el('dd', value));
    }
    return list;
  }
  function open(trigger, account, m, keyboard) {
    close();
    anchor = trigger;
    const title = el('strong', account.email ? account.provider : account.label);
    title.id = 'pool-account-name';
    panel.replaceChildren(title);
    if (account.email) panel.append(emailSwitch(account.email));
    panel.append(detailList([
      ['Workspace', account.workspace],
      ['Plan', account.plan],
      ['Signed in', account.on_device ? 'This computer' : null],
      ['Via', account.via.join(', ')],
    ]), detailList([
      ['Left', m.remaining + '%'],
      ['Resets', m.reset != null ? time(m.reset) + ' · ' + resetTime(m.reset) : null],
      ['Even pace', m.timeRemaining != null ? Math.round(m.timeRemaining) + '% left by now' : null],
      ['Restores', m.reset != null && m.restores > 0 ? '+' + m.restores + '% of pool' : null],
    ]));
    if (account.credits) {
      const bank = el('div');
      bank.className = 'pool-bank';
      const credits = el('span', account.credits + ' banked');
      if (account.reset) {
        credits.title = 'Next credit expires ' + time(account.reset.expires);
        const use = el('button', 'Use reset');
        use.type = 'button';
        use.onclick = () => {
          close();
          showCredit(account, trigger);
        };
        bank.append(credits, use);
      } else bank.append(credits);
      panel.append(bank);
    }
    // This DOM order lets Tab continue from the exact trigger that opened it.
    trigger.after(panel);
    panel.hidden = false;
    trigger.setAttribute('aria-expanded', 'true');
    panel.showPopover?.();
    position();
    if (keyboard) (panel.querySelector('button') ?? panel).focus({ preventScroll: true });
  }
  function scheduleClose() {
    clearTimers();
    if (!pinned && !panel.contains(document.activeElement))
      leaveTimer = setTimeout(() => close(), 160);
  }
  function bind(trigger, account, member) {
    trigger.setAttribute('aria-haspopup', 'dialog');
    trigger.setAttribute('aria-expanded', 'false');
    trigger.setAttribute('aria-controls', panel.id);
    trigger.onpointerenter = (event) => {
      if (event.pointerType !== 'mouse') return;
      clearTimers();
      if (anchor !== trigger && !pinned && !panel.contains(document.activeElement))
        enterTimer = setTimeout(() => open(trigger, account, member, false), 300);
    };
    trigger.onpointerleave = scheduleClose;
    trigger.onclick = (event) => {
      clearTimers();
      if (anchor === trigger && pinned) {
        close(true);
        return;
      }
      open(trigger, account, member, event.detail === 0);
      pinned = true;
    };
  }
  panel.onpointerenter = clearTimers;
  panel.onpointerleave = scheduleClose;
  document.addEventListener('pointerdown', (event) => {
    if (anchor && !anchor.contains(event.target) && !panel.contains(event.target)) close();
  }, true);
  document.addEventListener('focusin', (event) => {
    if (anchor && event.target !== anchor && !panel.contains(event.target)) close();
  });
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && anchor) {
      event.preventDefault();
      close(pinned || panel.contains(document.activeElement));
    }
  });
  window.addEventListener('resize', position);
  document.addEventListener('scroll', position, true);
  window.addEventListener('blur', () => close());
  window.addEventListener('hashchange', () => close());
  return { bind, close, isOpen: () => anchor !== null };
}
