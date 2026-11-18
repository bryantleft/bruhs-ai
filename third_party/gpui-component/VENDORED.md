# Vendored: gpui-component (GPUI Kit)

| | |
|---|---|
| Upstream | https://github.com/longbridge/gpui-component (now published as GPUI Kit) |
| License | Apache-2.0 (see `LICENSE-APACHE`; Longbridge) |
| Version | `gpui-base`, `gpui-component`, `gpui-component-macros`, `gpui-kit-assets` 0.10.0 |
| Upstream commit | `3fbbaec1cd13fa558d996f02721850093898e95e` (2026-11-18, "root: Make Tab work before anything is focused (#4017)") |

Four upstream crates are kept, unformatted, so a diff against upstream shows
only the local changes:

- `crates/base` (`gpui-base`): behavior. It owns input, text, scroll,
  resizable and `Root`.
- `crates/component` (`gpui-component`): the styled components and the theme.
- `crates/component-macros`.
- `crates/assets` (`gpui-kit-assets`).

`kit`, `shell`, `fps`, `webview`, `story` and the examples are not vendored.
`crates/base/examples` is dropped as well.

The crates are **excluded** from the Cargo workspace (`exclude` in the root
`Cargo.toml`). `bruhs-desktop` and `bruhs-ui` use `crates/component` by path.
The desktop's tests also use `crates/base` directly, for
`TextSelection::selected_text`.

The previous copy was 0.5.2 at `0775df3`, a single `crates/ui` crate. The
upgrade to 0.10.0 brought:

- Multi-line inputs became their own type (`TextareaState`, rendered with
  `Textarea`).
- The theme split into a styled `gpui_component::Theme` and a
  `gpui_base::Theme` projection for scrollbars and resize handles. Call
  `Theme::sync_base` after editing fields directly.
- `ScrollbarShow` became `ScrollbarMode`.

## Local changes

Every local change is marked `bruhs:` in the source.

- `Cargo.toml` (workspace):
  - gpui comes from zed git at the rev the root workspace pins, not from
    upstream's `gpui-pre` crates.io snapshots. 0.3.12, upstream's pin, is a
    repackage of the same zed commit with no patches. See "gpui pins" below.
  - `sum-tree` and `reqwest` point at zed's crates and zed's reqwest fork.
- `crates/*/Cargo.toml`:
  - Upstream's optional `gpui-fast` backend is removed. Its `gpui-fast`
    feature stays, empty, so upstream's `cfg`s still parse. Do not enable it.
- `crates/component-macros/src/crate_path.rs`: `IntoPlot` also accepts zed's
  own `gpui` package.
- `crates/base/src/lib.rs`, `root.rs`, `crates/component/src/input/content_type.rs`:
  - The macOS-native window paths (accessibility hit-test forwarder, text
    content type) are gated on `not(any(test, feature = "test-support"))`.
    Upstream uses `not(test)`.
  - `cfg(test)` only reaches this crate's own tests, so without this
    `bruhs-desktop`'s tests would reach the native paths with test windows.
- `crates/base/src/input`: app-owned plain-text ghost completions on
  `TextareaState`. Upstream offers inline completions only to the code
  editor.
  - `set_inline_completion_text` and `inline_completion_text` are new.
  - `TextareaExtras` stores the item, and the shared renderer paints it.
  - Edits and caret moves clear it. Selection and IME suppress it.
  - The empty placeholder is hidden while a completion shows.
  - The chat composer's prompt suggestions use it.
- `crates/component/src/input/textarea.rs`: `Textarea::focus_bordered`,
  forwarded to the `Input` it renders. The composer turns the focus border
  off.
- `crates/base/src/input/base/state.rs`: a disabled input is not a Tab stop,
  as a disabled `<input>` is not on the web. A click still focuses it.
  Upstream keeps every input in the Tab order. The desktop test
  `dialogs_open_on_their_first_control` counts a read-only event dialog's
  stops without its disabled fields.
