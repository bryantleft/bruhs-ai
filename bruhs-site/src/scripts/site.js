// Page behavior: the ⌘K dialog, the surface tour (with a dithered dissolve
// between screenshots) and the dot mascot.

import { drawBruh } from "./braille.js";
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
for (const button of document.querySelectorAll("[data-open-palette]")) {
  button.setAttribute("aria-label", `Jump to a section, ${button.querySelector("kbd")?.textContent?.trim() ?? ""}`);
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
 * @param {{ focus?: boolean, animate?: boolean }} [options]
 */
function selectTab(tab, options = {}) {
  const from = tabs.find((other) => other.getAttribute("aria-selected") === "true");
  const animate = options.animate ?? !options.focus;
  hydrateScreenshot(panelOf(tab));
  if (from && from !== tab) dissolve(panelOf(from), panelOf(tab), animate);
  for (const other of tabs) {
    const selected = other === tab;
    other.setAttribute("aria-selected", String(selected));
    other.tabIndex = selected ? 0 : -1;
    const panel = document.getElementById(other.getAttribute("aria-controls") ?? "");
    if (panel) panel.hidden = !selected;
  }
  syncImageHold();
  revealTab(tab, animate);
  if (options.focus) tab.focus({ preventScroll: true });
}

/**
 * On a narrow screen the tab row scrolls: center the selected tab in it, so
 * the tour never highlights a tab you can't see. Scrolls only the row, never
 * the page.
 * @param {HTMLButtonElement} tab
 * @param {boolean} animate
 */
function revealTab(tab, animate) {
  const row = tab.parentElement;
  if (!row || row.scrollWidth <= row.clientWidth) return;
  const tabBox = tab.getBoundingClientRect();
  const rowBox = row.getBoundingClientRect();
  const delta = tabBox.left - rowBox.left - (rowBox.width - tabBox.width) / 2;
  row.scrollBy({ left: delta, behavior: animate && !reducedMotion.matches ? "smooth" : "instant" });
}

// Mark which edges of the tab row have more tabs past them, for the fade.
/** @param {HTMLElement} row */
function observeScrollEdges(row) {
  const markEdges = () => {
    const more = [];
    if (row.scrollLeft > 2) more.push("left");
    if (row.scrollLeft + row.clientWidth < row.scrollWidth - 2) more.push("right");
    row.dataset.more = more.join(" ");
  };
  row.addEventListener("scroll", markEdges, { passive: true });
  new ResizeObserver(markEdges).observe(row);
  markEdges();
}
const tabRow = tabs[0]?.parentElement;
if (tabRow instanceof HTMLElement) observeScrollEdges(tabRow);
const sourceRow = document.querySelector(".source");
if (sourceRow instanceof HTMLElement) observeScrollEdges(sourceRow);

for (const tab of tabs) {
  tab.addEventListener("click", (event) => {
    stopTour();
    selectTab(tab, { animate: event.detail > 0 });
  });
  tab.addEventListener("keydown", (event) => {
    const index = tabs.indexOf(tab);
    /** @type {number | null} */
    let next = null;
    if (event.key === "ArrowRight") next = (index + 1) % tabs.length;
    else if (event.key === "ArrowLeft") next = (index - 1 + tabs.length) % tabs.length;
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

/** @param {HTMLButtonElement} tab */
function panelOf(tab) {
  return document.getElementById(tab.getAttribute("aria-controls") ?? "");
}
// ---------------------------------------------------------------------------
// Dissolve: the new screenshot opens through braille-sized dots over the old
// one, using the 2×2 ordered-dither mask in site.css (.revealing). It's all
// CSS: drawing the old screenshot onto a canvas each
// switch cost a dropped frame on a phone-speed CPU. Nothing waits on it, and
// a second switch cuts the first one short.

const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");

/**
 * @param {HTMLElement | null} from
 * @param {HTMLElement | null} to
 * @param {boolean} animate
 */
function dissolve(from, to, animate) {
  for (const panel of document.querySelectorAll(".panel.leaving")) panel.classList.remove("leaving");
  for (const figure of document.querySelectorAll(".window.revealing")) figure.classList.remove("revealing");
  const figure = to?.querySelector("figure");
  if (!(figure instanceof HTMLElement)) return;
  figure.classList.remove("revealing");
  const img = to?.querySelector("img");
  if (!animate || reducedMotion.matches || typeof CSS.registerProperty !== "function" || !from || !(img instanceof HTMLImageElement) || !img.complete || img.naturalWidth === 0) return;
  // The old screenshot stays put underneath until the new one has fully opened.
  from.classList.add("leaving");
  void figure.offsetWidth; // restart the mask animation
  figure.classList.add("revealing");
}

// ---------------------------------------------------------------------------
// Surface tour: steps through the tabs until the visitor takes over. Each
// step is the active tab's progress-bar animation (length in site.css), so
// pausing is just pausing that animation. Any tab pick ends the tour.

const tour = document.querySelector("[data-surfaces]");
const tourToggle = tour?.querySelector("[data-tour-toggle]");
tour?.addEventListener("animationend", (event) => {
  if (!(event instanceof AnimationEvent) || event.animationName !== "dot-d" || !(event.target instanceof HTMLElement)) return;
  event.target.classList.remove("revealing");
  for (const panel of tour.querySelectorAll(".panel.leaving")) panel.classList.remove("leaving");
});
let touring = false;
/** @type {Set<"hover" | "focus" | "offscreen" | "hidden" | "user" | "loading">} */
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
  holdTour("user", true);
}

/** @param {"hover" | "focus" | "offscreen" | "hidden" | "user" | "loading"} reason @param {boolean} on */
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
  tour.addEventListener("pointerover", (event) => {
    if (event.pointerType === "mouse" && event.target instanceof Node) holdTour("hover", !tourToggle.contains(event.target));
  });
  tour.addEventListener("pointerleave", () => holdTour("hover", false));
  tour.addEventListener("focusin", (event) => holdTour("focus", event.target !== tourToggle));
  tour.addEventListener("focusout", (event) => {
    if (!(event.relatedTarget instanceof Node) || !tour.contains(event.relatedTarget) || event.relatedTarget === tourToggle) holdTour("focus", false);
  });
  new IntersectionObserver(([entry]) => holdTour("offscreen", !entry?.isIntersecting)).observe(tour);
  document.addEventListener("visibilitychange", () => holdTour("hidden", document.hidden));
  reducedMotion.addEventListener("change", () => {
    if (reducedMotion.matches) {
      touring = false;
      syncTour();
      for (const figure of document.querySelectorAll(".window.revealing")) figure.classList.remove("revealing");
      for (const panel of document.querySelectorAll(".panel.leaving")) panel.classList.remove("leaving");
    }
  });
  syncTour();
}

function syncImageHold() {
  const active = tour?.querySelector('.panel:not([hidden]) img');
  holdTour("loading", active instanceof HTMLImageElement && (!active.complete || active.naturalWidth === 0));
}

/** @param {HTMLElement | null} panel */
function hydrateScreenshot(panel) {
  const image = panel?.querySelector("img");
  if (!(image instanceof HTMLImageElement) || !image.dataset.src) return;
  if (image.dataset.srcset) image.srcset = image.dataset.srcset;
  image.src = image.dataset.src;
  delete image.dataset.src;
  delete image.dataset.srcset;
}

for (const panel of document.querySelectorAll(".panel")) {
  const image = panel.querySelector("img");
  const figure = panel.querySelector("figure");
  const feedback = panel.querySelector(".shot-feedback");
  const status = panel.querySelector(".shot-status");
  const retry = panel.querySelector(".shot-retry");
  if (!(image instanceof HTMLImageElement) || !(figure instanceof HTMLElement) || !(feedback instanceof HTMLElement) || !(retry instanceof HTMLButtonElement) || !status) continue;
  let restoreRetryFocus = false;
  const update = () => {
    const loaded = image.complete && image.naturalWidth > 0;
    const failed = image.complete && image.hasAttribute("src") && !image.naturalWidth;
    retry.removeAttribute("aria-disabled");
    if ((loaded || failed) && restoreRetryFocus) {
      if ((document.activeElement === document.body || document.activeElement === retry) && panel instanceof HTMLElement) {
        (loaded ? panel : retry).focus({ preventScroll: true });
      }
      restoreRetryFocus = false;
    }
    figure.toggleAttribute("data-loading", !loaded);
    figure.setAttribute("aria-busy", String(!loaded && !failed));
    feedback.hidden = loaded;
    retry.hidden = !failed;
    status.textContent = loaded ? "" : failed ? "Screenshot couldn't load." : "Loading screenshot…";
    syncImageHold();
  };
  image.addEventListener("load", update);
  image.addEventListener("error", update);
  retry.addEventListener("click", () => {
    if (retry.getAttribute("aria-disabled") === "true") return;
    restoreRetryFocus = document.activeElement === retry;
    figure.setAttribute("aria-busy", "true");
    retry.setAttribute("aria-disabled", "true");
    status.textContent = "Loading screenshot…";
    const src = image.src;
    const srcset = image.srcset;
    image.removeAttribute("srcset");
    image.removeAttribute("src");
    image.srcset = srcset;
    image.src = src;
  });
  update();
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
    selectTab(tab, { animate: false });
    document.getElementById("surfaces")?.scrollIntoView({ block: "start" });
    tab.focus({ preventScroll: true });
    return;
  }
  const section = document.getElementById(id);
  if (!section) return;
  section.scrollIntoView({ block: "start" });
  const focusTarget = section.querySelector("h2");
  if (focusTarget instanceof HTMLElement) focusTarget.focus({ preventScroll: true });
}

// ---------------------------------------------------------------------------
// Command bar

/**
 * Wires a palette to the shared command list.
 * @param {HTMLElement} root
 * @param {(command: Command) => void} onPick
 * @returns {{ reset: () => void, input: HTMLInputElement }}
 */
function mountPalette(root, onPick) {
  const input = /** @type {HTMLInputElement} */ (root.querySelector('input[role="combobox"]'));
  const list = /** @type {HTMLUListElement} */ (root.querySelector('[role="listbox"]'));
  const empty = /** @type {HTMLElement} */ (root.querySelector("[data-palette-empty]"));
  /** @type {Command[]} */
  let shown = [];
  let active = 0;

  function render() {
    shown = filterCommands(COMMANDS, input.value);
    active = Math.min(active, Math.max(shown.length - 1, 0));
    list.replaceChildren(
      ...shown.map((command, index) => {
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
          }),
    );
    empty.hidden = shown.length > 0;
    list.hidden = shown.length === 0;
    input.setAttribute("aria-expanded", String(shown.length > 0));
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
    }
  });

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
dialog.querySelector("[data-close-palette]")?.addEventListener("click", () => dialog.close());

