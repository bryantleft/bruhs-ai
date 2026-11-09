// Demo data for landing-page screenshots. Fictional people/companies only.
// Writes raw seed JSON to argv[2]; validate with the bruhs-seed-validator crate.
import { writeFileSync } from 'node:fs';

// Fixed "now": Wed 2026-11-04 11:20 America/Chicago (CST, UTC-6).
export const NOW = Date.parse('2026-11-04T11:20:00-06:00');
const at = (s) => Date.parse(`${s}-06:00`); // 'YYYY-MM-DDTHH:MM'
const min = 60_000, hour = 60 * min, day = 24 * hour;

// ── Projects + tasks ────────────────────────────────────────────────────────
const projects = [
  { id: 'proj-launch', name: 'bruhs.ai launch', color: null },
  { id: 'proj-core', name: 'bruhs-core', color: null },
  { id: 'proj-northwind', name: 'Northwind pilot', color: null },
];
let ti = 0;
const task = (title, project_id, status, priority, due_date, extra = {}) => {
  ti += 1;
  return {
    id: `task-${String(ti).padStart(2, '0')}`,
    project_id, title,
    description: extra.description ?? null,
    status, priority, due_date,
    planning: {
      cycle: extra.cycle ?? null,
      dependency_ids: [],
      estimate_minutes: extra.estimate ?? null,
      recurrence: null,
      time_block_event_ids: [],
      waiting_on: extra.waiting_on ?? null,
    },
    created_at: NOW - (12 - ti) * day,
    updated_at: NOW - ti * hour,
  };
};
const work_items = [
  task('Fix flaky auth test', 'proj-core', 'InProgress', 'Urgent', '2026-11-04', { estimate: 45, description: 'session_expires_after_ttl fails ~1 in 5 runs on CI.' }),
  task('Write launch post draft', 'proj-launch', 'InProgress', 'High', '2026-11-05', { estimate: 90 }),
  task('Record 60s product demo', 'proj-launch', 'Todo', 'High', '2026-11-06', { estimate: 60 }),
  task('Send pilot SOW to Northwind', 'proj-northwind', 'Todo', 'Urgent', '2026-11-03', { estimate: 30, description: 'Scope: 10 seats, 6 weeks, weekly check-in with Dev.' }),
  task('Review SSO requirements doc', 'proj-northwind', 'Todo', 'Medium', '2026-11-02', { waiting_on: 'Dev Patel', description: 'Dev is sending the Okta config details.' }),
  task('Ledger CSV import: handle negative amounts', 'proj-core', 'Todo', 'Medium', '2026-11-05', { estimate: 60 }),
  task('Landing page screenshots', 'proj-launch', 'Todo', 'Medium', '2026-11-04', { estimate: 40 }),
  task('Security questionnaire answers', 'proj-northwind', 'Backlog', 'High', '2026-11-09', { waiting_on: 'Maya Chen' }),
  task('Cut v0.9 release notes', 'proj-core', 'Todo', 'Low', '2026-11-07', { estimate: 30 }),
  task('Waitlist confirmation email copy', 'proj-launch', 'Todo', 'Low', '2026-11-06', { waiting_on: 'Sam Ortiz' }),
  task('Benchmark transcript search on 10k segments', 'proj-core', 'Backlog', 'Medium', null, { estimate: 120 }),
  task('Book pilot kickoff with Northwind team', 'proj-northwind', 'Done', 'Medium', null),
  task('Pick pricing for the pilot tier', 'proj-launch', 'Done', 'High', null),
];

