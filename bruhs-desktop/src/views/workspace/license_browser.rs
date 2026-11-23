//! The notices shipped with this build, browsable offline from About.
use super::*;
use crate::views::language::Language;
use std::io::{Cursor, Read};

#[derive(Deserialize)]
struct Catalog {
    entries: Vec<Component>,
    texts: Vec<SharedString>,
}

#[derive(Deserialize)]
struct Component {
    name: String,
    version: Option<String>,
    bundle: String,
    source: Option<String>,
    notices: Vec<Notice>,
}

#[derive(Deserialize)]
struct Notice {
    license: String,
    text: usize,
}

impl Catalog {
    fn load() -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let dir = exe
            .parent()
            .ok_or("The application directory is unavailable.")?;
        let filename = "THIRD_PARTY_LICENSES.zip";
        let mut data = None;
        for path in [dir.join("../Resources").join(filename), dir.join(filename)] {
            match fs::read(path) {
                Ok(bytes) => {
                    data = Some(bytes);
                    break;
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.to_string()),
            }
        }
        let bytes = data.as_deref().unwrap_or(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/THIRD_PARTY_LICENSES.zip"
        )));
        Self::from_zip(bytes)
    }

    fn from_zip(bytes: &[u8]) -> Result<Self, String> {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
        let file = archive.by_name("notices.json").map_err(|e| e.to_string())?;
        const LIMIT: u64 = 32 * 1024 * 1024;
        if file.size() > LIMIT {
            return Err("The notice catalog is too large.".into());
        }
        let mut json = Vec::new();
        file.take(LIMIT + 1)
            .read_to_end(&mut json)
            .map_err(|e| e.to_string())?;
        let catalog: Self = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
        if catalog.entries.iter().any(|c| {
            c.notices.is_empty() || c.notices.iter().any(|n| n.text >= catalog.texts.len())
        }) {
            return Err("The notice catalog contains an invalid text reference.".into());
        }
        Ok(catalog)
    }
}

pub(super) struct LicenseBrowser {
    language: Language,
    catalog: Option<Catalog>,
    error: Option<String>,
    input: Entity<InputState>,
    search_open: bool,
    search_button: FocusHandle,
    matches: Vec<usize>,
    expanded: Option<usize>,
    rows: ListState,
    _change: gpui::Subscription,
}

impl Workspace {
    pub(super) fn open_license_browser(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let language = self.ui_language();
        self.license_browser = Some(cx.new(|cx| LicenseBrowser::new(language, window, cx)));
        self.license_forward = false;
        self.focus_modal_dialog(window, cx);
        cx.notify();
    }

    pub(super) fn return_to_about(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.license_browser = None;
        self.license_forward = true;
        self.license_trigger.focus(window, cx);
        cx.notify();
    }
}

