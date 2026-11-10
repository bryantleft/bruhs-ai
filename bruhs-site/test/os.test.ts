import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";
import { runInNewContext } from "node:vm";

// public/os.js marks <html data-os> so the page offers the Mac download, a
// "Send to your Mac" share or run-from-source. Run it against real devices.
const source = readFileSync(new URL("../public/os.js", import.meta.url), "utf8");

type Device = { ua: string; platform: string; touch?: number; hints?: { platform: string; mobile: boolean } };

function detect(device: Device): string | null {
  const attributes = new Map<string, string>();
  runInNewContext(source, {
    navigator: { userAgent: device.ua, platform: device.platform, maxTouchPoints: device.touch ?? 0, userAgentData: device.hints },
    document: { documentElement: { setAttribute: (name: string, value: string) => attributes.set(name, value) } },
  });
  return attributes.get("data-os") ?? null;
}

const MAC_SAFARI = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/19.0 Safari/605.1.15";

describe("os.js", () => {
  const cases: [string, Device, string][] = [
    ["Mac Chrome", { ua: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36", platform: "MacIntel", hints: { platform: "macOS", mobile: false } }, "mac"],
    ["Mac Safari", { ua: MAC_SAFARI, platform: "MacIntel" }, "mac"],
    ["iPhone", { ua: "Mozilla/5.0 (iPhone; CPU iPhone OS 19_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/19.0 Mobile/15E148 Safari/604.1", platform: "iPhone", touch: 5 }, "phone"],
    ["iPad asking for the desktop site", { ua: MAC_SAFARI, platform: "MacIntel", touch: 5 }, "phone"],
    ["Android", { ua: "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Mobile Safari/537.36", platform: "Linux armv81", touch: 5, hints: { platform: "Android", mobile: true } }, "phone"],
    ["Windows", { ua: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36", platform: "Win32", hints: { platform: "Windows", mobile: false } }, "pc"],
    ["Linux Firefox", { ua: "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0", platform: "Linux x86_64" }, "pc"],
    ["ChromeOS", { ua: "Mozilla/5.0 (X11; CrOS x86_64 14541.0.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36", platform: "Linux x86_64", hints: { platform: "Chrome OS", mobile: false } }, "pc"],
    ["an unknown browser", { ua: "", platform: "" }, "mac"],
  ];
  for (const [name, device, os] of cases) {
    it(`marks ${name} as ${os}`, () => assert.equal(detect(device), os));
  }
});

describe("index.html", () => {
  const html = readFileSync(new URL("../dist/index.html", import.meta.url), "utf8");

  it("runs os.js in <head>, before anything is drawn", () => {
    const head = html.slice(0, html.indexOf("</head>"));
    assert.match(head, /<script src="\/os\.js"><\/script>/);
    assert.doesNotMatch(head, /<script src="\/os\.js"[^>]*\b(?:async|defer|type="module")/);
  });

  it("offers every device its own button in the hero and at the end", () => {
    // An Intel Mac (src/scripts/chip.js) shares the run-from-source button.
    for (const kind of ["for-mac", "for-phone", "for-pc", "for-intel"]) {
      const count = html.match(new RegExp(`class="btn btn-primary[^"]*\\b${kind}\\b[^"]*"`, "g"))?.length ?? 0;
      assert.equal(count, 2, `${kind} buttons`);
    }
  });
});
