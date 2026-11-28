import { detectChip } from "./chip.js";

const REPO = "https://github.com/bryantleft/bruhs-ai";
const IS_MAC = /mac|iphone|ipad/i.test(
  (navigator as { userAgentData?: { platform?: string } }).userAgentData?.platform ?? navigator.platform,
);

function observeScrollEdges(row: HTMLElement) {
  const markEdges = () => {
    const more: string[] = [];
    if (row.scrollLeft > 2) more.push("left");
    if (row.scrollLeft + row.clientWidth < row.scrollWidth - 2) more.push("right");
    row.dataset.more = more.join(" ");
  };
  row.addEventListener("scroll", markEdges, { passive: true });
  new ResizeObserver(markEdges).observe(row);
  markEdges();
}

function wireSourceCopy() {
  const sourceRow = document.querySelector<HTMLElement>(".source");
  if (sourceRow) observeScrollEdges(sourceRow);

  for (const button of document.querySelectorAll<HTMLButtonElement>("button[data-copy]")) {
    const label = button.textContent ?? "Copy";
    let reset = 0;
    button.addEventListener("click", async () => {
      if (button.getAttribute("aria-disabled") === "true") return;
      const source = document.getElementById(button.getAttribute("data-copy") ?? "");
      const command = source?.querySelector("code") ?? source;
      const status = document.querySelector("[data-copy-status]");
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
}

async function applyChipDetection(): Promise<boolean> {
  const os = document.documentElement.dataset.os;
  if (os !== "mac") return true;
  const chip = await detectChip();
  if (chip !== "intel") return true;
  document.documentElement.dataset.chip = "intel";
  const note = document.querySelector<HTMLElement>("#intel-note");
  if (note) note.hidden = false;
  const recommended = document.querySelector<HTMLAnchorElement>("#recommended-download");
  if (recommended) recommended.href = "#source";
  return false;
}

async function loadDownloads() {
  const os = document.documentElement.dataset.os;
  const [supported, response] = await Promise.all([applyChipDetection(), fetch("/api/latest-release")]);
  if (!response.ok) return;
  const release = await response.json();
  if (!/^v\d+\.\d+\.\d+$/.test(release.tag_name) || !Array.isArray(release.assets)) return;

  const versionBadge = document.querySelector<HTMLAnchorElement>("#release-version");
  if (versionBadge) {
    versionBadge.textContent = release.tag_name;
    versionBadge.href = `${REPO}/releases/tag/${release.tag_name}`;
  }

  const date = new Date(release.published_at);
  const dateBadge = document.querySelector<HTMLElement>("#release-date");
  if (dateBadge && Number.isFinite(date.getTime())) {
    dateBadge.textContent = date.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
    dateBadge.hidden = false;
  }

  for (const card of document.querySelectorAll<HTMLElement>("[data-package]")) {
    const file = card.dataset.package;
    const link = card.querySelector<HTMLAnchorElement>("[data-package-link]");
    const asset = release.assets.find((candidate: { name?: string }) => candidate?.name === file);
    if (!link || !file) continue;
    if (!asset) {
      link.replaceWith(Object.assign(document.createElement("span"), { className: "badge", textContent: "Awaiting release" }));
      continue;
    }
    const name = card.querySelector("h3")?.textContent ?? "";
    const url = `${REPO}/releases/download/${release.tag_name}/${file}`;
    link.href = url;
    link.textContent = "Download";
    link.setAttribute("aria-label", `Download for ${name}`);

    const size = card.querySelector<HTMLElement>("[data-size]");
    if (size && Number.isFinite(asset.size)) {
      size.textContent = `${(asset.size / 1e6).toFixed(1)} MB`;
      size.hidden = false;
    }

    const recommended = document.querySelector<HTMLAnchorElement>("#recommended-download");
    if (recommended && supported && os === card.dataset.platform) {
      recommended.href = url;
    }
  }
}

wireSourceCopy();
// The static page links to the published files even if GitHub is unavailable.
loadDownloads().catch(() => {});
