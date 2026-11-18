use super::*;
use crate::views::web_button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;
use bruhs_ui::{input::Input, scroll::ScrollableElement};

impl WhiteboardLibrary {
    fn command(&mut self, command: BoardCommand, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.dialog = None;
        match command {
            BoardCommand::Action(action) => self.action(action, window, cx),
            BoardCommand::Tool(BoardTool::Image) => self.pick_file(true, window, cx),
            BoardCommand::Tool(tool) => {
                if let Some(view) = self.active().map(|s| s.view.clone()) {
                    view.update(cx, |view, cx| {
                        view.set_tool(tool, cx);
                        view.focus(window, cx);
                    });
                }
            }
        }
        cx.notify();
    }

    pub(super) fn action(
        &mut self,
        action: BoardAction,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.dialog = None;
        let mut handled = false;
        self.edit(
            |input| {
                handled = action.apply(input, now())?;
                Ok(())
            },
            cx,
        );
        if handled {
            return;
        }
        match action {
            BoardAction::AllBoards => self.current = None,
            BoardAction::NewBoard => self.new_board(window, cx),
            BoardAction::Fit => {
                if let Some(view) = self.active().map(|s| s.view.clone()) {
                    view.update(cx, |v, cx| v.fit(cx));
                }
            }
            BoardAction::Save => {
                if let Some(id) = self.current.clone() {
                    self.save(&id, true, cx);
                }
            }
            BoardAction::SaveCopy => {
                if let Some(portable) = self.portable(cx) {
                    match portable.import_copy(self.project.clone(), now()) {
                        Ok(p) => self.save_new(p, window, cx),
                        Err(e) => self.fail(e, cx),
                    }
                }
            }
            BoardAction::EditText => {
                let value = self.active().and_then(|s| {
                    let view = s.view.read(cx);
                    let input = view.input.borrow();
                    input
                        .editor
                        .selection
                        .first()
                        .and_then(|id| input.editor.document().shapes.get(id))
                        .map(|s| s.text.clone())
                });
                if let Some(value) = value {
                    self.show_dialog(DialogKind::Text, &value, window, cx);
                } else {
                    self.fail("Select a shape to edit its text.", cx);
                }
            }
            BoardAction::Rename => {
                if let Some(board) = self.document(cx) {
                    self.show_dialog(DialogKind::Rename, &board.title, window, cx);
                }
            }
            BoardAction::ConnectNext => self.show_dialog(DialogKind::Connect, "", window, cx),
            BoardAction::Link => self.show_dialog(DialogKind::Link, "", window, cx),
            BoardAction::ImportMermaid => self.show_dialog(
                DialogKind::Mermaid,
                "flowchart LR\n  Browser --> API\n  API --> Database",
                window,
                cx,
            ),
            BoardAction::Import => self.pick_file(false, window, cx),
            BoardAction::Copy => self.copy(cx),
            BoardAction::Paste => self.paste(cx),
            BoardAction::ExportJson
            | BoardAction::ExportSvg
            | BoardAction::ExportPng
            | BoardAction::ExportMermaid => self.export(action, window, cx),
            BoardAction::RefreshSources => self.check_sources(cx),
            _ => {}
        }
        cx.notify();
    }
    pub(super) fn portable(&self, cx: &App) -> Option<PortableBoard> {
        let seat = self.active()?;
        let board = seat.view.read(cx).input.borrow().editor.document().clone();
        let payloads = seat
            .payloads
            .iter()
            .filter(|(hash, _)| board.assets.values().any(|a| &a.sha256 == *hash))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Some(PortableBoard { board, payloads })
    }
    fn copy(&mut self, cx: &mut Context<'_, Self>) {
        let Some(seat) = self.active() else {
            return;
        };
        let result = seat
            .view
            .read(cx)
            .input
            .borrow()
            .editor
            .copy_selection()
            .and_then(|board| {
                let payloads = seat
                    .payloads
                    .iter()
                    .filter(|(hash, _)| board.assets.values().any(|a| &a.sha256 == *hash))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                serde_json::to_string(&PortableBoard { board, payloads })
                    .map_err(|e| BoardError::Invalid(e.to_string()))
            });
        match result {
            Ok(json) => {
                cx.write_to_clipboard(ClipboardItem::new_string(format!(
                    "bruhs-whiteboard:{json}"
                )));
                self.notice = Some("Selection copied.".into());
            }
            Err(e) => self.fail(e, cx),
        }
    }
    fn paste(&mut self, cx: &mut Context<'_, Self>) {
        let result = (|| {
            let text = cx
                .read_from_clipboard()
                .and_then(|item| item.text())
                .ok_or_else(|| BoardError::Invalid("Copy a whiteboard selection first.".into()))?;
            let json = text.strip_prefix("bruhs-whiteboard:").ok_or_else(|| {
                BoardError::Invalid("The clipboard does not contain a whiteboard selection.".into())
            })?;
            if u64::try_from(json.len()).unwrap_or(u64::MAX) > MAX_PORTABLE_BYTES {
                return Err(BoardError::Invalid(
                    "The clipboard exceeds the board budget.".into(),
                ));
            }
            let portable: PortableBoard =
                serde_json::from_str(json).map_err(|e| BoardError::Invalid(e.to_string()))?;
            portable.validate()?;
            let Some(seat) = self.current.as_ref().and_then(|id| self.seats.get_mut(id)) else {
                return Ok(());
            };
            if seat.view.read(cx).frozen {
                return Err(BoardError::Invalid(
                    "Save an editable copy of this board before pasting.".into(),
                ));
            }
            // Source paths and app links are meaningful only in their original project.
            let mut board = portable.board;
            if board.project_path != self.project {
                board.sources.clear();
                for shape in board.shapes.values_mut() {
                    shape.source_id = None;
                    shape.links.clear();
                }
            }
            Arc::make_mut(&mut seat.payloads).extend(portable.payloads);
            seat.view.update(cx, |view, cx| {
                view.edit(
                    |input| input.editor.paste(&board, Point::new(24.0, 24.0), now()),
                    cx,
                )
            });
            Ok(())
        })();
        if let Err(e) = result {
            self.fail(e, cx);
        }
    }
    pub(super) fn submit_dialog(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(dialog) = self.dialog.as_ref() else {
            return;
        };
        let kind = dialog.kind;
        let value = dialog.input.value(cx).to_string();
        let link_kind = dialog.link_kind.read(cx).value().to_string();
        if matches!(kind, DialogKind::Commands) {
            let query = value.to_lowercase();
            if let Some(action) =
                BoardCommand::all().find(|a| a.label().to_lowercase().contains(&query))
            {
                self.command(action, window, cx);
            }
            return;
        }
        self.error = None;
        match kind {
            DialogKind::Text => self.edit(|input| input.editor.set_text(value, now()), cx),
            DialogKind::Rename => self.edit(
                |input| {
                    input
                        .editor
                        .edit(vec![BoardOperation::Title { title: value }], now())
                },
                cx,
            ),
            DialogKind::Connect => self.edit(
                |input| {
                    input.editor.connect_next(value, now())?;
                    Ok(())
                },
                cx,
            ),
            DialogKind::Link => match parse_link(&link_kind, &value) {
                Ok(link) => self.edit(|input| input.editor.link(link, now()), cx),
                Err(e) => {
                    self.fail(e, cx);
                    return;
                }
            },
            DialogKind::Mermaid => {
                match import_mermaid(&value, self.project.clone(), "Mermaid board".into(), now())
                    .and_then(|board| {
                        self.set_preview(
                            PortableBoard {
                                board,
                                payloads: BTreeMap::new(),
                            },
                            cx,
                        )
                    }) {
                    Ok(()) => {}
                    Err(e) => {
                        self.fail(e, cx);
                        return;
                    }
                }
            }
            DialogKind::Commands => {}
        }
        self.dialog = None;
        if let Some(seat) = self.active() {
            seat.view.update(cx, |view, cx| view.focus(window, cx));
        }
        cx.notify();
    }
    fn render_dialog(&self, cx: &mut Context<'_, Self>) -> Option<Stateful<Div>> {
        let dialog = self.dialog.as_ref()?;
        let label = match dialog.kind {
            DialogKind::Commands => "Whiteboard commands",
            DialogKind::Text => "Shape text",
            DialogKind::Rename => "Board title",
            DialogKind::Connect => "Next node text",
            DialogKind::Link => "Link target",
            DialogKind::Mermaid => "Mermaid flowchart or sequence diagram",
        };
        let mut panel = div()
            .id("whiteboard-dialog")
            .role(Role::Dialog)
            .capture_action(
                cx.listener(|this, action: &bruhs_ui::input::Enter, window, cx| {
                    if action.secondary && !action.shift {
                        cx.stop_propagation();
                        this.submit_dialog(window, cx);
                    }
                }),
            )
            .p_3()
            .gap_2()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(theme::longan_800())
            .child(label)
            .child(Styled::h(
                dialog.input.element(label, false),
                px(
                    if matches!(dialog.kind, DialogKind::Text | DialogKind::Mermaid) {
                        120.0
                    } else {
                        32.0
                    },
                ),
            ));
        if matches!(dialog.kind, DialogKind::Link) {
            panel=panel.child("Type: file, thread, task, pr, meeting, person or document. File target: relative/path.rs:10-24.").child(Input::new(&dialog.link_kind).aria_label("Link type"));
        }
        if matches!(dialog.kind, DialogKind::Commands) {
            let query = dialog.input.value(cx).to_lowercase();
            let mut actions = div()
                .max_h(px(240.0))
                .overflow_y_scrollbar()
                .flex()
                .flex_col();
            for action in BoardCommand::all().filter(|a| a.label().to_lowercase().contains(&query))
            {
                actions = actions.child(
                    button(
                        SharedString::from(format!("board-command-{}", action.label())),
                        action.label(),
                    )
                    .activate(cx, move |this, w, cx| this.command(action, w, cx)),
                );
            }
            panel = panel.child(actions);
        } else {
            panel = panel.child(
                button(
                    "board-dialog-apply",
                    if matches!(dialog.kind, DialogKind::Mermaid) {
                        "Preview import"
                    } else {
                        "Apply"
                    },
                )
                .activate(cx, |this, w, cx| this.submit_dialog(w, cx)),
            );
        }
        Some(panel.child(
            button("board-dialog-cancel", "Cancel").activate(cx, |this, w, cx| {
                this.dialog = None;
                if let Some(s) = this.active() {
                    s.view.update(cx, |view, cx| view.focus(w, cx));
                }
                cx.notify();
            }),
        ))
    }
}
impl Render for WhiteboardLibrary {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let title = self.active().map_or_else(
            || "Whiteboards".to_string(),
            |seat| {
                seat.view
                    .read(cx)
                    .input
                    .borrow()
                    .editor
                    .document()
                    .title
                    .clone()
            },
        );
        let status = self.active().map(|seat| match &seat.writer {
            None => "Read-only · another window owns this board".into(),
            Some(writer) => match writer.status() {
                Ok(s) if s.error.is_some() => "Save failed · export a recovery copy".into(),
                Ok(s) if s.busy => "Saving…".into(),
                Ok(s) => format!("Saved · revision {}", s.revision),
                Err(e) => e.to_string(),
            },
        });
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(theme::longan_800())
            .child(
                button("board-all", "All boards").activate(cx, |this, _, cx| {
                    this.current = None;
                    this.dialog = None;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_ellipsis().child(title))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_ellipsis()
                            .text_color(theme::rambutan_400())
                            .child(self.project.clone()),
                    ),
            )
            .child(
                button("board-commands", "Commands ⌘K").activate(cx, |this, w, cx| {
                    this.show_dialog(DialogKind::Commands, "", w, cx)
                }),
            )
            .child(
                button("board-new", "New board").activate(cx, |this, w, cx| this.new_board(w, cx)),
            )
            .child(
                button("board-import", "Import")
                    .activate(cx, |this, w, cx| this.pick_file(false, w, cx)),
            );
        let mut body = div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .min_w_0()
            .text_color(theme::rambutan_200())
            .bg(theme::longan_950())
            .child(header);
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .id("whiteboard-error")
                    .role(Role::Alert)
                    .p_2()
                    .text_color(theme::lychee_300())
                    .child(error.clone()),
            );
        }
        if let Some(error) = self
            .active()
            .and_then(|seat| seat.view.read(cx).error.clone())
        {
            body = body.child(
                div()
                    .id("whiteboard-edit-error")
                    .role(Role::Alert)
                    .p_2()
                    .text_color(theme::lychee_300())
                    .child(error),
            );
        }
        if let Some(notice) = &self.notice {
            body = body.child(div().p_2().child(notice.clone()));
        }
        if let Some(dialog) = self.render_dialog(cx) {
            body = body.child(dialog);
        }
        if let Some((proposal, after)) = &self.reviewing {
            let accept = proposal.clone();
            let reject = proposal.id.clone();
            let before = self.seats.get(&proposal.board_id).map(|s| s.view.clone());
            let exact=before.as_ref().map(|view|{let input=view.read(cx).input.borrow();serde_json::to_string_pretty(&serde_json::json!({"before_metadata":{"title":input.editor.document().title,"background":input.editor.document().background,"sources":input.editor.document().sources,"assets":input.editor.document().assets},"exact_operations":proposal.operations})).unwrap_or_default()}).unwrap_or_default();
            let stale = before.as_ref().is_none_or(|v| {
                proposal
                    .validate_for(v.read(cx).input.borrow().editor.document())
                    .is_err()
            });
            body = body
                .child(div().p_3().child(format!(
                    "Review agent edit · {}{}",
                    proposal.description,
                    if stale {
                        " · stale: request a fresh proposal"
                    } else {
                        ""
                    }
                )))
                .child(
                    div()
                        .flex()
                        .h(px(320.0))
                        .when_some(before, |el, v| {
                            el.child(div().flex_1().child("Before").child(v))
                        })
                        .child(div().flex_1().child("After").child(after.clone())),
                )
                .child(
                    div()
                        .max_h(px(180.0))
                        .overflow_y_scrollbar()
                        .font_family("Monaspace Argon")
                        .text_size(px(11.0))
                        .child(exact),
                )
                .child(
                    button("board-proposal-accept", "Accept exact edit")
                        .activate(cx, move |this, _, cx| this.accept(accept.clone(), cx)),
                )
                .child(button("board-proposal-reject", "Reject").activate(
                    cx,
                    move |this, _, cx| {
                        this.statuses
                            .insert(reject.clone(), ProposalStatus::Rejected);
                        this.reviewing = None;
                        cx.notify();
                    },
                ))
                .child(button("board-proposal-close", "Close preview").activate(
                    cx,
                    |this, _, cx| {
                        this.reviewing = None;
                        cx.notify();
                    },
                ));
            return body;
        }
        if let Some((portable, preview)) = &self.preview {
            let copy = portable.clone();
            for warning in &self.import_warnings {
                body = body.child(div().p_2().child(warning.clone()));
            }
            return body
                .child(div().p_3().child(format!(
                    "Import preview · {} · {} editable shapes · private copy",
                    portable.board.title,
                    portable.board.shapes.len()
                )))
                .child(div().h(px(320.0)).min_h_0().child(preview.clone()))
                .child(
                    button("board-import-accept", "Save private copy")
                        .activate(cx, move |this, w, cx| this.save_new(copy.clone(), w, cx)),
                )
                .child(button("board-import-cancel", "Cancel import").activate(
                    cx,
                    |this, _, cx| {
                        this.preview = None;
                        cx.notify();
                    },
                ));
        }
        if let Some(seat) = self.active() {
            let view = seat.view.clone();
            let input = view.read(cx).input.borrow();
            let tool = input.tool;
            let selection = selection_caption(&input.editor);
            let links = selected_links(&input);
            let sources = input
                .editor
                .document()
                .sources
                .values()
                .map(|source| {
                    format!(
                        "{} · {}",
                        source.path,
                        if source.stale {
                            "changed or unavailable"
                        } else {
                            "source matches"
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join(" · ");
            let shared = input.editor.document().agent_read;
            drop(input);
            let pending = self
                .proposals
                .iter()
                .filter(|p| {
                    Some(&p.board_id) == self.current.as_ref()
                        && self.statuses.get(&p.id) == Some(&ProposalStatus::Pending)
                })
                .cloned()
                .collect::<Vec<_>>();
            for proposal in pending {
                let id = proposal.id.clone();
                body = body.child(
                    button(
                        SharedString::from(format!("board-review-{id}")),
                        format!("Review agent proposal: {}", proposal.description),
                    )
                    .activate(cx, move |this, _, cx| this.review(proposal.clone(), cx)),
                );
            }
            let mut toolbar = div().flex().flex_wrap().items_center().gap_1().p_2();
            for item in BoardTool::ALL {
                toolbar = toolbar.child(
                    button(
                        SharedString::from(format!("board-tool-{}", item.key())),
                        format!(
                            "{} {}{}",
                            item.label(),
                            item.key(),
                            if tool == item { " ✓" } else { "" }
                        ),
                    )
                    .activate(cx, move |this, w, cx| {
                        if item == BoardTool::Image {
                            this.pick_file(true, w, cx);
                        } else if let Some(view) = this.active().map(|s| s.view.clone()) {
                            view.update(cx, |view, cx| view.set_tool(item, cx));
                            view.update(cx, |view, cx| view.focus(w, cx));
                        }
                    }),
                );
            }
            toolbar = toolbar
                .child(
                    button("board-fit", "Fit")
                        .activate(cx, |this, w, cx| this.action(BoardAction::Fit, w, cx)),
                )
                .child(
                    button("board-edit", "Edit text")
                        .activate(cx, |this, w, cx| this.action(BoardAction::EditText, w, cx)),
                )
                .child(
                    button("board-connect", "Connect next").activate(cx, |this, w, cx| {
                        this.action(BoardAction::ConnectNext, w, cx)
                    }),
                )
                .child(
                    button(
                        "board-share",
                        if shared {
                            "Shared with agents"
                        } else {
                            "Share with agents"
                        },
                    )
                    .activate(cx, |this, w, cx| this.action(BoardAction::Share, w, cx)),
                );
            if !sources.is_empty() {
                body = body.child(div().p_2().text_size(px(12.0)).child(sources));
            }
            body = body
                .child(toolbar)
                .child(
                    div()
                        .p_2()
                        .text_size(px(12.0))
                        .child(status.unwrap_or_default()),
                )
                .child(div().flex_1().min_h_0().child(view));
            body=body.child(div().p_2().text_size(px(12.0)).child(format!("Tab: next shape · arrows: move · C: connect next · Enter: edit · R then Enter: create · {selection}")));
            for (index, link) in links.into_iter().enumerate() {
                let project = self.project.clone();
                body = body.child(button(("board-link", index), link.label()).activate(
                    cx,
                    move |_, _, cx| {
                        cx.emit(WhiteboardEvent::OpenLink(link.clone(), project.clone()))
                    },
                ));
            }
        } else if self.busy {
            body = body.child(div().p_4().child("Loading local boards…"));
        } else if self.project.is_empty() {
            body = body.child(
                div()
                    .p_4()
                    .child("Choose a project folder to create and reopen its boards."),
            );
        } else {
            let mut list = div()
                .id("board-catalog")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .p_4()
                .gap_2()
                .flex()
                .flex_col();
            if self.boards.is_empty() {
                list=list.child("Your project has no boards yet. Create one or import a Mermaid diagram, portable board, .tldr or .excalidraw file.");
            }
            for board in &self.boards {
                let id = board.id.clone();
                list = list.child(
                    button(
                        SharedString::from(format!("board-open-{id}")),
                        format!("{} · {} shapes", board.title, board.shape_count),
                    )
                    .activate(cx, move |this, w, cx| this.open(id.clone(), w, cx)),
                );
            }
            body = body.child(list);
        }
        body
    }
}
fn parse_link(kind: &str, value: &str) -> BoardResult<BoardLink> {
    let id = value.trim().to_string();
    let link = match kind.trim() {
        "file" => {
            let (path, range) = id.rsplit_once(':').unwrap_or((&id, "1"));
            let (start, end) = range.split_once('-').unwrap_or((range, range));
            BoardLink::File {
                path: path.into(),
                start_line: start
                    .parse()
                    .map_err(|_| BoardError::Invalid("Use path:10-24 for a file link.".into()))?,
                end_line: end
                    .parse()
                    .map_err(|_| BoardError::Invalid("Use path:10-24 for a file link.".into()))?,
            }
        }
        "thread" => BoardLink::Thread { id },
        "task" => BoardLink::Task { id },
        "meeting" => BoardLink::Meeting { id },
        "person" => BoardLink::Person { id },
        "document" => BoardLink::Document { id },
        "pr" => BoardLink::PullRequest { url: id },
        _ => {
            return Err(BoardError::Invalid(
                "Choose file, thread, task, pr, meeting, person or document.".into(),
            ));
        }
    };
    link.validate()?;
    Ok(link)
}
fn button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    let label = label.into();
    web_button(
        id,
        WebButtonVariant::Outline,
        WebButtonSize::Small,
        ButtonState::Enabled,
    )
    .aria_label(label.clone())
    .child(label)
}
trait Activate {
    fn activate(
        self,
        cx: &Context<'_, WhiteboardLibrary>,
        f: impl Fn(&mut WhiteboardLibrary, &mut Window, &mut Context<'_, WhiteboardLibrary>) + 'static,
    ) -> Self;
}
impl Activate for Stateful<Div> {
    fn activate(
        self,
        cx: &Context<'_, WhiteboardLibrary>,
        f: impl Fn(&mut WhiteboardLibrary, &mut Window, &mut Context<'_, WhiteboardLibrary>) + 'static,
    ) -> Self {
        on_press_view(self, cx, f)
    }
}

fn selected_links(input: &BoardInput) -> Vec<BoardLink> {
    input
        .editor
        .selection
        .iter()
        .filter_map(|id| input.editor.document().shapes.get(id))
        .flat_map(|shape| shape.links.iter().cloned())
        .collect()
}
