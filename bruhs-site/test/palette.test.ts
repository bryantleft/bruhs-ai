import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { COMMANDS, filterCommands } from "../public/palette.js";

const ids = (query: string): string[] => filterCommands(COMMANDS, query).map((command) => command.id);

describe("filterCommands", () => {
  it("shows the featured commands, in order, for an empty query", () => {
    const featured = COMMANDS.filter((command) => command.featured).map((command) => command.id);
    assert.ok(featured.length >= 4);
    assert.deepEqual(ids(""), featured);
    assert.deepEqual(ids("   "), featured);
  });

  it("finds a surface by its name", () => {
    assert.equal(ids("meetings")[0], "meetings");
    assert.equal(ids("calendar")[0], "calendar");
  });

  it("finds a surface by a keyword that is not in its label", () => {
    assert.ok(ids("finance").includes("life"));
    assert.ok(ids("codex").includes("agents"));
    assert.ok(ids("crm").includes("people"));
  });

  it("ranks a label prefix above a keyword match", () => {
    assert.equal(ids("join")[0], "join");
  });

  it("requires every word to match", () => {
    assert.deepEqual(ids("meeting transcript"), ["meetings"]);
    assert.deepEqual(ids("meeting zzz"), []);
  });

  it("ignores case", () => {
    assert.deepEqual(ids("MEETINGS"), ids("meetings"));
  });

  it("points every command at a section or a page", () => {
    for (const command of COMMANDS) {
      assert.match(command.target, /^(#[a-z-]+|\/[a-z-]+)$/, `${command.id} has no #section or /page target`);
    }
  });

  it("uses unique ids", () => {
    assert.equal(new Set(COMMANDS.map((command) => command.id)).size, COMMANDS.length);
  });
});