// ── Calendar ────────────────────────────────────────────────────────────────
const calendars = [
  { id: 'cal-local', source: 'local', provider_id: null, name: 'Personal', color: null, sync_enabled: true, primary: true, last_synced_at: null },
];
let ei = 0;
const me = { email: 'alex@bruhs.dev', display_name: 'Alex Rivera', response_status: 'accepted', organizer: false, is_self: true, optional: false };
const person = (name, email) => ({ email, display_name: name, response_status: 'accepted', organizer: false, is_self: false, optional: false });
const MAYA = person('Maya Chen', 'maya@northwind.io');
const DEV = person('Dev Patel', 'dev@northwind.io');
const SAM = person('Sam Ortiz', 'sam@ortiz.studio');
const PRIYA = person('Priya Nair', 'priya@fieldnote.vc');
const event = (title, start, end, extra = {}) => {
  ei += 1;
  return {
    id: `evt-${String(ei).padStart(2, '0')}`,
    title, start_at_ms: at(start), end_at_ms: at(end), all_day: false,
    color: extra.color ?? null, notes: extra.notes ?? null,
    created_at: NOW - 10 * day, updated_at: NOW - 2 * day,
    source: 'local', google_event_id: null, calendar_id: 'cal-local',
    attendees: extra.attendees ?? [], organizer: null,
    location: extra.location ?? null,
    conferencing: extra.meet ? { provider: 'Google Meet', url: 'https://meet.example.com/abc-defg-hij' } : null,
    recurring_event_id: null, html_link: null,
  };
};
const events = [
  // Mon 11-02
  event('Weekly planning', '2026-11-02T08:30', '2026-11-02T09:00'),
  event('Focus: auth refactor', '2026-11-02T09:30', '2026-11-02T12:00', { color: 'blueberry' }),
  event('Northwind pilot kickoff', '2026-11-02T13:00', '2026-11-02T14:00', { attendees: [me, MAYA, DEV], meet: true }),
  event('Gym', '2026-11-02T17:30', '2026-11-02T18:30', { color: 'pandan' }),
  // Tue 11-03
  event('Focus: ledger import', '2026-11-03T09:00', '2026-11-03T11:30', { color: 'blueberry' }),
  event('Design review w/ Sam', '2026-11-03T13:30', '2026-11-03T14:15', { attendees: [me, SAM], meet: true }),
  event('Investor update call', '2026-11-03T15:00', '2026-11-03T15:30', { attendees: [me, PRIYA], meet: true }),
  // Wed 11-04 (today)
  event('Standup w/ Northwind', '2026-11-04T09:00', '2026-11-04T09:20', { attendees: [me, MAYA, DEV], meet: true }),
  event('Focus: flaky auth test', '2026-11-04T09:30', '2026-11-04T11:45', { color: 'blueberry' }),
  event('Lunch with Sam', '2026-11-04T12:15', '2026-11-04T13:15', { location: 'Tacodeli', attendees: [me, SAM] }),
  event('SSO requirements w/ Dev', '2026-11-04T14:00', '2026-11-04T14:45', { attendees: [me, DEV], meet: true }),
  event('Focus: launch post', '2026-11-04T15:00', '2026-11-04T17:00', { color: 'blueberry' }),
  // Thu 11-05
  event('Focus: demo recording', '2026-11-05T09:00', '2026-11-05T11:00', { color: 'blueberry' }),
  event('Pricing sync w/ Maya', '2026-11-05T11:30', '2026-11-05T12:00', { attendees: [me, MAYA], meet: true }),
  event('Dentist', '2026-11-05T14:00', '2026-11-05T15:00', { location: 'South Lamar Dental' }),
  event('Gym', '2026-11-05T17:30', '2026-11-05T18:30', { color: 'pandan' }),
  // Fri 11-06
  event('Standup w/ Northwind', '2026-11-06T09:00', '2026-11-06T09:20', { attendees: [me, MAYA, DEV], meet: true }),
  event('Focus: release notes', '2026-11-06T09:30', '2026-11-06T11:30', { color: 'blueberry' }),
  event('Founder coffee w/ Priya', '2026-11-06T13:00', '2026-11-06T13:45', { location: 'Merit Coffee', attendees: [me, PRIYA] }),
  event('Weekly review', '2026-11-06T16:00', '2026-11-06T16:30'),
  // Evenings + weekend
  event('Long run', '2026-11-01T07:30', '2026-11-01T08:45', { color: 'pandan' }),
  event('Farmers market', '2026-11-01T10:00', '2026-11-01T11:00'),
  event('Plan the week', '2026-11-01T17:00', '2026-11-01T17:30'),
  event('Dinner w/ Jordan', '2026-11-03T19:00', '2026-11-03T20:30', { location: 'Odd Duck', attendees: [me, person('Jordan Lee', 'jordan@lumen.health')] }),
  event('Run', '2026-11-03T07:00', '2026-11-03T07:45', { color: 'pandan' }),
  event('Read: local-first essays', '2026-11-04T20:30', '2026-11-04T21:30'),
  event('Ship launch post draft', '2026-11-05T19:30', '2026-11-05T21:00', { color: 'blueberry' }),
  event('Climbing', '2026-11-07T10:00', '2026-11-07T11:30', { color: 'pandan' }),
  event('Pilot go-live prep', '2026-11-07T14:00', '2026-11-07T15:30', { color: 'blueberry' }),
];

