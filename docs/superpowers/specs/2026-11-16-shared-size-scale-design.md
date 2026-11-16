# Shared size scale (shadcn alignment): design

Date: 2026-11-16. Status: approved (B2) and rolled out; see Rollout status.

## Intent

The user found desktop controls oversized and inconsistent, for example a
44px hover box around the 16px sidebar toggle icon. They asked for padding,
sizing and icons across the app to follow shadcn. They chose one shared
scale for web and desktop over a desktop-only pass, so the two apps stay
alike. Fonts and colors stay bruhs.

## Goals

- One size scale for control heights, icon sizes, row heights, padding and
  gaps, taken from shadcn's dense components (Sidebar, DropdownMenu, and the
  `sm` and `icon-sm` Button sizes). It is written into `docs/ui-standards.md`.
- Both apps use the scale. Desktop gets named constants in
  `bruhs-desktop/src/theme.rs`. Web uses the matching Tailwind classes.
- Phones keep 44px tap targets through the existing coarse-pointer rule.
- The scale is named and defined once per surface, so the planned interface
  density and font size settings (`docs/configurability-goal-2026-11-09.md`
  §8) can change it in one place without clipping controls.
- The scale rolls out in slices: shared parts first, then the app frame,
  then chat, then pages. Each slice lands on both surfaces.

## Non-goals

- Typography. The Monaspace families and the 12, 13 and 14px type scale stay.
- Colors, theme tokens, the radius set (mapped below) and shadows (none).
- Layout or information architecture changes, new components, or behavior
  changes beyond size.
