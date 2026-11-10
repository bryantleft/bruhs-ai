import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { chipFrom } from "../src/scripts/chip.js";

// What each browser reports on each kind of Mac. The Apple silicon rows are
// what Chrome reports on an M2 Max; the rest follow each browser's format.
describe("chipFrom", () => {
  const cases: [string, Parameters<typeof chipFrom>[0], ReturnType<typeof chipFrom>][] = [
    ["Chrome on Apple silicon", { architecture: "arm", renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M2 Max, Unspecified Version)", astc: true }, "apple"],
    ["Chrome on Intel", { architecture: "x86", renderer: "ANGLE (Intel Inc., Intel(R) Iris(TM) Plus Graphics 655, OpenGL 4.1)", astc: false }, "intel"],
    ["Chrome on an Intel Mac with AMD graphics", { architecture: "x86", renderer: "ANGLE (ATI Technologies Inc., AMD Radeon Pro 5500M OpenGL Engine, OpenGL 4.1)", astc: false }, "intel"],
    ["Chrome under Rosetta on Apple silicon", { architecture: "x86", renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M1, Unspecified Version)", astc: true }, "apple"],
    ["Chrome with WebGL off on Intel", { architecture: "x86" }, "intel"],
    ["Firefox on Apple silicon", { renderer: "Apple M1, or similar", astc: true }, "apple"],
    ["Firefox on Intel", { renderer: "Intel(R) HD Graphics, or similar", astc: false }, "intel"],
    ["Safari on Apple silicon", { renderer: "Apple GPU", astc: true }, "apple"],
    ["Safari on Intel", { renderer: "Apple GPU", astc: false }, "intel"],
    ["Safari with WebGL off", {}, null],
    ["a virtual Mac", { renderer: "Apple Paravirtual device", astc: false }, null],
  ];
  for (const [name, signals, chip] of cases) {
    it(`reads ${name} as ${chip ?? "unknown"}`, () => assert.equal(chipFrom(signals), chip));
  }
});