// ── People + relationships ──────────────────────────────────────────────────
const people = [
  ['person-maya', 'Maya Chen', ['Maya']],
  ['person-dev', 'Dev Patel', ['Dev']],
  ['person-sam', 'Sam Ortiz', ['Sam']],
  ['person-priya', 'Priya Nair', ['Priya']],
  ['person-jordan', 'Jordan Lee', []],
  ['person-elena', 'Elena Rossi', []],
].map(([id, display_name, aliases], i) => ({
  id, display_name, aliases, voice_embedding: null,
  observations: [], tracked: true,
  created_at: NOW - (40 - i) * day, updated_at: NOW - (i + 1) * hour,
}));
people[0].observations = [
  { at_meeting_id: 'mtg-kickoff', kind: 'decision', text: 'Pilot starts with 10 seats on the Growth plan.', created_at: at('2026-11-02T14:00') },
  { at_meeting_id: 'mtg-kickoff', kind: 'topic', text: 'Wants weekly usage numbers for the pilot review.', created_at: at('2026-11-02T14:00') },
  { at_meeting_id: 'mtg-kickoff', kind: 'quote', text: '“If it saves my team the Friday status meeting, it pays for itself.”', created_at: at('2026-11-02T14:00') },
];
people[1].observations = [
  { at_meeting_id: 'mtg-kickoff', kind: 'action_item_owner', text: 'Sends Okta SSO config details.', created_at: at('2026-11-02T14:00') },
];
let ri = 0;
const item = (kind, text, extra = {}) => ({
  id: `rel-${++ri}`, kind, text,
  due_at_ms: extra.due ?? null, completed_at_ms: extra.done ?? null,
  source_record_id: extra.source ?? null, created_at_ms: extra.created ?? NOW - 2 * day,
});
const relationships = [
  {
    person_id: 'person-maya', company: 'Northwind', role: 'VP Operations', email: 'maya@northwind.io', phone: '+1 512 555 0142', location: 'Austin, TX',
    notes: 'Champion for the pilot. Prefers short written updates over calls. Runs a 40-person ops team; cares about fewer status meetings. Two kids, coaches U10 soccer on Saturdays.',
    items: [
      item('commitment', 'Send the pilot SOW with 10 seats and a 6-week timeline', { due: at('2026-11-03T17:00'), source: 'mtg-kickoff', created: at('2026-11-02T14:00') }),
      item('reminder', 'Share week-1 usage numbers before the Friday review', { due: at('2026-11-06T12:00') }),
      item('interaction', 'Pilot kickoff call — agreed on scope and success metrics', { created: at('2026-11-02T14:00'), source: 'mtg-kickoff' }),
      item('note', 'Asked whether transcripts can stay on-device for legal. Yes — confirm in SOW.', { created: at('2026-11-02T14:10') }),
      item('interaction', 'Intro via Priya; first demo went well', { created: at('2026-10-21T10:00') }),
    ],
    last_contacted_at_ms: at('2026-11-04T09:20'), updated_at_ms: NOW - hour,
  },
  {
    person_id: 'person-dev', company: 'Northwind', role: 'IT lead', email: 'dev@northwind.io', phone: '', location: 'Austin, TX',
    notes: 'Owns SSO and security review. Detail-oriented; send docs ahead of calls.',
    items: [
      item('commitment', 'Dev sends Okta SSO config details', { due: at('2026-11-04T12:00'), source: 'mtg-kickoff' }),
      item('reminder', 'Follow up on the security questionnaire', { due: at('2026-11-09T10:00') }),
    ],
    last_contacted_at_ms: at('2026-11-04T09:20'), updated_at_ms: NOW - 2 * hour,
  },
  {
    person_id: 'person-sam', company: 'Ortiz Studio', role: 'Brand designer', email: 'sam@ortiz.studio', phone: '', location: 'Austin, TX',
    notes: 'Did the bruh mascot. Freelance, books 2 weeks out.',
    items: [item('commitment', 'Send final waitlist email copy', { due: at('2026-11-06T17:00') })],
    last_contacted_at_ms: at('2026-11-03T14:15'), updated_at_ms: NOW - 3 * hour,
  },
  {
    person_id: 'person-priya', company: 'Fieldnote Ventures', role: 'Partner', email: 'priya@fieldnote.vc', phone: '', location: 'San Francisco, CA',
    notes: 'Angel check in the pre-seed. Monthly update on the first Tuesday.',
    items: [item('reminder', 'Send November investor update', { due: at('2026-11-03T18:00'), done: at('2026-11-03T15:40') })],
    last_contacted_at_ms: at('2026-11-03T15:30'), updated_at_ms: NOW - 4 * hour,
  },
  { person_id: 'person-jordan', company: 'Lumen Health', role: 'Founder', email: 'jordan@lumen.health', phone: '', location: 'Denver, CO', notes: 'Met at the Austin founders dinner. Building on-call tooling.', items: [item('reminder', 'Check in about a design-partner swap', { due: at('2026-11-12T10:00') })], last_contacted_at_ms: at('2026-10-15T19:00'), updated_at_ms: NOW - 5 * hour },
  { person_id: 'person-elena', company: 'Northwind', role: 'Finance', email: 'elena@northwind.io', phone: '', location: 'Austin, TX', notes: 'Signs off on the pilot invoice.', items: [], last_contacted_at_ms: at('2026-10-29T11:00'), updated_at_ms: NOW - 6 * hour },
];

