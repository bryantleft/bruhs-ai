# Semantic action requirements (incomplete)

The private successor records 128 source-reviewed intent requirements: 62 preserved command/mail intents plus 66 whiteboard intents. These are measurement requirements, not completed action acceptances. The broader source discovery remains a pinned historical 376-entrypoint/1,383-binding snapshot. Whole-action and matched T3 acceptances are zero.

The JSON companion preserves source profiles, line/hash anchors, required variants and completion boundaries. The new 43-file whiteboard capture predates landing and is not an atomic or compiled-build snapshot. [Machine-readable requirements](performance-action-catalogue.json) · [Whiteboard review](performance-whiteboard-actions-2026-11-14.md).

## Earlier 62 preserved intents

| Intent | Requirement | Completion to verify |
| --- | --- | --- |
| calendar.open_scheduling | Open smart scheduling | Scheduling controls and their loaded/empty/error state are usable. |
| capture.open_editor | Open quick capture editor | A fresh capture editor is visible with no edit identity retained. |
| day_plan.request | Request a day plan draft | The Today request is consumed and its draft/feedback is ready. |
| inbox.open_import | Open conversation import | Inbox import/create controls are visible and usable; saving/importing is separate. |
| knowledge.new_document | Create and open a knowledge document | The new document is visible in its detail editor and its durable write result is observed. |
| knowledge.open_clip | Open web clip creation | Knowledge clip creation controls are usable; fetching/saving a clip is separate. |
| meeting.open_note | Open new meeting note editor | A fresh note editor is visible on Meetings; committing the note is separate. |
| model.open_picker | Open model selector | The intended selector state and model/provider choices are usable. |
| panel.diff | Toggle diff panel | The diff surface reaches usable content/empty/error or closed state. |
| panel.files | Toggle files panel | File controls reach ready/empty/error or closed state. |
| panel.preview | Toggle preview panel | Preview reaches usable content/empty/error or closes. |
| panel.right | Toggle right panel | The requested right-panel surface/launcher is usable or closed. |
| panel.tasks | Toggle tasks panel | Tasks panel reaches ready/empty/error or closed state. |
| project.open_editor | Open active project in editor | Editor launch result/feedback is observed, or no-project feedback is visible. |
| project.open_picker | Open project chooser | The chooser is visible and usable; opening the selected project is a separate action. |
| prompt.stash | Stash prompt or open stash menu | A nonempty draft/chips are stashed and cleared, or the empty-draft stash chooser is usable. |
| pull.copy_link | Copy displayed pull request link | Clipboard contains the displayed pull URL, or the panel/no-pull notice is shown. |
| pull.copy_number | Copy displayed pull request number | Clipboard contains the displayed pull number, or the panel/no-pull notice is shown. |
| servers.open_picker | Open local server chooser | The local-server chooser is usable; launching/navigating a chosen server is separate. |
| sidebar.toggle | Toggle sidebar | The applicable sidebar reaches the requested visible state. |
| task.open_create | Open task creation form | Task form is visible on the Tasks surface; submitting the task is a separate action. |
| terminal.create_session | Create integrated terminal session | The server-created PTY session is selected and usable, or creation feedback is shown. |
| terminal.open_system | Open system Terminal | The operating-system Terminal launch is acknowledged or an error is reported. |
| terminal.panel | Toggle integrated terminal | The terminal panel is visible and reaches its session/empty/error state, or closes. |
| thread.continue_t3 | Continue active thread in T3 | The controlled bridge acknowledges the created thread, or missing-project/bridge-error feedback is shown. |
| thread.copy_link | Copy active thread link | Clipboard contains the client-specific thread address, or the no-active-thread outcome is observed. |
| thread.copy_markdown | Copy thread Markdown | Clipboard contains the bounded transcript and the expected notice appears. |
| thread.copy_resume | Copy CLI resume command | Clipboard contains the provider/session/project command, or the correct missing-session/no-active-thread outcome is observed. |
| thread.import_dialog | Open thread import | Import chooser is visible and usable; selection/import/commit are separate actions. |
| thread.new | Create a thread or choose its project | A fresh thread is selected with a ready composer and durable result, or the required project chooser is usable. |
| thread.pin | Pin or unpin the active thread | Pinned state and ordering are reflected, with the writer result observed. |
| thread.settle | Settle or unsettle the active thread | Settled state, unread state and ordering are reflected, with the writer result observed. |
| view.automations | Navigate to Automations | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| view.calendar | Navigate to Calendar | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| view.chat | Navigate to Chat | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| view.inbox | Navigate to Inbox | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| view.knowledge | Navigate to Knowledge | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| view.life | Navigate to Life | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| view.meetings | Navigate to Meetings | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| view.people | Navigate to People | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| view.search | Navigate to Search | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| view.settings | Navigate to Settings | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| view.tasks | Navigate to Tasks | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| view.terminal | Navigate to Terminal | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| view.today | Navigate to Today | Requested view reaches ready, empty or error state and is usable; setting AppView alone is not completion. |
| mail.open_connected | Open connected Email | Account and thread controls reach their loaded, empty or actionable error state. |
| mail.open_imported | Return to imported conversations | Imported Inbox and its preserved local search/draft controls are usable. |
| mail.open_connections | Open mail Connections | Connections view has usable loaded, empty or actionable error controls. |
| mail.refresh | Refresh saved accounts and visible mail | Fresh account state and dependent current list/reader settle with a usable result or error. |
| mail.select_account | Choose a saved mail account | Selected account's first result page is usable and stale account results cannot replace it. |
| mail.search | Search saved email | Applied query's first result page, empty state or error is usable. |
| mail.filter | Filter saved email | Chosen filter's first result page, empty state or error is usable. |
| mail.page | Page through saved email | Requested or server-clamped page is usable with correct controls and count. |
| mail.open_thread | Open a saved email thread | Selected thread's current saved message, envelope and attachment controls or actionable error are usable. |
| mail.back | Return from a saved thread to email | Saved list is usable with restored focus; abandoned detail transport settles without stale feedback. |
| mail.message | Choose an older or newer saved message | Correct message text, envelope, attachment controls and index are usable. |
| mail.copy_message | Copy saved message text | Clipboard contains exact selected text, or visible failure offers manual copy. |
| mail.download | Download an explicit mail attachment | Validated exact bytes reach browser-owned download completion or native private atomic save; failure is visible. |
| mail.sync_gmail | Sync connected Gmail | Provider pages and cursor are durably saved, account/list refresh completes, and final status or recoverable error is usable. |
| mail.sync_outlook | Sync connected Outlook | Provider pages and cursor are durably saved, account/list refresh completes, and final status or recoverable error is usable. |
| mail.restart_sync | Restart the selected provider sync | Selected provider's restarted traversal reaches saved/current results or an actionable error. |
| mail.stop_sync | Stop sync after the current page | Current provider page commits, no next page starts, resumable cursor is retained, and refreshed list plus stopped status are usable. |

