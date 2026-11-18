use super::*;
use bruhs_models::logic::rich_response::standalone_widget_document;
use bruhs_ui::zoom::px;
use bruhs_ui::FocusTrapElement as _;

fn standalone_document(entry: &Entry, app: &App) -> Option<String> {
    let ResponseBlock::Html(html) = entry.resolved.as_deref()? else {
        return None;
    };
    // Saving source is distinct from explicitly exporting entered widget state.
    Some(standalone_widget_document(
        &html.html,
        &WidgetState::default(),
        &widget_theme(app),
    ))
}

fn invalidate_widget(entry: &mut Entry, reset: bool) {
    entry.host.clear();
    entry.nonce = uuid::Uuid::new_v4().to_string();
    entry.error = None;
    entry.errors.clear();
    entry.action = None;
    entry.flushing = false;
    entry.flush_acked = false;
    entry.reload_pending = false;
    entry.reload_acked = false;
    if reset {
        entry.state = WidgetState::default();
        entry.revision = entry.revision.saturating_add(1);
    }
}

async fn save_document(path: &std::path::Path, document: String) -> Result<(), String> {
    let temporary = path.with_file_name(format!(".bruhs-widget-{}.part", uuid::Uuid::new_v4()));
    let result = async {
        tokio::fs::write(&temporary, document)
            .await
            .map_err(|error| error.to_string())?;
        tokio::fs::rename(&temporary, path)
            .await
            .map_err(|error| error.to_string())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}

fn fenced_source(html: &str) -> String {
    let fence = "`".repeat(
        html.split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .max(2)
            + 1,
    );
    format!("{fence}html\n{html}\n{fence}")
}

impl ChatView {
    fn restart_widget(&self, key: &BlockKey, reset: bool, cx: &mut Context<'_, Self>) {
        {
            let mut embeds = self.rich_embeds.borrow_mut();
            let Some(entry) = embeds.entries.get_mut(key) else {
                return;
            };
            if entry.preparing || entry.state_key.is_empty() {
                cx.emit(ChatViewEvent::Notice(
                    "The widget is still opening. Try again when its preview is ready.".into(),
                ));
                return;
            }
            if !reset && !entry.reload_pending && entry.host.is_live() {
                entry.reload_pending = true;
                entry.reload_acked = false;
                entry.host.flush();
            } else {
                if reset {
                    // Cancel foreground bookkeeping, never the native IO
                    // receipt. The default state is queued after it even if
                    // the older write fails.
                    entry.save_task.take();
                    entry.saving = false;
                }
                invalidate_widget(entry, reset);
            }
        }
        if reset {
            self.save_rich_widget(key, cx);
        }
        cx.notify();
    }

    pub(super) fn apply_acked_widget_reload(&self, key: &BlockKey) {
        let mut embeds = self.rich_embeds.borrow_mut();
        if let Some(entry) = embeds
            .entries
            .get_mut(key)
            .filter(|entry| entry.reload_pending && entry.reload_acked)
        {
            // Run during rendering, after the IPC callback has unwound.
            invalidate_widget(entry, false);
        }
    }

    fn expand_widget(
        &self,
        key: &BlockKey,
        fullscreen: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let mut embeds = self.rich_embeds.borrow_mut();
        if embeds.expanded.is_none() {
            embeds.previous_focus = window.focused(cx);
            let focus = embeds
                .expanded_focus
                .get_or_insert_with(|| cx.focus_handle())
                .clone();
            focus.focus(window, cx);
        }
        embeds.expanded = Some(key.clone());
        embeds.hide();
        if fullscreen && !window.is_fullscreen() {
            embeds.entered_fullscreen = true;
            window.toggle_fullscreen();
        }
        cx.notify();
    }

    fn collapse_widget(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let mut embeds = self.rich_embeds.borrow_mut();
        embeds.expanded = None;
        if embeds.entered_fullscreen && window.is_fullscreen() {
            window.toggle_fullscreen();
        }
        embeds.entered_fullscreen = false;
        let focus = embeds.previous_focus.take();
        drop(embeds);
        focus
            .as_ref()
            .unwrap_or(&self.messages_focus)
            .focus(window, cx);
        cx.notify();
    }

    pub(in crate::views::chat) fn sync_widget_expansion(&self, window: &Window) {
        let mut embeds = self.rich_embeds.borrow_mut();
        if embeds
            .expanded
            .as_ref()
            .is_some_and(|key| !embeds.entries.contains_key(key))
        {
            embeds.expanded = None;
            embeds.exit_fullscreen |= embeds.entered_fullscreen;
            embeds.entered_fullscreen = false;
        }
        if embeds.exit_fullscreen {
            if window.is_fullscreen() {
                window.toggle_fullscreen();
            }
            embeds.exit_fullscreen = false;
        }
    }

    fn save_widget_html(&self, key: &BlockKey, cx: &mut Context<'_, Self>) {
        let document = self
            .rich_embeds
            .borrow()
            .entries
            .get(key)
            .and_then(|entry| standalone_document(entry, cx));
        let Some(document) = document else {
            cx.emit(ChatViewEvent::Notice(
                "The widget is still opening. Try Save HTML when its preview is ready.".into(),
            ));
            return;
        };
        let directory = directories::BaseDirs::new().map_or_else(
            || PathBuf::from("."),
            |dirs| dirs.home_dir().join("Downloads"),
        );
        let picker = cx.prompt_for_new_path(&directory, Some("widget.html"));
        cx.spawn(async move |chat, cx| {
            let Ok(Ok(Some(path))) = picker.await else {
                return;
            };
            let work = chat.update(cx, |_, cx| {
                spawn_tokio(cx, async move { save_document(&path, document).await })
            });
            if let Ok(work) = work {
                if let Err(error) = work.await {
                    let _ = chat.update(cx, |_, cx| {
                        cx.emit(ChatViewEvent::Notice(format!(
                            "Could not save widget HTML: {error}"
                        )))
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn render_widget_controls(
        &self,
        key: &BlockKey,
        expanded: bool,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let pending = self
            .rich_embeds
            .borrow()
            .entries
            .get(key)
            .is_some_and(|entry| entry.reload_pending);
        let mut row = div().flex().flex_wrap().items_center().gap_1();
        for (name, label, reset) in [
            (
                "reload",
                if pending { "Reload now" } else { "Reload" },
                false,
            ),
            ("reset", "Reset saved values", true),
        ] {
            let key = key.clone();
            let selector = format!(
                "desktop-widget-{}{}-{}-{}",
                if expanded { "expanded-" } else { "" },
                name,
                key.0,
                key.1
            );
            row = row.child(
                runtime::action_button(name, label, cx, move |this, _, cx| {
                    this.restart_widget(&key, reset, cx)
                })
                .debug_selector(move || selector),
            );
        }
        if pending {
            row =
                row.child("Waiting for widget state. Reload now uses the latest captured values.");
        }
        let save = key.clone();
        row = row.child(runtime::action_button(
            "save-widget-html",
            "Save HTML",
            cx,
            move |this, _, cx| this.save_widget_html(&save, cx),
        ));
        if !expanded {
            let key = key.clone();
            let selector = format!("desktop-widget-expand-{}-{}", key.0, key.1);
            row = row.child(
                runtime::action_button("expand-widget", "Expand", cx, move |this, window, cx| {
                    this.expand_widget(&key, false, window, cx)
                })
                .debug_selector(move || selector),
            );
        }
        let fullscreen = key.clone();
        row = row.child(runtime::action_button(
            "fullscreen-widget",
            "Fullscreen",
            cx,
            move |this, window, cx| this.expand_widget(&fullscreen, true, window, cx),
        ));
        if expanded {
            let source = key.clone();
            row = row.child(runtime::action_button(
                "expanded-widget-source",
                "Source / Preview",
                cx,
                move |this, _, cx| {
                    this.toggle_rich_source(&source);
                    cx.notify();
                },
            ));
            row = row.child(
                runtime::action_button(
                    "collapse-widget",
                    "Close expanded view",
                    cx,
                    |this, window, cx| this.collapse_widget(window, cx),
                )
                .debug_selector(|| "desktop-widget-collapse".into()),
            );
        }
        row.into_any_element()
    }

    pub(in crate::views::chat) fn render_expanded_widget(
        &self,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let expanded = self.rich_embeds.borrow().expanded.clone()?;
        self.apply_acked_widget_reload(&expanded);
        let (key, block, source_open, error, focus) = {
            let embeds = self.rich_embeds.borrow();
            let key = embeds.expanded.as_ref()?;
            let entry = embeds.entries.get(key)?;
            (
                key.clone(),
                entry.resolved.clone()?,
                entry.source_open,
                entry.error.clone(),
                embeds.expanded_focus.clone()?,
            )
        };
        let ResponseBlock::Html(html) = block.as_ref() else {
            return None;
        };
        let source_open = source_open || error.is_some() || !NativeEmbed::available();
        let body = if source_open {
            TextView::markdown("expanded-widget-source-content", fenced_source(&html.html))
                .selectable(true)
                .into_any_element()
        } else {
            self.rich_canvas(key.clone(), Rc::clone(&block), key.3.clone(), cx)
        };
        let mut element = div()
            .id("expanded-widget")
            .role(Role::Dialog)
            .aria_label("Expanded interactive response")
            .track_focus(&focus)
            .debug_selector(|| "desktop-widget-expanded".into())
            .absolute()
            .inset_0()
            .size_full()
            .bg(theme::longan_950())
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.collapse_widget(window, cx);
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }))
            .child(div().text_size(px(12.0)).child(if html.title.is_empty() {
                "Interactive response".to_string()
            } else {
                html.title.clone()
            }))
            .child(self.render_widget_controls(&key, true, cx));
        if let Some(error) = error {
            element = element.child(
                div()
                    .text_color(theme::lychee_400())
                    .child(format!("Preview unavailable: {error}")),
            );
        }
        Some(
            self.render_rich_details(element, &key, &block, cx)
                .child(
                    div()
                        .id("expanded-widget-body")
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .overflow_hidden()
                        .when(source_open, |body| body.overflow_y_scroll())
                        .child(body),
                )
                .focus_trap("expanded-widget-focus", &focus)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        fenced_source, standalone_document, BlockKey, ChatView, WidgetEvent, WidgetMessage,
        WidgetState, WIDGET_PROTOCOL,
    };
    use crate::views::chat::behavior_tests::{chat_window, rich_message};
    use gpui::{Entity, TestAppContext, VisualTestContext};

    fn widget(cx: &mut TestAppContext) -> (Entity<ChatView>, &mut VisualTestContext, BlockKey) {
        let (chat, cx) = chat_window(cx);
        chat.update(cx, |chat, cx| {
            chat.messages = vec![rich_message("```bruhs-html Example\n<input id='name' value='Original'><button>Click</button>\n```")];
            cx.notify();
        });
        cx.run_until_parked();
        let key = chat.read_with(cx, |chat, _| {
            chat.rich_embeds
                .borrow()
                .entries
                .keys()
                .next()
                .unwrap()
                .clone()
        });
        (chat, cx, key)
    }

    #[gpui::test]
    fn reload_retains_values_and_reset_rejects_old_frame_state(cx: &mut TestAppContext) {
        let (chat, cx, key) = widget(cx);
        let old = chat.update(cx, |chat, _| {
            let mut embeds = chat.rich_embeds.borrow_mut();
            let entry = embeds.entries.get_mut(&key).unwrap();
            entry.state.custom = serde_json::json!({"counter":8});
            entry.revision = 8;
            entry.nonce.clone()
        });
        let reload = cx.debug_bounds("desktop-widget-reload-0-0").unwrap();
        cx.simulate_click(reload.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        let before_reset = chat.read_with(cx, |chat, _| {
            let embeds = chat.rich_embeds.borrow();
            let entry = &embeds.entries[&key];
            assert_ne!(entry.nonce, old);
            assert_eq!(entry.state.custom, serde_json::json!({"counter":8}));
            entry.nonce.clone()
        });
        let reset = cx.debug_bounds("desktop-widget-reset-0-0").unwrap();
        cx.simulate_click(reset.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        chat.update(cx, |chat, cx| {
            let packet = serde_json::to_string(&WidgetMessage {
                bruhs: WIDGET_PROTOCOL,
                nonce: before_reset.clone(),
                event: WidgetEvent::State {
                    state: WidgetState {
                        custom: serde_json::json!({"counter":999}),
                        ..WidgetState::default()
                    },
                },
            })
            .unwrap();
            chat.receive_widget_message(&key, &before_reset, &packet, cx);
            let embeds = chat.rich_embeds.borrow();
            assert_eq!(embeds.entries[&key].state, WidgetState::default());
            assert_eq!(embeds.entries[&key].revision, 9);
        });
    }

    #[gpui::test]
    fn expanded_and_fullscreen_views_keep_the_widget_instance(cx: &mut TestAppContext) {
        let (chat, cx, key) = widget(cx);
        let nonce = chat.read_with(cx, |chat, _| {
            chat.rich_embeds.borrow().entries[&key].nonce.clone()
        });
        let expand = cx.debug_bounds("desktop-widget-expand-0-0").unwrap();
        cx.simulate_click(expand.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-widget-expanded").is_some());
        chat.read_with(cx, |chat, _| {
            let embeds = chat.rich_embeds.borrow();
            assert_eq!(embeds.expanded.as_ref(), Some(&key));
            assert_eq!(embeds.entries[&key].nonce, nonce);
        });
        cx.update(|window, cx| {
            chat.update(cx, |chat, cx| chat.expand_widget(&key, true, window, cx));
            assert!(window.is_fullscreen());
            chat.update(cx, |chat, cx| chat.collapse_widget(window, cx));
            assert!(!window.is_fullscreen());
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-widget-expanded").is_none());
        chat.read_with(cx, |chat, _| {
            assert_eq!(chat.rich_embeds.borrow().entries[&key].nonce, nonce)
        });
    }

    #[gpui::test]
    fn standalone_html_excludes_entered_state(cx: &mut TestAppContext) {
        let (chat, cx, key) = widget(cx);
        chat.update(cx, |chat, cx| {
            let mut embeds = chat.rich_embeds.borrow_mut();
            let entry = embeds.entries.get_mut(&key).unwrap();
            entry.state.custom = serde_json::json!({"secret":"private-entered-value"});
            let document = standalone_document(entry, cx).unwrap();
            assert!(document.contains("Original"));
            assert!(!document.contains("private-entered-value"));
        });
    }

    #[gpui::test]
    fn reload_ack_captures_latest_state_before_rebinding_the_frame(cx: &mut TestAppContext) {
        let (chat, cx, key) = widget(cx);
        chat.update(cx, |chat, cx| {
            let nonce = {
                let mut embeds = chat.rich_embeds.borrow_mut();
                let entry = embeds.entries.get_mut(&key).unwrap();
                entry.reload_pending = true;
                entry.nonce.clone()
            };
            let state = WidgetState {
                custom: serde_json::json!({"latest":12}),
                ..WidgetState::default()
            };
            let packet = serde_json::to_string(&WidgetMessage {
                bruhs: WIDGET_PROTOCOL,
                nonce: nonce.clone(),
                event: WidgetEvent::State {
                    state: state.clone(),
                },
            })
            .unwrap();
            chat.receive_widget_message(&key, &nonce, &packet, cx);
            assert_eq!(chat.rich_embeds.borrow().entries[&key].nonce, nonce);
            chat.apply_acked_widget_reload(&key);
            let embeds = chat.rich_embeds.borrow();
            assert_ne!(embeds.entries[&key].nonce, nonce);
            assert_eq!(embeds.entries[&key].state, state);
            assert!(!embeds.entries[&key].reload_pending);
        });
    }

    #[gpui::test]
    fn reset_persists_default_after_a_failed_older_write_without_foreground_dispatch(
        cx: &mut TestAppContext,
    ) {
        use crate::app_state::DesktopState;
        use crate::server_client::ServerClient;
        use futures_util::FutureExt as _;
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let (chat, cx, key) = widget(cx);
        let runtime = chat.read_with(cx, |_, app| {
            Arc::clone(&app.global::<DesktopState>().runtime)
        });
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let request = runtime.spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let (header, length) = loop {
                let mut chunk = [0_u8; 4096];
                let read = socket.read(&mut chunk).await.unwrap();
                assert_ne!(read, 0);
                bytes.extend_from_slice(&chunk[..read]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let header = end + 4;
                    let text = String::from_utf8_lossy(&bytes[..header]);
                    let length = text
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    break (header, length);
                }
            };
            while bytes.len() < header + length {
                let mut chunk = [0_u8; 4096];
                let read = socket.read(&mut chunk).await.unwrap();
                assert_ne!(read, 0);
                bytes.extend_from_slice(&chunk[..read]);
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            serde_json::from_slice::<WidgetState>(&bytes[header..header + length]).unwrap()
        });
        let write = chat.update(cx, |chat, cx| {
            {
                let mut embeds = chat.rich_embeds.borrow_mut();
                embeds.client = Some(ServerClient::new(base, "http://127.0.0.1:9").unwrap());
                let entry = embeds.entries.get_mut(&key).unwrap();
                entry.saving = true;
                entry.revision = 2;
                entry.state.custom = serde_json::json!({"old":2});
                entry.write = Some(
                    async { Err("older request failed".to_string()) }
                        .boxed()
                        .shared(),
                );
            }
            chat.restart_widget(&key, true, cx);
            chat.rich_embeds.borrow().entries[&key]
                .write
                .clone()
                .unwrap()
        });
        runtime.block_on(async {
            // No GPUI run_until_parked: shutdown must complete from Tokio IO
            // receipts even after ordinary foreground scheduling has stopped.
            tokio::time::timeout(std::time::Duration::from_secs(5), write)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(request.await.unwrap(), WidgetState::default());
        });
        chat.update(cx, |chat, _| {
            let mut embeds = chat.rich_embeds.borrow_mut();
            let entry = embeds.entries.get_mut(&key).unwrap();
            entry.saved_revision = entry.revision;
            entry.write = None;
            entry.save_task.take();
            entry.saving = false;
        });
    }

    #[test]
    fn expanded_source_fence_cannot_be_closed_by_widget_source() {
        let source = "````\n<img src='https://example.com/private'>\n````";
        assert!(fenced_source(source).starts_with("`````html\n"));
    }
}
