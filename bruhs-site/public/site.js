// Page behavior: the ⌘K bar (inline in the hero and as a dialog), the surface
// tour, and key tiles that light up when pressed.

import { COMMANDS, filterCommands } from "./palette.js";

/** @typedef {import("./palette.js").Command} Command */

const IS_MAC = /mac|iphone|ipad/i.test(
  /** @type {{ userAgentData?: { platform?: string } }} */ (navigator).userAgentData?.platform ?? navigator.platform,
);

// ---------------------------------------------------------------------------
// Modifier labels

if (!IS_MAC) {
  for (const el of document.querySelectorAll(".mod")) el.textContent = "Ctrl ";
}

/**
 * Chord string in bruhs keybinding form, e.g. "mod+shift+y".
 * @param {KeyboardEvent} event
 * @returns {string}
 */
function chordOf(event) {
  const parts = [];
  if (IS_MAC ? event.metaKey : event.ctrlKey) parts.push("mod");
  if (event.altKey) parts.push("alt");
  if (event.shiftKey) parts.push("shift");
  const key = event.code === "Space" ? "space" : event.code.startsWith("Key") ? event.code.slice(3).toLowerCase() : event.key.toLowerCase();
  parts.push(key === "bracketleft" ? "[" : key);
  return parts.join("+");
}

/** @param {Element | null} el */
function isTypingIn(el) {
  return el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || (el instanceof HTMLElement && el.isContentEditable);
}

// ---------------------------------------------------------------------------
// Surface tabs

const tabs = /** @type {HTMLButtonElement[]} */ ([...document.querySelectorAll('[role="tab"]')]);

/**
 * @param {HTMLButtonElement} tab
 * @param {{ focus?: boolean }} [options]
 */
function selectTab(tab, options = {}) {
  for (const other of tabs) {
    const selected = other === tab;
    other.setAttribute("aria-selected", String(selected));
    other.tabIndex = selected ? 0 : -1;
    const panel = document.getElementById(other.getAttribute("aria-controls") ?? "");
    if (panel) panel.hidden = !selected;
  }
  if (options.focus) tab.focus();
}

for (const tab of tabs) {
  tab.addEventListener("click", () => {
    stopTour();
    selectTab(tab);
  });
  tab.addEventListener("keydown", (event) => {
    const index = tabs.indexOf(tab);
    /** @type {number | null} */
    let next = null;
    if (event.key === "ArrowDown" || event.key === "ArrowRight") next = (index + 1) % tabs.length;
    else if (event.key === "ArrowUp" || event.key === "ArrowLeft") next = (index - 1 + tabs.length) % tabs.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = tabs.length - 1;
    else if (/^[1-9]$/.test(event.key) && !event.metaKey && !event.ctrlKey) next = Number(event.key) - 1;
    const target = next === null ? undefined : tabs[next];
    if (!target) return;
    event.preventDefault();
    stopTour();
    selectTab(target, { focus: true });
  });
}

// ---------------------------------------------------------------------------
// Surface tour: steps through the tabs until the visitor takes over. Each
// step is the active tab's progress-bar animation (length in site.css), so
// pausing is just pausing that animation. Any tab pick ends the tour.

const tour = document.querySelector("[data-surfaces]");
const tourToggle = tour?.querySelector("[data-tour-toggle]");
const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");
let touring = false;
/** @type {Set<"hover" | "focus" | "offscreen" | "hidden" | "user">} */
const tourHolds = new Set(["offscreen"]);

function syncTour() {
  if (!(tour instanceof HTMLElement) || !(tourToggle instanceof HTMLButtonElement)) return;
  tour.classList.toggle("is-cycling", touring);
  tour.classList.toggle("is-paused", tourHolds.size > 0);
  tourToggle.hidden = !touring;
  tourToggle.dataset.held = String(tourHolds.has("user"));
  const label = tourToggle.querySelector(".cycle-label");
  if (label) label.textContent = tourHolds.has("user") ? "Play tour" : "Pause tour";
}

function stopTour() {
  touring = false;
  syncTour();
}

/** @param {"hover" | "focus" | "offscreen" | "hidden" | "user"} reason @param {boolean} on */
function holdTour(reason, on) {
  if (on) tourHolds.add(reason);
  else tourHolds.delete(reason);
  syncTour();
}