impl LicenseBrowser {
    fn new(language: Language, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder(language.text("Search licenses")));
        let change = cx.subscribe(
            &input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Change => this.filter(cx),
                InputEvent::Blur if this.input.read(cx).value().is_empty() => {
                    this.search_open = false;
                    cx.notify();
                }
                _ => (),
            },
        );
        let mut this = Self {
            language,
            catalog: None,
            error: None,
            input,
            search_open: false,
            search_button: cx.focus_handle(),
            matches: Vec::new(),
            expanded: None,
            rows: ListState::new(0, ListAlignment::Top, px(120.)),
            _change: change,
        };
        this.load(cx);
        this
    }

    fn load(&mut self, cx: &mut Context<'_, Self>) {
        self.error = None;
        let task = cx.background_spawn(async { Catalog::load() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(data) => this.catalog = Some(data),
                    Err(e) => this.error = Some(e),
                }
                this.filter(cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn filter(&mut self, cx: &mut Context<'_, Self>) {
        let q = self.input.read(cx).value().to_lowercase();
        let words = q.split_whitespace().collect::<Vec<_>>();
        self.matches.clear();
        if let Some(catalog) = &self.catalog {
            for (index, c) in catalog.entries.iter().enumerate() {
                if c.matches(&words) {
                    self.matches.push(index);
                }
            }
        }
        self.rows.reset(self.matches.len());
        if !self.matches.is_empty() {
            self.rows.scroll_to_reveal_item(0);
        }
        cx.notify();
    }

    pub(super) fn escape_search(&mut self, w: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        if !self.search_open || !self.input.read(cx).focus_handle(cx).is_focused(w) {
            return false;
        }
        self.input.update(cx, |i, cx| i.set_value("", w, cx));
        self.search_open = false;
        self.filter(cx);
        self.search_button.focus(w, cx);
        true
    }

    fn row(&self, position: usize, cx: &Context<'_, Self>) -> AnyElement {
        let Some((&index, cat)) = self.matches.get(position).zip(self.catalog.as_ref()) else {
            return div().into_any_element();
        };
        let c = &cat.entries[index];
        let open = self.expanded == Some(index);
        let id = SharedString::from(format!("license-notice-{index}"));
        let toggle = on_press_view(
            div()
                .id(id.clone())
                .debug_selector(move || id.to_string())
                .role(Role::Button)
                .aria_label(c.name.clone())
                .aria_expanded(open)
                .tab_index(0)
                .cursor_pointer()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap_2()
                .py_2()
                .px_2()
                .rounded_sm()
                .focus_visible(|el| el.bg(theme::longan_800()))
                .hover(|el| el.bg(theme::longan_800()))
                .child(
                    svg()
                        .path(if open {
                            assets::CHEVRON_DOWN_ICON_PATH
                        } else {
                            assets::CHEVRON_RIGHT_ICON_PATH
                        })
                        .text_color(theme::rambutan_400())
                        .size(px(13.))
                        .flex_shrink_0(),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .flex()
                        .items_baseline()
                        .gap_2()
                        .child(div().min_w_0().truncate().child(c.name.clone()))
                        .when_some(c.version.as_ref(), |row, version| {
                            row.child(
                                div()
                                    .text_xs()
                                    .text_color(theme::rambutan_400())
                                    .child(version.clone()),
                            )
                        }),
                )
                .child(
                    div()
                        .max_w(relative(0.42))
                        .truncate()
                        .text_xs()
                        .text_color(theme::rambutan_400())
                        .child(format!("{} · {}", c.license_label(), c.bundle)),
                ),
            cx,
            move |this, _, cx| {
                let previous = this.expanded.take();
                if previous != Some(index) {
                    this.expanded = Some(index);
                }
                if let Some(old) = previous.and_then(|i| this.matches.iter().position(|v| *v == i))
                {
                    this.rows.remeasure_items(old..old + 1);
                }
                this.rows.remeasure_items(position..position + 1);
                cx.notify();
            },
        );
        let mut header = div().flex().items_center().pr(px(8.0)).child(toggle);
        if let Some(url) = c.source.as_ref().filter(|s| {
            reqwest::Url::parse(s).is_ok_and(|u| matches!(u.scheme(), "https" | "http"))
        }) {
            let url = url.clone();
            header = header.child(on_press_view(
                web_button(
                    SharedString::from(format!("license-source-{index}")),
                    WebButtonVariant::Ghost,
                    WebButtonSize::IconSmall,
                    ButtonState::Enabled,
                )
                .role(Role::Link)
                .aria_label(
                    self.language
                        .text("View project source for {name}")
                        .replace("{name}", &c.name),
                )
                .child(
                    svg()
                        .path(assets::EXTERNAL_LINK_ICON_PATH)
                        .text_color(theme::rambutan_400())
                        .size(px(13.)),
                ),
                cx,
                move |_, _, cx| cx.open_url(&url),
            ));
        }
        let mut el = div()
            .w_full()
            .min_w_0()
            .border_b_1()
            .border_color(theme::longan_800())
            .child(header);
        if open {
            for n in &c.notices {
                el = el.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_xs()
                        .child(cat.texts[n.text].clone()),
                );
            }
        }
        el.into_any_element()
    }
}

impl Component {
    fn matches(&self, words: &[&str]) -> bool {
        if words.is_empty() {
            return true;
        }
        let fields = format!(
            "{} {} {} {}",
            self.name,
            self.version.as_deref().unwrap_or(""),
            self.bundle,
            self.license_label()
        )
        .to_lowercase();
        words.iter().all(|word| fields.contains(word))
    }

    fn license_label(&self) -> String {
        let mut labels = Vec::new();
        for n in &self.notices {
            if !labels.contains(&n.license.as_str()) {
                labels.push(n.license.as_str());
            }
        }
        labels.join(" / ")
    }
}

impl Render for LicenseBrowser {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let root = div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .p_4()
            .gap_2()
            .text_sm();
        let Some(cat) = &self.catalog else {
            return if let Some(error) = &self.error {
                root.child(self.language.text("Could not load open-source notices"))
                    .child(error.clone())
                    .child(on_press_view(
                        web_button(
                            "licenses-retry",
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child(self.language.text("Retry")),
                        cx,
                        |this, _, cx| this.load(cx),
                    ))
            } else {
                root.child(self.language.text("Loading open-source notices…"))
            };
        };
        let total = cat.entries.len();
        let count = if total == self.matches.len() {
            self.language
                .text("{count} notices")
                .replace("{count}", &total.to_string())
        } else {
            self.language
                .text("{matched} of {total}")
                .replace("{matched}", &self.matches.len().to_string())
                .replace("{total}", &total.to_string())
        };
        let mut header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(self.language.text("Third-party notices")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::rambutan_400())
                    .child(count),
            );
        if self.search_open {
            header = header.child(
                div().w(px(150.)).min_w_0().child(
                    Input::new(&self.input)
                        .aria_label(self.language.text("Search open-source licenses")),
                ),
            );
        } else {
            header = header.child(on_press_view(
                web_button(
                    "licenses-search",
                    WebButtonVariant::Ghost,
                    WebButtonSize::IconSmall,
                    ButtonState::Enabled,
                )
                .debug_selector(|| "licenses-search".into())
                .track_focus(&self.search_button)
                .aria_label(self.language.text("Search open-source licenses"))
                .child(
                    svg()
                        .path(assets::TOOL_SEARCH_ICON_PATH)
                        .text_color(theme::rambutan_400())
                        .size(px(14.)),
                ),
                cx,
                |this, w, cx| {
                    this.search_open = true;
                    this.input.update(cx, |i, cx| i.focus(w, cx));
                    cx.notify();
                },
            ));
        }
        let root = root.child(header);
        if self.matches.is_empty() {
            return root.child(
                div()
                    .p_4()
                    .child(self.language.text("No licenses match that search.")),
            );
        }
        root.child(
            div()
                .relative()
                .border_1()
                .border_color(theme::longan_800())
                .rounded_md()
                .overflow_hidden()
                .min_h_0()
                .flex_1()
                .child(
                    list(
                        self.rows.clone(),
                        cx.processor(|this, position, _, cx| this.row(position, cx)),
                    )
                    .size_full(),
                )
                .vertical_scrollbar(&self.rows),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Catalog;

    #[test]
    fn source_build_notices_are_searchable_without_a_packaged_app() {
        let catalog = Catalog::from_zip(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/THIRD_PARTY_LICENSES.zip"
        )))
        .unwrap();
        let fonts = catalog
            .entries
            .iter()
            .find(|component| component.matches(&["ofl-1.1", "mona", "assets"]))
            .unwrap();
        assert!(!fonts.matches(&["monaspace", "rust"]));
        assert_eq!(
            catalog.texts[fonts.notices[0].text].as_ref(),
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/LICENSE"))
        );
        assert!(catalog
            .entries
            .iter()
            .any(|component| component.name == "serde"
                && component.version.is_some()
                && component.source.is_some()));
        assert!(catalog
            .entries
            .iter()
            .any(|component| component.bundle == "Composer syntaxes"));
        assert!(Catalog::from_zip(b"incomplete download").is_err());
    }
}