// ── Meetings ────────────────────────────────────────────────────────────────
const kickoffStart = at('2026-11-02T13:00');
const meetings = [
  {
    id: 'mtg-kickoff', title: 'Northwind pilot kickoff', started_at_ms: kickoffStart, ended_at_ms: kickoffStart + 47 * min,
    status: 'ended', calendar_event_id: 'evt-03', audio_path: null, language: 'en',
    speakers: [
      { id: 'spk-0', label: 'Alex Rivera', person_id: null, color: null },
      { id: 'spk-1', label: 'Maya Chen', person_id: 'person-maya', color: null },
      { id: 'spk-2', label: 'Dev Patel', person_id: 'person-dev', color: null },
    ],
    summary: {
      tldr: 'Northwind starts a 6-week, 10-seat pilot on Nov 9. Success means replacing the Friday status meeting for one ops team. SSO via Okta is the only blocker.',
      action_items: [
        { text: 'Send the pilot SOW (10 seats, 6 weeks)', assignee_person_id: null, due_hint: 'Tue Nov 3' },
        { text: 'Send Okta SSO config details', assignee_person_id: 'person-dev', due_hint: 'Wed Nov 4' },
        { text: 'Share week-1 usage numbers', assignee_person_id: null, due_hint: 'Fri Nov 13' },
        { text: 'Loop in Elena for the pilot invoice', assignee_person_id: 'person-maya', due_hint: 'this week' },
      ],
      decisions: [
        'Pilot runs 6 weeks with 10 seats on the Growth plan.',
        'Transcripts stay on-device; only summaries are shared with the team.',
        'Weekly 20-minute check-in on Wednesdays.',
      ],
      topics: [
        { title: 'Scope', bullets: ['One ops team, 10 seats', 'Start Mon Nov 9, review Dec 18'] },
        { title: 'Security', bullets: ['Okta SSO required', 'Security questionnaire due before go-live'] },
        { title: 'Success metric', bullets: ['Replace the Friday status meeting', 'Weekly usage report to Maya'] },
      ],
      generated_by: 'claude-sonnet-4-6', generated_at: kickoffStart + 50 * min,
    },
    thread_id: null, skip_observations: false,
    created_at: kickoffStart, updated_at: kickoffStart + 50 * min,
  },
  {
    id: 'mtg-design', title: 'Design review w/ Sam', started_at_ms: at('2026-11-03T13:30'), ended_at_ms: at('2026-11-03T14:12'),
    status: 'ended', calendar_event_id: 'evt-06', audio_path: null, language: 'en',
    speakers: [{ id: 'spk-0', label: 'Alex Rivera', person_id: null, color: null }, { id: 'spk-1', label: 'Sam Ortiz', person_id: 'person-sam', color: null }],
    summary: { tldr: 'Landing page goes with real product screenshots instead of illustrations. Mascot stays in the footer and the 404.', action_items: [{ text: 'Capture product screenshots for the landing page', assignee_person_id: null, due_hint: 'Wed' }, { text: 'Send final waitlist email copy', assignee_person_id: 'person-sam', due_hint: 'Fri' }], decisions: ['Use real UI screenshots on the landing page.'], topics: [], generated_by: 'claude-sonnet-4-6', generated_at: at('2026-11-03T14:15') },
    thread_id: null, skip_observations: false, created_at: at('2026-11-03T13:30'), updated_at: at('2026-11-03T14:15'),
  },
  {
    id: 'mtg-investor', title: 'Investor update call', started_at_ms: at('2026-11-03T15:00'), ended_at_ms: at('2026-11-03T15:28'),
    status: 'ended', calendar_event_id: 'evt-07', audio_path: null, language: 'en',
    speakers: [{ id: 'spk-0', label: 'Alex Rivera', person_id: null, color: null }, { id: 'spk-1', label: 'Priya Nair', person_id: 'person-priya', color: null }],
    summary: { tldr: 'Shared October numbers: 1,240 waitlist signups, first paid pilot. Priya offered two intros to ops leaders.', action_items: [{ text: 'Send the deck to Priya’s two intros', assignee_person_id: null, due_hint: 'next week' }], decisions: [], topics: [], generated_by: 'claude-sonnet-4-6', generated_at: at('2026-11-03T15:30') },
    thread_id: null, skip_observations: false, created_at: at('2026-11-03T15:00'), updated_at: at('2026-11-03T15:30'),
  },
  {
    id: 'mtg-standup', title: 'Standup w/ Northwind', started_at_ms: at('2026-11-04T09:00'), ended_at_ms: at('2026-11-04T09:18'),
    status: 'ended', calendar_event_id: 'evt-08', audio_path: null, language: 'en',
    speakers: [{ id: 'spk-0', label: 'Alex Rivera', person_id: null, color: null }, { id: 'spk-1', label: 'Maya Chen', person_id: 'person-maya', color: null }, { id: 'spk-2', label: 'Dev Patel', person_id: 'person-dev', color: null }],
    summary: { tldr: 'SOW signed. Dev will send the Okta config by noon; pilot start on Monday is confirmed.', action_items: [{ text: 'Send Okta config', assignee_person_id: 'person-dev', due_hint: 'today noon' }], decisions: ['Pilot start confirmed for Mon Nov 9.'], topics: [], generated_by: 'claude-sonnet-4-6', generated_at: at('2026-11-04T09:20') },
    thread_id: null, skip_observations: false, created_at: at('2026-11-04T09:00'), updated_at: at('2026-11-04T09:20'),
  },
];
const lines = [
  ['spk-0', 'Thanks for making time. I figured we lock the pilot scope first and then talk security.'],
  ['spk-1', 'Works for me. I’d like to start with one team — my ops leads, about ten people.'],
  ['spk-0', 'Ten seats is easy. Six weeks feels right so you get a real read before the holidays.'],
  ['spk-1', 'Six weeks is good. What I really want to know is whether it replaces our Friday status meeting.'],
  ['spk-1', 'If it saves my team that meeting, it pays for itself.'],
  ['spk-0', 'Then let’s make that the success metric. I’ll send you a usage report every week.'],
  ['spk-2', 'On my side, the only hard requirement is SSO. We’re on Okta.'],
  ['spk-0', 'Okta’s supported. If you send the config details I can have it wired before Monday.'],
  ['spk-2', 'I’ll send those by Wednesday. There’s also a security questionnaire, maybe forty questions.'],
  ['spk-1', 'Legal will ask where transcripts live. Do they leave the laptop?'],
  ['spk-0', 'No. Transcripts stay on-device. Only the summaries get shared with the team, and only if you choose to.'],
  ['spk-1', 'Great, put that in the SOW. Let’s do a twenty-minute check-in every Wednesday.'],
  ['spk-0', 'Done. I’ll have the SOW over by tomorrow end of day.'],
];
let seg = 0, t = 4_000;
const segments = lines.map(([speaker_id, text]) => {
  const dur = 2_500 + text.length * 55;
  const s = { id: `seg-${String(++seg).padStart(3, '0')}`, meeting_id: 'mtg-kickoff', speaker_id, start_ms: t, end_ms: t + dur, text, confidence: 0.94, partial: false, speaker_confidence: 0.91 };
  t += dur + 1_200 + (seg % 3) * 9_000;
  return s;
});