if (tour instanceof HTMLElement && tourToggle instanceof HTMLButtonElement && !reducedMotion.matches) {
  for (const tab of tabs) tab.append(Object.assign(document.createElement("span"), { className: "bar" }));
  touring = true;

  tour.addEventListener("animationend", (event) => {
    if (!touring || !(event.target instanceof Element) || !event.target.classList.contains("bar")) return;
    const index = tabs.findIndex((tab) => tab.getAttribute("aria-selected") === "true");
    const next = tabs[(index + 1) % tabs.length];
    if (next) selectTab(next);
  });
  tourToggle.addEventListener("click", () => holdTour("user", !tourHolds.has("user")));
  // Touch taps fire pointerenter without a matching leave, so only a mouse holds it.
  tour.addEventListener("pointerenter", (event) => event.pointerType === "mouse" && holdTour("hover", true));
  tour.addEventListener("pointerleave", () => holdTour("hover", false));
  tour.addEventListener("focusin", (event) => holdTour("focus", event.target !== tourToggle));
  tour.addEventListener("focusout", (event) => {
    if (!(event.relatedTarget instanceof Node) || !tour.contains(event.relatedTarget) || event.relatedTarget === tourToggle) holdTour("focus", false);
  });
  new IntersectionObserver(([entry]) => holdTour("offscreen", !entry?.isIntersecting), { threshold: 0.5 }).observe(tour);
  document.addEventListener("visibilitychange", () => holdTour("hidden", document.hidden));
  reducedMotion.addEventListener("change", () => reducedMotion.matches && stopTour());
  syncTour();
}

// ---------------------------------------------------------------------------
// Jumping to a command's target

/** @param {Command} command */
function go(command) {
  if (command.target.startsWith("/")) {
    location.assign(command.target);
    return;
  }
  const id = command.target.slice(1);
  const tab = tabs.find((candidate) => candidate.getAttribute("aria-controls") === id);
  if (tab) {
    stopTour();
    selectTab(tab);
    document.getElementById("surfaces")?.scrollIntoView({ block: "start" });
    tab.focus({ preventScroll: true });
    return;
  }
  const section = document.getElementById(id);
  if (!section) return;
  section.scrollIntoView({ block: "start" });
  const focusTarget = id === "get" ? section.querySelector("a[data-primary]") : section.querySelector("h2");
  if (focusTarget instanceof HTMLElement) focusTarget.focus({ preventScroll: true });
}

// ---------------------------------------------------------------------------
// Command bar

/**
 * Wires one palette (hero or dialog) to the shared command list.
 * @param {HTMLElement} root
 * @param {(command: Command) => void} onPick
 * @returns {{ reset: () => void, input: HTMLInputElement }}
 */
function mountPalette(root, onPick) {
  const input = /** @type {HTMLInputElement} */ (root.querySelector('input[role="combobox"]'));
  const list = /** @type {HTMLUListElement} */ (root.querySelector('[role="listbox"]'));
  /** @type {Command[]} */
  let shown = [];
  let active = 0;

  function render() {
    shown = filterCommands(COMMANDS, input.value);
    active = Math.min(active, Math.max(shown.length - 1, 0));
    list.replaceChildren(
      ...(shown.length === 0
        ? [Object.assign(document.createElement("li"), { className: "palette-empty", textContent: "Nothing matches. Try “tasks” or “privacy”." })]
        : shown.map((command, index) => {
            const item = document.createElement("li");
            item.className = "palette-item";
            item.id = `${list.id}-${command.id}`;
            item.setAttribute("role", "option");
            item.setAttribute("aria-selected", String(index === active));

            const tag = document.createElement("span");
            tag.className = `tag tag-${command.tag}`;
            tag.textContent = command.tag;

            const what = document.createElement("span");
            what.className = "what";
            what.append(
              Object.assign(document.createElement("span"), { textContent: command.label }),
              Object.assign(document.createElement("span"), { textContent: command.detail }),
            );

            const hint = document.createElement("span");
            hint.className = "hint";
            if (command.hint) {
              hint.append(Object.assign(document.createElement("kbd"), { textContent: IS_MAC ? command.hint : command.hint.replace("⌘", "Ctrl ") }));
            }

            item.append(tag, what, hint);
            item.addEventListener("pointermove", () => setActive(index));
            item.addEventListener("click", () => onPick(command));
            return item;
          })),
    );
    syncActive();
  }

  function syncActive() {
    const current = shown[active];
    if (current) input.setAttribute("aria-activedescendant", `${list.id}-${current.id}`);
    else input.removeAttribute("aria-activedescendant");
  }

  /** @param {number} index */
  function setActive(index) {
    if (index === active) return;
    list.children[active]?.setAttribute("aria-selected", "false");
    active = index;
    list.children[active]?.setAttribute("aria-selected", "true");
    list.children[active]?.scrollIntoView({ block: "nearest" });
    syncActive();
  }

  input.addEventListener("input", () => {
    active = 0;
    render();
  });
  input.addEventListener("keydown", (event) => {
    if (event.key === "ArrowDown" && shown.length > 0) {
      event.preventDefault();
      setActive((active + 1) % shown.length);
    } else if (event.key === "ArrowUp" && shown.length > 0) {
      event.preventDefault();
      setActive((active - 1 + shown.length) % shown.length);
    } else if (event.key === "Enter") {
      const command = shown[active];
      if (command) {
        event.preventDefault();
        onPick(command);
      }
    } else if (event.key === "Escape" && input.value !== "" && root.dataset.paletteMode === "inline") {
      event.preventDefault();
      input.value = "";
      active = 0;
      render();
    }
  });

  for (const chip of root.querySelectorAll("[data-try]")) {
    chip.addEventListener("click", () => {
      input.value = chip.getAttribute("data-try") ?? "";
      active = 0;
      render();
      input.focus();
    });
  }

  render();
  return {
    input,
    reset() {
      input.value = "";
      active = 0;
      render();
    },
  };
}