// ---------------------------------------------------------------------------
// Global keys: ⌘K opens the bar, "/" too when not typing.

document.addEventListener("keydown", (event) => {
  const chord = chordOf(event);
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
// Dot Bruh: the mascot as a halftone sphere of dots. The hero one resolves
// out of noise once; the closing one peeks up from the bottom edge. Both turn
// their head toward the pointer, easing there over a few frames. Redraws happen
// only on resize, the reveal and while a head is still turning.

const BRUH_COLORS = { face: "#fada5e", mouth: "#fc4141" };
// How far the head turns at the pointer's farthest reach (1 = its full turn).
const LOOK_REACH = 0.7;
// Exponential smoothing time constant for the turn, in ms: about 200ms to
// settle, the same at 60 or 120 Hz and through dropped frames.
const LOOK_TAU_MS = 70;
const REVEAL_MS = 700;

/**
 * @typedef {object} Bruh
 * @property {HTMLCanvasElement} canvas
 * @property {CanvasRenderingContext2D} ctx
 * @property {"hero" | "peek"} kind
 * @property {number} width
 * @property {number} height
 * @property {import("./braille.js").Face} face
 * @property {{ x: number, y: number }} target  Where the pointer asks the head to look.
 * @property {number} reveal
 * @property {number} revealFrame
 * @property {boolean} visible
 */

/** @type {Bruh[]} */
const bruhs = [];
for (const canvas of document.querySelectorAll("canvas[data-bruh]")) {
  const ctx = canvas instanceof HTMLCanvasElement ? canvas.getContext("2d") : null;
  if (!ctx || !(canvas instanceof HTMLCanvasElement)) continue;
  bruhs.push({
    canvas,
    ctx,
    kind: canvas.dataset.bruh === "peek" ? "peek" : "hero",
    width: 0,
    height: 0,
    face: { cx: 0, cy: 0, r: 1, lookX: 0, lookY: 0 },
    target: { x: 0, y: 0 },
    reveal: reducedMotion.matches ? 1 : 0,
    revealFrame: 0,
    visible: false,
  });
}

/** @param {Bruh} bruh */
function layoutBruh(bruh) {
  const width = bruh.canvas.clientWidth;
  const height = bruh.canvas.clientHeight;
  const scale = Math.min(devicePixelRatio || 1, 2);
  bruh.canvas.width = Math.round(width * scale);
  bruh.canvas.height = Math.round(height * scale);
  bruh.ctx.setTransform(scale, 0, 0, scale, 0, 0);
  bruh.width = width;
  bruh.height = height;
  const r = bruh.kind === "hero" ? Math.min(width, height) * 0.4 : Math.min(height * 0.92, width * 0.32);
  bruh.face.r = r;
  bruh.face.cx = width / 2;
  bruh.face.cy = bruh.kind === "hero" ? height / 2 : height + r * 0.1;
}

/** @param {Bruh} bruh */
function drawOne(bruh) {
  if (bruh.width === 0) return;
  drawBruh(bruh.ctx, bruh.width, bruh.height, bruh.face, { reveal: bruh.reveal, colors: BRUH_COLORS });
}

/** @param {Bruh} bruh */
function revealBruh(bruh) {
  if (reducedMotion.matches) {
    bruh.reveal = 1;
    drawOne(bruh);
    return;
  }
  if (bruh.reveal >= 1 || bruh.revealFrame) return;
  const start = performance.now();
  /** @param {number} now */
  const step = (now) => {
    const t = Math.min(1, (now - start) / REVEAL_MS);
    bruh.reveal = t === 1 ? 1 : 1 - (1 - t) ** 3;
    drawOne(bruh);
    bruh.revealFrame = t < 1 ? requestAnimationFrame(step) : 0;
  };
  bruh.revealFrame = requestAnimationFrame(step);
}

const bruhSeen = new IntersectionObserver((entries) => {
  for (const entry of entries) {
    const bruh = bruhs.find((candidate) => candidate.canvas === entry.target);
    if (!bruh) continue;
    bruh.visible = entry.isIntersecting;
    if (bruh.visible) {
      drawOne(bruh);
      revealBruh(bruh);
    }
  }
});
const bruhResized = new ResizeObserver((entries) => {
  for (const entry of entries) {
    const bruh = bruhs.find((candidate) => candidate.canvas === entry.target);
    if (!bruh) continue;
    layoutBruh(bruh);
    drawOne(bruh);
  }
});
for (const bruh of bruhs) {
  bruhSeen.observe(bruh.canvas);
  bruhResized.observe(bruh.canvas);
}

let lookFrame = 0;
let lastTurn = 0;
/** @param {number} now */
function turnHeads(now) {
  lookFrame = 0;
  // Frame-rate independent: cover 1 - e^(-dt/τ) of the remaining turn.
  const dt = lastTurn ? Math.min(64, now - lastTurn) : 16;
  lastTurn = now;
  const k = 1 - Math.exp(-dt / LOOK_TAU_MS);
  let turning = false;
  for (const bruh of bruhs) {
    if (!bruh.visible) continue;
    const dx = bruh.target.x - (bruh.face.lookX ?? 0);
    const dy = bruh.target.y - (bruh.face.lookY ?? 0);
    if (Math.abs(dx) < 0.001 && Math.abs(dy) < 0.001) continue;
    bruh.face.lookX = (bruh.face.lookX ?? 0) + dx * k;
    bruh.face.lookY = (bruh.face.lookY ?? 0) + dy * k;
    drawOne(bruh);
    turning = true;
  }
  if (turning) lookFrame = requestAnimationFrame(turnHeads);
  else lastTurn = 0;
}

document.addEventListener("pointermove", (event) => {
  // A still face under reduced motion; touch has no hover to follow.
  if (event.pointerType !== "mouse" || reducedMotion.matches) return;
  for (const bruh of bruhs) {
    if (!bruh.visible) continue;
    const rect = bruh.canvas.getBoundingClientRect();
    const dx = event.clientX - (rect.left + bruh.face.cx);
    const dy = event.clientY - (rect.top + bruh.face.cy);
    bruh.target.x = Math.max(-1, Math.min(1, dx / 480)) * LOOK_REACH;
    bruh.target.y = Math.max(-1, Math.min(1, dy / 360)) * LOOK_REACH;
  }
  lookFrame ||= requestAnimationFrame(turnHeads);
}, { passive: true });

// The pointer left the window: look ahead again.
document.documentElement.addEventListener("mouseleave", () => {
  if (reducedMotion.matches) return;
  for (const bruh of bruhs) bruh.target = { x: 0, y: 0 };
  lookFrame ||= requestAnimationFrame(turnHeads);
});

reducedMotion.addEventListener("change", () => {
  if (!reducedMotion.matches) return;
  cancelAnimationFrame(lookFrame);
  lookFrame = 0;
  lastTurn = 0;
  for (const bruh of bruhs) {
    cancelAnimationFrame(bruh.revealFrame);
    bruh.revealFrame = 0;
    bruh.reveal = 1;
    bruh.target = { x: 0, y: 0 };
    bruh.face.lookX = 0;
    bruh.face.lookY = 0;
    drawOne(bruh);
  }
});

// ---------------------------------------------------------------------------
// Copy buttons: `data-copy` names the element whose text goes to the clipboard.

for (const button of document.querySelectorAll("button[data-copy]")) {
  if (!(button instanceof HTMLButtonElement)) continue;
  const label = button.textContent;
  let reset = 0;
  button.addEventListener("click", async () => {
    if (button.getAttribute("aria-disabled") === "true") return;
    const source = document.getElementById(button.getAttribute("data-copy") ?? "");
    const command = source?.querySelector("code") ?? source;
    const status = document.querySelector("[data-copy-status]");
    // Copy the command itself, without the "$ " prompt.
    const text = (command?.textContent ?? "").replace(/^\s*\$\s*/, "").trim();
    clearTimeout(reset);
    button.textContent = label;
    if (status) status.textContent = "";
    button.setAttribute("aria-disabled", "true");
    try {
      await navigator.clipboard.writeText(text);
      button.textContent = "Copied";
      if (status) status.textContent = "Command copied.";
      reset = window.setTimeout(() => {
        button.textContent = label;
        if (status) status.textContent = "";
      }, 1600);
    } catch {
      if (command) {
        const range = document.createRange();
        range.selectNodeContents(command);
        const selection = window.getSelection();
        selection?.removeAllRanges();
        selection?.addRange(range);
        if (status) status.textContent = `Command selected. Press ${IS_MAC ? "⌘C" : "Ctrl+C"} to copy.`;
      }
    } finally {
      button.removeAttribute("aria-disabled");
    }
  });
}

// "Send to desktop" (phones and tablets, see public/os.js): the share sheet
// hands the link to AirDrop, Messages or Mail; without one, copy it.
for (const button of document.querySelectorAll("button[data-share]")) {
  if (!(button instanceof HTMLButtonElement)) continue;
  const label = button.querySelector("[data-label]");
  const text = label?.textContent ?? "";
  const status = button.parentElement?.querySelector("[data-share-status]");
  const fallback = button.parentElement?.querySelector("[data-share-url]");
  let reset = 0;
  button.addEventListener("click", async () => {
    if (button.getAttribute("aria-disabled") === "true") return;
    const link = { title: "bruhs", text: "bruhs, one window for your whole day. Download it on your desktop.", url: "https://bruhs.ai/download" };
    clearTimeout(reset);
    if (label) label.textContent = text;
    if (status) status.textContent = "";
    button.setAttribute("aria-disabled", "true");
    try {
      if (navigator.share) {
        try {
          await navigator.share(link);
          return;
        } catch (error) {
          if (error instanceof DOMException && error.name === "AbortError") return;
        }
      }
      try {
        await navigator.clipboard.writeText(link.url);
        if (label) label.textContent = "Link copied";
        if (status) status.textContent = "Link copied. Open it on your desktop.";
      } catch {
        if (fallback instanceof HTMLInputElement) {
          fallback.hidden = false;
          if (document.activeElement === button || document.activeElement === document.body) {
            fallback.focus();
            fallback.select();
          }
        }
        if (status) status.textContent = "Select and copy the link to open it on your desktop.";
      }
    } finally {
      button.removeAttribute("aria-disabled");
    }
    reset = window.setTimeout(() => {
      if (label) label.textContent = text;
    }, 1600);
  });
}
