//! Small, portable project artwork shared by Settings, project selection and sidebar folders.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::{
    io::{self, Cursor, Read},
    path::Path,
    sync::{Arc, OnceLock},
};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Artwork {
    Emoji(String),
    Png {
        data: String,
        #[serde(skip)]
        image: OnceLock<Option<Arc<gpui::Image>>>,
    },
}
impl PartialEq for Artwork {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Emoji(a), Self::Emoji(b)) => a == b,
            (Self::Png { data: a, .. }, Self::Png { data: b, .. }) => a == b,
            _ => false,
        }
    }
}
impl Eq for Artwork {}
impl Artwork {
    fn from_file(path: &Path) -> io::Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(io::Error::other("Choose an image smaller than 16 MB."));
        }
        let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        let thumb = reader
            .decode()
            .map_err(|_| {
                io::Error::other(
                    "Choose a PNG, JPEG, GIF or WebP image up to 8192 pixels per side.",
                )
            })?
            .thumbnail(96, 96);
        let mut png = Cursor::new(Vec::new());
        thumb
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(io::Error::other)?;
        Ok(Self::Png {
            data: STANDARD.encode(png.into_inner()),
            image: Default::default(),
        })
    }
    fn element(&self, size: f32) -> Option<AnyElement> {
        match self {
            Self::Emoji(text) => Some(
                div()
                    .size(px(size))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(size * 0.88))
                    .child(text.clone())
                    .into_any_element(),
            ),
            Self::Png { data, image } => {
                let picture = image.get_or_init(|| {
                    if data.len() > 100_000 {
                        return None;
                    }
                    Some(Arc::new(gpui::Image::from_bytes(
                        gpui::ImageFormat::Png,
                        STANDARD.decode(data).ok()?,
                    )))
                });
                picture.as_ref().map(|picture| {
                    gpui::img(picture.clone())
                        .size(px(size))
                        .flex_shrink_0()
                        .object_fit(gpui::ObjectFit::Contain)
                        .into_any_element()
                })
            }
        }
    }
}
impl Workspace {
    pub(super) fn project_icon(&self, scope: &str, size: f32) -> AnyElement {
        self.project_defaults
            .get(scope)
            .and_then(|p| p.artwork.as_ref())
            .and_then(|art| art.element(size))
            .unwrap_or_else(|| {
                svg()
                    .path(assets::FOLDER_ICON_PATH)
                    .size(px(size))
                    .text_color(theme::rambutan_500())
                    .flex_shrink_0()
                    .into_any_element()
            })
    }
    fn save_project_icon(
        &mut self,
        scope: String,
        art: Option<Artwork>,
        cx: &mut Context<'_, Self>,
    ) {
        if scope.is_empty() {
            return;
        }
        let prefs = self.project_defaults.entry(scope.clone()).or_default();
        prefs.artwork = art;
        if *prefs == project_defaults::ProjectDefaults::default() {
            self.project_defaults.remove(&scope);
        }
        self.action_editor.art_error = None;
        self.action_editor.art_open = false;
        self.persist_and_notify(cx);
    }
    fn pick_art_image(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let scope = self.settings_project_scope.clone();
        let choice = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose project icon".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let path = match choice.await {
                Ok(Ok(Some(mut paths))) => paths.pop(),
                _ => None,
            };
            let Some(path) = path else {
                return;
            };
            let result = cx
                .background_spawn(async move { Artwork::from_file(&path) })
                .await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(art) => this.save_project_icon(scope, Some(art), cx),
                Err(error) => {
                    this.action_editor.art_error = Some(error.to_string());
                    cx.notify();
                }
            });
        })
        .detach();
    }
    pub(super) fn project_art_row(&self, cx: &Context<'_, Self>) -> AnyElement {
        let button = |id: &'static str, text: &'static str| {
            web_button(
                id,
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .aria_label(text)
            .child(text)
        };
        let scope = &self.settings_project_scope;
        let mut controls = div().flex().flex_col().min_w_0().gap_2().child(
            div()
                .flex()
                .items_center()
                .flex_wrap()
                .gap_2()
                .child(self.project_icon(scope, 24.))
                .child(on_press_view(
                    button("choose-project-emoji", "Choose icon"),
                    cx,
                    |this, window, cx| {
                        this.action_editor.art_open = !this.action_editor.art_open;
                        this.action_editor.art_error = None;
                        if this.action_editor.art_open {
                            this.action_editor.art_input.update(cx, |f, cx| {
                                f.set_value("", window, cx);
                                f.focus(window, cx);
                            });
                        }
                        cx.notify();
                    },
                )),
        );
        if self.action_editor.art_open {
            let image_actions = div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(on_press_view(
                    button("choose-project-image", "Choose image"),
                    cx,
                    |this, window, cx| this.pick_art_image(window, cx),
                ))
                .when(
                    self.project_defaults
                        .get(scope)
                        .is_some_and(|p| p.artwork.is_some()),
                    |el| {
                        el.child(on_press_view(
                            button("reset-project-icon", "Reset icon"),
                            cx,
                            |this, _, cx| {
                                this.save_project_icon(
                                    this.settings_project_scope.clone(),
                                    None,
                                    cx,
                                )
                            },
                        ))
                    },
                );
            controls = controls.child(image_actions);
            let mut samples = div().flex().flex_wrap().gap_1();
            for (i, emoji) in ["🍑", "🥝", "🔧", "🌱", "💻", "🎨", "📚", "🚀"]
                .into_iter()
                .enumerate()
            {
                samples = samples.child(on_press_view(
                    web_button(
                        ("project-emoji", i),
                        WebButtonVariant::Ghost,
                        WebButtonSize::Small,
                        ButtonState::Enabled,
                    )
                    .aria_label(format!("Choose {emoji}"))
                    .child(emoji),
                    cx,
                    move |this, _, cx| {
                        this.save_project_icon(
                            this.settings_project_scope.clone(),
                            Some(Artwork::Emoji(emoji.into())),
                            cx,
                        )
                    },
                ));
            }
            controls = controls
                .child(samples)
                .child(Input::new(&self.action_editor.art_input).aria_label("Custom project emoji"))
                .child(on_press_view(
                    button("apply-project-emoji", "Apply emoji"),
                    cx,
                    |this, _, cx| {
                        let value = this
                            .action_editor
                            .art_input
                            .read(cx)
                            .value()
                            .trim()
                            .to_owned();
                        if value.len() > 64
                            || value.graphemes(true).count() != 1
                            || value.chars().any(char::is_control)
                        {
                            this.action_editor.art_error =
                                Some("Choose one emoji or symbol.".into());
                            cx.notify();
                            return;
                        }
                        this.save_project_icon(
                            this.settings_project_scope.clone(),
                            Some(Artwork::Emoji(value)),
                            cx,
                        );
                    },
                ));
        }
        self.settings_row(
            "settings-project-icon",
            "Project icon",
            "Identify this checkout in the project picker and sidebar.",
            settings_ui::control_column(
                controls
                    .when_some(self.action_editor.art_error.clone(), |el, e| el.child(e))
                    .into_any_element(),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{project_defaults, Artwork, BTreeMap, SettingsSection, STANDARD};
    use base64::Engine as _;
    #[gpui::test]
    async fn project_icon_picker_keeps_a_portable_thumbnail_and_resets_only_the_selected_project(
        cx: &mut gpui::TestAppContext,
    ) {
        let (w, cx, _, _) = super::super::tests::thread_menu_workspace(cx);
        let d = tempfile::tempdir().unwrap();
        let selected = d.path().join("icon.png");
        image::RgbaImage::from_pixel(240, 120, image::Rgba([100u8, 42, 200, 255]))
            .save(&selected)
            .unwrap();
        w.update_in(cx, |w, window, cx| {
            w.set_settings_section(SettingsSection::Projects, cx);
            w.settings_project_scope = "/first".into();
            w.save_project_icon("/second".into(), Some(Artwork::Emoji("🥝".into())), cx);
            w.pick_art_image(window, cx);
        });
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(move |options| {
            assert!(options.files && !options.multiple && !options.directories);
            Some(vec![selected])
        });
        cx.condition(&w, |w, _| {
            w.project_defaults
                .get("/first")
                .is_some_and(|p| matches!(p.artwork, Some(Artwork::Png { .. })))
        })
        .await;
        drop(d);
        w.update(cx, |w, cx| {
            let json = serde_json::to_string(&w.project_defaults).unwrap();
            let saved: BTreeMap<String, project_defaults::ProjectDefaults> =
                serde_json::from_str(&json).unwrap();
            let art = saved["/first"].artwork.as_ref().unwrap();
            let Artwork::Png { data, .. } = art else {
                panic!("no image")
            };
            let thumb = image::load_from_memory(&STANDARD.decode(data).unwrap()).unwrap();
            assert_eq!((thumb.width(), thumb.height()), (96, 48));
            assert!(art.element(18.).is_some());
            w.save_project_icon("/first".into(), None, cx);
            assert!(!w.project_defaults.contains_key("/first"));
            assert!(matches!(
                w.project_defaults["/second"].artwork,
                Some(Artwork::Emoji(_))
            ));
        });
    }
}