const dialog = /** @type {HTMLDialogElement} */ (document.querySelector("dialog.palette-dialog"));
/** @type {Element | null} */
let returnFocus = null;

const dialogPalette = mountPalette(/** @type {HTMLElement} */ (dialog.querySelector("[data-palette]")), (command) => {
  dialog.close();
  go(command);
});

const heroRoot = document.querySelector('[data-palette-mode="inline"]');
if (heroRoot instanceof HTMLElement) mountPalette(heroRoot, go);

function openPalette() {
  if (dialog.open) return;
  returnFocus = document.activeElement;
  dialogPalette.reset();
  dialog.showModal();
  dialogPalette.input.focus();
}

dialog.addEventListener("close", () => {
  // A pick moves focus to its target; only an Escape or backdrop close
  // should hand focus back to where it was.
  if (dialog.contains(document.activeElement) || document.activeElement === document.body) {
    if (returnFocus instanceof HTMLElement) returnFocus.focus({ preventScroll: true });
  }
});
dialog.addEventListener("click", (event) => {
  if (event.target === dialog) dialog.close();
});
for (const button of document.querySelectorAll("[data-open-palette]")) {
  button.addEventListener("click", openPalette);
}

// ---------------------------------------------------------------------------
// Global keys: ⌘K opens the bar, "/" too when not typing; key tiles light up.

const tilesByChord = new Map(
  [...document.querySelectorAll("[data-keys]")].map((tile) => [tile.getAttribute("data-keys") ?? "", tile]),
);
/** @type {WeakMap<Element, number>} */
const tileTimers = new WeakMap();

document.addEventListener("keydown", (event) => {
  const chord = chordOf(event);

  const tile = tilesByChord.get(chord);
  if (tile) {
    tile.classList.add("is-pressed");
    clearTimeout(tileTimers.get(tile));
    tileTimers.set(tile, window.setTimeout(() => tile.classList.remove("is-pressed"), 700));
  }

  if (chord === "mod+k") {
    event.preventDefault();
    if (dialog.open) dialog.close();
    else openPalette();
  } else if (event.key === "/" && !event.metaKey && !event.ctrlKey && !isTypingIn(document.activeElement) && !dialog.open) {
    event.preventDefault();
    openPalette();
  }
});

// ---------------------------------------------------------------------------
// Copy buttons: `data-copy` names the element whose text goes to the clipboard.

for (const button of document.querySelectorAll("button[data-copy]")) {
  const label = button.textContent;
  button.addEventListener("click", async () => {
    const source = document.getElementById(button.getAttribute("data-copy") ?? "");
    if (!source) return;
    try {
      await navigator.clipboard.writeText(source.textContent ?? "");
      button.textContent = "Copied";
    } catch {
      button.textContent = "Select and copy";
    }
    setTimeout(() => { button.textContent = label; }, 1600);
  });
}
