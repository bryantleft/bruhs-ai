use super::*;
use bruhs_models::logic::rich_response::HtmlBlock;
use bruhs_ui::press::on_press_view;
use futures_util::FutureExt as _;
use gpui_component::zoom::px;

pub(super) fn action_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    cx: &Context<'_, ChatView>,
    action: impl Fn(&mut ChatView, &mut Window, &mut Context<'_, ChatView>) + 'static,
) -> Stateful<Div> {
    web_button(
        id,
        WebButtonVariant::Ghost,
        WebButtonSize::Small,
        ButtonState::Enabled,
    )
    .child(label.into())
    .map(|el| on_press_view(el, cx, action))
}

/// A labelled checkbox for the export panel (web `<input type=checkbox>`):
/// Space toggles it, like the other desktop checkboxes.
fn export_checkbox(
    id: impl Into<ElementId>,
    label: &'static str,
    checked: bool,
    cx: &Context<'_, ChatView>,
    toggle: impl Fn(&mut RichEmbeds) + 'static,
) -> Stateful<Div> {
    let toggle = Rc::new(toggle);
    let toggle_for_key = Rc::clone(&toggle);
    div()
        .id(id)
        .min_h(px(28.0))
        .flex()
        .items_center()
        .gap_2()
        .role(Role::CheckBox)
        .aria_label(label)
        .aria_toggled(if checked {
            Toggled::True
        } else {
            Toggled::False
        })
        .tab_index(0)
        .rounded(px(4.0))
        .border_1()
        .border_color(gpui::transparent_black())
        .focus_visible(|el| el.border_color(theme::persimmon_400()))
        .cursor_pointer()
        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
            if !event.is_keyboard() {
                toggle(&mut this.rich_embeds.borrow_mut());
                cx.notify();
            }
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "space" {
                window.prevent_default();
                cx.stop_propagation();
                toggle_for_key(&mut this.rich_embeds.borrow_mut());
                cx.notify();
            }
        }))
        .child(
            div()
                .size(px(14.0))
                .flex_shrink_0()
                .rounded(px(4.0))
                .border_1()
                .border_color(if checked {
                    theme::persimmon_500()
                } else {
                    theme::rambutan_500()
                })
                .bg(if checked {
                    theme::persimmon_500()
                } else {
                    theme::longan_900()
                })
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.0))
                .text_color(theme::longan_950())
                .child(if checked { "✓" } else { "" }),
        )
        .child(label)
}

/// Persist directly on Tokio: GPUI stops ordinary foreground dispatch while
/// awaiting shutdown futures, so an app-owned foreground Task cannot be a fence.
fn close_state_flush(embeds: &mut RichEmbeds, app: &App) -> Option<tokio::task::JoinHandle<()>> {
    let client = embeds.client.clone()?;
    let runtime = Arc::clone(&app.try_global::<DesktopState>()?.runtime);
    let snapshots = embeds
        .entries
        .values()
        .filter(|entry| entry.revision > entry.saved_revision && !entry.state_key.is_empty())
        .map(|entry| (entry.state_key.clone(), entry.state.clone()))
        .collect::<Vec<_>>();
    let pending = embeds
        .entries
        .values_mut()
        .filter_map(|entry| {
            entry.save_task.take();
            entry.saving = false;
            entry.write.take()
        })
        .collect::<Vec<_>>();
    if snapshots.is_empty() && pending.is_empty() {
        return None;
    }
    Some(runtime.spawn(async move {
        for write in pending {
            let _ = write.await;
        }
        for (key, state) in snapshots {
            if let Err(error) = client.save_widget_state(&key, &state).await {
                tracing::warn!("could not save widget state while closing: {error}");
            }
        }
    }))
}

pub(super) fn save_closed_widget_states(embeds: &mut RichEmbeds, app: &App) {
    let mut task = close_state_flush(embeds, app);
    if task.is_none() {
        return;
    }
    app.on_app_quit(move |_| {
        let task = task.take();
        async move {
            if let Some(task) = task {
                let _ = task.await;
            }
        }
    })
    .detach();
}

