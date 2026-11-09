import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { FACE, HALO, MOUTH, sample, threshold } from "../public/braille.js";

const face = { cx: 0, cy: 0, r: 100, lookX: 0, lookY: 0, minRy: 0.05 };

describe("threshold", () => {
  it("uses every Bayer level once per 4×4 tile", () => {
    const levels = new Set<number>();
    for (let row = 0; row < 4; row++) for (let col = 0; col < 4; col++) levels.add(threshold(col, row));
    assert.equal(levels.size, 16);
    for (const level of levels) assert.ok(level > 0 && level < 1);
  });

  it("repeats every four dots", () => {
    assert.equal(threshold(1, 2), threshold(5, 6));
  });
});

describe("sample", () => {
  it("lights the face brightest toward the upper left", () => {
    const lit = sample(-30, -55, face);
    const shaded = sample(70, 60, face);
    assert.equal(lit.kind, FACE);
    assert.equal(shaded.kind, FACE);
    assert.ok(lit.value > shaded.value);
  });

  it("leaves holes for the eyes, and they follow the look", () => {
    assert.equal(sample(-33.6, -25.7, face).kind, 0);
    assert.equal(sample(33.6, -25.7, face).kind, 0);
    const lookingRight = { ...face, lookX: 1 };
    assert.equal(sample(-33.6 + 7, -25.7, lookingRight).kind, 0);
    assert.equal(sample(-33.6 - 12, -25.7, lookingRight).kind, FACE);
  });

  it("draws the mouth below center", () => {
    assert.equal(sample(2.3, 26, face).kind, MOUTH);
  });

  it("fades the halo with distance", () => {
    const near = sample(110, 0, face);
    const far = sample(200, 0, face);
    assert.equal(near.kind, HALO);
    assert.ok(near.value > far.value);
  });
});
