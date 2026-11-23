mod color_picker;
mod inspector;
mod panel;
mod stylesheet;
use super::color_families::{self, GROUPS};
use super::import::{filename_label, read_selection, Selection};
use super::library::{InstalledTheme, ThemeFile};
use super::picker::ThemeChoice;
use super::*;
use bruhs_ui::press::on_press_view;
use bruhs_ui::theme::ThemeMode;
use std::io::Write;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ThemeAction {
    Duplicate,
    Edit,
    Export,
    Remove,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ColorField {
    role: &'static str,
    input: Entity<InputState>,
    swatch: FocusHandle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Draft {
    file: ThemeFile,
    edit: bool,
    name: Entity<InputState>,
    mode: ThemeMode,
    fields: Vec<ColorField>,
    picker: Option<Entity<color_picker::ColorPicker>>,
    advanced: bool,
    regenerate: bool,
    filter: Entity<InputState>,
    panel: Entity<panel::Panel>,
    css: Entity<stylesheet::Stylesheet>,
}

impl Draft {
    fn fields(
        file: &ThemeFile,
        mode: ThemeMode,
        window: &mut Window,
        cx: &mut Context<'_, Workspace>,
    ) -> Vec<ColorField> {
        let colors = file.colors_for(mode);
        GROUPS
            .iter()
            .flat_map(|(_, families)| families.iter().map(|family| family.role))
            .map(|role| {
                let value = colors
                    .and_then(|map| map.get(role))
                    .cloned()
                    .unwrap_or_default();
                let field = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("Inherited")
                        .default_value(value)
                });
                cx.subscribe_in(&field, window, move |ws, source, event, w, cx| {
                    if matches!(event, InputEvent::Change) {
                        ws.edit_theme_color(source, role, w, cx);
                    } else if matches!(event, InputEvent::Focus) {
                        ws.select_theme_usage(role, false, cx);
                    }
                })
                .detach();
                ColorField {
                    role,
                    input: field,
                    swatch: cx.focus_handle(),
                }
            })
            .collect()
    }
    fn sync_fields(&self, except: Option<&Entity<InputState>>, w: &mut Window, cx: &mut App) {
        let colors = self.file.colors_for(self.mode);
        for ColorField {
            role, input: field, ..
        } in &self.fields
        {
            let value = colors.and_then(|c| c.get(*role)).map_or("", String::as_str);
            if Some(field) != except && field.read(cx).value().as_ref() != value {
                field.update(cx, |input, cx| input.set_value(value.to_owned(), w, cx));
            }
        }
    }

    fn current_file(&self, cx: &App) -> ThemeFile {
        let mut file = self.file.clone();
        file.name = self.name.read(cx).value().trim().to_owned();
        if !self.edit {
            file.id.clear();
        }
        file
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::views::workspace) struct ThemeEditor {
    json: Entity<TextareaState>,
    pub(super) community: Option<Entity<super::community::Browser>>,
    draft: Option<Draft>,
    pending: Vec<Arc<InstalledTheme>>,
    source_name: Option<String>,
    error: Option<String>,
    reading: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Collision {
    Ask,
    Update,
    KeepBoth,
}

pub(super) fn theme_button(id: &'static str, label: &'static str, primary: bool) -> Stateful<Div> {
    theme_button_state(id, label, primary, ButtonState::Enabled)
}

fn theme_button_state(
    id: &'static str,
    label: impl Into<SharedString>,
    primary: bool,
    state: ButtonState,
) -> Stateful<Div> {
    let label = label.into();
    web_button(
        id,
        if primary {
            WebButtonVariant::Primary
        } else {
            WebButtonVariant::Outline
        },
        WebButtonSize::Small,
        state,
    )
    .debug_selector(move || id.into())
    .aria_label(label.clone())
    .child(label)
}

impl Workspace {
    pub(super) fn show_theme_import_notice(&mut self, message: String, cx: &mut Context<'_, Self>) {
        let since = std::time::Instant::now();
        self.theme_import_notice = Some((message, since));
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(5))
                .await;
            let _ = this.update(cx, |ws, cx| {
                if ws
                    .theme_import_notice
                    .as_ref()
                    .is_some_and(|(_, at)| *at == since)
                {
                    ws.theme_import_notice = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    pub(in crate::views::workspace) fn render_theme_import_notice(&self) -> Option<AnyElement> {
        let (message, _) = self.theme_import_notice.as_ref()?;
        Some(
            div()
                .id("community-theme-notice")
                .role(Role::Status)
                .absolute()
                .right_4()
                .bottom_4()
                .bg(theme::longan_900())
                .text_color(theme::rambutan_100())
                .border_1()
                .border_color(theme::ring_modal())
                .rounded_lg()
                .px_3()
                .py_2()
                .text_sm()
                .child(message.clone())
                .into_any_element(),
        )
    }
    pub(super) fn theme_action_button(
        &self,
        action: ThemeAction,
        choice: ThemeChoice,
        key: &str,
        collection_label: Option<&str>,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let (verb, icon) = match action {
            ThemeAction::Duplicate => ("Duplicate", assets::COPY_ICON_PATH),
            ThemeAction::Edit => ("Edit", assets::TOOL_PENCIL_ICON_PATH),
            ThemeAction::Export => ("Export", assets::DOWNLOAD_ICON_PATH),
            ThemeAction::Remove => ("Remove", assets::CLOSE_ICON_PATH),
        };
        let id = format!("theme-{}-{key}", verb.to_ascii_lowercase());
        let label = match (action, collection_label) {
            (ThemeAction::Remove, Some(collection)) => format!("Remove themes from {collection}"),
            _ => format!("{verb} {}", choice.label()),
        };
        let button = web_button(
            SharedString::from(id.clone()),
            WebButtonVariant::Ghost,
            WebButtonSize::IconSmall,
            ButtonState::Enabled,
        )
        .debug_selector(move || id)
        .aria_label(label.clone())
        .tooltip(move |w, cx| Tooltip::new(label.clone()).build(w, cx))
        .child(
            svg()
                .path(icon)
                .size(px(14.))
                .text_color(theme::rambutan_400()),
        );
        on_press_view(button, cx, move |ws, w, cx| match (action, &choice) {
            (ThemeAction::Duplicate, _) => ws.create_theme(Some(&choice), w, cx),
            (ThemeAction::Edit, ThemeChoice::Custom(t)) => {
                ws.start_theme_editor(Some((t.file.clone(), true)), !t.file.managed(), w, cx)
            }
            (ThemeAction::Export, ThemeChoice::Custom(t)) => ws.export_theme(t.clone(), w, cx),
            (ThemeAction::Remove, ThemeChoice::Custom(t)) => ws.remove_theme(t.clone(), w, cx),
            _ => {}
        })
    }

    pub(super) fn create_theme(
        &mut self,
        source: Option<&ThemeChoice>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let file = source.map_or_else(
            || {
                let mut file = ThemeFile::seed(
                    "",
                    self.selected_palette(ThemeMode::Light),
                    self.selected_palette(ThemeMode::Dark),
                );
                for mode in [ThemeMode::Light, ThemeMode::Dark] {
                    if let Some(colors) = self
                        .selected_custom_theme(mode)
                        .and_then(|theme| theme.file.colors_for(mode))
                    {
                        file.set_colors(mode, colors.clone());
                    }
                }
                file
            },
            ThemeChoice::duplicate,
        );
        let advanced = matches!(source,Some(ThemeChoice::Custom(theme)) if !theme.file.managed())
            || (source.is_none()
                && self
                    .selected_custom_theme(cx.theme().mode)
                    .is_some_and(|t| !t.file.managed()));
        self.start_theme_editor(Some((file, false)), advanced, window, cx);
    }
    pub(super) fn open_theme_import(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.start_theme_editor(None, false, window, cx);
    }
    fn start_theme_editor(
        &mut self,
        source: Option<(ThemeFile, bool)>,
        advanced: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let json = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(10)
                .placeholder("{\n  \"version\": 1,\n  \"name\": \"Aurora\",\n  \"appearance\": \"light\",\n  \"colors\": { \"canvas\": \"#faf6f1\" }\n}")
        });
        cx.subscribe(&json, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        let draft = source.map(|(file, edit)| {
            let active = cx.theme().mode;
            let mode = if file.colors_for(active).is_some() {
                active
            } else {
                file.appearance
            };
            let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter colors"));
            cx.subscribe(&filter, |ws, source, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    if let Some(Overlay::ThemeEditor(ThemeEditor {
                        draft: Some(draft), ..
                    })) = &mut ws.overlay
                    {
                        if draft.filter == source {
                            draft.picker = None;
                        }
                    }
                    cx.notify();
                }
            })
            .detach();
            let name = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(if edit { "Theme name" } else { "e.g. Aurora" })
                    .default_value(file.name.clone())
            });
            cx.subscribe_in(&name, window, |ws, field, event: &InputEvent, w, cx| {
                if matches!(event, InputEvent::Change) {
                    if matches!(&ws.overlay,
                        Some(Overlay::ThemeEditor(ThemeEditor { draft: Some(d), .. }))
                            if !d.edit && &d.name == field
                    ) {
                        ws.preview_theme_draft(w, cx);
                    }
                    cx.notify();
                }
            })
            .detach();
            let fields = Draft::fields(&file, mode, window, cx);
            let capture = crate::views::theme_regions::start(window, cx);
            let panel = cx.new(|_| panel::Panel::new(capture));
            cx.observe(&panel, |_, _, cx| cx.notify()).detach();
            let css = cx.new(|cx| {
                stylesheet::Stylesheet::new(
                    file.css(),
                    edit.then(|| file.id.clone()),
                    name.clone(),
                    window,
                    cx,
                )
            });
            cx.subscribe_in(
                &css,
                window,
                |ws, source, _: &stylesheet::Changed, window, cx| {
                    let Some(Overlay::ThemeEditor(ThemeEditor {
                        draft: Some(d),
                        error,
                        ..
                    })) = &mut ws.overlay
                    else {
                        return;
                    };
                    if &d.css != source {
                        return;
                    }
                    d.file.set_css(&source.read(cx).value(cx));
                    *error = None;
                    ws.preview_theme_draft(window, cx);
                    cx.notify();
                },
            )
            .detach();
            Draft {
                file,
                edit,
                name,
                mode,
                fields,
                picker: None,
                advanced,
                regenerate: advanced,
                filter,
                panel,
                css,
            }
        });
        let owner = cx.weak_entity();
        let community = draft
            .is_none()
            .then(|| cx.new(|cx| super::community::Browser::new(owner, window, cx)));
        let focus = community
            .as_ref()
            .map(|browser| browser.read(cx).query.focus_handle(cx))
            .or_else(|| draft.as_ref().map(|d| d.name.focus_handle(cx)));
        self.theme_navigation.return_focus = window.focused(cx);
        self.open_overlay(Overlay::ThemeEditor(ThemeEditor {
            json,
            community,
            draft,
            pending: vec![],
            source_name: None,
            error: None,
            reading: false,
        }));
        self.preview_theme_draft(window, cx);
        if let Some(focus) = focus {
            cx.defer_in(window, move |_, w, cx| focus.focus(w, cx));
        }
        cx.notify();
    }
    pub(in crate::views::workspace) fn restore_theme_preview(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.theme_preview_active
            && !matches!(
                self.overlay,
                Some(Overlay::ThemeEditor(ThemeEditor { draft: Some(_), .. }))
            )
        {
            self.theme_preview_active = false;
            self.apply_device_appearance(window, cx);
        }
    }

    pub(in crate::views::workspace) fn preview_theme_draft(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(Overlay::ThemeEditor(ThemeEditor {
            draft: Some(draft), ..
        })) = &self.overlay
        else {
            return;
        };
        let Some(palette) = draft.file.colors_for(draft.mode).and_then(|colors| {
            super::library::native_palette(draft.mode, colors, draft.file.css()).ok()
        }) else {
            return;
        };
        let id = if draft.edit {
            draft.file.id.clone()
        } else {
            super::library::identifier(draft.name.read(cx).value().trim())
        };
        super::background::select(Some(&id), palette.images, window, cx);
        self.theme_preview_active = true;
        bruhs_ui::Theme::use_color_roles(
            palette.palette,
            Some(palette.roles),
            draft.mode,
            Some(window),
            cx,
        );
    }

    pub(in crate::views::workspace) fn dismiss_theme_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if !matches!(
            self.overlay,
            Some(Overlay::ThemeEditor(_) | Overlay::RemoveThemeVariants(_))
        ) {
            return false;
        }
        self.open_overlay(Overlay::Settings {
            open_dropdown: None,
        });
        self.restore_theme_preview(window, cx);
        self.focus_modal_dialog(window, cx);
        cx.notify();
        true
    }

    pub(in crate::views::workspace) fn restore_theme_dialog_focus(
        &mut self,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.settings_visible() {
            return;
        }
        let Some(target) = self.theme_navigation.return_focus.take() else {
            return;
        };
        cx.defer_in(window, move |ws, w, cx| {
            if ws.settings_visible() && ws.modal_start_focus_handle.contains(&target, w) {
                target.focus(w, cx);
            }
        });
    }

    fn edit_theme_color(
        &mut self,
        source: &Entity<InputState>,
        role: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(Overlay::ThemeEditor(ThemeEditor {
            draft: Some(draft),
            error,
            ..
        })) = &mut self.overlay
        else {
            return;
        };
        if !draft.fields.iter().any(|f| &f.input == source) {
            return;
        }
        let mut colors = draft
            .file
            .colors_for(draft.mode)
            .cloned()
            .unwrap_or_default();
        color_families::edit(
            &mut colors,
            draft.mode,
            role,
            source.read(cx).value().trim(),
            draft.advanced,
        );
        draft.regenerate |= draft.advanced;
        draft.file.set_colors(draft.mode, colors);
        draft.sync_fields(Some(source), window, cx);
        *error = None;
        self.preview_theme_draft(window, cx);
        cx.notify();
    }

    fn toggle_advanced_colors(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(Overlay::ThemeEditor(ThemeEditor {
            draft: Some(draft), ..
        })) = &mut self.overlay
        else {
            return;
        };
        draft.picker = None;
        draft.panel.update(cx, |p, cx| {
            p.inspector = Default::default();
            cx.notify()
        });
        draft.advanced = !draft.advanced;
        if !draft.advanced && draft.regenerate {
            for mode in [ThemeMode::Light, ThemeMode::Dark] {
                if !draft.edit && mode != draft.mode {
                    continue;
                }
                if let Some(colors) = draft.file.colors_for(mode) {
                    draft
                        .file
                        .set_colors(mode, color_families::guided(colors, mode));
                }
            }
            draft.regenerate = false;
        }
        draft.sync_fields(None, window, cx);
        self.preview_theme_draft(window, cx);
        cx.notify();
    }

    fn change_theme_variant(
        &mut self,
        mode: ThemeMode,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(Overlay::ThemeEditor(editor)) = &mut self.overlay else {
            return;
        };
        let Some(draft) = &mut editor.draft else {
            return;
        };
        if mode == draft.mode {
            return;
        }
        draft.picker = None;
        draft.file = draft.current_file(cx);
        if mode != draft.file.appearance && !draft.file.variants.contains_key(&mode) {
            let inherited = ThemeFamily::Bruhs.palette(mode).surface[0];
            draft.file.variants.insert(
                mode,
                BTreeMap::from([("canvas".into(), format!("#{inherited:06x}"))]),
            );
        }
        draft.fields = Draft::fields(&draft.file, mode, window, cx);
        draft.panel.update(cx, |p, cx| {
            p.inspector = Default::default();
            cx.notify()
        });
        draft.mode = mode;
        self.preview_theme_draft(window, cx);
        cx.notify();
    }

    fn read_theme_files(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Choose theme files".into()),
        });
        self.read_theme_paths(
            async move {
                match paths.await {
                    Ok(Ok(selection)) => Ok(selection.unwrap_or_default()),
                    _ => Err("The file picker could not be opened.".into()),
                }
            },
            window,
            cx,
        );
    }

    fn read_theme_paths(
        &mut self,
        paths: impl std::future::Future<Output = Result<Vec<PathBuf>, String>> + 'static,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(Overlay::ThemeEditor(editor)) = &mut self.overlay else {
            return;
        };
        if editor.reading {
            return;
        }
        let session = editor.json.clone();
        editor.reading = true;
        editor.error = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = match paths.await {
                Ok(paths) => {
                    cx.background_spawn(async move { read_selection(paths) })
                        .await
                }
                Err(error) => Err(error),
            };
            let _ = this.update_in(cx, |ws, w, cx| {
                let Some(Overlay::ThemeEditor(editor)) = &mut ws.overlay else {
                    return;
                };
                if editor.json != session {
                    return;
                }
                editor.reading = false;
                match result {
                    Ok(Selection::Single { name, text }) => {
                        editor.source_name = Some(name);
                        editor.pending.clear();
                        editor
                            .json
                            .update(cx, |field, cx| field.set_value(text, w, cx));
                    }
                    Ok(Selection::Batch { themes, errors }) => {
                        editor.source_name = None;
                        editor.error = (!errors.is_empty()).then(|| errors.join("\n"));
                        ws.install_themes(themes, Collision::Ask, true, w, cx);
                    }
                    Ok(Selection::Canceled) => {}
                    Err(e) => editor.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn save_theme_editor(
        &mut self,
        collision: Collision,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(Overlay::ThemeEditor(editor)) = &mut self.overlay else {
            return;
        };
        if editor.reading {
            return;
        }
        if editor.pending.is_empty() {
            editor.error = None;
        }
        let edit = editor.draft.as_ref().is_some_and(|d| d.edit);
        let themes = if !editor.pending.is_empty() {
            Ok(editor.pending.clone())
        } else {
            let theme = match &editor.draft {
                Some(d) => InstalledTheme::try_from(d.current_file(cx)),
                None => InstalledTheme::parse(&editor.json.read(cx).value()),
            };
            theme.map(|t| vec![Arc::new(t)])
        };
        match themes {
            Ok(themes) => self.install_themes(
                themes,
                if edit { Collision::Update } else { collision },
                false,
                w,
                cx,
            ),
            Err(e) => {
                editor.error = Some(e);
                cx.notify();
            }
        }
    }
    fn install_themes(
        &mut self,
        themes: Vec<Arc<InstalledTheme>>,
        collision: Collision,
        batch: bool,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(Overlay::ThemeEditor(editor)) = &mut self.overlay else {
            return;
        };
        let activate = themes.len() == 1 && !batch && collision == Collision::Ask;
        let copy_name = (themes.len() == 1)
            .then(|| {
                editor
                    .source_name
                    .as_deref()
                    .and_then(|name| filename_label(Path::new(name)))
            })
            .flatten();
        let mut library = (*self.custom_themes).clone();
        let mut added = Vec::new();
        editor.pending.clear();
        for mut theme in themes {
            if let Some(index) = library
                .themes
                .iter()
                .position(|t| t.file.id == theme.file.id)
            {
                match collision {
                    Collision::Ask => {
                        editor.pending.push(theme);
                        continue;
                    }
                    Collision::Update => {
                        Arc::make_mut(&mut theme).keep_collection(&library.themes[index]);
                        for mode in [ThemeMode::Light, ThemeMode::Dark] {
                            if library.selection(mode) == Some(theme.file.id.as_str())
                                && theme.palette(mode).is_none()
                            {
                                library.select(mode, None);
                            }
                        }
                        library.themes[index] = theme.clone();
                    }
                    Collision::KeepBoth => {
                        let named = copy_name
                            .clone()
                            .filter(|name| !name.eq_ignore_ascii_case(&theme.file.name));
                        let names = named.into_iter().chain((1..100).map(|n| {
                            let suffix = format!(" ({n})");
                            let mut length = suffix.len();
                            let prefix: String = theme
                                .file
                                .name
                                .chars()
                                .take_while(|ch| {
                                    length += ch.len_utf16();
                                    length <= 48
                                })
                                .collect();
                            format!("{prefix}{suffix}")
                        }));
                        let Some(copy) = names
                            .filter_map(|name| theme.relabel(name))
                            .find(|t| library.find(&t.file.id).is_none())
                        else {
                            editor.error = Some(format!("Too many copies of {}.", theme.file.name));
                            continue;
                        };
                        theme = Arc::new(copy);
                        library.themes.push(theme.clone());
                    }
                }
            } else {
                library.themes.push(theme.clone());
            }
            added.push(theme);
        }
        let done = editor.error.is_none() && editor.pending.is_empty();
        if done {
            self.dismiss_theme_editor(w, cx);
        }
        if added.is_empty() {
            cx.notify();
            return;
        }
        self.custom_themes = Arc::new(library);
        if activate {
            self.select_palette_choice(&ThemeChoice::Custom(added.remove(0)), None, w, cx);
        } else {
            self.apply_picker_change(w, cx);
        }
    }
    fn remove_theme(
        &mut self,
        theme: Arc<InstalledTheme>,
        w: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(id) = theme.file.collection() {
            let variants = self.custom_themes.collection(id);
            if variants.len() > 1 {
                self.open_variant_removal(&theme, variants, w, cx);
                return;
            }
        }
        let answer = w.prompt(
            PromptLevel::Warning,
            &format!("Remove “{}”?", theme.file.name),
            Some("This theme will be removed from this device."),
            &[
                PromptButton::Cancel("Cancel".into()),
                PromptButton::Ok("Remove".into()),
            ],
            cx,
        );
        cx.spawn_in(w, async move |this, cx| {
            if answer.await == Ok(1) {
                let _ = this.update_in(cx, |ws, w, cx| {
                    if ws.custom_themes.find(&theme.file.id) != Some(&theme) {
                        return;
                    }
                    let lib = Arc::make_mut(&mut ws.custom_themes);
                    lib.themes.retain(|t| t.file.id != theme.file.id);
                    lib.forget_selection(&theme.file.id);
                    ws.apply_picker_change(w, cx);
                });
            }
        })
        .detach();
    }
    fn export_theme(&mut self, t: Arc<InstalledTheme>, w: &mut Window, cx: &mut Context<'_, Self>) {
        let dir = directories::UserDirs::new()
            .and_then(|u| u.download_dir().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        let picker = cx.prompt_for_new_path(&dir, Some(&format!("{}.json", t.file.id)));
        cx.spawn_in(w, async move |this, cx| {
            let result = match picker.await {
                Ok(Ok(Some(path))) => cx
                    .background_spawn(async move {
                        let mut file = tempfile::NamedTempFile::new_in(
                            path.parent().unwrap_or(Path::new(".")),
                        )?;
                        file.write_all(t.json().as_bytes())?;
                        file.as_file().sync_all()?;
                        file.persist(&path)?;
                        Ok::<_, std::io::Error>(())
                    })
                    .await
                    .map_err(|e| e.to_string()),
                Ok(Ok(None)) => Ok(()),
                _ => Err("The save dialog could not be opened.".into()),
            };
            if let Err(e) = result {
                let _ = this.update_in(cx, |_, w, cx| {
                    drop(w.prompt(
                        PromptLevel::Warning,
                        "Could not export theme",
                        Some(&e),
                        &[PromptButton::Ok("Close".into())],
                        cx,
                    ));
                });
            }
        })
        .detach();
    }

    pub(in crate::views::workspace) fn render_theme_editor(
        &self,
        editor: ThemeEditor,
        w: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let mut content = div().p_4().flex().flex_col().gap_3();
        let title = match &editor.draft {
            Some(d) if d.edit => "Edit theme",
            Some(_) => "Create theme",
            None => "Add a theme",
        };
        if !editor.pending.is_empty() {
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child("Already installed")
                    .child(
                        div().text_sm().text_color(theme::rambutan_400()).child(
                            editor
                                .pending
                                .iter()
                                .map(|t| t.file.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", "),
                        ),
                    ),
            );
        } else if let Some(draft) = &editor.draft {
            let form_row = |label: &'static str, control: AnyElement| {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(116.)).flex_shrink_0().text_sm().child(label))
                    .child(div().min_w_0().flex_1().child(control))
            };
            content = content.p_3().child(form_row(
                "Theme name",
                Input::new(&draft.name)
                    .aria_label("Theme name")
                    .into_any_element(),
            ));
            let modes = [
                (ThemeMode::Light, "Light", "theme-edit-light"),
                (ThemeMode::Dark, "Dark", "theme-edit-dark"),
            ];
            content = content.child(form_row(
                "Appearance",
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(modes.map(|(mode, label, id)| {
                        let has = mode == draft.file.appearance
                            || draft.file.variants.contains_key(&mode);
                        let name = if has {
                            label
                        } else if mode == ThemeMode::Light {
                            "Add light variant"
                        } else {
                            "Add dark variant"
                        };
                        on_press_view(
                            theme_button(id, name, mode == draft.mode),
                            cx,
                            move |ws, w, cx| ws.change_theme_variant(mode, w, cx),
                        )
                    }))
                    .into_any_element(),
            ));
            let query = draft.filter.read(cx).value().trim().to_ascii_lowercase();
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(div().flex().flex_col().gap_1().child("Colors").when(
                        !draft.advanced,
                        |el| {
                            el.child(
                                div()
                                    .text_xs()
                                    .text_color(theme::rambutan_400())
                                    .child("Two colors, rest derived"),
                            )
                        },
                    ))
                    .child(div().flex().items_center().gap_2().child("Advanced").child(
                        settings_ui::switch(
                            "theme-advanced-colors",
                            draft.advanced,
                            false,
                            "Use advanced theme colors",
                            cx,
                            |ws, w, cx| ws.toggle_advanced_colors(w, cx),
                        ),
                    )),
            );
            if draft.advanced {
                content = content.child(
                    div()
                        .debug_selector(|| "theme-filter-colors".into())
                        .child(Input::new(&draft.filter).aria_label("Filter colors")),
                );
            }
            let mut matched = false;
            for &(section, families) in GROUPS {
                let mut group = div().flex().flex_col().gap_1();
                let mut any = false;
                for family in families {
                    if draft.advanced {
                        if !family.matches(&query) {
                            continue;
                        }
                    } else if !matches!(family.role, "canvas" | "accent") {
                        continue;
                    }
                    let role = family.role;
                    let field = draft
                        .fields
                        .iter()
                        .find(|f| f.role == role)
                        .expect("theme family field");
                    let label = if role == "accent" && !draft.advanced {
                        "Accent"
                    } else {
                        family.label
                    };
                    let panel = draft.panel.read(cx);
                    let selected = panel.inspector.selected == Some(role);
                    let name = format!("{} {label} usage", if selected { "Hide" } else { "Show" });
                    let usage = div()
                        .id(SharedString::from(format!("theme-usage-{role}")))
                        .debug_selector(move || format!("theme-usage-{role}"))
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .rounded_md()
                        .tab_index(0)
                        .role(Role::Button)
                        .aria_label(name)
                        .aria_toggled(selected.into())
                        .cursor_pointer()
                        .focus_visible(|s| s.border_1().border_color(theme::persimmon_400()))
                        .child(label);
                    group = group.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .min_h(px(40.))
                            .map(|row| {
                                inspector::reveal_selected_row(row, panel, role, cx.entity_id())
                            })
                            .rounded_md()
                            .when(selected, |el| el.bg(theme::persimmon_500().opacity(0.13)))
                            .child(on_press_view(usage, cx, move |ws, _, cx| {
                                ws.select_theme_usage(role, true, cx)
                            }))
                            .child(self.theme_swatch(draft, field, label, cx))
                            .child(
                                div()
                                    .w(px(120.))
                                    .min_w_0()
                                    .max_w_full()
                                    .debug_selector(move || format!("theme-role-{role}"))
                                    .child(Input::new(&field.input).aria_label(label)),
                            ),
                    );
                    any = true;
                }
                if any {
                    if draft.advanced {
                        content = content.child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(section),
                        );
                    }
                    content = content.child(group);
                    matched = true;
                }
            }
            if !matched {
                content = content.child(
                    div()
                        .id("theme-no-colors")
                        .role(Role::Status)
                        .debug_selector(|| "theme-no-colors".into())
                        .text_sm()
                        .text_color(theme::rambutan_400())
                        .child("No matches."),
                );
            }
            content = content.child(draft.css.clone());
        } else {
            content = content
                .children(editor.community)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().h(px(1.)).flex_1().bg(theme::ring_modal()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::rambutan_400())
                                .child("Or import a file"),
                        )
                        .child(div().h(px(1.)).flex_1().bg(theme::ring_modal())),
                )
                .child(
                    div()
                        .id("theme-file-drop")
                        .debug_selector(|| "theme-file-drop".into())
                        .border_1()
                        .border_color(theme::ring_modal())
                        .rounded_lg()
                        .p_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .drag_over::<gpui::ExternalPaths>(|el, _, _, _| {
                            el.border_color(theme::persimmon_400())
                        })
                        .on_drop(cx.listener(|ws, paths: &gpui::ExternalPaths, w, cx| {
                            let paths = paths.paths().to_vec();
                            ws.read_theme_paths(async move { Ok(paths) }, w, cx);
                        }))
                        .child(
                            div().flex().flex_col().gap_1().child("Theme file").child(
                                div()
                                    .text_xs()
                                    .text_color(theme::rambutan_400())
                                    .child("Drop theme or VS Code .json files"),
                            ),
                        )
                        .child(
                            theme_button_state(
                                "theme-choose-files",
                                if editor.reading {
                                    "Reading…".into()
                                } else {
                                    editor
                                        .source_name
                                        .clone()
                                        .unwrap_or_else(|| "Choose files".into())
                                },
                                false,
                                ButtonState::disabled_if(editor.reading),
                            )
                            .max_w(relative(0.55))
                            .min_w_0()
                            .truncate()
                            .when(!editor.reading, |button| {
                                on_press_view(button, cx, |ws, w, cx| ws.read_theme_files(w, cx))
                            }),
                        ),
                )
                .child("Theme JSON")
                .child(div().debug_selector(|| "theme-json-input".into()).child(
                    form_textarea(Textarea::new(&editor.json), 210.).aria_label("Theme JSON"),
                ));
        }
        if let Some(error) = editor.error {
            content = content.child(
                div()
                    .id("theme-editor-error")
                    .role(Role::Alert)
                    .debug_selector(|| "theme-editor-error".into())
                    .text_color(theme::lychee_200())
                    .child(error),
            );
        }
        let footer = div()
            .flex()
            .flex_wrap()
            .justify_end()
            .gap_2()
            .child(on_press_view(
                theme_button("theme-editor-cancel", "Cancel", false),
                cx,
                |ws, w, cx| {
                    ws.dismiss_theme_editor(w, cx);
                },
            ))
            .when(!editor.pending.is_empty(), |el| {
                el.child(on_press_view(
                    theme_button("theme-conflict-back", "Back", false),
                    cx,
                    |ws, _, cx| {
                        if let Some(Overlay::ThemeEditor(editor)) = &mut ws.overlay {
                            editor.pending.clear();
                            cx.notify();
                        }
                    },
                ))
                .child(on_press_view(
                    theme_button("theme-keep-both", "Keep both", false),
                    cx,
                    |ws, w, cx| ws.save_theme_editor(Collision::KeepBoth, w, cx),
                ))
            });
        let conflict = !editor.pending.is_empty();
        let label = if conflict {
            "Update existing"
        } else if let Some(draft) = &editor.draft {
            if draft.edit {
                "Save changes"
            } else {
                "Create theme"
            }
        } else {
            "Add theme"
        };
        let disabled = editor.reading
            || (!conflict
                && editor
                    .draft
                    .as_ref()
                    .is_some_and(|draft| draft.name.read(cx).value().trim().is_empty()))
            || (!conflict
                && editor.draft.is_none()
                && editor.json.read(cx).value().trim().is_empty());
        let save = theme_button_state(
            "theme-editor-save",
            label,
            true,
            ButtonState::disabled_if(disabled),
        );
        let footer = footer.child(save.when(!disabled, |button| {
            on_press_view(button, cx, move |ws, w, cx| {
                ws.save_theme_editor(
                    if conflict {
                        Collision::Update
                    } else {
                        Collision::Ask
                    },
                    w,
                    cx,
                );
            })
        }));
        if let Some(draft) = &editor.draft {
            return panel::render(title, content, footer, draft, w, cx);
        }
        self.render_modal_shell_sized(title, content, Some(footer), 768., w, cx)
    }
}