// ── Knowledge ───────────────────────────────────────────────────────────────
const doc = (id, title, body_markdown, tags, created, updated, extra = {}) => ({
  id, title, body_markdown, tags, attachments: [], links: extra.links ?? [], citations: extra.citations ?? [], versions: [],
  created_at_ms: at(created), updated_at_ms: at(updated),
});
const knowledge_documents = [
  doc('doc-pilot', 'Northwind pilot plan', `Six weeks, ten seats, one ops team. Kickoff notes and the plan we agreed with Maya.

## Goal
Replace the Friday status meeting for one ops team (10 seats) within six weeks.

## Timeline
- **Nov 9** — go-live after Okta SSO is wired
- **Wednesdays** — 20-minute check-in with Maya
- **Dec 18** — pilot review and expansion decision

## Success metrics
1. Status meeting cancelled for 3 consecutive weeks
2. 8 of 10 seats active weekly
3. Weekly summary read by Maya's leads

## Open questions
- Security questionnaire owner on our side
- Invoice timing with Elena`, ['northwind', 'pilot', 'sales'], '2026-11-02T14:30', '2026-11-04T09:40', { links: [{ kind: 'meeting', target_id: 'mtg-kickoff', label: 'Northwind pilot kickoff' }, { kind: 'person', target_id: 'person-maya', label: 'Maya Chen' }], citations: [{ clip_id: 'clip-meetings', locator: '§2', note: 'Status-meeting cost estimate' }] }),
  doc('doc-launch', 'Launch checklist', `Everything that has to ship before the launch post goes out Tue Nov 10.

- [x] Pricing for the pilot tier
- [x] Waitlist endpoint + rate limit
- [ ] Landing page screenshots
- [ ] 60s demo video
- [ ] Launch post (draft Thu, publish Tue)
- [ ] Email the 1,240 waitlist signups`, ['launch'], '2026-10-28T10:00', '2026-11-04T08:10'),
  doc('doc-pricing', 'Pricing notes', `Solo is free while local-first. Growth at $18/seat/month covers the shared summaries and SSO. Pilots get 6 weeks at 50% off, converting at list price.`, ['pricing', 'launch'], '2026-10-30T16:00', '2026-11-03T11:00'),
  doc('doc-arch', 'Agent run architecture', `Each thread owns a worktree and a provider session. Turns stream JSON events from the CLI; the server fans them out over SSE. Tool calls are recorded in order so the timeline can replay them.`, ['engineering', 'bruhs-core'], '2026-10-12T09:00', '2026-11-01T20:15'),
  doc('doc-interview', 'Customer interview: Lumen Health', `Jordan runs on-call for a 12-person team. Biggest pain is context lost between handoffs — wants meeting notes linked to the tickets they produced.`, ['research'], '2026-10-15T19:30', '2026-10-16T08:00'),
];
const clip = (id, title, source_url, source_name, author, published_on, excerpt, tags, captured) => ({
  id, title, source_url, source_name, author, published_on, excerpt, tags, captured_at_ms: at(captured), updated_at_ms: at(captured),
});
const source_clips = [
  clip('clip-meetings', 'The real cost of status meetings', 'https://example.com/blog/status-meetings', 'Ops Weekly', 'R. Okafor', '2026-09-18', 'A 30-minute weekly status meeting for ten people costs roughly 250 hours a year — most of it spent repeating what is already written down somewhere.', ['research', 'pilot'], '2026-10-29T21:10'),
  clip('clip-localfirst', 'Local-first software: you own your data', 'https://example.com/essays/local-first', 'Ink & Switch (summary)', '', '2019-04-01', 'Seven ideals for local-first software: fast, multi-device, offline, collaboration, longevity, privacy, and user control.', ['local-first', 'engineering'], '2026-10-20T08:45'),
  clip('clip-sso', 'Okta OIDC app integration guide', 'https://example.com/docs/okta-oidc', 'Okta docs', '', null, 'Create an OIDC web app, set the sign-in redirect URI, then copy the client ID and issuer URL into your app’s SSO settings.', ['northwind', 'sso'], '2026-11-04T09:25'),
];