- `crates/base/src/resizable/mod.rs`: `ResizableState::is_resizing`, so the
  desktop's docked right panel can keep its remembered width (as the web does)
  except while its handle is dragged.
- `crates/base/src/scrollbar.rs`: only the primary button drags the thumb or
  jumps the track. A right-click on a scrollbar no longer starts a drag.
- `crates/base/src/text/state.rs`, streamed Markdown:
  - Upstream's append path re-parses only the last block, with a fresh
    reference table. That can disagree with a full parse:
    - after a list, quote or code block the new text continues;
    - with raw HTML;
    - with reference definitions on either side.
  - The background parser now judges each append against its own document.
    It keeps the fast path only for a paragraph tail that a blank line
    separates from the block before it. Otherwise it parses the whole text
    and reports a replacement.
  - The previous vendored copy followed the same rule.
  - Desktop tests check 11,118 streamed prefixes, plus bursts and nested
    definitions, against fresh parses.
  - Upstream's own `set_text_streaming_markdown_matches_a_full_parse`, which
    expects lists and code to append, no longer holds here. The vendored
    crates' tests are not run.
  - `TextViewState::parsed_content_eq` (`test-support` only) is the hook
    those tests compare with.
- `crates/component/src/title_bar.rs`:
  - `TITLE_BAR_HEIGHT` 34 -> 44.
  - macOS `TITLE_BAR_LEFT_PADDING` 80 -> 88.
  - Traffic lights at (12, 15). Upstream has (9, 9).
- `crates/base/src/zoom.rs`, `crates/component/src/zoom.rs` (new), and every
  module that sizes with `px`: page zoom for bruhs-desktop's ⌘+ / ⌘− / ⌘0,
  without patching gpui.
  - Modules import `crate::zoom::px` in place of `gpui::px`, so design
    lengths are CSS pixels drawn at the zoom.
  - Lengths measured from the window use `window_px`: bounds, mouse
    positions, scroll offsets, text layout, plot coordinates.
  - `css_px` converts a measurement for comparison with a design length.
  - `gpui_component::zoom::set_zoom` rescales the styled theme's font sizes
    and radii and re-syncs the base theme. `Root` sets the rem size from the
    font size, so rem-based styles follow.
  - Window chrome stays in window points, as a browser keeps its own, using
    `gpui::px`: the scrollbar, window border, resize handles, caret, title
    bar and popover edge constants.
  - Test modules are left on gpui's `px`. Tests run at 100%, where the two
    agree.

These earlier local changes are now upstream, so they were dropped:

- `Input::aria_label`.
- The scrollbar applying the release position before it clears drag state.
- Scrollable maximum sizes staying on the viewport.
- Markdown append-only parsing.
- LSP inline completions.

Two other earlier changes were dropped:

- The visible-lines inline selection geometry walk. Upstream's own text
  layout replaces that code. The desktop test that compared it with a full
  scan went with it.
- The 4px resize-handle strip. Upstream now takes a handle renderer, so
  `bruhs_ui::resize_handle::handle_appearance` draws it from the app.

## gpui pins

This directory is not a workspace member, so `Cargo.toml` here keeps its own
copy of the gpui git pins: `gpui`, `gpui_platform`, `gpui_web`, `gpui_macros`,
`reqwest_client` and `sum-tree`. All are at zed rev
`d2a4fb6ded7c470be25d1541281da3882495b65b`, the commit upstream's `gpui-pre`
0.3.12 was cut from. The root `Cargo.toml` states the same rev once for the
rest of the repo, and `third_party/gpui-windows` carries it too.

When moving gpui forward:

- Bump all three.
- Move to the `gpui-pre` version upstream pins.
- Keep `version = "0.2"` on `gpui`. Without it, Cargo can fall back to zed's
  `tooling/lints/test_fixture/gpui` (0.0.0) when gpui's own dependencies
  conflict, and the error then names the wrong crate.

Zed at this rev needs Rust 1.98 (see `rust-toolchain.toml`).