## New 66 whiteboard intent requirements

| Intent | Requirement | Completion to verify |
| --- | --- | --- |
| whiteboard.undo | Undo | Previous revision content is restored, committed and rendered; redo remains valid. |
| whiteboard.redo | Redo | Redo content is restored, committed and rendered; history remains valid. |
| whiteboard.select_all | Select all | All eligible shapes are selected and rendered without a durable edit. |
| whiteboard.delete | Delete selection | Exact intended board revision is committed through IndexedDB/native writer, saved status is current, and final canvas/selection is rendered. |
| whiteboard.group | Group | Exact intended board revision is committed through IndexedDB/native writer, saved status is current, and final canvas/selection is rendered. |
| whiteboard.ungroup | Ungroup | Exact intended board revision is committed through IndexedDB/native writer, saved status is current, and final canvas/selection is rendered. |
| whiteboard.align_left | Align left | Exact intended board revision is committed through IndexedDB/native writer, saved status is current, and final canvas/selection is rendered. |
| whiteboard.align_center | Align center | Exact intended board revision is committed through IndexedDB/native writer, saved status is current, and final canvas/selection is rendered. |
| whiteboard.align_right | Align right | Exact intended board revision is committed through IndexedDB/native writer, saved status is current, and final canvas/selection is rendered. |
| whiteboard.align_top | Align top | Exact intended board revision is committed through IndexedDB/native writer, saved status is current, and final canvas/selection is rendered. |
| whiteboard.align_middle | Align middle | Exact intended board revision is committed through IndexedDB/native writer, saved status is current, and final canvas/selection is rendered. |
| whiteboard.align_bottom | Align bottom | Exact intended board revision is committed through IndexedDB/native writer, saved status is current, and final canvas/selection is rendered. |
| whiteboard.distribute_horizontal | Distribute horizontally | Exact intended board revision is committed through IndexedDB/native writer, saved status is current, and final canvas/selection is rendered. |
| whiteboard.distribute_vertical | Distribute vertically | Exact intended board revision is committed through IndexedDB/native writer, saved status is current, and final canvas/selection is rendered. |
| whiteboard.copy | Copy | Clipboard write acknowledgement and recoverable exact selection payload; native OS clipboard and browser permission paths measured separately. |
| whiteboard.paste | Paste | Clipboard read/validation and asset ownership finish; pasted selection commits and renders. |
| whiteboard.edit_text | Edit text | Dialog opens/focuses; applying edited shape text commits and renders. Cancellation closes without changing content. |
| whiteboard.connect_next | Connect next node | Text entry produces the intended new node and bound connector, commits and renders; invalid/empty selection paths remain explicit. |
| whiteboard.rename | Rename board | Title dialog opens; applying a valid title commits and appears in board and catalog. |
| whiteboard.link | Link selection | Typed link dialog validates target and optional line range; link commits and appears on the selection. |
| whiteboard.import | Import board | OS/browser file selection, bounded read, parser and raster preparation produce an inspectable preview or explicit error. Saving the preview is separately catalogued. |
| whiteboard.import_mermaid | Import Mermaid | Mermaid editor opens and parsing produces an inspectable editable-shape preview or actionable error. |
| whiteboard.export_mermaid | Export Mermaid | Supported diagram/selection serialization completes, then browser-owned download completion and exact text are verified; native file-write confirmation and readback are verified separately. Handoff alone is not completion. |
| whiteboard.export_json | Export portable JSON | Portable board and referenced assets serialize, then browser-owned download completion and exact bytes are verified; native file-write confirmation and readback are verified separately. Handoff alone is not completion. |
| whiteboard.export_svg | Export SVG | SVG serialization and asset embedding finish, then browser-owned download completion or native file-write confirmation is verified with the exact resulting SVG. Handoff alone is not completion. |
| whiteboard.export_png | Export PNG | Font/image decode, rasterization and encoding finish, then browser-owned download completion or native file-write confirmation is verified with bytes and decoded pixels. Handoff alone is not completion. |
| whiteboard.fit | Fit board | Visible content bounds are fitted and rendered, including deferred viewport readiness; no persistence is implied. |
| whiteboard.reset_zoom | Reset zoom | Camera scale resets and renders without changing board content. |
| whiteboard.toggle_snap | Toggle snapping | Snapping state changes and the next gesture obeys it; no durable edit until the gesture commits. |
| whiteboard.share | Toggle agent sharing | Agent-read flag commits, status renders, and authorization reads reflect the new state. |
| whiteboard.save | Retry save | Explicit retry completes the current pending revision or leaves actionable conflict/quota/IO feedback without losing the recovery copy. |
| whiteboard.save_copy | Save a copy | A new private board ID/content/assets commit and open; original bytes/revision remain intact. |
| whiteboard.refresh_sources | Check source drift | All relevant fingerprints settle; changed/unavailable sources update with revision guards, commit when needed and render while user edits remain intact. |
| whiteboard.all_boards | All project boards | Project catalog is loaded and usable with correct ownership, summary ordering and empty/error feedback. |
| whiteboard.new_board | New board | A new private board commits in the selected project and its ready canvas is focused/rendered. |
| whiteboard.tool_select | Select select tool | Tool state and pressed/focus indicators update; any prior preview is canceled. Drawing/selection/panning gestures are separate intents. |
| whiteboard.tool_hand | Select hand tool | Tool state and pressed/focus indicators update; any prior preview is canceled. Drawing/selection/panning gestures are separate intents. |
| whiteboard.tool_rectangle | Select rectangle tool | Tool state and pressed/focus indicators update; any prior preview is canceled. Drawing/selection/panning gestures are separate intents. |
| whiteboard.tool_ellipse | Select ellipse tool | Tool state and pressed/focus indicators update; any prior preview is canceled. Drawing/selection/panning gestures are separate intents. |
| whiteboard.tool_arrow | Select arrow tool | Tool state and pressed/focus indicators update; any prior preview is canceled. Drawing/selection/panning gestures are separate intents. |
| whiteboard.tool_text | Select text tool | Tool state and pressed/focus indicators update; any prior preview is canceled. Drawing/selection/panning gestures are separate intents. |
| whiteboard.tool_note | Select note tool | Tool state and pressed/focus indicators update; any prior preview is canceled. Drawing/selection/panning gestures are separate intents. |
| whiteboard.tool_frame | Select frame tool | Tool state and pressed/focus indicators update; any prior preview is canceled. Drawing/selection/panning gestures are separate intents. |
| whiteboard.tool_freehand | Select draw tool | Tool state and pressed/focus indicators update; any prior preview is canceled. Drawing/selection/panning gestures are separate intents. |
| whiteboard.tool_image | Insert local image | File picker/read/validation/decode and current-board image insertion commit; decoded image renders or an actionable error leaves the original unchanged. |
| whiteboard.open_board | Open an existing board | Local record, revision/lease and referenced assets load; decoded first canvas, current save state and controls are ready. |
| whiteboard.commands | Open and filter board commands | Command dialog is focused, query-filtered actions are usable, keyboard/click activation dispatches the same command, and dismiss restores editor focus. |
| whiteboard.focus_node | Focus next or previous shape | Tab/Shift-Tab changes the intended selection and canvas/accessible selection caption without mutating content. |
| whiteboard.nudge | Nudge selected shapes | Arrow/Shift-Arrow moves intended selection and bindings, commits and renders; empty/frozen selection remains safe. |
| whiteboard.cancel_gesture | Cancel an unfinished gesture | Escape or pointer cancellation removes draft/move/selection preview without a durable mutation and releases interaction state. |
| whiteboard.point_select | Select shapes by pointer | Hit selection/additive toggle and focus render correctly under zoom/rotation/group/overlap and touch/mouse inputs. |
| whiteboard.box_select | Select shapes with a marquee | Drag selection and additive prior selection are correct through completion/cancel; no content revision is written. |
| whiteboard.move_selection | Drag a selection | Every live preview stays responsive; pointer-up produces exactly the intended committed edit, including snaps and connector/group bindings. |
| whiteboard.pan | Pan the board | Hand/middle-button/right-button where supported and unmodified wheel update camera/rendering without board content writes. |
| whiteboard.zoom | Zoom around the pointer | Modified wheel updates a clamped camera around the pointer and final canvas remains usable; zoom bounds and input devices are exercised. |
| whiteboard.draw | Create shapes with a gesture or keyboard | Active drawing tool previews correctly and commits the intended shape/bindings at gesture end; keyboard creation and text entry are separate measured phases. |
| whiteboard.accept_import | Save an imported preview as a private board | Reviewed imported content and referenced assets commit under a new private ID and open without altering the source/original board. |
| whiteboard.cancel_import | Cancel an imported preview | Preview closes without adding a board, leaving original data and recovery state unchanged. |
| whiteboard.follow_link | Follow a shape link | Correct typed target and project context open; target view/file/OS URL handoff completes or an explicit missing-target error appears. |
| whiteboard.review_proposal | Review an agent proposal | Exact before/after preview and immutable operations/metadata become inspectable; stale or invalid proposal is clearly blocked without mutation. |
| whiteboard.accept_proposal | Accept an exact agent proposal | Only pending, current, saved proposal is committed; accepted revision/state and canvas agree, errors preserve recovery and release the frozen state. |
| whiteboard.reject_proposal | Reject an agent proposal | Pending status becomes rejected and preview closes without changing stored board content. |
| whiteboard.save_chat_mermaid | Save a chat diagram as a private board | Previewed editable diagram and retained thread links commit as a private project board and become usable. The originating chat-renderer button remains outside this capture. |
| whiteboard.dismiss_overlay | Dismiss a board dialog or local error | Target dialog/error closes, pending proposal status/content remain unchanged, and editor focus/keyboard behavior recover. |
| whiteboard.resize | Resize the board viewport | Canvas backing size/DPR and viewport transform update; existing content/selection remain correct and the next interaction is usable. |
| whiteboard.return_focus | Return to a whiteboard window | Visible board and guarded source verification settle while edits remain intact. App activation/source hashing/network work retain separate boundaries. |

## Remaining coverage

Review every remaining user-visible context and lifecycle branch, supported client, downstream persistence/network/OS operation, error, cancellation and retry. Measure complete actions with their actual data and build/source context. Source declaration counts and partial rendering measurements do not establish completion.