// ── Life (opt-ins enabled) ──────────────────────────────────────────────────
const tx = (id, merchant, category, amount, when, account_id = 'acct-checking', note = null) => ({ id, account_id, merchant, category, amount_minor: amount, currency: 'USD', occurred_at_ms: at(when), note });
const life = {
  finance: {
    accounts: [
      { id: 'acct-checking', name: 'Everyday checking', kind: 'checking', balance_minor: 842_317, currency: 'USD', institution: 'Lone Star Credit Union', updated_at_ms: NOW - 3 * hour },
      { id: 'acct-savings', name: 'Runway savings', kind: 'savings', balance_minor: 3_150_000, currency: 'USD', institution: 'Lone Star Credit Union', updated_at_ms: NOW - 3 * hour },
      { id: 'acct-card', name: 'Business card', kind: 'credit', balance_minor: -184_250, currency: 'USD', institution: 'Northbank', updated_at_ms: NOW - 3 * hour },
      { id: 'acct-brokerage', name: 'Brokerage', kind: 'investment', balance_minor: 2_284_500, currency: 'USD', institution: 'Index Co.', updated_at_ms: NOW - day },
    ],
    transactions: [
      tx('tx-01', 'Northwind', 'Income', 450_000, '2026-11-04T08:05', 'acct-checking', 'Pilot deposit (50%)'),
      tx('tx-02', 'H-E-B', 'Groceries', -8_642, '2026-11-03T18:40'),
      tx('tx-03', 'Fly.io', 'Software', -4_900, '2026-11-03T02:00', 'acct-card'),
      tx('tx-04', 'Merit Coffee', 'Dining', -1_275, '2026-11-03T08:12', 'acct-card'),
      tx('tx-05', 'Anthropic', 'Software', -10_000, '2026-11-02T00:10', 'acct-card'),
      tx('tx-06', 'Austin Energy', 'Utilities', -11_830, '2026-11-02T09:00'),
      tx('tx-07', 'Tacodeli', 'Dining', -2_460, '2026-11-01T12:30', 'acct-card'),
      tx('tx-08', 'Rent — Eastside Lofts', 'Housing', -185_000, '2026-11-01T08:00'),
      tx('tx-09', 'Vercel', 'Software', -2_000, '2026-11-01T01:00', 'acct-card'),
      tx('tx-10', 'Whole Foods', 'Groceries', -6_318, '2026-10-30T19:05'),
      tx('tx-11', 'Ortiz Studio', 'Contractors', -120_000, '2026-10-30T10:00', 'acct-checking', 'Mascot + email templates'),
      tx('tx-12', 'Gold’s Gym', 'Health', -4_500, '2026-10-29T06:30', 'acct-card'),
    ],
    budgets: [
      { id: 'bud-groceries', category: 'Groceries', monthly_limit_minor: 60_000, currency: 'USD', alert_percent: 80, enabled: true },
      { id: 'bud-dining', category: 'Dining', monthly_limit_minor: 25_000, currency: 'USD', alert_percent: 80, enabled: true },
      { id: 'bud-software', category: 'Software', monthly_limit_minor: 40_000, currency: 'USD', alert_percent: 90, enabled: true },
      { id: 'bud-contractors', category: 'Contractors', monthly_limit_minor: 200_000, currency: 'USD', alert_percent: 75, enabled: true },
    ],
    income_streams: [
      { id: 'inc-northwind', name: 'Northwind pilot', expected_monthly_minor: 450_000, currency: 'USD', active: true },
      { id: 'inc-consulting', name: 'Consulting retainer', expected_monthly_minor: 600_000, currency: 'USD', active: true },
    ],
    holdings: [
      { id: 'hold-vti', account_id: 'acct-brokerage', symbol: 'VTI', name: 'Total Stock Market ETF', quantity: 62, current_value_minor: 1_798_200, currency: 'USD', cost_basis_minor: 1_520_000, updated_at_ms: NOW - day },
      { id: 'hold-bnd', account_id: 'acct-brokerage', symbol: 'BND', name: 'Total Bond Market ETF', quantity: 66, current_value_minor: 486_300, currency: 'USD', cost_basis_minor: 495_000, updated_at_ms: NOW - day },
    ],
    privacy: { allow_agent_context: true, reveal_exact_amounts_to_agent: false },
    updated_at_ms: NOW - 3 * hour,
  },
  health: {
    workouts: [
      { id: 'wo-1', activity: 'Strength — upper body', duration_minutes: 55, perceived_exertion: 7, occurred_at_ms: at('2026-11-02T17:35') },
      { id: 'wo-2', activity: 'Run — Lady Bird Lake loop', duration_minutes: 42, perceived_exertion: 6, occurred_at_ms: at('2026-11-03T07:00') },
      { id: 'wo-3', activity: 'Climbing', duration_minutes: 75, perceived_exertion: 8, occurred_at_ms: at('2026-10-31T10:00') },
      { id: 'wo-4', activity: 'Strength — lower body', duration_minutes: 50, perceived_exertion: 7, occurred_at_ms: at('2026-10-29T17:40') },
      { id: 'wo-5', activity: 'Run — easy', duration_minutes: 30, perceived_exertion: 4, occurred_at_ms: at('2026-10-28T07:10') },
    ],
    sleep: [
      ['2026-11-04T06:40', 452, 86], ['2026-11-03T06:35', 418, 78], ['2026-11-02T06:50', 471, 90],
      ['2026-11-01T08:10', 505, 92], ['2026-10-31T07:45', 389, 64], ['2026-10-30T06:40', 432, 81], ['2026-10-29T06:30', 401, 72],
    ].map(([end, d, q], i) => ({ id: `sl-${i + 1}`, ended_at_ms: at(end), duration_minutes: d, quality_percent: q })),
    recovery: [
      { id: 'rc-1', recorded_at_ms: at('2026-11-04T07:00'), score_percent: 82, resting_heart_rate: 54, note: null },
      { id: 'rc-2', recorded_at_ms: at('2026-11-03T07:00'), score_percent: 71, resting_heart_rate: 57, note: 'Late night on the ledger import' },
      { id: 'rc-3', recorded_at_ms: at('2026-11-02T07:00'), score_percent: 88, resting_heart_rate: 53, note: null },
    ],
    habits: [
      { id: 'hb-1', title: 'No screens after 10pm', target_days_per_week: 5, completion_days: [], archived: false },
      { id: 'hb-2', title: 'Walk after lunch', target_days_per_week: 5, completion_days: [], archived: false },
      { id: 'hb-3', title: 'Stretch 10 min', target_days_per_week: 7, completion_days: [], archived: false },
    ],
    privacy: { allow_agent_context: true },
    updated_at_ms: NOW - 4 * hour,
  },
  dismissed_insight_ids: [],
};
// Habit completion days are local-midnight timestamps.
const mid = (d) => at(`${d}T00:00`);
life.health.habits[0].completion_days = ['2026-11-02', '2026-11-03', '2026-10-30', '2026-10-29'].map(mid);
life.health.habits[1].completion_days = ['2026-11-02', '2026-11-03', '2026-11-04', '2026-10-30'].map(mid);
life.health.habits[2].completion_days = ['2026-11-01', '2026-11-02', '2026-11-03', '2026-11-04', '2026-10-31', '2026-10-30'].map(mid);

