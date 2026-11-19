import { detectChip } from "./chip.js";
async function loadDownloads() {
  const response = await fetch("/api/latest-release");
  if (!response.ok) return;
  const release = await response.json();
  if (!/^v\d+\.\d+\.\d+$/.test(release.tag_name) || !Array.isArray(release.assets)) return;
  const os = document.documentElement.dataset.os;
  const supported = os !== "mac" || (await detectChip()) !== "intel";
  const date = new Date(release.published_at);
  const info = document.querySelector<HTMLAnchorElement>("#release-info a");
  if (info && Number.isFinite(date.getTime())) {
    info.textContent = release.tag_name + " · " + date.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
    info.href = "https://github.com/bryantleft/bruhs-ai/releases/tag/" + release.tag_name;
  }

  for (const row of document.querySelectorAll<HTMLElement>("[data-package]")) {
    const file = row.dataset.package;
    const link = row.querySelector<HTMLAnchorElement>("[data-package-link]");
    const asset = release.assets.find((candidate: { name?: string }) => candidate?.name === file);
    if (!link || !file) continue;
    if (!asset) {
      link.replaceWith(Object.assign(document.createElement("span"), { className: "label", textContent: "Awaiting release" }));
      continue;
    }
    const name = row.querySelector("h3")?.textContent;
    const url = "https://github.com/bryantleft/bruhs-ai/releases/download/" + release.tag_name + "/" + file;
    link.href = url;
    link.textContent = "Download";
    link.setAttribute("aria-label", "Download for " + name);
    const size = row.querySelector("[data-size]");
    if (size && Number.isFinite(asset.size)) size.textContent = " · " + (asset.size / 1e6).toFixed(1) + " MB";
    const recommended = document.querySelector<HTMLAnchorElement>("#recommended-download");
    if (recommended && supported && os === row.dataset.platform) {
      recommended.href = url;
      recommended.textContent = "Download for " + name;
    }
  }
}
// The static page links to the published files even if GitHub is unavailable.
loadDownloads().catch(() => {});