- The native mobile app.
- Bar and header heights. The 44px app header (`theme::APP_HEADER_HEIGHT_PX`,
  web `h-11` bars) stays, because it is also the window drag area and holds
  the macOS traffic lights. Only the controls inside bars follow the scale.
  The same goes for other row heights that are layout rather than controls,
  such as the 40px Tasks and Calendar header rows (builder's call).
- The floating meeting dock (`meeting_dock.rs`, `synthetic_dock.rs`, web
  `meeting_live.rs`). It is a separate always-on-top pill, and its Stop
  button ends a live recording, so it keeps its 44px buttons. It is a
  recorded exception, and its tests stay (builder's call).

## Anti-goals

- Phones losing 44px targets, or iOS input zoom protection (16px inputs).
- Labels truncating where they did not before, or content shifting on hover
  or focus (`docs/ui-standards.md`).
- Desktop and web drifting apart. A slice is not done until both surfaces
  match.
- Focus rings disappearing, or any target smaller than 24px (the WCAG 2.2
  minimum target size).

## Constraints found

- Desktop already has a default of 32px and a compact size of 28px
  (`theme::APP_CONTROL_HEIGHT_PX`, `APP_COMPACT_CONTROL_HEIGHT_PX`, and
  `docs/ui-standards.md`). But 23 desktop controls use 44px and 15 use 40px.
  Most desktop icon constants are 12 or 14px; only 3 are 16px.
- Web hard-codes the touch size about 90 times (`min-h-11` 68, `h-11` 8,
  `min-w-11` 8, `size-11` 4) and `min-h-10` 19 times. Web icon sizes are 12px
  (42 uses), 14px (42) and 16px (12).
- Web already enlarges targets on touch. `bruhs-web/tailwind.css` has
  `@media (pointer: coarse)`, which sets 44px on `.bruhs-control`, composer
  chips, `[role="menuitem"]` and `[role="option"]`, and 16px on inputs. The
  hard-coded 44px classes are redundant on phones and oversized with a mouse.
- Some tests pin today's sizes. Examples: the desktop notice Dismiss
  must be at least 44px (`notification_tests.rs`), and dialog metric
  constants live in `tests.rs`.
- The bruhs radii are seed 4px, grape 8px and lychee 14px.
- Trunk bundles Tailwind 3.3, which has no spacing-based minimum sizes.
  `tailwind.config.js` adds only `min-h-10` (40px) and `min-h-11` (44px).
  That is why web buttons use `min-h-[32px]`.
- `docs/configurability-goal-2026-11-09.md` §8 commits to user-set font size
  and interface density "without clipped controls".

## Approaches considered

- **A. Desktop only.** Fastest, but it widens the gap with web: parity
  cases PARITY-08 to PARITY-11 are already open. The user rejected it.
- **B. One shared scale on both surfaces.** The user chose this. It has two
  variants:
  - **B1. shadcn's default sizes as they are:** 36px buttons and inputs, 32px
    small. Rejected (builder's call). It is larger than today's 32px desktop
    default and runs against the "too large" feedback.
  - **B2. shadcn's dense sizes**, the ones its Sidebar and menus use: 32px
    rows and buttons, 28px icon toggles and 16px icons. Chosen (builder's
    call). It matches the existing 32 and 28px standard and the app's dense
    workbench feel.

## The scale

| Name | Size | shadcn source | Use |
| --- | --- | --- | --- |
| control | 32px | Button `sm` (`h-8`), SidebarMenuButton (`h-8`), SidebarInput (`h-8`), DropdownMenuItem (about 32px) | Default buttons, inputs in bars, sidebars and menus, menu rows, list rows, nav items, footer icon buttons |
| control-sm | 28px | SidebarTrigger (`size-7`), SidebarMenuButton `sm` (`h-7`) | Icon toggles in bars and headers, compact secondary buttons |
| control-xs | 24px | SidebarMenuAction (20px; bruhs keeps 24px) | Actions inside rows and cards, such as the thread menu, delete and copy |
| icon | 16px | `[&_svg]:size-4` | Every icon inside a control |
| icon-inline | 14px or 12px | | Icons that decorate text that is not clickable: 14px beside 13 to 14px text, 12px beside 12px text |
| touch | 44px | | Phones only (coarse pointer), on web |

Spacing:

- Sidebar sections and other panel groups get 8px of padding (`p-2`). Menu
  content gets 4px (`p-1`).
- An icon and its label sit 8px apart (`gap-2`), and so do control groups.
  Adjacent icon buttons in a toolbar sit 4px apart (`gap-1`).
- A 32px text button has 12px of horizontal padding (`px-3`). A 28px text
  button has 8px (`px-2`). Icon buttons have a fixed size and no padding.
- A menu row has 8px of horizontal and 6px of vertical padding
  (`px-2 py-1.5`).

Radii, from shadcn to bruhs:

- `rounded-md` (controls and menus) becomes grape (8px).
- `rounded-sm` (menu items) becomes seed (4px).
- `rounded-xl` (dialogs and cards) becomes lychee (14px).

## Decisions

- **Prerequisite.** Today's uncommitted desktop work in the shared checkout
  lands on main before slice 1, because slices 1 and 2 edit the same files.
  That work covers the thread menu, fonts, notices, the one-line sidebar,
  the removed Threads tab, centered dialogs and the 28px toggle. The scale
  builds on it, and the "Constraints found" counts describe it.
- **Where it lives.** The scale has names, defined once per surface:
  - Desktop: `theme.rs` gets `CONTROL_PX`, `CONTROL_SM_PX`, `CONTROL_XS_PX`,
    `ICON_PX`, `ICON_INLINE_PX` and `ICON_INLINE_SM_PX`. The existing
    `APP_CONTROL_HEIGHT_PX` and `APP_COMPACT_CONTROL_HEIGHT_PX` become
    aliases of the first two. Call sites use the names, never raw numbers,
    so a later density setting changes one module.
  - Web: CSS variables in `bruhs-web/tailwind.css` (`--size-control`,
    `--size-control-sm`, `--size-control-xs`, `--size-icon`), exposed as
    Tailwind theme keys in `tailwind.config.js`, for example `min-h-control`
    and `size-control-sm`. `button_class` sizes and `Menu`/`MenuItem` use
    them.
- **Minimum, not fixed, heights.** Text controls use minimum heights, so
  larger text grows them instead of clipping. Only icon buttons have a
  fixed size.
- **Touch.**
  - Web: hard-coded 44px classes are replaced with scale classes. Elements
    rely on the coarse-pointer rule, and `bruhs-control` (or the right role)
    is added where an element lacks it.
  - Desktop has no touch, so it uses the scale directly.
- **Standards doc.** In `docs/ui-standards.md`, the scale table replaces the
  sentence "Default desktop controls use 32px targets; compact secondary
  controls use 28px."
- **Thread menu rows** go from 40px to 32px on both surfaces, like
  DropdownMenuItem.
- **Rollout order.** Each slice covers both surfaces, with tests and
  screenshots:
  1. Shared parts: web `button_class` and desktop `web_button`, icon
     buttons, menus (web `Menu`/`MenuItem`, the thread menu, desktop
     menus), dialog close buttons and dropdowns.
  2. App frame: the top bar (the desktop sidebar toggle is already 28px),
     the sidebar (controls, card actions, fold headers, footer nav) and
     notices.
  3. Chat: header toggles, message actions and the composer. The composer
     keeps its `docs/ui-standards.md` rules, and on web its chips keep 44px
     on touch.
  4. Pages, one per slice: Today, Inbox, Tasks, Calendar, Meetings and
     notes, People, Knowledge, Settings, Workbench, then the rest.
- **Tests.** Existing tests that pin 44 or 40px controls on desktop are
  updated to the scale. The dock and the header bars keep theirs. Each slice
  adds tests that check the scale on representative controls.

## Verification

Each slice is checked as follows:

- Desktop: GPUI size checks on representative controls.
- Desktop: real-window screenshots through smoke scenes, with an isolated
  HOME.
- Web: headless Chrome screenshots at 1280×800 and 640×480, plus 390×844
  touch to confirm the 44px targets remain.
- The full desktop suite, clippy (native and wasm32) and
  `scripts/gate.sh --quick` pass before landing.
- `docs/web-desktop-parity-goal-2026-11-15.md` is updated.

## Left to the builder

- The exact mapping for each component inside a slice, and the order of
  pages in slice 4.
- Names for web helper classes, if a repeated pattern needs one.
- Whether to add a source scan that flags new hard-coded 44px sizes outside
  the touch rules.

## Rollout status

2026-11-16, on both surfaces:

- Slice 1: the names exist (`theme.rs`, `tailwind.css`, `tailwind.config.js`).
  Web `button_class`, `Input` and `MenuItem`, desktop `web_button` (`px-3`),
  and the thread menu rows (32px, `px-2`) use them.
- Slice 2: the sidebar (search, buttons, card actions at 24px with 16px
  icons, 32px fold headers, footer), the 28px sidebar toggle, and notice
  buttons (32px, pulled into the notice's padding).
- Slice 3: chat header toggles (28px, 16px icons), the rename button and the
  composer's square actions (16px icons). Composer chips keep their sizes
  under the composer rules.
- Slice 4: every remaining web `min-h-11`, `size-11`, `min-w-11` and
  `min-h-10` control outside bars, the undo bar and the meeting dock moved to
  the scale with `bruhs-control`; desktop page controls at 40 or 44px moved
  to `CONTROL_PX`. Tests that pinned 44px now pin the scale.

- Follow-up, the same day: the scale gained `icon-xs` (12px), for icons
  beside 12px text in a chip or button, like shadcn's `xs` Button
  (`size-3` icons). The composer chips use it and `control-xs` by name, and
  the composer's queue, attachment and saved prompt buttons hold 16px icons
  in 24px boxes on both surfaces. Desktop page constants that equal a scale
  value, such as the dropdown trigger, the calendar's month buttons and the
  diff panel's actions, now use its name.

- Icon pass, the same day: icons inside controls follow the text beside
  them on both surfaces, 16px in icon-only controls and beside 13 or 14px
  text, 12px beside 12px text. That covered the diff, terminal, preview and
  right panels, the updates pill, the settings close and navigation, the
  stepper, the dropdowns, the project and model pickers, the files list, the
  copy buttons and the dialog close. Close buttons smaller than 24px (the
  terminal tabs, the project delete) became 24px targets.

Nothing in this design is open. Controls that differ in kind rather than
size are tracked as PARITY-14 in `docs/web-desktop-parity-goal-2026-11-15.md`.
