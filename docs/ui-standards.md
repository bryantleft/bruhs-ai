# UI standards

Keep the composer footer on one row. Expose attachment, model, one options control, and Send or Stop. Put access, Build or Plan, effort, saved prompts, and context actions in the options panel. When active, keep Plan and elevated access, including Full access, visible before sending even when options are closed. Preserve every keyboard shortcut and return focus when a menu closes.

Use 640×480 as the compact desktop verification cell and 390×844 for touch. When footer space tightens, truncate the model label first and retain its full name in its accessible label and tooltip. Keep all four primary controls and active access or Plan status visible.

Use direct labels and short sentences in app-owned interface copy. Do not use dash punctuation, bullet markers, or middle-dot separators. Use spacing and semantic layout to separate information, colons for labels, and “to” for ranges. Do not rewrite user-authored messages, document content, code, technical identifiers, dates, or mathematical minus signs.

Use the shared compact scale: 13px interface labels, 14px desktop inputs, and 12px small labels. Default desktop controls use 32px targets; compact secondary controls use 28px. Shared controls and composer actions retain 44px touch targets and 16px touch inputs. Keep icons and labels centered in a common control box. Hover and focus must change paint without shifting content or resizing hit areas. Focus remains visible.

Every overflow region must have reachable content and a usable scrollbar. Use warm theme colors, a clearly visible thumb, a generous drag target, and scrolling and dragging on each axis where content overflows. Keep scroll handles on the element that owns overflow. Keyboard navigation must reveal the selected option. Respect platform high-contrast behavior.

Verify shared controls on empty, populated, focused, hovered, disabled, expanded, and loading states. Check the compact desktop and touch cells as well as larger windows. Record any tool or platform limit instead of claiming untested coverage.
