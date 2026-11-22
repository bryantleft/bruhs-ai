//! Folder identities for the project picker, palette and manual ordering.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Order {
    Created,
    Manual,
    #[serde(other)]
    #[default]
    LastMessage,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Entry {
    path: String,
    name: String,
    created: i64,
    last_message: Option<i64>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct ProjectList {
    mode: Order,
    #[serde(skip_serializing_if = "project_grouping::Preferences::is_default")]
    pub(super) grouping: project_grouping::Preferences,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    titles: BTreeMap<String, String>,
    entries: Vec<Entry>,
}

impl ProjectList {
    pub(super) fn empty(&self) -> bool {
        self.mode == Order::default() && self.entries.is_empty() && self.grouping.is_default() && self.titles.is_empty()
    }
    pub(super) fn forget(&mut self, path: &str) {
        self.entries.retain(|entry| entry.path != path);
        self.titles.remove(path);
        self.grouping.forget_checkout(path);
    }
    pub(super) fn saved_title(&self, path: &str) -> Option<Option<String>> {
        self.entries.iter().any(|e| e.path == path).then(|| self.titles.get(path).cloned())
    }

    pub(super) fn title(&self, path: &str) -> Option<&str> {
        self.titles.get(path).map(String::as_str)
    }

    pub(super) fn set_title(&mut self, path: &str, title: Option<&str>) {
        if let Some(title) = title {
            if !self.entries.iter().any(|e| e.path == path) {
                let folder = project_picker_ui::path_display_name(Path::new(path));
                self.register(path, &folder, EpochMs::now().as_millis(), None);
            }
            self.titles.insert(path.to_owned(), title.to_owned());
        } else {
            self.titles.remove(path);
        }
    }

    fn register(&mut self, path: &str, name: &str, created: i64, last_message: Option<i64>) {
        if path.is_empty() || name.is_empty() {
            return;
        }
        if let Some(existing) = self.entries.iter_mut().find(|e| e.path == path) {
            existing.name = name.to_owned();
            if created > 0 && (existing.created <= 0 || created < existing.created) {
                existing.created = created;
            }
            existing.last_message = existing.last_message.max(last_message);
        } else {
            self.entries.push(Entry {
                path: path.into(),
                name: name.into(),
                created,
                last_message,
            });
        }
    }
}

impl Workspace {
    pub(super) fn restore_project_list(&mut self, cx: &App) {
        // Migrate old workspaces once. Message ordering thereafter is updated by TurnStarted.
        if !self.project_list.entries.is_empty() {
            return;
        }
        for (offset, p) in self.recent_projects.iter().rev().enumerate() {
            // Legacy recents have relative order but no creation timestamp.
            let created = offset as i64 - self.recent_projects.len() as i64;
            self.project_list.register(&p.path, &p.name, created, None);
        }
        for t in &self.threads {
            let chat = t.chat.read(cx);
            if !chat.project_scoped {
                continue;
            }
            let mut sent = chat
                .messages
                .iter()
                .filter(|m| m.role == crate::views::chat::MessageRole::User)
                .filter_map(|m| m.started_at_ms);
            let first = sent.next();
            let last = sent.next_back().or(first);
            self.project_list.register(
                chat.settings_project_path(),
                &t.project_name,
                first.unwrap_or(0),
                last,
            );
        }
    }
    pub(super) fn register_project_message(&mut self, chat: &Entity<ChatView>, cx: &App) {
        let chat = chat.read(cx);
        if chat.project_scoped {
            let now = EpochMs::now().as_millis();
            self.project_list.register(
                chat.settings_project_path(),
                &chat.project_name,
                now,
                Some(now),
            );
        }
    }
    pub(super) fn remember_project(&mut self, project: DesktopProjectLocation, cx: &mut Context<'_, Self>) {
        self.project_list.register(
            &project.path,
            &project.name,
            EpochMs::now().as_millis(),
            None,
        );
        project_picker_ui::push_recent_project(&mut self.recent_projects, project);
        self.refresh_project_identities(cx);
    }
    pub(super) fn listed_projects(&self) -> Vec<DesktopProjectLocation> {
        let mut ordered: Vec<_> = self.project_list.entries.iter().collect();
        match self.project_list.mode {
            Order::LastMessage => {
                ordered.sort_by_key(|p| std::cmp::Reverse((p.last_message, p.created)))
            }
            Order::Created => ordered.sort_by_key(|p| std::cmp::Reverse(p.created)),
            Order::Manual => {}
        }
        let mut list: Vec<_> = ordered
            .into_iter()
            .map(|p| DesktopProjectLocation {
                path: p.path.clone(),
                name: self.project_list.title(&p.path).unwrap_or(&p.name).to_owned(),
            })
            .collect();
        for p in &self.recent_projects {
            if !list.iter().any(|e| e.path == p.path) {
                list.push(p.clone());
            }
        }
        list
    }
    pub(super) fn choose_project_order(&mut self, mode: &str, cx: &mut Context<'_, Self>) {
        let mode = match mode {
            "last-message" => Order::LastMessage,
            "created" => Order::Created,
            "manual" => Order::Manual,
            _ => return,
        };
        if mode == Order::Manual && self.project_list.mode != Order::Manual {
            let ranks: std::collections::HashMap<_, _> = self
                .listed_projects()
                .into_iter()
                .enumerate()
                .map(|(rank, p)| (p.path, rank))
                .collect();
            self.project_list.entries.sort_by_key(|e| ranks[&e.path]);
        }
        self.project_list.mode = mode;
        self.persist_and_notify(cx);
    }
    fn move_listed_project(&mut self, path: &str, direction: isize, cx: &mut Context<'_, Self>) {
        let list = &mut self.project_list;
        if list.mode != Order::Manual {
            return;
        }
        let Some(index) = list.entries.iter().position(|p| p.path == path) else {
            return;
        };
        let Some(to) = index
            .checked_add_signed(direction)
            .filter(|i| *i < list.entries.len())
        else {
            return;
        };
        list.entries.swap(index, to);
        self.persist_and_notify(cx);
    }
    pub(super) fn render_project_order_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let mut rows = vec![self.settings_row(
            "desktop-settings-row-project-order",
            "Project order",
            "Sort folders in the picker and palette.",
            settings_ui::control_column(self.choice_control(
                "settings-project-order",
                "Project order",
                match self.project_list.mode {
                    Order::LastMessage => "last-message",
                    Order::Created => "created",
                    Order::Manual => "manual",
                },
                &[
                    ("last-message", "Last user message"),
                    ("created", "Created at"),
                    ("manual", "Manual"),
                ],
                cx,
            )),
        )];
        if self.project_list.mode == Order::Manual {
            let count = self.project_list.entries.len();
            for (i, p) in self.project_list.entries.iter().enumerate() {
                let move_button = |label: &'static str, dir: isize, off: bool| {
                    let path = p.path.clone();
                    on_press_view(
                        web_button(
                            format!("project-order-{i}-{dir}"),
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::disabled_if(off),
                        )
                        .aria_label(format!("{label} {}", p.name))
                        .child(label),
                        cx,
                        move |this, _, cx| this.move_listed_project(&path, dir, cx),
                    )
                };
                rows.push(
                    self.settings_row(
                        "desktop-settings-row-project-position",
                        self.project_list.title(&p.path).unwrap_or(&p.name).to_owned(),
                        p.path.clone(),
                        div()
                            .flex()
                            .gap_1()
                            .child(move_button("Move up", -1, i == 0))
                            .child(move_button("Move down", 1, i + 1 == count))
                            .into_any_element(),
                    ),
                );
            }
        }
        rows
    }

    pub(super) fn open_palette_project(
        &mut self,
        path: String,
        name: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(id) = self
            .threads
            .iter()
            .filter(|t| {
                t.chat.read(cx).project_scoped && t.chat.read(cx).settings_project_path() == path
            })
            .max_by_key(|t| t.updated_at_ms)
            .map(|t| t.id.clone())
        {
            self.close_transient_surfaces();
            self.select_thread_by_id(&id, cx);
        } else {
            self.open_top_layer(TopLayer::ProjectPicker {
                for_terminal: false,
                thread: None,
            });
            let name = self.listed_projects().into_iter().find(|p| p.path == path).map(|p| p.name).unwrap_or(name);
            self.select_project_location(path, name, window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProjectList;
    #[gpui::test]
    fn project_order_preserves_distinct_checkouts_across_modes(cx: &mut gpui::TestAppContext) {
        let (workspace, cx, _, _) = super::super::tests::thread_menu_workspace(cx);
        workspace.update_in(cx, |this, _, cx| {
            this.recent_projects.clear();
            this.project_list = ProjectList::default();
            this.project_list.register("/a/foo", "foo", 10, Some(40));
            this.project_list.register("/b/foo", "foo", 30, Some(35));
            assert_eq!(this.listed_projects()[0].path, "/a/foo");
            this.choose_project_order("created", cx);
            assert_eq!(this.listed_projects()[0].path, "/b/foo");
            this.choose_project_order("manual", cx);
            this.move_listed_project("/a/foo", -1, cx);
            this.project_list.register("/b/foo", "foo", 50, Some(90));
            this.project_list = this.snapshot(cx).project_list;
            assert_eq!(this.listed_projects()[0].path, "/a/foo");
            assert_eq!(this.listed_projects().len(), 2);
        });
    }
}