// ── Threads + messages ──────────────────────────────────────────────────────
const ROOT = '/Users/alex/code/bruhs-core';
const thread = (id, title, project_name, updated_at, extra = {}) => ({
  id, title, project_name, updated_at,
  project_path: project_name ? `/Users/alex/code/${project_name}` : undefined,
  branch: extra.branch, model: extra.model ?? 'claude-opus-4-8',
  pinned: extra.pinned ?? false, settled: extra.settled ?? false,
});
const threads = [
  thread('th-auth', 'Fix flaky auth test', 'bruhs-core', NOW - 2 * min, { branch: 'fix/flaky-auth-test' }),
  thread('th-ledger', 'Ledger CSV import: negative amounts', 'bruhs-core', NOW - 48 * min, { branch: 'feat/csv-negatives', model: 'gpt-5.5' }),
  thread('th-sse', 'Stream calendar sync over SSE', 'bruhs-server', NOW - 2 * hour, { branch: 'feat/calendar-sse' }),
  thread('th-palette', 'Command bar: rank recent actions first', 'bruhs-web', NOW - 3 * hour, { branch: 'feat/palette-recents', model: 'claude-sonnet-4-6' }),
  thread('th-ratelimit', 'Rate-limit the waitlist endpoint', 'bruhs-site', NOW - 5 * hour, { branch: 'fix/waitlist-rate-limit' }),
  thread('th-search', 'Profile transcript search on 10k segments', 'bruhs-core', NOW - 7 * hour, { branch: 'perf/transcript-search', model: 'gpt-5.5' }),
  thread('th-okta', 'Add Okta OIDC sign-in', 'bruhs-server', NOW - 9 * hour, { branch: 'feat/okta-oidc' }),
  thread('th-release', 'Draft v0.9 release notes', 'bruhs-core', NOW - 3 * day, { branch: 'main', settled: true }),
  thread('th-sow', 'Draft the Northwind pilot SOW', '', NOW - 20 * hour, { model: 'claude-opus-4-8' }),
  thread('th-launch', 'Outline the launch post', '', NOW - 30 * hour, { model: 'claude-sonnet-4-6' }),
];
for (const th of threads) if (th.branch === undefined) delete th.branch;

