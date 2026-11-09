// Page behavior: the ⌘K dialog, the surface tour (with a dithered dissolve
// between screenshots), the braille mascot, and key tiles that light up.

import { drawBruh, threshold } from "./braille.js";
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
  const from = tabs.find((other) => other.getAttribute("aria-selected") === "true");
  if (from && from !== tab) dissolveFrom(panelImage(from), panelFigure(tab));
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

/** @param {HTMLButtonElement} tab */
function panelOf(tab) {
  return document.getElementById(tab.getAttribute("aria-controls") ?? "");
}
/** @param {HTMLButtonElement} tab */
function panelImage(tab) {
  const img = panelOf(tab)?.querySelector("img");
  return img instanceof HTMLImageElement ? img : null;
}
/** @param {HTMLButtonElement} tab */
function panelFigure(tab) {
  const figure = panelOf(tab)?.querySelector("figure");
  return figure instanceof HTMLElement ? figure : null;
}

// ---------------------------------------------------------------------------
// Dissolve: the new screenshot shows at once; the old one sits on a canvas
// above it and is eaten away in braille-sized dots, in Bayer order. Nothing
// waits on it, and a second switch cuts the first one short.

const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");
const DISSOLVE_MS = 280;
const DISSOLVE_PITCH = 7;
let dissolveRun = 0;

/**
 * @param {HTMLImageElement | null} img
 * @param {HTMLElement | null} figure
 */
function dissolveFrom(img, figure) {
  const run = ++dissolveRun;
  for (const old of document.querySelectorAll("canvas.dissolve")) old.remove();
  if (reducedMotion.matches || !img || !figure || !img.complete || img.naturalWidth === 0) return;

  const width = figure.clientWidth;
  const height = figure.clientHeight;
  if (width === 0 || height === 0) return;
  const scale = Math.min(devicePixelRatio || 1, 2);
  const canvas = document.createElement("canvas");
  canvas.className = "dissolve";
  canvas.setAttribute("aria-hidden", "true");
  canvas.width = Math.round(width * scale);
  canvas.height = Math.round(height * scale);
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.scale(scale, scale);
  figure.append(canvas);

  const cols = Math.ceil(width / DISSOLVE_PITCH);
  const rows = Math.ceil(height / DISSOLVE_PITCH);
  const start = performance.now();

  /** @param {number} now */
  function frame(now) {
    if (run !== dissolveRun || !ctx) return;
    const progress = (now - start) / DISSOLVE_MS;
    if (progress >= 1) {
      canvas.remove();
      return;
    }
    ctx.globalCompositeOperation = "source-over";
    ctx.clearRect(0, 0, width, height);
    ctx.drawImage(/** @type {HTMLImageElement} */ (img), 0, 0, width, height);
    ctx.globalCompositeOperation = "destination-out";
    const holes = new Path2D();
    for (let row = 0; row < rows; row++) {
      for (let col = 0; col < cols; col++) {
        // Each dot starts at its threshold and takes 40% of the run to open.
        const local = Math.min(1, (progress - threshold(col, row) * 0.6) / 0.4);
        if (local <= 0) continue;
        const x = (col + 0.5) * DISSOLVE_PITCH;
        const y = (row + 0.5) * DISSOLVE_PITCH;
        const radius = local * DISSOLVE_PITCH * 0.75;
        holes.moveTo(x + radius, y);
        holes.arc(x, y, radius, 0, Math.PI * 2);
      }
    }
    ctx.fill(holes);
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
}

// ---------------------------------------------------------------------------
// Surface tour: steps through the tabs until the visitor takes over. Each
// step is the active tab's progress-bar animation (length in site.css), so
// pausing is just pausing that animation. Any tab pick ends the tour.

const tour = document.querySelector("[data-surfaces]");
const tourToggle = tour?.querySelector("[data-tour-toggle]");
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
// Braille bruh: the mascot drawn in dithered dots. The hero one resolves out
// of noise once; the closing one peeks up from the bottom edge. Both watch
// the pointer. Redraws happen only on resize, pointer moves and the reveal.

const BRUH_COLORS = { face: "#fada5e", mouth: "#fc4141", halo: "#443a2c" };
const REVEAL_MS = 700;

/**
 * @typedef {object} Bruh
 * @property {HTMLCanvasElement} canvas
 * @property {CanvasRenderingContext2D} ctx
 * @property {"hero" | "peek"} kind
 * @property {number} width
 * @property {number} height
 * @property {import("./braille.js").Face} face
 * @property {number} pitch
 * @property {number} reveal
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
    face: { cx: 0, cy: 0, r: 1, lookX: 0, lookY: 0, minRy: 0 },
    pitch: 5,
    reveal: reducedMotion.matches ? 1 : 0,
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
  bruh.pitch = Math.max(3.5, Math.min(6, r / 34));
  bruh.face.minRy = (bruh.pitch * 1.1) / r;
}

/** @param {Bruh} bruh */
function drawOne(bruh) {
  if (bruh.width === 0) return;
  drawBruh(bruh.ctx, bruh.width, bruh.height, bruh.face, {
    pitch: bruh.pitch,
    gap: bruh.pitch * 0.6,
    dot: bruh.pitch * 0.36,
    reveal: bruh.reveal,
    colors: BRUH_COLORS,
  });
}

let bruhFrame = 0;
function scheduleBruhs() {
  if (bruhFrame) return;
  bruhFrame = requestAnimationFrame(() => {
    bruhFrame = 0;
    for (const bruh of bruhs) if (bruh.visible) drawOne(bruh);
  });
}

/** @param {Bruh} bruh */
function revealBruh(bruh) {
  if (bruh.reveal >= 1) return;
  const start = performance.now();
  /** @param {number} now */
  const step = (now) => {
    const t = Math.min(1, (now - start) / REVEAL_MS);
    bruh.reveal = t === 1 ? 1 : 1 - (1 - t) ** 3;
    drawOne(bruh);
    if (t < 1) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
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

document.addEventListener("pointermove", (event) => {
  if (event.pointerType !== "mouse") return;
  for (const bruh of bruhs) {
    if (!bruh.visible) continue;
    const rect = bruh.canvas.getBoundingClientRect();
    const dx = event.clientX - (rect.left + bruh.face.cx);
    const dy = event.clientY - (rect.top + bruh.face.cy);
    bruh.face.lookX = Math.max(-1, Math.min(1, dx / 480));
    bruh.face.lookY = Math.max(-1, Math.min(1, dy / 360));
  }
  scheduleBruhs();
}, { passive: true });
