const nav = document.querySelector(".nav");
const viewport = window.visualViewport;
if (viewport) {
  const syncViewport = () => document.documentElement.style.setProperty("--visible-height", `${viewport.height}px`);
  viewport.addEventListener("resize", syncViewport);
  syncViewport();
}

if (nav instanceof HTMLElement) {
  new ResizeObserver(() => {
    document.documentElement.style.setProperty("--nav-height", `${nav.getBoundingClientRect().height}px`);
    const id = location.hash.slice(1);
    const target = id ? document.getElementById(id) : null;
    if (!target) return;
    const top = target.getBoundingClientRect().top;
    if (top >= 0 && top < nav.getBoundingClientRect().bottom) target.scrollIntoView({ block: "start", behavior: "instant" });
  }).observe(nav);
}
