# Product configurability goal

Recorded 2026-11-09. **Status: active.** All eight areas below are accepted
product scope. This document establishes the goal and delivery criteria; it
does not claim that these features have shipped.

Users should be able to control meaningful app behavior without editing source
code or restarting services for ordinary preference changes. Keep useful
defaults, explain what each choice affects, and maintain the same behavior on
web and desktop.

## Starting point

The app already exposes chat models, access modes, keybindings, some recording
rules, speaker controls, and limited global/project thread-title instructions.
Those settings should be extended and preserved rather than replaced with a
second configuration system. Meeting summaries still select a fixed model in
[`summarize`](../bruhs-core/src/summarize/mod.rs), and audio capture uses the
system default microphone in [`audio.rs`](../bruhs-capture/src/audio.rs).

The first transcription-provider implementation supports local Whisper or one
configured OpenAI-compatible endpoint. Model/provider preferences are in the
app, but endpoint, credential, and request-deadline setup remain native-side
configuration. Making service setup easier belongs to this goal too. See the
[transcription goal](transcription-goal-2026-11-09.md) and
[current provider setup](transcription-providers.md).

## Shared settings rules

- Resolve **global defaults → project overrides → individual task/run
  overrides** where those scopes make sense. Devices and credentials remain
  device-scoped; personal display preferences do not require project profiles.
- Show the effective value and where it came from. Support returning to an
  inherited value, resetting a section, and previewing a profile before applying
  it. Resetting a preference must not delete user content.
- Keep one typed definition, validation policy, and resolution order in shared
  models. Use native persistence and credential storage where needed; web and
  desktop consume the same rules rather than maintaining independent defaults.
- Version stored preferences and migrate existing choices. Report invalid or
  unsaved settings visibly; distinguish unavailable features from disabled ones.
- State when changes take effect. Snapshot settings for a running operation;
  edits normally affect the next operation. Immediate appearance changes may
  apply immediately without restarting work.
- Offer portable import/export of non-secret profiles. Show what an import
  changes; never export credentials or silently broaden permissions, upload
  behavior, recording consent, or memory collection.
- Keep common choices easy to find and put specialist controls under Advanced.
  Do not expose every internal timeout, buffer size, or model threshold as a
  product setting. Operational limits still need evidence and validation.

## Accepted areas and delivery criteria

### 1. AI models by task

Allow separate provider/model defaults for chat, meeting summaries, and
second-opinion reviews, with supported effort or response controls. Carry the
effective selection through the real execution path and record which model ran.
Explain capability limits and unavailable models. Failures must not silently
switch providers, billing mode, or data destination.

### 2. Project profiles

Provide named profiles for model, effort, instructions, access mode, and
workspace defaults. Bind overrides to stable project identity. Users can apply,
edit, duplicate, export, and reset a profile, and override it for one run. Show
inherited settings and preserve existing project/thread choices during migration.

### 3. Summary preferences

Configure output language, concise/detailed output, templates, and inclusion of
decisions, action items, quotes, and observations. Support global defaults and
meeting-specific choices. Templates must preserve the structured fields and
validation required by downstream meeting and People features. Provide a clear
preview and reset path.

### 4. Privacy and retention

Give audio, transcripts, and personal observations independent retention
policies, including whether to keep audio after successful transcription.
Explain which records and derived content a policy affects. Users can inspect
and preview cleanup before enabling it, and see cleanup results or failures.
Preserve source audio needed for failed transcription/recovery unless the user
explicitly chooses to remove it. Do not introduce automatic deletion through a
migration or a new default.

### 5. Recording devices and sources

Expose microphone selection, microphone-only versus microphone-plus-system
audio where supported, and preferred headset behavior. Persist device choices
using stable identities and explain missing-device or permission failures.
Changing a preference must not silently switch an active recording's source.
Keep manual and automatic recordings consistent and make platform limits clear.

### 6. Notifications and automation

Allow notification choices by event type, quiet hours with an explicit time
zone, recording countdown preferences, and automation rules by calendar or
meeting type. Preserve existing skip rules and recording consent. Explain which
rule matched, what will happen, and when; keep preview, cancellation, and failure
feedback available. Test quiet-hour boundaries and overlapping rules.

### 7. Memory controls

Let users choose what is remembered, its personal/project scope, and expiration.
Distinguish factual notes from inferred tone or other observations. Provide ways
to inspect, correct, export, and forget stored memory with source provenance.
Respect person/meeting opt-outs throughout extraction, regeneration, and agent
context assembly; deletion and expiration must affect the actual stored and
retrieved data, not just hide a row.

### 8. Appearance and accessibility

Support font size, interface density, date/time formats, week-start day, and
motion preferences while keeping Bruhs typography, colors, and dark-first
design. Respect system accessibility preferences with understandable app
overrides. Every added control must work with the keyboard, retain visible
focus, and handle text scaling without clipped controls or lost content.

## Delivery order

1. Establish shared settings resolution and migration, then deliver **per-task
   models, project profiles, and privacy/retention** first. Define privacy
   semantics before implementing cleanup or broader memory collection.
2. Deliver summary preferences and memory controls using those shared scopes
   and storage rules.
3. Deliver device/source selection and notification/automation rules, including
   missing-device, consent, quiet-hour, and recovery behavior.
4. Deliver appearance preferences and finish cross-surface persistence,
   import/export, and reset coverage. Accessibility and performance are required
   in every earlier phase as well.

Each area remains open until its settings change the real behavior, survive
reload, preserve legacy preferences, and pass focused behavioral checks on both
surfaces. Track implementation and remaining gaps here as work lands; do not
mark an area complete because its controls merely render.

## Verification and maintenance

Exercise inheritance and override removal, profile switching, invalid imports,
failed saves, reload/migration, unavailable providers/devices, and changes made
during an active operation. Add domain-specific cases for cleanup/recovery,
memory exclusion, automation boundaries, and generated-summary validation.
Check that secrets never enter portable preferences or logs.

Use synthetic data for automated checks. Record implementation evidence and
user-visible limitations per delivered area. Keep settings responsive on
representative data; measure before adding
machine-specific latency thresholds. Revisit defaults and missing controls as
user feedback exposes friction. This standing goal creates no scheduled task
and enables no new recording, upload, retention, or automation policy itself.
