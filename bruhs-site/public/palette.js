// Commands for the page's ⌘K bar, and the matcher that filters them.
// No DOM here, so the tests can import it under Node.

/**
 * @typedef {"calendar" | "tasks" | "meetings" | "agents" | "people" | "notes" | "life" | "page"} Tag
 * @typedef {object} Command
 * @property {string} id
 * @property {Tag} tag
 * @property {string} label
 * @property {string} detail
 * @property {string} keywords
 * @property {`#${string}` | `/${string}`} target  Surface tab or section to jump to, or another page.
 * @property {string} [hint]        Shortcut for the same thing in the app.
 * @property {boolean} [featured]   Shown before anything is typed.
 */

/** @type {readonly Command[]} */
export const COMMANDS = [
  {
    id: "calendar",
    tag: "calendar",
    label: "Plan my day",
    detail: "focus blocks around your meetings",
    keywords: "calendar schedule focus booking availability time block",
    target: "#calendar",
    featured: true,
  },
  {
    id: "tasks",
    tag: "tasks",
    label: "Today",
    detail: "overdue, ready, and waiting on",
    keywords: "tasks todo issues cycles priority triage estimates",
    target: "#tasks",
    hint: "⌘⇧Y",
    featured: true,
  },
  {
    id: "meetings",
    tag: "meetings",
    label: "Meeting notes",
    detail: "local transcripts and action items",
    keywords: "record transcribe speakers summary standup",
    target: "#meetings",
    featured: true,
  },
  {
    id: "agents",
    tag: "agents",
    label: "Ask Claude or Codex",
    detail: "agent threads with diffs and a terminal",
    keywords: "chat claude codex code coding thread steer diff terminal worktree",
    target: "#agents",
    hint: "⌘N",
    featured: true,
  },
  {
    id: "people",
    tag: "people",
    label: "People",
    detail: "follow-ups from every conversation",
    keywords: "crm contacts relationships follow-up inbox messages",
    target: "#people",
  },
  {
    id: "notes",
    tag: "notes",
    label: "Notes and clips",
    detail: "docs and web clips your agent can read",
    keywords: "docs knowledge clip markdown research library",
    target: "#notes",
  },
  {
    id: "life",
    tag: "life",
    label: "Money and health",
    detail: "ledger, budgets, sleep, workouts",
    keywords: "finance budget income transactions health sleep workout habits recovery",
    target: "#life",
    featured: true,
  },
  {
    id: "subs",
    tag: "page",
    label: "Bring your own sub",
    detail: "use the Claude and Codex logins you have",
    keywords: "subscription provider login claude codex pricing",
    target: "#subs",
  },
  {
    id: "privacy",
    tag: "page",
    label: "What stays on your machine",
    detail: "and what goes online",
    keywords: "privacy local data offline security",
    target: "#privacy",
  },
  {
    id: "keys",
    tag: "page",
    label: "Keyboard map",
    detail: "the app's default shortcuts",
    keywords: "keyboard shortcuts hotkeys keys",
    target: "#keys",
  },
  {
    id: "shipping",
    tag: "page",
    label: "Changelog",
    detail: "what shipped, week by week",
    keywords: "changelog updates releases new log",
    target: "/changelog",
  },
  {
    id: "get",
    tag: "page",
    label: "Get bruhs",
    detail: "open source, run it from source",
    keywords: "download install mac macos app get open source github source clone release",
    target: "#get",
  },
];

/**
 * Best place a single lowercase word appears in a command, or 0 if nowhere.
 * Label matches outrank tag matches, which outrank keyword and detail matches.
 * @param {Command} command
 * @param {string} word
 * @returns {number}
 */
function scoreWord(command, word) {
  const label = command.label.toLowerCase();
  if (label.startsWith(word)) return 4;
  if (label.split(/\s+/).some((part) => part.startsWith(word))) return 3;
  if (command.tag.startsWith(word)) return 3;
  if (label.includes(word)) return 2;
  if (`${command.keywords} ${command.detail}`.toLowerCase().includes(word)) return 1;
  return 0;
}

/**
 * Commands where every word of the query appears, best first. An empty
 * query gives the featured commands in their listed order.
 * @param {readonly Command[]} commands
 * @param {string} query
 * @returns {Command[]}
 */
export function filterCommands(commands, query) {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return commands.filter((command) => command.featured);

  return commands
    .map((command, index) => {
      const scores = words.map((word) => scoreWord(command, word));
      const total = scores.includes(0) ? 0 : scores.reduce((sum, score) => sum + score, 0);
      return { command, index, total };
    })
    .filter((entry) => entry.total > 0)
    .sort((a, b) => b.total - a.total || a.index - b.index)
    .map((entry) => entry.command);
}