fn queue_state_write(
    entry: &mut Entry,
    client: &crate::server_client::ServerClient,
    runtime: &Arc<tokio::runtime::Runtime>,
) -> (StateWrite, u64, u64) {
    let previous = entry.write.clone();
    let state = entry.state.clone();
    let state_key = entry.state_key.clone();
    let client = client.clone();
    let task = runtime.spawn(async move {
        if let Some(previous) = previous {
            let _ = previous.await;
        }
        client
            .save_widget_state(&state_key, &state)
            .await
            .map_err(|error| error.to_string())
    });
    let write = async move { task.await.map_err(|error| error.to_string())? }
        .boxed()
        .shared();
    entry.write_generation = entry.write_generation.saturating_add(1);
    entry.write = Some(write.clone());
    (write, entry.revision, entry.write_generation)
}

impl ChatView {
    pub(super) fn approve_rich_media(&self, key: &BlockKey) {
        let mut embeds = self.rich_embeds.borrow_mut();
        let entry = embeds.entries.entry(key.clone()).or_default();
        entry.remote_allowed = true;
        if entry.resolved.as_deref().is_some_and(|block|matches!(block,ResponseBlock::Media(media) if !media.tracks.is_empty()&&media_has_remote(media))) {
            entry.resolved=None;entry.body=None;
        }
    }
    pub(super) fn flush_rich_widgets_on_quit(
        &self,
        cx: &Context<'_, Self>,
    ) -> impl std::future::Future<Output = ()> + 'static {
        let mut embeds = self.rich_embeds.borrow_mut();
        embeds.quitting = true;
        let write = close_state_flush(&mut embeds, cx);
        // GPUI clears native windows before polling the returned future.
        embeds.drop_native_hosts("app_quit");
        async move {
            if let Some(write) = write {
                let _ = write.await;
            }
        }
    }

    pub(super) fn save_response_media(&self, key: &BlockKey, src: String, cx: &Context<'_, Self>) {
        let directory = directories::BaseDirs::new().map_or_else(
            || PathBuf::from("."),
            |dirs| dirs.home_dir().join("Downloads"),
        );
        let name = src
            .split(['?', '#'])
            .next()
            .unwrap_or("media")
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or("media")
            .to_string();
        let picker = cx.prompt_for_new_path(&directory, Some(&name));
        let project = key.3.to_string();
        let client = self.rich_embeds.borrow().client.clone();
        cx.spawn(async move |chat, cx| {
            let Ok(Ok(Some(path))) = picker.await else {
                return;
            };
            let work = chat.update(cx, |_, cx| {
                spawn_tokio(cx, async move {
                    let client = match client {
                        Some(client) => client,
                        None => crate::server_client::ServerClient::from_env()?,
                    };
                    client.save_rich_media(&src, &project, &path).await
                })
            });
            if let Ok(work) = work {
                if let Err(error) = work.await {
                    let _ = chat.update(cx, |_, cx| {
                        cx.emit(ChatViewEvent::Notice(error.to_string()))
                    });
                }
            }
        })
        .detach();
    }

    pub(super) fn export_response_bundle(&self, message: usize, cx: &Context<'_, Self>) {
        let Some(reply) = self.messages.get(message) else {
            return;
        };
        let markdown = super::publications::export_markdown(reply);
        let project = self
            .worktree_path
            .as_deref()
            .unwrap_or(&self.project_path)
            .to_string();
        let include_state = self.rich_embeds.borrow().export_state.contains(&message);
        let include_remote = self.rich_embeds.borrow().export_remote.contains(&message);
        if include_state {
            for (key, entry) in &self.rich_embeds.borrow().entries {
                if key.0 == message {
                    entry.host.flush();
                }
            }
        }
        let widgets = parse_assistant_content(&markdown, false)
            .into_iter()
            .enumerate()
            .filter_map(|(index, block)| match block {
                ParsedAssistantBlock::Rich(ResponseBlock::Html(html)) => {
                    Some((self.rich_scope(message, index), Some(html.html), None))
                }
                ParsedAssistantBlock::Rich(ResponseBlock::Artifact(artifact)) => Some((
                    self.artifact_scope(message, &artifact.id),
                    None,
                    Some(artifact.id),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        let directory = directories::BaseDirs::new().map_or_else(
            || PathBuf::from("."),
            |dirs| dirs.home_dir().join("Downloads"),
        );
        let picker = cx.prompt_for_new_path(&directory, Some("response.zip"));
        let client = self.rich_embeds.borrow().client.clone();
        cx.spawn(async move |chat, cx| {
            let Ok(Ok(Some(path))) = picker.await else {
                return;
            };
            if include_state {
                let pending = chat
                    .update(cx, |this, cx| {
                        let mut embeds = this.rich_embeds.borrow_mut();
                        let Some(client) = embeds.client.clone() else {
                            return Vec::new();
                        };
                        let runtime = Arc::clone(&cx.global::<DesktopState>().runtime);
                        embeds
                            .entries
                            .iter_mut()
                            .filter(|(key, entry)| key.0 == message && !entry.state_key.is_empty())
                            .map(|(_, entry)| queue_state_write(entry, &client, &runtime).0)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                for write in pending {
                    let _ = write.await;
                }
            }

            let work = chat.update(cx, |_, cx| {
                spawn_tokio(cx, async move {
                    let client = match client {
                        Some(client) => client,
                        None => crate::server_client::ServerClient::from_env()?,
                    };
                    let mut keys = Vec::new();
                    if include_state {
                        for (scope, html, artifact) in widgets {
                            let html = match (html, artifact) {
                                (Some(html), _) => html,
                                (_, Some(id)) => client.rich_artifact(&id).await?.html,
                                _ => continue,
                            };
                            keys.push(widget_state_key(&scope, &html));
                        }
                    }
                    client
                        .export_rich_reply(
                            &markdown,
                            &project,
                            &keys,
                            include_state,
                            include_remote,
                            &path,
                        )
                        .await
                })
            });
            if let Ok(work) = work {
                if let Err(error) = work.await {
                    let _ = chat.update(cx, |_, cx| {
                        cx.emit(ChatViewEvent::Notice(error.to_string()))
                    });
                }
            }
        })
        .detach();
    }

    /// Web `ExportResponse`: one quiet "Export reply" toggle; its choices
    /// open in a panel under the turn's meta row.
    pub(in crate::views::chat) fn render_response_export(
        &self,
        message: usize,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let open = self.rich_embeds.borrow().export_open.contains(&message);
        action_button(
            ("export-reply", message),
            "Export reply",
            cx,
            move |this, _, cx| {
                {
                    let mut embeds = this.rich_embeds.borrow_mut();
                    if !embeds.export_open.remove(&message) {
                        embeds.export_open.insert(message);
                    }
                }
                cx.notify();
            },
        )
        .debug_selector(move || format!("desktop-export-reply-{message}"))
        .aria_expanded(open)
        .into_any_element()
    }

    /// Reply `message`'s export choices: (saved widget values, external media).
    #[cfg(test)]
    pub(in crate::views::chat) fn export_choices(&self, message: usize) -> (bool, bool) {
        let embeds = self.rich_embeds.borrow();
        (
            embeds.export_state.contains(&message),
            embeds.export_remote.contains(&message),
        )
    }

    /// The open "Export reply" panel (web `data-response-export`): whether
    /// to fetch linked media and include saved widget values, then saving
    /// the bundle (a ZIP, through the save dialog).
    pub(in crate::views::chat) fn render_response_export_panel(
        &self,
        message: usize,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let (include_state, include_remote) = {
            let embeds = self.rich_embeds.borrow();
            if !embeds.export_open.contains(&message) {
                return None;
            }
            (
                embeds.export_state.contains(&message),
                embeds.export_remote.contains(&message),
            )
        };
        let note = |text: &'static str| div().text_color(theme::rambutan_500()).child(text);
        Some(
            div()
                .id(("export-panel", message))
                .debug_selector(move || format!("desktop-export-panel-{message}"))
                .role(Role::Group)
                .aria_label("Export reply")
                .mt_1()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded(px(8.0))
                .bg(theme::longan_900().opacity(0.6))
                .border_1()
                .border_color(theme::longan_800())
                .text_size(px(12.0))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
                .text_color(theme::rambutan_300())
                .child(
                    export_checkbox(
                        ("export-external-media", message),
                        "Include external media",
                        include_remote,
                        cx,
                        move |embeds| {
                            if !embeds.export_remote.remove(&message) {
                                embeds.export_remote.insert(message);
                            }
                        },
                    )
                    .debug_selector(move || format!("desktop-export-external-media-{message}")),
                )
                .child(note(if include_remote {
                    "Downloads linked media for this bundle."
                } else {
                    "External media stays as links. Local media and interactive content are included."
                }))
                .child(
                    export_checkbox(
                        ("export-widget-state", message),
                        "Include saved widget values",
                        include_state,
                        cx,
                        move |embeds| {
                            if !embeds.export_state.remove(&message) {
                                embeds.export_state.insert(message);
                            }
                        },
                    )
                    .debug_selector(move || format!("desktop-export-widget-state-{message}")),
                )
                .child(note("Saved values can include text entered in widget forms."))
                .child(
                    div().flex().child(
                        web_button(
                            ("export-response", message),
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .debug_selector(move || format!("desktop-export-save-{message}"))
                        .aria_label("Save ZIP")
                        .child("Save ZIP")
                        .map(|el| {
                            on_press_view(el, cx, move |this, _, cx| {
                                this.export_response_bundle(message, cx);
                            })
                        }),
                    ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn rich_scope(&self, message: usize, block: usize) -> String {
        format!(
            "{}/{}/{message}/{block}",
            self.thread_id.as_deref().unwrap_or("unsaved"),
            self.messages
                .get(message)
                .and_then(|message| message.started_at_ms)
                .unwrap_or_default()
        )
    }

    fn artifact_scope(&self, message: usize, id: &str) -> String {
        format!("{}/artifact/{id}", self.rich_scope(message, 0))
    }

    pub(super) fn prepare_rich_response(
        &self,
        key: &BlockKey,
        block: &Rc<ResponseBlock>,
        cx: &Context<'_, Self>,
    ) {
        let mut embeds = self.rich_embeds.borrow_mut();
        let client = embeds.client.clone();
        let entry = embeds.entries.entry(key.clone()).or_default();
        if entry.original_media.is_none() {
            if let ResponseBlock::Media(media) = block.as_ref() {
                entry.original_media = Some(media.clone());
            }
        }
        if entry.remote_allowed && entry.resolved.as_deref().is_some_and(|block|matches!(block,ResponseBlock::Media(media) if !media.tracks.is_empty()&&media_has_remote(media))) {entry.resolved=None;entry.body=None;}
        if entry.resolved.is_some() || entry.preparing || entry.error.is_some() {
            return;
        }
        if cfg!(test) {
            entry.resolved = Some(Rc::clone(block));
            if let ResponseBlock::Html(html) = block.as_ref() {
                entry.state_key = widget_state_key(&self.rich_scope(key.0, key.1), &html.html);
            }
            return;
        }
        let Some(client) = client else { return };
        entry.preparing = true;
        let approved = entry.remote_allowed;
        let scope = match block.as_ref() {
            ResponseBlock::Artifact(artifact) => self.artifact_scope(key.0, &artifact.id),
            _ => self.rich_scope(key.0, key.1),
        };
        let block = block.as_ref().clone();
        let work_key = key.clone();
        let work = spawn_tokio(cx, async move {
            let mut artifact_id = None;
            let block = match block {
                ResponseBlock::Artifact(artifact) => {
                    let loaded = client
                        .rich_artifact(&artifact.id)
                        .await
                        .map_err(|error| error.to_string())?;
                    artifact_id = Some(loaded.id);
                    ResponseBlock::Html(HtmlBlock {
                        html: loaded.html,
                        title: loaded.title,
                    })
                }
                ResponseBlock::Media(mut media) => {
                    let proxy_remote = approved && !media.tracks.is_empty();
                    let sources = std::iter::once(media.src.clone())
                        .chain(media.poster.iter().cloned())
                        .chain(media.tracks.iter().map(|track| track.src.clone()))
                        .filter(|src| !media_is_remote(src) || proxy_remote)
                        .collect::<std::collections::BTreeSet<_>>();
                    let operations = sources.into_iter().map(|src| {
                        let client = client.clone();
                        let project = work_key.3.clone();
                        async move {
                            let captured = if media_is_remote(&src) {
                                client
                                    .proxy_rich_media(&src)
                                    .await
                                    .map_err(|error| error.to_string())?
                                    .src
                            } else {
                                client
                                    .capture_rich_media(&src, &project)
                                    .await
                                    .map_err(|error| error.to_string())?
                            };
                            Ok::<_, String>((src, captured))
                        }
                    });
                    let mut operations = futures_util::StreamExt::buffer_unordered(
                        futures_util::stream::iter(operations),
                        bruhs_models::logic::attachments::MAX_FILES_PER_MESSAGE,
                    );
                    let mut paths = std::collections::BTreeMap::new();
                    while let Some(result) = futures_util::StreamExt::next(&mut operations).await {
                        let (src, durable) = result?;
                        paths.insert(src, durable);
                    }
                    if let Some(path) = paths.get(&media.src) {
                        media.src = path.clone();
                    }
                    if let Some(poster) = media.poster.as_mut() {
                        if let Some(path) = paths.get(poster) {
                            *poster = path.clone();
                        }
                    }
                    for track in &mut media.tracks {
                        if let Some(path) = paths.get(&track.src) {
                            track.src = path.clone();
                        }
                    }
                    ResponseBlock::Media(media)
                }
                block => block,
            };
            let (state_key, state) = match &block {
                ResponseBlock::Html(html) => {
                    let state_key = widget_state_key(&scope, &html.html);
                    let state = client
                        .widget_state(&state_key)
                        .await
                        .map_err(|error| error.to_string())?;
                    (state_key, state)
                }
                _ => (String::new(), WidgetState::default()),
            };
            Ok::<_, String>((block, state_key, state, artifact_id))
        });
        let key = key.clone();
        entry.prepare_task=Some(cx.spawn(async move |chat,cx| {
            let result=work.await;
            let _=chat.update(cx,|this,cx| {
                if let Some(entry)=this.rich_embeds.borrow_mut().entries.get_mut(&key) {
                    entry.preparing=false;
                    match result {
                        Ok((block,state_key,state,artifact_id))=>{
                            if matches!(&block,ResponseBlock::Media(media) if media.kind==MediaKind::Audio) {entry.height=64.0;}
                            entry.resolved=Some(Rc::new(block));entry.state_key=state_key;entry.state=state;entry.artifact_id=artifact_id;
                        }
                        Err(error)=>entry.error=Some(error),
                    }
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn receive_widget_message(
        &mut self,
        key: &BlockKey,
        nonce: &str,
        packet: &str,
        cx: &mut Context<'_, Self>,
    ) {
        if packet.len() > MAX_WIDGET_STATE_BYTES.saturating_mul(2) {
            return;
        }
        let Ok(packet) = serde_json::from_str::<WidgetMessage>(packet) else {
            return;
        };
        if packet.bruhs != WIDGET_PROTOCOL || packet.nonce != nonce {
            return;
        }
        let (save, report) = {
            let mut embeds = self.rich_embeds.borrow_mut();
            let Some(entry) = embeds
                .entries
                .get_mut(key)
                .filter(|entry| entry.nonce == nonce)
            else {
                return;
            };
            match packet.event {
                WidgetEvent::Ready => (
                    false,
                    Some((entry.artifact_id.clone(), entry.errors.clone())),
                ),
                WidgetEvent::Resize { height } => {
                    if height.is_finite() {
                        entry.height = height.clamp(MIN_WIDGET_HEIGHT, MAX_WIDGET_HEIGHT);
                    }
                    (false, None)
                }
                WidgetEvent::State { state } => {
                    if state.validate().is_err() {
                        return;
                    }
                    entry.state = state;
                    if entry.reload_pending {
                        entry.reload_acked = true;
                    }
                    entry.revision = entry.revision.saturating_add(1);
                    if entry.flushing {
                        entry.flush_acked = true;
                        entry.flushing = false;
                    }
                    (true, None)
                }
                WidgetEvent::Error { message } => {
                    if !entry.errors.contains(&message) {
                        entry.errors.push(message);
                    }
                    (
                        false,
                        Some((entry.artifact_id.clone(), entry.errors.clone())),
                    )
                }
                WidgetEvent::Action { action } => {
                    if action.message.trim().is_empty()
                        || action.message.len() > MAX_WIDGET_STATE_BYTES
                    {
                        return;
                    }
                    entry.action = Some(action);
                    (false, None)
                }
            }
        };
        if save && !self.rich_embeds.borrow().quitting {
            self.save_rich_widget(key, cx);
        }
        if let Some((Some(id), errors)) = report.filter(|_| !self.rich_embeds.borrow().quitting) {
            let client = self.rich_embeds.borrow().client.clone();
            if let Some(client) = client {
                let previous = self
                    .rich_embeds
                    .borrow_mut()
                    .entries
                    .get_mut(key)
                    .and_then(|entry| entry.diagnostics_task.take());
                let runtime = Arc::clone(&cx.global::<DesktopState>().runtime);
                let task = cx.spawn(async move |chat, cx| {
                    if let Some(previous) = previous {
                        previous.await;
                    }
                    let work = cx.background_spawn(async move {
                        runtime.block_on(client.report_artifact(&id, &errors))
                    });
                    if let Err(error) = work.await {
                        let _ = chat.update(cx, |_, cx| {
                            cx.emit(ChatViewEvent::Notice(error.to_string()))
                        });
                    }
                });
                if let Some(entry) = self.rich_embeds.borrow_mut().entries.get_mut(key) {
                    entry.diagnostics_task = Some(task);
                }
            }
        }
        if !self.rich_embeds.borrow().quitting {
            cx.notify();
        }
    }

    pub(super) fn save_rich_widget(&self, key: &BlockKey, cx: &Context<'_, Self>) {
        let mut embeds = self.rich_embeds.borrow_mut();
        let Some(client) = embeds.client.clone() else {
            return;
        };
        let Some(entry) = embeds.entries.get_mut(key) else {
            return;
        };
        if entry.saving || entry.state_key.is_empty() {
            return;
        }
        entry.saving = true;
        let runtime = Arc::clone(&cx.global::<DesktopState>().runtime);
        let mut pending = queue_state_write(entry, &client, &runtime);
        let key = key.clone();
        entry.save_task = Some(cx.spawn(async move |chat, cx| loop {
            let (write, revision, generation) = pending;
            let result = write.await;
            let Ok(next) = chat.update(cx, |this, cx| {
                let mut embeds = this.rich_embeds.borrow_mut();
                let entry = embeds.entries.get_mut(&key)?;
                if entry.write_generation == generation {
                    entry.write = None;
                }
                if let Err(error) = result {
                    entry
                        .errors
                        .push(format!("Could not save widget state: {error}"));
                    entry.saving = false;
                    cx.notify();
                    return None;
                }
                entry.saved_revision = entry.saved_revision.max(revision);
                if entry.revision > revision {
                    Some(queue_state_write(entry, &client, &runtime))
                } else {
                    entry.saving = false;
                    None
                }
            }) else {
                return;
            };
            let Some(next) = next else { break };
            pending = next;
        }));
    }

    pub(super) fn render_rich_details(
        &self,
        mut element: Stateful<Div>,
        key: &BlockKey,
        block: &Rc<ResponseBlock>,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let (action, errors, transcript) = {
            let embeds = self.rich_embeds.borrow();
            let entry = embeds.entries.get(key);
            (
                entry.and_then(|entry| entry.action.clone()),
                entry.map_or_else(Vec::new, |entry| entry.errors.clone()),
                match block.as_ref() {
                    ResponseBlock::Media(media) => media.transcript.clone(),
                    _ => None,
                },
            )
        };
        if let ResponseBlock::Artifact(artifact) = block.as_ref() {
            element = element.child(
                div()
                    .text_size(px(11.0))
                    .text_color(theme::rambutan_500())
                    .child(format!("Artifact {}", artifact.id)),
            );
        }
        for error in errors {
            element = element.child(div().text_color(theme::lychee_400()).child(error));
        }
        if let Some(transcript) = transcript {
            element = element.child(div().text_size(px(12.0)).child(transcript));
        }
        if let Some(action) = action {
            let approve = key.clone();
            let dismiss = key.clone();
            element = element.child(
                div()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(theme::blueberry_400())
                    .p_2()
                    .child(format!("Proposed draft: {}", action.label))
                    .child(
                        div()
                            .id("widget-proposed-message")
                            .max_h(px(160.0))
                            .overflow_y_scroll()
                            .child(action.message),
                    )
                    .child(
                        action_button(
                            "use-widget-draft",
                            "Use draft in composer",
                            cx,
                            move |this, window, cx| this.accept_widget_draft(&approve, window, cx),
                        )
                        .debug_selector(|| "desktop-widget-accept".into()),
                    )
                    .child(action_button(
                        "dismiss-widget-draft",
                        "Dismiss",
                        cx,
                        move |this, _, cx| {
                            if let Some(entry) =
                                this.rich_embeds.borrow_mut().entries.get_mut(&dismiss)
                            {
                                entry.action = None;
                            }
                            cx.notify();
                        },
                    )),
            );
        }
        element
    }

    fn accept_widget_draft(
        &mut self,
        key: &BlockKey,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let action = self
            .rich_embeds
            .borrow_mut()
            .entries
            .get_mut(key)
            .and_then(|entry| entry.action.take());
        if let Some(action) = action {
            let current = self.input_state.read(cx).value().to_string();
            let text = if current.trim().is_empty() {
                action.message
            } else {
                format!("{current}\n\n{}", action.message)
            };
            self.write_composer(&text, window, cx);
            self.input_state
                .update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::BlockKey;
    use crate::views::chat::{
        behavior_tests::{chat_window, rich_message},
        AgentUiEvent, ChatView, Message,
    };
    use bruhs_models::logic::rich_response::{
        WidgetAction, WidgetEvent, WidgetMessage, WidgetState,
    };
    use gpui::{Entity, TestAppContext, VisualTestContext};

    fn binding(chat: &Entity<ChatView>, cx: &mut VisualTestContext) -> (BlockKey, String) {
        chat.read_with(cx, |chat, _| {
            let embeds = chat.rich_embeds.borrow();
            let (key, entry) = embeds.entries.iter().next().unwrap();
            (key.clone(), entry.nonce.clone())
        })
    }

    fn send(
        chat: &Entity<ChatView>,
        key: &BlockKey,
        nonce: &str,
        event: WidgetEvent,
        cx: &mut VisualTestContext,
    ) {
        let body = serde_json::to_string(&WidgetMessage {
            bruhs: 1,
            nonce: nonce.into(),
            event,
        })
        .unwrap();
        let expected = chat.read_with(cx, |chat, _| {
            chat.rich_embeds.borrow().entries[key].nonce.clone()
        });
        chat.update(cx, |chat, cx| {
            chat.receive_widget_message(key, &expected, &body, cx)
        });
        cx.run_until_parked();
    }

    fn seed(chat: &Entity<ChatView>, cx: &mut VisualTestContext) {
        chat.update(cx, |chat, cx| {
            chat.messages = vec![rich_message("```bruhs-html\n<button>Widget</button>\n```")];
            cx.notify();
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn widget_draft_waits_for_review_then_preserves_existing_composer(cx: &mut TestAppContext) {
        let (chat, cx) = chat_window(cx);
        seed(&chat, cx);
        let (key, nonce) = binding(&chat, cx);
        chat.update_in(cx, |chat, window, cx| {
            chat.write_composer("Existing draft", window, cx)
        });
        send(
            &chat,
            &key,
            &nonce,
            WidgetEvent::Action {
                action: WidgetAction {
                    label: "Use selection".into(),
                    message: "Selected A".into(),
                },
            },
            cx,
        );
        chat.read_with(cx, |chat, cx| {
            assert_eq!(chat.input_state.read(cx).value(), "Existing draft");
            assert_eq!(chat.messages.len(), 1);
            assert!(!chat.is_streaming);
            assert!(chat.agent_task.is_none());
        });
        let approve = cx
            .debug_bounds("desktop-widget-accept")
            .expect("visible proposed-draft review");
        cx.simulate_click(approve.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        chat.read_with(cx, |chat, cx| {
            assert_eq!(
                chat.input_state.read(cx).value(),
                "Existing draft\n\nSelected A"
            );
            assert_eq!(chat.messages.len(), 1);
            assert!(chat.agent_task.is_none());
        });
    }

    #[gpui::test]
    fn wrong_nonce_cannot_propose_actions_or_change_widget_state(cx: &mut TestAppContext) {
        let (chat, cx) = chat_window(cx);
        seed(&chat, cx);
        let (key, _) = binding(&chat, cx);
        send(
            &chat,
            &key,
            "forged",
            WidgetEvent::Action {
                action: WidgetAction {
                    label: "Do it".into(),
                    message: "Injected".into(),
                },
            },
            cx,
        );
        send(
            &chat,
            &key,
            "forged",
            WidgetEvent::State {
                state: WidgetState {
                    custom: serde_json::json!({"injected":true}),
                    ..WidgetState::default()
                },
            },
            cx,
        );
        chat.read_with(cx, |chat, _| {
            let embeds = chat.rich_embeds.borrow();
            let entry = &embeds.entries[&key];
            assert!(entry.action.is_none());
            assert_eq!(entry.state, WidgetState::default());
            assert_eq!(entry.revision, 0);
        });
    }

    #[gpui::test]
    fn widget_height_is_bounded_and_state_survives_view_disposal(cx: &mut TestAppContext) {
        let (chat, cx) = chat_window(cx);
        seed(&chat, cx);
        let (key, nonce) = binding(&chat, cx);
        send(
            &chat,
            &key,
            &nonce,
            WidgetEvent::Resize { height: 50000.0 },
            cx,
        );
        chat.read_with(cx, |chat, _| {
            assert_eq!(chat.rich_embeds.borrow().entries[&key].height, 20000.0)
        });
        send(
            &chat,
            &key,
            &nonce,
            WidgetEvent::Resize { height: 100.0 },
            cx,
        );
        let state = WidgetState {
            custom: serde_json::json!({"count":8}),
            ..WidgetState::default()
        };
        send(
            &chat,
            &key,
            &nonce,
            WidgetEvent::State {
                state: state.clone(),
            },
            cx,
        );
        chat.update(cx, |chat, _| {
            chat.rich_embeds
                .borrow_mut()
                .entries
                .get_mut(&key)
                .unwrap()
                .host
                .clear()
        });
        chat.read_with(cx, |chat, _| {
            let embeds = chat.rich_embeds.borrow();
            assert_eq!(embeds.entries[&key].state, state);
            assert_eq!(embeds.entries[&key].height, 100.0);
        });
    }

    #[gpui::test]
    fn immutable_source_rewrite_keeps_the_same_media_instance(cx: &mut TestAppContext) {
        let (chat, cx) = chat_window(cx);
        chat.update(cx, |chat, cx| {
            chat.messages = vec![rich_message(
                "```bruhs-media\n{\"kind\":\"video\",\"src\":\"/project/output.mp4\"}\n```",
            )];
            cx.notify();
        });
        cx.run_until_parked();
        let (key, nonce) = binding(&chat, cx);
        chat.update(cx, |chat, cx| {
            chat.apply_agent_event(AgentUiEvent::ResponseMedia(
                std::collections::BTreeMap::from([(
                    "/project/output.mp4".into(),
                    "/immutable/first.mp4".into(),
                )]),
            ));
            cx.notify();
        });
        cx.run_until_parked();
        chat.read_with(cx, |chat, _| {
            assert!(chat.messages[0].content.contains("/immutable/first.mp4"));
            let embeds = chat.rich_embeds.borrow();
            assert_eq!(embeds.entries.len(), 1);
            assert_eq!(embeds.entries[&key].nonce, nonce);
        });
    }

    #[gpui::test]
    fn result_only_reply_fills_empty_text_without_duplicating_streamed_text(
        cx: &mut TestAppContext,
    ) {
        let (chat, cx) = chat_window(cx);
        chat.update(cx, |chat, _| {
            chat.messages = vec![Message::assistant_streaming(1, None)];
            chat.apply_agent_event(AgentUiEvent::Result {
                result: "Result-only reply".into(),
                duration_ms: 1,
                usage: bruhs_core::UsageInfo::default(),
            });
            assert_eq!(chat.messages[0].content, "Result-only reply");
            chat.apply_agent_event(AgentUiEvent::Result {
                result: "Result-only reply".into(),
                duration_ms: 1,
                usage: bruhs_core::UsageInfo::default(),
            });
            assert_eq!(chat.messages[0].content, "Result-only reply");
        });
    }
}