const t0 = NOW - 9 * min;
const call = (id, tool, input, output, s, e) => ({ kind: 'tool_call', value: { id, tool, input, output, is_error: false, started_at_ms: t0 + s * 1000, ended_at_ms: t0 + e * 1000 } });
const text = (value) => ({ kind: 'text', value });
const sessionOld = `pub fn is_expired(&self) -> bool {
    CLOCK.now() >= self.expires_at
}`;
const sessionNew = `/// Checked against the caller's clock, so each test can own time.
#[must_use]
pub fn is_expired(&self, clock: &dyn Clock) -> bool {
    clock.now() >= self.expires_at
}`;
const testOld = `#[test]
fn session_expires_after_ttl() {
    let session = Session::new(user(), Duration::from_secs(60));
    CLOCK.advance(Duration::from_secs(61));
    assert!(session.is_expired());
}`;
const testNew = `#[test]
fn session_expires_after_ttl() {
    let clock = FakeClock::new();
    let session = Session::new(user(), Duration::from_secs(60));
    clock.advance(Duration::from_secs(61));
    assert!(session.is_expired(&clock));
}

#[test]
fn session_is_live_before_ttl() {
    assert!(!Session::new(user(), Duration::from_secs(60)).is_expired(&FakeClock::new()));
}`;
const authMessages = [
  { id: 'msg-auth-1', role: 'user', content: 'The auth test fails about one run in five. Find out why and fix it.', tool_calls: [], is_streaming: false, thinking: '', usage: null, started_at_ms: t0, completed_at_ms: t0, model: null, parts: [text('The auth test fails about one run in five. Find out why and fix it.')], error: null },
  {
    id: 'msg-auth-2', role: 'assistant', is_streaming: false, thinking: '', content: '', tool_calls: [],
    usage: null,
    started_at_ms: t0 + 2_000, completed_at_ms: t0 + 214_000, model: 'claude-opus-4-8', error: null,
    parts: [
      text('Let me look at how sessions check expiry and how the test drives time.'),
      call('tc-1', 'Read', { file_path: `${ROOT}/src/auth/session.rs` }, '     1\tuse crate::clock::{Clock, CLOCK};\n     2\tuse std::time::{Duration, Instant};\n   ...\n    41\tpub fn is_expired(&self) -> bool {\n    42\t    CLOCK.now() >= self.expires_at\n    43\t}', 4, 5),
      call('tc-3', 'Bash', { command: 'cargo test -p bruhs-core auth', description: 'Run the auth tests' }, 'running 9 tests\ntest auth::session::session_expires_after_ttl ... FAILED\n\nfailures:\n    assertion failed: session.is_expired()\n\ntest result: FAILED. 8 passed; 1 failed; finished in 0.38s', 9, 31),
      text('`session_expires_after_ttl` reads the global `CLOCK`, and `auth_refresh` resets that clock while it runs.'),
      call('tc-4', 'Edit', { file_path: `${ROOT}/src/auth/session.rs`, old_string: sessionOld, new_string: sessionNew }, 'The file src/auth/session.rs has been updated.', 80, 81),
      call('tc-5', 'Edit', { file_path: `${ROOT}/tests/auth_session.rs`, old_string: testOld, new_string: testNew }, 'The file tests/auth_session.rs has been updated.', 84, 85),
      call('tc-6', 'Bash', { command: 'cargo test -p bruhs-core auth -- --test-threads=1', description: 'Run the auth tests serially' }, 'running 10 tests\ntest auth::session::session_expires_after_ttl ... ok\ntest auth::session::session_is_live_before_ttl ... ok\ntest auth::refresh::refresh_extends_expiry ... ok\ntest result: ok. 10 passed; 0 failed; finished in 0.41s', 90, 212),
      text('The test shares one clock across threads, so expiry checks race. I gave each test its own FakeClock.'),
    ],
  },
];
// Derive the legacy flat fields the app writes alongside `parts`.
for (const m of authMessages) {
  m.content = m.parts.filter(p => p.kind === 'text').map(p => p.value).join('');
  m.tool_calls = m.parts.filter(p => p.kind === 'tool_call').map(p => p.value);
}
const simple = (thread_id, prompt, answer, when, model = 'claude-opus-4-8') => ({
  thread_id,
  messages: [
    { id: `${thread_id}-u`, role: 'user', content: prompt, tool_calls: [], is_streaming: false, thinking: '', usage: null, started_at_ms: when, completed_at_ms: when, model: null, parts: [text(prompt)], error: null },
    { id: `${thread_id}-a`, role: 'assistant', content: answer, tool_calls: [], is_streaming: false, thinking: '', usage: null, started_at_ms: when + 2000, completed_at_ms: when + 40_000, model, parts: [text(answer)], error: null },
  ],
});
const messages = [
  { thread_id: 'th-auth', messages: authMessages },
  simple('th-ledger', 'CSV rows with negative amounts in parentheses import as positive. Fix the parser.', 'Fixed: `(12.50)` now parses as −12.50, with a test per bank format.', NOW - 60 * min, 'gpt-5.5'),
  simple('th-sse', 'Move calendar sync from polling to the SSE stream.', 'Calendar sync now pushes over `/api/calendar/stream`; polling is gone.', NOW - 130 * min),
  simple('th-palette', 'Rank recently used actions first in the command bar.', 'Recents now sort above fuzzy matches when the query is empty.', NOW - 200 * min, 'claude-sonnet-4-6'),
  simple('th-ratelimit', 'Add a per-IP rate limit to the waitlist endpoint.', 'Added a 5-per-minute limit per IP with a 429 and Retry-After.', NOW - 310 * min),
  simple('th-search', 'Profile transcript search with 10k segments.', 'p95 is 18 ms; tokenization dominates. Caching lowercase text halves it.', NOW - 26 * hour, 'gpt-5.5'),
  simple('th-okta', 'Add Okta OIDC sign-in for team workspaces.', 'OIDC login works against a dev Okta tenant; config lives in settings.', NOW - 28 * hour),
  simple('th-release', 'Draft v0.9 release notes from the merged PRs.', 'Drafted notes grouped into Agents, Meetings and Life.', NOW - 3 * day),
  simple('th-sow', 'Draft a pilot SOW for Northwind: 10 seats, 6 weeks.', 'Here is a one-page SOW with scope, timeline, success metrics and pricing.', NOW - 20 * hour),
  simple('th-launch', 'Outline the launch post.', 'Hook, the one-surface idea, three demos, local-first, pricing, CTA.', NOW - 30 * hour, 'claude-sonnet-4-6'),
];

const seed = { projects, work_items, calendars, events, people, relationships, meetings, segments, knowledge_documents, source_clips, life, threads, messages };
writeFileSync(process.argv[2], JSON.stringify(seed, null, 1));
console.log('wrote', process.argv[2]);
