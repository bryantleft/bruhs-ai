# UI standards

Keep the composer footer on one row. Expose attachment, model, one options control, and Send or Stop. Put access, Build or Plan, effort, saved prompts, and context actions in the options panel. When active, keep Plan and elevated access, including Full access, visible before sending even when options are closed. Preserve every keyboard shortcut and return focus when a menu closes.

Use 640×480 as the compact desktop verification cell and 390×844 for touch. When footer space tightens, truncate the model label first and retain its full name in its accessible label and tooltip. Keep all four primary controls and active access or Plan status visible.

Use direct labels and short sentences in app-owned interface copy. Do not use dash punctuation, bullet markers, or middle-dot separators. Use spacing and semantic layout to separate information, colons for labels, and “to” for ranges. Do not rewrite user-authored messages, document content, code, technical identifiers, dates, or mathematical minus signs.

Use the shared compact type scale: 13px interface labels, 14px desktop inputs, and 12px small labels. Keep icons and labels centered in a common control box. Hover and focus must change paint without shifting content or resizing hit areas. Focus remains visible.

Size controls with the shared size scale, from shadcn's dense Sidebar and menu sizes. Each app defines it once: desktop in `bruhs-desktop/src/theme.rs`, web as CSS variables in `bruhs-web/tailwind.css` with Tailwind keys such as `min-h-control` and `size-control-sm`. Use the names, never raw numbers, so an interface density setting can change them in one place.

| Name | Size | Desktop | Web | Use |
| --- | --- | --- | --- | --- |
| control | 32px | `CONTROL_PX` | `h-control`, `min-h-control`, `size-control` | Buttons, inputs, menu rows, list rows, nav and footer buttons |
| control-sm | 28px | `CONTROL_SM_PX` | `h-control-sm`, `size-control-sm` | Icon toggles in bars and headers, compact secondary buttons |
| control-xs | 24px | `CONTROL_XS_PX` | `size-control-xs`, `min-h-control-xs` | Actions inside rows and cards, such as a thread card's menu and delete |
| icon | 16px | `ICON_PX` | `size=16` | Icons in icon-only controls, and beside 13 or 14px text in a control |
| icon-xs | 12px | `ICON_XS_PX` | `size=12` | Icons beside 12px text in a control: chips, small buttons, tabs, and menu, list and picker rows (shadcn's `xs` Button) |
| icon-inline | 14px or 12px | `ICON_INLINE_PX`, `ICON_INLINE_SM_PX` | `size=14`, `size=12` | Icons beside text that is not clickable: 14px beside 13 to 14px text, 12px beside 12px text |
| touch | 44px | none | `bruhs-control`, `[role=menuitem]`, `[role=option]` | Phones and other coarse pointers |

An icon follows the text beside it: 16px in an icon-only control or beside 13 or 14px text, 12px beside 12px text. Two kinds of control keep 16px icons beside 12px labels: sidebar section headers, as in shadcn's Sidebar, and the chat header's toggles, which line up with the header's icon-only buttons. Brand logos, checkbox ticks (14px, as in shadcn's Checkbox), icons in tiles above or beside a label (the capture dialog's kinds, search results) and the transcript's rows keep their own sizes. Every target, close buttons included, is at least 24px; the sidebar footer's view buttons are the open exception (see the size scale design).

Text controls take a minimum height, so larger text grows them instead of clipping; only icon buttons have a fixed size. Menu rows use 8px of horizontal padding, 32px text buttons 12px, and 28px text buttons 8px. Adjacent icon buttons sit 4px apart. On the web, give a control `bruhs-control` (or a menu role) instead of a hard-coded 44px class: the coarse pointer rule in `tailwind.css` makes it 44px on touch, and composer actions and inputs keep 44px targets and 16px text there. Bars and headers keep their own heights, such as the 44px app header, and so does the floating meeting dock.

Every overflow region must have reachable content and a usable scrollbar. Use warm theme colors, a clearly visible thumb, a generous drag target, and scrolling and dragging on each axis where content overflows. Keep scroll handles on the element that owns overflow. Keyboard navigation must reveal the selected option. Respect platform high-contrast behavior.

Verify shared controls on empty, populated, focused, hovered, disabled, expanded, and loading states. Check the compact desktop and touch cells as well as larger windows. Record any tool or platform limit instead of claiming untested coverage.
