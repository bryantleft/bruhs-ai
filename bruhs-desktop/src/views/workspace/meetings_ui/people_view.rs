//! Web `PeoplePage`: create, search and paging, the profile card with
//! aliases, linked meetings and conversations, relationship details and
//! reminders, merge and delete confirmations, and the undo bar. Copy comes
//! from `bruhs_models::people_copy` so both surfaces match word for word.

use std::cell::{Cell, RefCell};

use gpui_component::scroll::ScrollableElement;
use gpui_component::text::TextView;

use super::*;
use bruhs_models::{
    normalize_aliases, people_copy as copy, people_page_count, people_page_of,
    person_conversations, person_matches_query, person_meetings, relink_person,
    restore_person_links, review_alias, unlink_person, PeopleUndo, PeopleUndoError,
    RelationshipDue, RelationshipItem, RelationshipItemEdit, RELATIONSHIP_ITEM_KINDS,
};

#[cfg(test)]
mod parity_tests;

/// One relationship item being edited in place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::views::workspace) struct ItemEdit {
    pub(in crate::views::workspace) person_id: String,
    pub(in crate::views::workspace) item_id: String,
    pub(in crate::views::workspace) kind: RelationshipItemKind,
    pub(in crate::views::workspace) due: bool,
}

/// People page state that is not a hub record. Never persisted: the undo
/// receipt, like the web's, lasts for this session.
pub(in crate::views::workspace) struct PeoplePageState {
    pub(in crate::views::workspace) create_input: Entity<InputState>,
    pub(in crate::views::workspace) search_input: Entity<InputState>,
    pub(in crate::views::workspace) alias_input: Entity<InputState>,
    pub(in crate::views::workspace) item_edit_input: Entity<InputState>,
    pub(in crate::views::workspace) create_error: Option<&'static str>,
    pub(in crate::views::workspace) alias_error: Option<String>,
    /// A failed or refused People change, shown until the next change.
    pub(in crate::views::workspace) notice: Option<String>,
    /// The directory page. A cell because render follows an expanded person
    /// opened from elsewhere onto its page.
    pub(in crate::views::workspace) page: Cell<usize>,
    seen_expanded: RefCell<Option<String>>,
    pub(in crate::views::workspace) due_tomorrow: bool,
    pub(in crate::views::workspace) editing_item: Option<ItemEdit>,
    /// Shown when an item edit is saved blank; the edit stays open.
    pub(in crate::views::workspace) item_error: Option<&'static str>,
    pub(in crate::views::workspace) confirm_delete: Option<String>,
    pub(in crate::views::workspace) undo: Option<PeopleUndo>,
    pub(in crate::views::workspace) undo_in_flight: Option<PeopleUndo>,
    /// The merge this client confirmed, so its local commit offers undo.
    pub(in crate::views::workspace) pending_merge: Option<(String, String)>,
    /// The kept merge originals the person asked to see, as JSON, with how
    /// many there were then. Hidden once they change, so an update never
    /// serializes private data unasked (web `ContactMergeStatus`).
    pub(in crate::views::workspace) merge_recovery: Option<(usize, String)>,
    /// The person a meeting or conversation was opened from, and that view.
    pub(in crate::views::workspace) return_to: Option<(String, AppView)>,
}

impl PeoplePageState {
    pub(in crate::views::workspace) fn new(
        window: &mut Window,
        cx: &mut Context<'_, Workspace>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Self {
        let create_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(copy::CREATE_PLACEHOLDER));
        subscriptions.push(cx.subscribe_in(
            &create_input,
            window,
            |this: &mut Workspace, _input, ev: &InputEvent, window, cx| match ev {
                InputEvent::PressEnter { .. } => this.create_person_from_page(window, cx),
                InputEvent::Change => {
                    this.people_page.create_error = None;
                    cx.notify();
                }
                _ => {}
            },
        ));
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(copy::SEARCH_PLACEHOLDER));
        subscriptions.push(cx.subscribe_in(
            &search_input,
            window,
            |this: &mut Workspace, _input, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.people_page.page.set(0);
                    cx.notify();
                }
            },
        ));
        let alias_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(copy::ALIAS_PLACEHOLDER));
        subscriptions.push(cx.subscribe_in(
            &alias_input,
            window,
            |this: &mut Workspace, _input, ev: &InputEvent, window, cx| match ev {
                InputEvent::PressEnter { .. } => {
                    if let Some(id) = this.expanded_person_id.clone() {
                        this.add_person_alias(&id, window, cx);
                    }
                }
                InputEvent::Change => {
                    this.people_page.alias_error = None;
                    cx.notify();
                }
                _ => {}
            },
        ));
        let item_edit_input = cx.new(|cx| InputState::new(window, cx));
        subscriptions.push(cx.subscribe_in(
            &item_edit_input,
            window,
            |this: &mut Workspace, _input, ev: &InputEvent, _window, cx| match ev {
                InputEvent::PressEnter { .. } => this.save_item_edit(cx),
                InputEvent::Change => {
                    this.people_page.item_error = None;
                    cx.notify();
                }
                _ => {}
            },
        ));
        Self {
            create_input,
            search_input,
            alias_input,
            item_edit_input,
            create_error: None,
            alias_error: None,
            notice: None,
            page: Cell::new(0),
            seen_expanded: RefCell::new(None),
            due_tomorrow: false,
            editing_item: None,
            item_error: None,
            confirm_delete: None,
            undo: None,
            undo_in_flight: None,
            pending_merge: None,
            merge_recovery: None,
            return_to: None,
        }
    }
}

/// Web's read-only "Contact merge recovery data" field: the JSON in a
/// scrolling, selectable code block, so it can be copied.
fn render_merge_recovery_data(json: &str, cx: &App) -> AnyElement {
    let longest = json.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    div()
        .id("desktop-people-merge-recovery-data")
        .debug_selector(|| "desktop-people-merge-recovery-data".into())
        .role(Role::Region)
        .aria_label("Contact merge recovery data")
        .h(px(128.0))
        .w_full()
        .overflow_y_scrollbar()
        .rounded(px(8.0))
        .bg(theme::longan_950())
        .p_2()
        .child(
            TextView::markdown(
                "desktop-people-merge-recovery-json",
                format!("{fence}json\n{json}\n{fence}"),
            )
            .style(crate::views::chat::desktop_markdown_style(cx))
            .selectable(true),
        )
        .into_any_element()
}

/// A people page item kind button: pressed state is visible and announced.
fn kind_button(
    id: impl Into<ElementId>,
    kind: RelationshipItemKind,
    pressed: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(kind.label())
        .aria_toggled(if pressed {
            Toggled::True
        } else {
            Toggled::False
        })
        .tab_index(0)
        .min_h(px(theme::APP_COMPACT_CONTROL_HEIGHT_PX))
        .flex()
        .items_center()
        .rounded(px(4.0))
        .px_2()
        .border_1()
        .border_color(gpui::transparent_black())
        .focus_visible(|el| el.border_color(theme::persimmon_400()))
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .text_size(px(LABEL_SM_PX))
        .cursor_pointer()
        .when(pressed, |el| {
            el.bg(theme::blueberry_500().opacity(0.15))
                .text_color(theme::blueberry_200())
        })
        .when(!pressed, |el| {
            el.text_color(theme::rambutan_500())
                .hover(|el| el.bg(theme::longan_800()))
        })
        .child(kind.label())
}

impl Workspace {
    // ── Actions ──────────────────────────────────────────────────────────

    pub(super) fn load_relationship_inputs(
        &mut self,
        person_id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let profile = self
            .relationships
            .iter()
            .find(|profile| profile.person_id == person_id)
            .cloned()
            .unwrap_or_else(|| RelationshipProfile::new(person_id, EpochMs::now().as_millis()));
        for (input, value) in [
            (&self.relationship_company_input, profile.company),
            (&self.relationship_role_input, profile.role),
            (&self.relationship_email_input, profile.email),
            (&self.relationship_phone_input, profile.phone),
            (&self.relationship_location_input, profile.location),
            (&self.relationship_notes_input, profile.notes),
        ] {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        for input in [
            &self.relationship_item_input,
            &self.people_page.alias_input,
            &self.people_page.item_edit_input,
        ] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.relationship_item_kind = RelationshipItemKind::Interaction;
        self.people_page.due_tomorrow = false;
        self.people_page.editing_item = None;
        self.people_page.alias_error = None;
    }

    fn relationship_profile_mut(&mut self, person_id: &str) -> &mut RelationshipProfile {
        if let Some(index) = self
            .relationships
            .iter()
            .position(|profile| profile.person_id == person_id)
        {
            &mut self.relationships[index]
        } else {
            self.relationships.push(RelationshipProfile::new(
                person_id,
                EpochMs::now().as_millis(),
            ));
            self.relationships
                .last_mut()
                .expect("relationship profile was inserted")
        }
    }

    fn save_relationship_details(&mut self, person_id: &str, cx: &mut Context<'_, Self>) {
        let read = |input: &Entity<InputState>, cx: &Context<'_, Self>| {
            input.read(cx).value().trim().to_string()
        };
        let company = read(&self.relationship_company_input, cx);
        let role = read(&self.relationship_role_input, cx);
        let email = read(&self.relationship_email_input, cx);
        let phone = read(&self.relationship_phone_input, cx);
        let location = read(&self.relationship_location_input, cx);
        let notes = read(&self.relationship_notes_input, cx);
        let profile = self.relationship_profile_mut(person_id);
        profile.company = company;
        profile.role = role;
        profile.email = email;
        profile.phone = phone;
        profile.location = location;
        profile.notes = notes;
        profile.updated_at_ms = EpochMs::now().as_millis();
        self.persist_and_notify(cx);
    }

    pub(in crate::views::workspace) fn add_relationship_item(
        &mut self,
        person_id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let text = self.relationship_item_input.read(cx).value().to_string();
        let now = EpochMs::now().as_millis();
        let due = if self.people_page.due_tomorrow {
            RelationshipDue::Tomorrow
        } else {
            RelationshipDue::None
        };
        let item = RelationshipItem::with_due(self.relationship_item_kind, text, due, now);
        if item.text.is_empty() {
            return;
        }
        self.relationship_profile_mut(person_id).add_item(item, now);
        self.relationship_item_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.people_page.due_tomorrow = false;
        self.persist_and_notify(cx);
    }

    fn toggle_relationship_item(
        &mut self,
        person_id: &str,
        item_id: &str,
        cx: &mut Context<'_, Self>,
    ) {
        self.relationship_profile_mut(person_id)
            .toggle_item(item_id, EpochMs::now().as_millis());
        self.persist_and_notify(cx);
    }

    pub(in crate::views::workspace) fn begin_item_edit(
        &mut self,
        person_id: &str,
        item_id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(item) = self
            .relationships
            .iter()
            .find(|profile| profile.person_id == person_id)
            .and_then(|profile| profile.items.iter().find(|item| item.id == item_id))
            .cloned()
        else {
            return;
        };
        self.people_page.item_error = None;
        self.people_page.editing_item = Some(ItemEdit {
            person_id: person_id.to_string(),
            item_id: item.id,
            kind: item.kind,
            due: item.due_at_ms.is_some(),
        });
        self.people_page.item_edit_input.update(cx, |input, cx| {
            input.set_value(item.text, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(in crate::views::workspace) fn save_item_edit(&mut self, cx: &mut Context<'_, Self>) {
        let Some(edit) = self.people_page.editing_item.clone() else {
            return;
        };
        let text = self
            .people_page
            .item_edit_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if text.is_empty() {
            self.people_page.item_error = Some(copy::ITEM_BLANK);
            cx.notify();
            return;
        }
        self.people_page.item_error = None;
        let due = if edit.due {
            RelationshipDue::Tomorrow
        } else {
            RelationshipDue::None
        };
        self.relationship_profile_mut(&edit.person_id).edit_item(
            &edit.item_id,
            RelationshipItemEdit {
                kind: edit.kind,
                text,
                due,
            },
            EpochMs::now().as_millis(),
        );
        self.people_page.editing_item = None;
        self.persist_and_notify(cx);
    }

    pub(in crate::views::workspace) fn delete_relationship_item(
        &mut self,
        person_id: &str,
        item_id: &str,
        cx: &mut Context<'_, Self>,
    ) {
        if self
            .people_page
            .editing_item
            .as_ref()
            .is_some_and(|edit| edit.item_id == item_id)
        {
            self.people_page.editing_item = None;
        }
        self.relationship_profile_mut(person_id)
            .remove_item(item_id, EpochMs::now().as_millis());
        self.persist_and_notify(cx);
    }

    /// PEOPLE-01: create a profile from the People page.
    pub(in crate::views::workspace) fn create_person_from_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let name = self
            .people_page
            .create_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if name.is_empty() {
            // The typed value stays so nothing is lost.
            self.people_page.create_error = Some(copy::CREATE_BLANK);
            cx.notify();
            return;
        }
        let now = EpochMs::now().as_millis();
        let mut person = PersonProfile::new(name);
        person.created_at = now;
        person.updated_at = now;
        self.meetings_hub
            .update(cx, |hub, cx| hub.capture_person(person, cx));
        self.people_page
            .create_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.people_page
            .search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.people_page.create_error = None;
        self.people_page.notice = None;
        self.people_page.page.set(0);
        self.persist_and_notify(cx);
    }

    /// PEOPLE-04: add a reviewed alias. A refusal keeps the typed alias.
    pub(in crate::views::workspace) fn add_person_alias(
        &mut self,
        person_id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let typed = self.people_page.alias_input.read(cx).value().to_string();
        let reviewed = {
            let hub = self.meetings_hub.read(cx);
            match review_alias(hub.people.iter().map(Arc::as_ref), person_id, &typed) {
                Err(problem) => Err(problem.message()),
                Ok(alias) => match hub.person(person_id) {
                    Some(person) => {
                        let mut aliases = person.aliases.clone();
                        aliases.push(alias);
                        Ok(normalize_aliases(&person.display_name, &aliases))
                    }
                    None => return,
                },
            }
        };
        match reviewed {
            Err(message) => {
                self.people_page.alias_error = Some(message);
                cx.notify();
            }
            Ok(aliases) => {
                let id = person_id.to_string();
                self.meetings_hub
                    .update(cx, |hub, cx| hub.set_person_aliases(id, aliases, cx));
                self.people_page
                    .alias_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.people_page.alias_error = None;
                cx.notify();
            }
        }
    }

    pub(in crate::views::workspace) fn remove_person_alias(
        &mut self,
        person_id: &str,
        alias: &str,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(aliases) = self.meetings_hub.read(cx).person(person_id).map(|person| {
            person
                .aliases
                .iter()
                .filter(|candidate| candidate.as_str() != alias)
                .cloned()
                .collect::<Vec<_>>()
        }) else {
            return;
        };
        let id = person_id.to_string();
        self.meetings_hub
            .update(cx, |hub, cx| hub.set_person_aliases(id, aliases, cx));
    }

    /// PEOPLE-06: open a meeting from the profile and remember the way back.
    pub(in crate::views::workspace) fn open_person_meeting(
        &mut self,
        person_id: &str,
        meeting_id: String,
        cx: &mut Context<'_, Self>,
    ) {
        self.people_page.return_to = Some((
            person_id.to_string(),
            AppView::MeetingDetail {
                meeting_id: meeting_id.clone(),
            },
        ));
        self.open_meeting_detail(meeting_id, cx);
    }

    pub(in crate::views::workspace) fn open_person_conversation(
        &mut self,
        person_id: &str,
        conversation_id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self
            .inbox_conversations
            .iter()
            .any(|conversation| conversation.id == conversation_id)
        {
            return;
        }
        self.people_page.return_to = Some((
            person_id.to_string(),
            AppView::ConversationDetail {
                conversation_id: conversation_id.to_string(),
            },
        ));
        self.select_inbox_conversation(conversation_id, window, cx);
    }

    /// Open People on one profile, as the web's `SelectedPersonSignal`
    /// does: the profile opens on its page, and a search that hides it is
    /// cleared. A profile that is gone leaves People as it was.
    pub(in crate::views::workspace) fn open_person_profile(
        &mut self,
        person_id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let hidden_by_search = self.meetings_hub.read(cx).person(person_id).map(|person| {
            let query = self.people_page.search_input.read(cx).value().to_string();
            !person_matches_query(person, &query)
        });
        if let Some(hidden_by_search) = hidden_by_search {
            if hidden_by_search {
                self.people_page
                    .search_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
            if self.expanded_person_id.as_deref() != Some(person_id) {
                self.load_relationship_inputs(person_id, window, cx);
            }
            self.expanded_person_id = Some(person_id.to_string());
            // Render moves the directory to the person's page.
            *self.people_page.seen_expanded.borrow_mut() = None;
        }
        self.activate_sidebar_view(AppView::People, window, cx);
    }

    /// Return to People with the person the detail was opened from.
    pub(in crate::views::workspace) fn back_to_people(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some((person_id, _)) = self.people_page.return_to.take() else {
            return;
        };
        if self.meetings_hub.read(cx).person(&person_id).is_some() {
            if self.expanded_person_id.as_deref() != Some(person_id.as_str()) {
                self.load_relationship_inputs(&person_id, window, cx);
            }
            self.expanded_person_id = Some(person_id);
            // Render moves the directory to the person's page.
            *self.people_page.seen_expanded.borrow_mut() = None;
        }
        self.active_view = AppView::People;
        self.close_transient_surfaces();
        self.persist_and_notify(cx);
    }

    /// PEOPLE-10: delete after the confirmation was accepted.
    pub(in crate::views::workspace) fn confirm_person_delete(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(person_id) = self.people_page.confirm_delete.take() else {
            return;
        };
        self.people_page.notice = None;
        self.meetings_hub
            .update(cx, |hub, cx| hub.delete_person(person_id, cx));
        cx.notify();
    }

    /// The hub deleted a contact: remove its reminders and unlink its
    /// private links, then offer undo. Capture receipts are left untouched.
    pub(in crate::views::workspace) fn finish_person_delete(
        &mut self,
        person: PersonProfile,
        local_only: bool,
        cx: &mut Context<'_, Self>,
    ) {
        let id = person.id.clone();
        let relationship = self
            .relationships
            .iter()
            .position(|profile| profile.person_id == id)
            .map(|index| self.relationships.remove(index));
        self.relationships.retain(|profile| profile.person_id != id);
        let links = unlink_person(
            &id,
            &mut self.inbox_conversations,
            &mut self.knowledge_documents,
        );
        for slot in [&mut self.expanded_person_id, &mut self.renaming_person_id] {
            if slot.as_deref() == Some(id.as_str()) {
                *slot = None;
            }
        }
        if self.merge.as_ref().is_some_and(|merge| {
            merge.person_id == id || merge.target_id.as_deref() == Some(id.as_str())
        }) {
            self.merge = None;
        }
        if self
            .people_page
            .editing_item
            .as_ref()
            .is_some_and(|edit| edit.person_id == id)
        {
            self.people_page.editing_item = None;
        }
        self.people_page.undo = Some(PeopleUndo::Delete {
            person,
            relationship,
            links,
            local_only,
        });
        self.persist_and_notify(cx);
    }

    /// PEOPLE-09: undo the latest People change if its records are unchanged.
    pub(in crate::views::workspace) fn run_people_undo(&mut self, cx: &mut Context<'_, Self>) {
        let Some(undo) = self.people_page.undo.take() else {
            return;
        };
        self.people_page.notice = None;
        let checked = {
            let hub = self.meetings_hub.read(cx);
            if hub.person_merge_in_flight || self.people_page.undo_in_flight.is_some() {
                Err(PeopleUndoError::Busy)
            } else {
                undo.check(
                    |id| hub.person(id),
                    |id| {
                        self.relationships
                            .iter()
                            .find(|profile| profile.person_id == id)
                    },
                )
            }
        };
        if let Err(error) = checked {
            if error == PeopleUndoError::Busy {
                self.people_page.undo = Some(undo);
            }
            self.people_page.notice = Some(error.message().to_string());
            cx.notify();
            return;
        }
        match undo.clone() {
            PeopleUndo::Rename {
                person_id, before, ..
            } => self.meetings_hub.update(cx, |hub, cx| {
                hub.rename_person_with(person_id, before, false, cx)
            }),
            PeopleUndo::Tracked {
                person_id, before, ..
            } => self.meetings_hub.update(cx, |hub, cx| {
                hub.set_person_tracked_with(person_id, before, false, cx)
            }),
            PeopleUndo::Merge { source_id, .. } => {
                self.people_page.undo_in_flight = Some(undo);
                self.meetings_hub
                    .update(cx, |hub, cx| hub.unmerge_person(source_id, cx));
            }
            PeopleUndo::Delete {
                person, local_only, ..
            } => {
                self.people_page.undo_in_flight = Some(undo);
                self.meetings_hub
                    .update(cx, |hub, cx| hub.restore_person(person, local_only, cx));
            }
        }
        cx.notify();
    }

    /// The contact is back: restore its reminders and relink its links.
    pub(in crate::views::workspace) fn finish_person_restore(
        &mut self,
        person_id: &str,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(PeopleUndo::Delete {
            person,
            relationship,
            links,
            ..
        }) = self.people_page.undo_in_flight.take()
        else {
            return;
        };
        if person.id != person_id {
            return;
        }
        if let Some(relationship) = relationship {
            if !self
                .relationships
                .iter()
                .any(|profile| profile.person_id == person_id)
            {
                self.relationships.push(relationship);
            }
        }
        relink_person(
            person_id,
            &links,
            &mut self.inbox_conversations,
            &mut self.knowledge_documents,
        );
        self.persist_and_notify(cx);
    }

    /// Both merged contacts are back: restore both relationships as they
    /// were before the merge and move the merged links back to the source.
    pub(in crate::views::workspace) fn finish_person_unmerge(
        &mut self,
        source_id: &str,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(PeopleUndo::Merge {
            source_id: undo_source,
            target_id,
            before_source,
            before_target,
            links,
            ..
        }) = self.people_page.undo_in_flight.take()
        else {
            return;
        };
        if undo_source != source_id {
            return;
        }
        self.relationships
            .retain(|profile| profile.person_id != source_id && profile.person_id != target_id);
        self.relationships
            .extend(before_source.into_iter().chain(before_target));
        restore_person_links(
            source_id,
            &target_id,
            &links,
            &mut self.inbox_conversations,
            &mut self.knowledge_documents,
        );
        self.persist_and_notify(cx);
    }

    pub(in crate::views::workspace) fn people_undo_failed(
        &mut self,
        message: String,
        cx: &mut Context<'_, Self>,
    ) {
        // A failed server step changed nothing, so the receipt stays usable.
        if let Some(undo) = self.people_page.undo_in_flight.take() {
            self.people_page.undo = Some(undo);
        }
        self.people_page.notice = Some(message);
        cx.notify();
    }

    fn begin_person_rename(
        &mut self,
        person_id: String,
        current: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self
            .merge
            .as_ref()
            .is_some_and(|merge| merge.person_id == person_id)
        {
            self.merge = None;
        }
        self.renaming_person_id = Some(person_id);
        self.person_rename_input.update(cx, |input, cx| {
            input.set_value(current, window, cx);
        });
        self.person_rename_input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        cx.notify();
    }

    pub(in crate::views::workspace) fn commit_person_rename(&mut self, cx: &mut Context<'_, Self>) {
        let Some(person_id) = self.renaming_person_id.take() else {
            return;
        };
        let name = self.person_rename_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.people_page.notice = Some(copy::RENAME_BLANK.to_string());
        } else {
            self.people_page.notice = None;
            let unchanged = self
                .meetings_hub
                .read(cx)
                .person(&person_id)
                .is_some_and(|person| person.display_name == name);
            if !unchanged {
                self.meetings_hub
                    .update(cx, |hub, cx| hub.rename_person(person_id, name, cx));
            }
        }
        cx.notify();
    }

    pub(in crate::views::workspace) fn cancel_person_rename(&mut self, cx: &mut Context<'_, Self>) {
        self.renaming_person_id = None;
        cx.notify();
    }

    // ── Render ───────────────────────────────────────────────────────────

    /// The people matching the search, newest change first.
    fn people_matching_search(&self, cx: &Context<'_, Self>) -> Vec<Arc<PersonProfile>> {
        let query = self.people_page.search_input.read(cx).value().to_string();
        let mut people = self
            .meetings_hub
            .read(cx)
            .people
            .iter()
            .filter(|person| person_matches_query(person, &query))
            .cloned()
            .collect::<Vec<_>>();
        people.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        people
    }

    /// The page to show: follows a person expanded from elsewhere, and is
    /// clamped when the result shrinks.
    fn people_page_index(&self, people: &[Arc<PersonProfile>]) -> usize {
        let expanded = self.expanded_person_id.clone();
        if *self.people_page.seen_expanded.borrow() != expanded {
            if let Some(index) = expanded
                .as_deref()
                .and_then(|id| people.iter().position(|person| person.id == id))
            {
                self.people_page.page.set(people_page_of(index));
            }
            *self.people_page.seen_expanded.borrow_mut() = expanded;
        }
        let page = self
            .people_page
            .page
            .get()
            .min(people_page_count(people.len()).saturating_sub(1));
        self.people_page.page.set(page);
        page
    }

    /// Web `PeoplePage`.
    pub(in crate::views::workspace) fn render_people_page(
        &self,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let people = self.people_matching_search(cx);
        let page = self.people_page_index(&people);
        let page_count = people_page_count(people.len());
        let visible = bruhs_models::people_page(&people, page).to_vec();
        let hub = self.meetings_hub.read(cx);
        let directory_empty = hub.people.is_empty();
        let settled = hub.people_settled;
        let enroll = hub.enroll.clone();
        let mut page_el = div()
            .id("desktop-people-page")
            .debug_selector(|| "desktop-people-page".to_string())
            .relative()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .bg(theme::longan_950())
            .p_4()
            .flex()
            .flex_col()
            .gap_3()
            .child(label("People", H4_PX, theme::rambutan_100()).font_weight(FontWeight::MEDIUM));
        if let Some(undo) = self.render_people_undo(cx) {
            page_el = page_el.child(undo);
        }
        page_el = page_el.child(
            div()
                .flex()
                .flex_wrap()
                .items_start()
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(240.0))
                        .child(self.render_people_create(cx)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(240.0))
                        .child(self.render_people_search(cx)),
                ),
        );
        if let Some(notice) = &self.people_page.notice {
            page_el = page_el.child(
                div()
                    .id("desktop-people-notice")
                    .debug_selector(|| "desktop-people-notice".into())
                    .role(Role::Alert)
                    .aria_label("Profile change failed")
                    .child(body(notice.clone(), BODY_SM_PX, theme::lychee_300())),
            );
        }
        if let Some(recovery) = self.render_people_merge_status(cx) {
            page_el = page_el.child(recovery);
        }
        if hub.people_refreshing() {
            page_el = page_el.child(
                div()
                    .id("desktop-people-refreshing")
                    .debug_selector(|| "desktop-people-refreshing".into())
                    .role(Role::Status)
                    .aria_label("Contact directory sync")
                    .child(label(
                        "Refreshing contacts… Cached contacts remain available.",
                        LABEL_SM_PX,
                        theme::rambutan_400(),
                    )),
            );
        }
        if hub.needs_people_retry() {
            page_el = page_el.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(label(
                        if hub.pending_people_count() == 0 {
                            "Contact refresh needs attention. Local records are retained.".into()
                        } else {
                            format!(
                                "{} contact(s) awaiting sync or merge recovery. Check local save status.",
                                hub.pending_people_count()
                            )
                        },
                        LABEL_PX,
                        theme::rambutan_400(),
                    ))
                    .child(
                        web_button(
                            "retry-contact-sync",
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .aria_label("Retry contact sync")
                        .debug_selector(|| "desktop-retry-contact-sync".into())
                        .child("Retry contact sync")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.meetings_hub
                                .update(cx, |hub, cx| hub.retry_pending_people(cx));
                        })),
                    ),
            );
        }
        if directory_empty {
            let (text, selector) = if settled {
                (copy::EMPTY, "desktop-people-empty")
            } else {
                (copy::LOADING, "desktop-people-loading")
            };
            page_el = page_el.child(
                div()
                    .id(selector)
                    .debug_selector(move || selector.to_string())
                    .role(Role::Status)
                    .child(label(text, LABEL_PX, theme::rambutan_500())),
            );
        } else if visible.is_empty() {
            page_el = page_el.child(
                div()
                    .id("desktop-people-search-empty")
                    .debug_selector(|| "desktop-people-search-empty".into())
                    .role(Role::Status)
                    .child(label(copy::SEARCH_EMPTY, LABEL_PX, theme::rambutan_500())),
            );
        } else {
            let all = hub.people.clone();
            page_el = page_el.child(
                div().flex().flex_col().gap_2().children(
                    visible
                        .into_iter()
                        .enumerate()
                        .map(|(i, p)| self.render_person_card(i, p, &all, cx)),
                ),
            );
        }
        if page_count > 1 {
            page_el = page_el.child(self.render_people_pager(page, page_count, cx));
        }
        if let Some((person_id, state)) = enroll {
            let name = hub
                .people
                .iter()
                .find(|p| p.id == person_id)
                .map_or_else(String::new, |p| p.display_name.clone());
            page_el = page_el.child(self.render_enroll_modal(name, state, cx));
        }
        page_el.into_any_element()
    }

    fn render_people_create(&self, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(label(
                copy::CREATE_LABEL,
                LABEL_SM_PX,
                theme::rambutan_500(),
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .debug_selector(|| "desktop-people-create-input".into())
                            .flex_1()
                            .min_w(px(160.0))
                            .child(
                                Input::new(&self.people_page.create_input)
                                    .aria_label(copy::CREATE_LABEL),
                            ),
                    )
                    .child(
                        web_button(
                            "desktop-people-create",
                            WebButtonVariant::Primary,
                            WebButtonSize::Default,
                            ButtonState::Enabled,
                        )
                        .debug_selector(|| "desktop-people-create".into())
                        .aria_label(copy::CREATE_BUTTON)
                        .child(copy::CREATE_BUTTON)
                        .on_click(cx.listener(|this, _ev, window, cx| {
                            this.create_person_from_page(window, cx)
                        })),
                    ),
            )
            .when_some(self.people_page.create_error, |el, message| {
                el.child(
                    div()
                        .id("desktop-people-create-error")
                        .debug_selector(|| "desktop-people-create-error".into())
                        .role(Role::Alert)
                        .child(body(message, BODY_SM_PX, theme::lychee_300())),
                )
            })
            .into_any_element()
    }

    fn render_people_search(&self, _cx: &Context<'_, Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(label(
                copy::SEARCH_LABEL,
                LABEL_SM_PX,
                theme::rambutan_500(),
            ))
            .child(
                div()
                    .debug_selector(|| "desktop-people-search".into())
                    .w_full()
                    .child(
                        Input::new(&self.people_page.search_input).aria_label(copy::SEARCH_LABEL),
                    ),
            )
            .into_any_element()
    }

    fn render_people_pager(&self, page: usize, count: usize, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .id("desktop-people-pager")
            .debug_selector(|| "desktop-people-pager".into())
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .py_2()
            .child(
                web_button(
                    "desktop-people-previous",
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(page == 0),
                )
                .debug_selector(|| "desktop-people-previous".into())
                .aria_label(copy::PREVIOUS_PAGE)
                .child(copy::PREVIOUS_PAGE)
                .when(page > 0, |el| {
                    el.on_click(cx.listener(|this, _ev, _window, cx| {
                        let page = this.people_page.page.get();
                        this.people_page.page.set(page.saturating_sub(1));
                        cx.notify();
                    }))
                }),
            )
            .child(
                div()
                    .id("desktop-people-page-status")
                    .debug_selector(|| "desktop-people-page-status".into())
                    .role(Role::Status)
                    .child(label(
                        copy::page_status(page, count),
                        LABEL_SM_PX,
                        theme::rambutan_400(),
                    )),
            )
            .child(
                web_button(
                    "desktop-people-next",
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(page + 1 >= count),
                )
                .debug_selector(|| "desktop-people-next".into())
                .aria_label(copy::NEXT_PAGE)
                .child(copy::NEXT_PAGE)
                .when(page + 1 < count, |el| {
                    el.on_click(cx.listener(|this, _ev, _window, cx| {
                        let page = this.people_page.page.get();
                        this.people_page.page.set(page.saturating_add(1));
                        cx.notify();
                    }))
                }),
            )
            .into_any_element()
    }

    fn render_people_undo(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        if self.people_page.undo_in_flight.is_some() {
            return Some(
                div()
                    .id("desktop-people-undoing")
                    .role(Role::Status)
                    .child(label("Undoing…", LABEL_SM_PX, theme::rambutan_400()))
                    .into_any_element(),
            );
        }
        let undo = self.people_page.undo.as_ref()?;
        Some(
            div()
                .id("desktop-people-undo-bar")
                .debug_selector(|| "desktop-people-undo-bar".into())
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .rounded(px(8.0))
                .bg(theme::longan_900())
                .border_1()
                .border_color(theme::longan_800())
                .p_2()
                .child(
                    div()
                        .id("desktop-people-undo-summary")
                        .role(Role::Status)
                        .flex_1()
                        .min_w_0()
                        .child(label(undo.summary(), LABEL_PX, theme::rambutan_300())),
                )
                .child(
                    web_button(
                        "desktop-people-undo",
                        WebButtonVariant::Outline,
                        WebButtonSize::Small,
                        ButtonState::Enabled,
                    )
                    .debug_selector(|| "desktop-people-undo".into())
                    .aria_label(format!("{} {}", undo.action_label(), undo.summary()))
                    .child(undo.action_label())
                    .on_click(cx.listener(|this, _ev, _window, cx| this.run_people_undo(cx))),
                )
                .child(
                    web_button(
                        "desktop-people-undo-dismiss",
                        WebButtonVariant::Ghost,
                        WebButtonSize::Small,
                        ButtonState::Enabled,
                    )
                    .debug_selector(|| "desktop-people-undo-dismiss".into())
                    .aria_label("Dismiss undo")
                    .child(copy::UNDO_DISMISS)
                    .on_click(cx.listener(|this, _ev, _window, cx| {
                        this.people_page.undo = None;
                        cx.notify();
                    })),
                )
                .into_any_element(),
        )
    }

    /// Web `ContactMergeStatus`, from what the desktop keeps: a merge on its
    /// way to the server, server-confirmed merges waiting for local recovery,
    /// and originals kept after a conflicting recovery.
    fn render_people_merge_status(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let hub = self.meetings_hub.read(cx);
        let (waiting, kept) = hub.person_merge_recovery_counts();
        if !hub.person_merge_in_flight && waiting == 0 && kept == 0 {
            return None;
        }
        let mut section = div()
            .id("desktop-people-merge-status")
            .debug_selector(|| "desktop-people-merge-status".into())
            .aria_label("Contact merge recovery")
            .flex()
            .flex_col()
            .gap_2()
            .rounded(px(8.0))
            .bg(theme::longan_900())
            .border_1()
            .border_color(theme::longan_800())
            .p_3();
        if hub.person_merge_in_flight {
            section = section.child(
                div().id("desktop-people-merging").role(Role::Status).child(label(
                    "Merge request sent; awaiting server confirmation. Both originals are retained.",
                    LABEL_SM_PX,
                    theme::rambutan_400(),
                )),
            );
        }
        if waiting > 0 {
            section = section
                .child(
                    div()
                        .id("desktop-people-merge-waiting")
                        .role(Role::Status)
                        .child(label(
                            format!(
                            "Server confirmed; local merge still pending for {waiting} contact(s)."
                        ),
                            LABEL_SM_PX,
                            theme::rambutan_400(),
                        )),
                )
                .child(
                    web_button(
                        "desktop-people-retry-merge",
                        WebButtonVariant::Outline,
                        WebButtonSize::Small,
                        ButtonState::Enabled,
                    )
                    .aria_label("Retry contact merge")
                    .child("Retry contact merge")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.meetings_hub
                            .update(cx, |hub, cx| hub.retry_pending_people(cx));
                    })),
                );
        }
        if kept > 0 {
            section = section
                .child(label(
                    "Both originals are retained privately on this device. Recovery data may contain sensitive notes and voice information.",
                    LABEL_SM_PX,
                    theme::rambutan_400(),
                ))
                .child(
                    div().flex().child(
                        web_button(
                            "desktop-people-view-merge-recovery",
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .debug_selector(|| "desktop-people-view-merge-recovery".into())
                        .aria_label("View contact merge recovery (private)")
                        .child("View contact merge recovery (private)")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.show_people_merge_recovery(kept, cx);
                        })),
                    ),
                );
            if let Some((_, json)) = self
                .people_page
                .merge_recovery
                .as_ref()
                .filter(|(shown, _)| *shown == kept)
            {
                section = section.child(render_merge_recovery_data(json, cx));
            }
        }
        Some(section.into_any_element())
    }

    /// Web "View contact merge recovery (private)": show the kept originals.
    fn show_people_merge_recovery(&mut self, kept: usize, cx: &mut Context<'_, Self>) {
        match self.meetings_hub.read(cx).person_merge_recovery_json() {
            Ok(json) => self.people_page.merge_recovery = Some((kept, json)),
            Err(_) => {
                self.people_page.notice =
                    Some("Private merge recovery data could not be encoded.".into());
            }
        }
        cx.notify();
    }

    fn render_person_card(
        &self,
        index: usize,
        profile: Arc<PersonProfile>,
        all: &[Arc<PersonProfile>],
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let id = profile.id.clone();
        let expanded = self.expanded_person_id.as_deref() == Some(id.as_str());
        let renaming = self.renaming_person_id.as_deref() == Some(id.as_str());
        let merge_open = self
            .merge
            .as_ref()
            .is_some_and(|merge| merge.person_id == id);
        let confirming_delete = self.people_page.confirm_delete.as_deref() == Some(id.as_str());
        let tracked = profile.tracked;
        let obs_count = profile.observations.len();
        let display_name = profile.display_name.clone();
        let expand_label = format!(
            "{} {display_name}",
            if expanded { "Collapse" } else { "Expand" }
        );

        let id_toggle = id.clone();
        let (id_rename, name_rename) = (id.clone(), display_name.clone());
        let id_track = id.clone();
        let id_merge = id.clone();
        let id_enroll = id.clone();
        let id_delete = id.clone();
        let name_selector = format!("desktop-person-name-{id}");

        let name_slot: AnyElement = if renaming {
            div()
                .flex_1()
                .min_w_0()
                .rounded(px(4.0))
                .bg(theme::longan_800())
                .border_1()
                .border_color(theme::longan_600())
                .px_1()
                .py(px(2.0))
                .capture_action(cx.listener(|this, _: &InputEscape, _window, cx| {
                    this.cancel_person_rename(cx);
                    cx.stop_propagation();
                }))
                .child(
                    Input::new(&self.person_rename_input)
                        .appearance(false)
                        .aria_label("Profile name"),
                )
                .into_any_element()
        } else {
            // Long and Unicode names truncate inside the card; the full name
            // stays in the accessible label.
            body(display_name.clone(), BODY_SM_PX, theme::rambutan_200())
                .debug_selector(move || name_selector)
                .flex_1()
                .min_w_0()
                .truncate()
                .into_any_element()
        };

        let mut card = div()
            .id(("desktop-person-card", index))
            .debug_selector(move || format!("desktop-person-card-{index}"))
            .rounded(px(4.0))
            .bg(theme::longan_900().opacity(0.5))
            .border_1()
            .border_color(theme::longan_700())
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .when(!tracked, |el| el.opacity(0.6))
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_x_3()
                    .gap_y_1()
                    .child(
                        div()
                            .id(("desktop-person-expand", index))
                            .debug_selector(move || format!("desktop-person-expand-{index}"))
                            .flex_1()
                            .min_w(px(160.0))
                            .flex()
                            .items_center()
                            .gap_2()
                            // Web renders this expander as a <button>.
                            .role(Role::Button)
                            .aria_label(expand_label)
                            .aria_expanded(expanded)
                            .tab_index(0)
                            .border_1()
                            .border_color(gpui::transparent_black())
                            .focus_visible(|el| el.border_color(theme::persimmon_400()))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _ev, window, cx| {
                                if this.renaming_person_id.is_some() {
                                    return;
                                }
                                let opening =
                                    this.expanded_person_id.as_deref() != Some(id_toggle.as_str());
                                this.expanded_person_id = if !opening {
                                    None
                                } else {
                                    Some(id_toggle.clone())
                                };
                                if opening {
                                    this.load_relationship_inputs(&id_toggle, window, cx);
                                }
                                *this.people_page.seen_expanded.borrow_mut() =
                                    this.expanded_person_id.clone();
                                cx.notify();
                            }))
                            .child(name_slot)
                            .when(!tracked, |el| {
                                el.child(
                                    div()
                                        .flex_none()
                                        .rounded(px(4.0))
                                        .bg(theme::longan_800())
                                        .border_1()
                                        .border_color(theme::longan_600())
                                        .px_1()
                                        .py(px(2.0))
                                        .font_family(LABEL_FONT)
                                        .text_size(px(LABEL_SM_PX))
                                        .text_color(theme::rambutan_300())
                                        .child("NOT TRACKED"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .child(label(
                                format!("{obs_count} obs"),
                                LABEL_SM_PX,
                                theme::rambutan_500(),
                            ))
                            .child(
                                text_button(
                                    ("desktop-person-rename", index),
                                    "rename",
                                    theme::rambutan_500(),
                                    theme::rambutan_300(),
                                )
                                .aria_label(format!("Rename {}", profile.display_name))
                                .on_click(cx.listener(
                                    move |this, _ev, window, cx| {
                                        this.begin_person_rename(
                                            id_rename.clone(),
                                            name_rename.clone(),
                                            window,
                                            cx,
                                        );
                                    },
                                )),
                            )
                            .child(
                                text_button(
                                    ("desktop-person-track", index),
                                    if tracked { "untrack" } else { "track" },
                                    if tracked {
                                        theme::pandan_400()
                                    } else {
                                        theme::rambutan_500()
                                    },
                                    if tracked {
                                        theme::pandan_100()
                                    } else {
                                        theme::rambutan_400()
                                    },
                                )
                                .aria_label(format!(
                                    "{} {}",
                                    if tracked { "Untrack" } else { "Track" },
                                    profile.display_name
                                ))
                                .on_click(cx.listener(
                                    move |this, _ev, _window, cx| {
                                        let pid = id_track.clone();
                                        this.people_page.notice = None;
                                        this.meetings_hub.update(cx, |hub, cx| {
                                            hub.set_person_tracked(pid, !tracked, cx)
                                        });
                                    },
                                )),
                            )
                            .child(
                                text_button(
                                    ("desktop-person-merge", index),
                                    "⋯",
                                    theme::rambutan_500(),
                                    theme::rambutan_300(),
                                )
                                .aria_label(format!(
                                    "Merge {} into another profile",
                                    profile.display_name
                                ))
                                .aria_expanded(merge_open)
                                .on_click(cx.listener(
                                    move |this, _ev, _window, cx| {
                                        this.merge = if this
                                            .merge
                                            .as_ref()
                                            .is_some_and(|merge| merge.person_id == id_merge)
                                        {
                                            None
                                        } else {
                                            Some(PersonMergePanel {
                                                person_id: id_merge.clone(),
                                                target_id: None,
                                                dropdown_open: false,
                                                confirming: false,
                                            })
                                        };
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                text_button(
                                    ("desktop-person-enroll", index),
                                    "enroll",
                                    theme::rambutan_500(),
                                    theme::pandan_300(),
                                )
                                .aria_label(format!("Enroll voice for {}", profile.display_name))
                                .on_click(cx.listener(
                                    move |this, _ev, _window, cx| {
                                        let pid = id_enroll.clone();
                                        this.meetings_hub
                                            .update(cx, |hub, cx| hub.open_enroll(pid, cx));
                                    },
                                )),
                            )
                            .child(
                                text_button(
                                    ("desktop-person-delete", index),
                                    "delete",
                                    theme::rambutan_500(),
                                    theme::lychee_300(),
                                )
                                .debug_selector(move || format!("desktop-person-delete-{index}"))
                                .aria_label(format!("Delete {}", profile.display_name))
                                .aria_expanded(confirming_delete)
                                .on_click(cx.listener(
                                    move |this, _ev, _window, cx| {
                                        this.people_page.confirm_delete =
                                            if this.people_page.confirm_delete.as_deref()
                                                == Some(id_delete.as_str())
                                            {
                                                None
                                            } else {
                                                Some(id_delete.clone())
                                            };
                                        cx.notify();
                                    },
                                )),
                            ),
                    ),
            );
        if confirming_delete {
            card = card.child(self.render_delete_confirm(&display_name, cx));
        }
        if merge_open {
            card = card.child(self.render_merge_panel(&id, all, cx));
        }
        if expanded {
            if !tracked {
                card = card.child(
                    label(
                        "Observation tracking is off. Summaries add no observations about this person, and agents cannot read the saved ones.",
                        LABEL_SM_PX,
                        theme::rambutan_400(),
                    )
                    .debug_selector(|| "desktop-person-opt-out".into()),
                );
            }
            card = card
                .child(self.render_person_aliases(&profile, cx))
                .child(self.render_person_links(&profile, cx))
                .child(self.render_relationship_panel(&id, cx))
                .child(self.render_observations_by_kind(index, &profile.observations, cx));
        }
        card.into_any_element()
    }

    fn render_delete_confirm(&self, name: &str, cx: &Context<'_, Self>) -> AnyElement {
        div()
            .id("desktop-person-delete-confirm")
            .debug_selector(|| "desktop-person-delete-confirm".into())
            .role(Role::Group)
            .aria_label("Confirm delete")
            .mt_1()
            .p_2()
            .rounded(px(4.0))
            .bg(theme::longan_800().opacity(0.8))
            .border_1()
            .border_color(theme::longan_700())
            .flex()
            .flex_col()
            .gap_2()
            .child(body(
                copy::delete_confirm(name),
                BODY_SM_PX,
                theme::rambutan_200(),
            ))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        web_button(
                            "desktop-person-delete-cancel",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .aria_label(copy::CANCEL)
                        .child(copy::CANCEL)
                        .on_click(cx.listener(|this, _ev, _window, cx| {
                            this.people_page.confirm_delete = None;
                            cx.notify();
                        })),
                    )
                    .child(
                        web_button(
                            "desktop-person-delete-commit",
                            WebButtonVariant::Destructive,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .debug_selector(|| "desktop-person-delete-commit".into())
                        .aria_label(format!("{} {name}", copy::DELETE_BUTTON))
                        .child(copy::DELETE_BUTTON)
                        .on_click(
                            cx.listener(|this, _ev, _window, cx| this.confirm_person_delete(cx)),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn render_person_aliases(&self, profile: &PersonProfile, cx: &Context<'_, Self>) -> AnyElement {
        let person_id = profile.id.clone();
        let aliases = if profile.aliases.is_empty() {
            label(copy::ALIASES_EMPTY, LABEL_SM_PX, theme::rambutan_500()).into_any_element()
        } else {
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .children(profile.aliases.iter().map(|alias| {
                    let remove_id = person_id.clone();
                    let remove_alias = alias.clone();
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .rounded(px(4.0))
                        .bg(theme::longan_800())
                        .px_2()
                        .child(label(alias.clone(), LABEL_SM_PX, theme::rambutan_200()))
                        .child(
                            text_button(
                                ("desktop-person-alias-remove", element_key(alias)),
                                "remove",
                                theme::rambutan_500(),
                                theme::lychee_300(),
                            )
                            .aria_label(format!("Remove alias {alias}"))
                            .on_click(cx.listener(
                                move |this, _ev, _window, cx| {
                                    this.remove_person_alias(&remove_id, &remove_alias, cx)
                                },
                            )),
                        )
                }))
                .into_any_element()
        };
        let add_id = person_id.clone();
        div()
            .id("desktop-person-aliases")
            .debug_selector(|| "desktop-person-aliases".into())
            .role(Role::Group)
            .aria_label(copy::ALIASES_HEADING)
            .mt_2()
            .pt_3()
            .border_t_1()
            .border_color(theme::longan_700())
            .flex()
            .flex_col()
            .gap_2()
            .child(label(
                copy::ALIASES_HEADING,
                LABEL_SM_PX,
                theme::rambutan_500(),
            ))
            .child(aliases)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div().flex_1().min_w(px(160.0)).child(
                            Input::new(&self.people_page.alias_input)
                                .aria_label(copy::ALIAS_INPUT_LABEL),
                        ),
                    )
                    .child(
                        web_button(
                            ("desktop-person-alias-add", element_key(&person_id)),
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .aria_label(copy::ALIAS_BUTTON)
                        .child(copy::ALIAS_BUTTON)
                        .on_click(cx.listener(
                            move |this, _ev, window, cx| this.add_person_alias(&add_id, window, cx),
                        )),
                    ),
            )
            .when_some(self.people_page.alias_error.clone(), |el, message| {
                el.child(
                    div()
                        .id("desktop-person-alias-error")
                        .debug_selector(|| "desktop-person-alias-error".into())
                        .role(Role::Alert)
                        .child(body(message, BODY_SM_PX, theme::lychee_300())),
                )
            })
            .into_any_element()
    }

    /// PEOPLE-06: the person's meetings and Inbox conversations.
    fn render_person_links(&self, profile: &PersonProfile, cx: &Context<'_, Self>) -> AnyElement {
        let hub = self.meetings_hub.read(cx);
        let meetings = person_meetings(profile, &hub.meetings)
            .into_iter()
            .enumerate()
            .map(|(index, meeting)| {
                let person_id = profile.id.clone();
                let meeting_id = meeting.id.clone();
                let title = meeting.title.clone();
                div()
                    .flex()
                    .flex_wrap()
                    .items_baseline()
                    .gap_2()
                    .child(
                        text_button(
                            ("desktop-person-meeting", index),
                            title.clone(),
                            theme::blueberry_200(),
                            theme::rambutan_100(),
                        )
                        .debug_selector(move || format!("desktop-person-meeting-{index}"))
                        .aria_label(format!("Open meeting {title}"))
                        .on_click(cx.listener(
                            move |this, _ev, _window, cx| {
                                this.open_person_meeting(&person_id, meeting_id.clone(), cx)
                            },
                        )),
                    )
                    .child(label(
                        format_started_at(meeting.started_at_ms),
                        LABEL_SM_PX,
                        theme::rambutan_500(),
                    ))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let conversations = person_conversations(&profile.id, &self.inbox_conversations)
            .into_iter()
            .enumerate()
            .map(|(index, conversation)| {
                let person_id = profile.id.clone();
                let conversation_id = conversation.id.clone();
                let subject = conversation.subject.clone();
                text_button(
                    ("desktop-person-conversation", index),
                    subject.clone(),
                    theme::blueberry_200(),
                    theme::rambutan_100(),
                )
                .debug_selector(move || format!("desktop-person-conversation-{index}"))
                .aria_label(format!("Open conversation {subject}"))
                .on_click(cx.listener(move |this, _ev, window, cx| {
                    this.open_person_conversation(&person_id, &conversation_id, window, cx)
                }))
                .into_any_element()
            })
            .collect::<Vec<_>>();
        let section = |heading: &'static str, empty: &'static str, rows: Vec<AnyElement>| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(label(heading, LABEL_SM_PX, theme::rambutan_500()))
                .when(rows.is_empty(), |el| {
                    el.child(label(empty, LABEL_SM_PX, theme::rambutan_500()))
                })
                .children(rows)
        };
        div()
            .debug_selector(|| "desktop-person-links".into())
            .flex()
            .flex_col()
            .gap_3()
            .child(section(
                copy::MEETINGS_HEADING,
                copy::MEETINGS_EMPTY,
                meetings,
            ))
            .child(section(
                copy::CONVERSATIONS_HEADING,
                copy::CONVERSATIONS_EMPTY,
                conversations,
            ))
            .into_any_element()
    }

    /// The return button a meeting or conversation shows when it was opened
    /// from a profile.
    pub(in crate::views::workspace) fn render_back_to_people(
        &self,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let (_, view) = self.people_page.return_to.as_ref()?;
        if *view != self.active_view {
            return None;
        }
        Some(
            div()
                .px_6()
                .pt_3()
                .child(
                    ring_text_button(
                        "desktop-back-to-people",
                        copy::BACK_TO_PEOPLE,
                        theme::rambutan_400(),
                        theme::rambutan_200(),
                    )
                    .debug_selector(|| "desktop-back-to-people".into())
                    .on_click(cx.listener(|this, _ev, window, cx| this.back_to_people(window, cx))),
                )
                .into_any_element(),
        )
    }

    /// Wrap a meeting or conversation view with the return button when it
    /// was opened from a profile; otherwise the view is unchanged.
    pub(in crate::views::workspace) fn with_back_to_people(
        &self,
        detail: Div,
        cx: &Context<'_, Self>,
    ) -> Div {
        match self.render_back_to_people(cx) {
            None => detail,
            Some(bar) => div()
                .flex_1()
                .h_full()
                .min_w_0()
                .min_h_0()
                .flex()
                .flex_col()
                .child(bar)
                .child(div().flex_1().min_h_0().min_w_0().flex().child(detail)),
        }
    }

    fn render_relationship_panel(&self, person_id: &str, cx: &Context<'_, Self>) -> AnyElement {
        let person_for_save = person_id.to_string();
        let person_for_add = person_id.to_string();
        let profile = self
            .relationships
            .iter()
            .find(|profile| profile.person_id == person_id);
        let items = profile
            .map(|profile| profile.items.clone())
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(index, item)| self.render_relationship_item(person_id, index, item, cx))
            .collect::<Vec<_>>();
        let conversation_ids = self
            .inbox_conversations
            .iter()
            .map(|conversation| conversation.id.as_str())
            .collect::<HashSet<_>>();
        let suggestions = bruhs_models::suggested_follow_ups(
            &self.relationships,
            &self.inbox_conversations,
            EpochMs::now().as_millis(),
        )
        .into_iter()
        .filter(|suggestion| suggestion.person_id == person_id)
        .enumerate()
        .map(|(index, suggestion)| {
            let conversation = suggestion
                .source_record_id
                .clone()
                .filter(|id| conversation_ids.contains(id.as_str()));
            match conversation {
                Some(conversation_id) => {
                    let person_id = person_id.to_string();
                    text_button(
                        ("desktop-follow-up", index),
                        suggestion.reason.clone(),
                        theme::blueberry_200(),
                        theme::rambutan_100(),
                    )
                    .debug_selector(move || format!("desktop-follow-up-{index}"))
                    .aria_label(format!("Open {}", suggestion.reason))
                    .on_click(cx.listener(move |this, _ev, window, cx| {
                        this.open_person_conversation(&person_id, &conversation_id, window, cx)
                    }))
                    .into_any_element()
                }
                None => {
                    label(suggestion.reason, LABEL_SM_PX, theme::blueberry_200()).into_any_element()
                }
            }
        })
        .collect::<Vec<_>>();
        let kinds = RELATIONSHIP_ITEM_KINDS.into_iter().map(|kind| {
            kind_button(
                ("desktop-relationship-kind", element_key(kind.label())),
                kind,
                self.relationship_item_kind == kind,
            )
            .on_click(cx.listener(move |this, _event, _window, cx| {
                this.relationship_item_kind = kind;
                cx.notify();
            }))
        });
        let due_applies = RelationshipDue::applies_to(self.relationship_item_kind);

        div()
            .mt_2()
            .pt_3()
            .border_t_1()
            .border_color(theme::longan_700())
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme::rambutan_200())
                            .child("Relationship"),
                    )
                    .child(
                        web_button(
                            ("save-relationship", element_key(person_id)),
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .aria_label("Save details")
                        .child("Save details")
                        .on_click(cx.listener(
                            move |this, _event, _window, cx| {
                                this.save_relationship_details(&person_for_save, cx)
                            },
                        )),
                    ),
            )
            .child(
                div()
                    .grid()
                    .grid_cols(3)
                    .gap_2()
                    .child(Input::new(&self.relationship_company_input))
                    .child(Input::new(&self.relationship_role_input))
                    .child(Input::new(&self.relationship_email_input))
                    .child(Input::new(&self.relationship_phone_input))
                    .child(Input::new(&self.relationship_location_input))
                    .child(Input::new(&self.relationship_notes_input)),
            )
            .children(suggestions)
            .child(div().flex().flex_wrap().gap_1().children(kinds))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(160.0))
                            .child(Input::new(&self.relationship_item_input)),
                    )
                    .when(due_applies, |el| {
                        el.child(
                            Checkbox::new("desktop-relationship-due")
                                .label(copy::DUE_TOMORROW)
                                .checked(self.people_page.due_tomorrow)
                                .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                                    this.people_page.due_tomorrow = *checked;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        web_button(
                            ("add-relationship-item", element_key(person_id)),
                            WebButtonVariant::Primary,
                            WebButtonSize::Small,
                            ButtonState::disabled_if(
                                self.relationship_item_input
                                    .read(cx)
                                    .value()
                                    .trim()
                                    .is_empty(),
                            ),
                        )
                        .aria_label("Add")
                        .child("Add")
                        .on_click(cx.listener(
                            move |this, _event, window, cx| {
                                this.add_relationship_item(&person_for_add, window, cx)
                            },
                        )),
                    ),
            )
            .children(items)
            .into_any_element()
    }

    fn render_relationship_item(
        &self,
        person_id: &str,
        index: usize,
        item: RelationshipItem,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let editing = self
            .people_page
            .editing_item
            .as_ref()
            .filter(|edit| edit.person_id == person_id && edit.item_id == item.id)
            .cloned();
        if let Some(edit) = editing {
            return self.render_item_editor(index, &item, edit, cx);
        }
        let complete = item.completed_at_ms.is_some();
        let (toggle_person, toggle_item) = (person_id.to_string(), item.id.clone());
        let (edit_person, edit_item) = (person_id.to_string(), item.id.clone());
        let (delete_person, delete_item) = (person_id.to_string(), item.id.clone());
        let meta = format!(
            "{}{}",
            item.kind.label(),
            if item.due_at_ms.is_some() {
                ", due"
            } else {
                ""
            }
        );
        div()
            .flex()
            .items_start()
            .gap_2()
            .child(
                div()
                    .id(("relationship-item", index))
                    .debug_selector(move || format!("desktop-relationship-item-{index}"))
                    .flex_1()
                    .min_w_0()
                    .role(Role::Button)
                    .aria_label(format!(
                        "{} {}",
                        if complete { "Reopen" } else { "Complete" },
                        item.text
                    ))
                    .tab_index(0)
                    .cursor_pointer()
                    .rounded(px(4.0))
                    .px_2()
                    .py_1()
                    .hover(|element| element.bg(theme::longan_800().opacity(0.7)))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|element| element.border_color(theme::persimmon_400()))
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.toggle_relationship_item(&toggle_person, &toggle_item, cx);
                    }))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(if complete {
                                theme::rambutan_400()
                            } else {
                                theme::rambutan_300()
                            })
                            .child(format!(
                                "{} {}",
                                if complete { "✓" } else { "○" },
                                item.text
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme::rambutan_400())
                            .child(meta),
                    ),
            )
            .child(
                text_button(
                    ("desktop-relationship-edit", index),
                    "edit",
                    theme::rambutan_500(),
                    theme::rambutan_300(),
                )
                .debug_selector(move || format!("desktop-relationship-edit-{index}"))
                .aria_label(format!("Edit {}", item.text))
                .on_click(cx.listener(move |this, _event, window, cx| {
                    this.begin_item_edit(&edit_person, &edit_item, window, cx)
                })),
            )
            .child(
                text_button(
                    ("desktop-relationship-delete", index),
                    "delete",
                    theme::rambutan_500(),
                    theme::lychee_300(),
                )
                .debug_selector(move || format!("desktop-relationship-delete-{index}"))
                .aria_label(format!("Delete {}", item.text))
                .on_click(cx.listener(move |this, _event, _window, cx| {
                    this.delete_relationship_item(&delete_person, &delete_item, cx)
                })),
            )
            .into_any_element()
    }

    fn render_item_editor(
        &self,
        index: usize,
        item: &RelationshipItem,
        edit: ItemEdit,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let kinds = RELATIONSHIP_ITEM_KINDS.into_iter().map(|kind| {
            kind_button(
                ("desktop-relationship-edit-kind", element_key(kind.label())),
                kind,
                edit.kind == kind,
            )
            .on_click(cx.listener(move |this, _event, _window, cx| {
                if let Some(edit) = &mut this.people_page.editing_item {
                    edit.kind = kind;
                }
                cx.notify();
            }))
        });
        div()
            .id(("desktop-relationship-editor", index))
            .debug_selector(|| "desktop-relationship-editor".into())
            .role(Role::Group)
            .aria_label(format!("Edit {}", item.text))
            .flex()
            .flex_col()
            .gap_2()
            .rounded(px(4.0))
            .bg(theme::longan_800().opacity(0.6))
            .p_2()
            .child(div().flex().flex_wrap().gap_1().children(kinds))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w(px(160.0)).child(
                        Input::new(&self.people_page.item_edit_input).aria_label("Item text"),
                    ))
                    .child(
                        Checkbox::new("desktop-relationship-edit-due")
                            .label(copy::DUE_TOMORROW)
                            .checked(edit.due)
                            .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                                if let Some(edit) = &mut this.people_page.editing_item {
                                    edit.due = *checked;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        web_button(
                            "desktop-relationship-edit-cancel",
                            WebButtonVariant::Ghost,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .aria_label(copy::CANCEL)
                        .child(copy::CANCEL)
                        .on_click(cx.listener(
                            |this, _event, _window, cx| {
                                this.people_page.editing_item = None;
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        web_button(
                            "desktop-relationship-edit-save",
                            WebButtonVariant::Primary,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .debug_selector(|| "desktop-relationship-edit-save".into())
                        .aria_label(copy::ITEM_SAVE)
                        .child(copy::ITEM_SAVE)
                        .on_click(cx.listener(|this, _event, _window, cx| this.save_item_edit(cx))),
                    ),
            )
            .when_some(self.people_page.item_error, |el, message| {
                el.child(
                    div()
                        .id("desktop-relationship-edit-error")
                        .debug_selector(|| "desktop-relationship-edit-error".into())
                        .role(Role::Alert)
                        .child(body(message, BODY_SM_PX, theme::lychee_300())),
                )
            })
            .into_any_element()
    }

    /// Web `MergePanel`, with the confirmation step.
    fn render_merge_panel(
        &self,
        source_id: &str,
        all: &[Arc<PersonProfile>],
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let options: Vec<DropdownOption> = all
            .iter()
            .filter(|p| p.id != source_id)
            .map(|p| DropdownOption {
                value: Box::leak(p.id.clone().into_boxed_str()),
                label: p.display_name.clone(),
                description: None,
            })
            .collect();
        let groups = vec![DropdownGroup {
            label: None,
            mark: None,
            options,
        }];
        let entity = cx.entity().downgrade();
        let toggle_entity = entity.clone();
        let on_toggle: ToggleHandler = Rc::new(move |_window, cx| {
            let _ = toggle_entity.update(cx, |this, cx| {
                if let Some(merge) = &mut this.merge {
                    merge.dropdown_open = !merge.dropdown_open;
                }
                cx.notify();
            });
        });
        let on_select: SelectHandler = Rc::new(move |value, _window, cx| {
            let _ = entity.update(cx, |this, cx| {
                if let Some(merge) = &mut this.merge {
                    merge.target_id = Some(value.to_string());
                    merge.dropdown_open = false;
                    merge.confirming = false;
                }
                cx.notify();
            });
        });
        let (value, dropdown_open, confirming) =
            self.merge
                .as_ref()
                .map_or((String::new(), false, false), |merge| {
                    (
                        merge.target_id.clone().unwrap_or_default(),
                        merge.dropdown_open,
                        merge.confirming,
                    )
                });
        let can_merge = !value.is_empty();
        let source_name = all
            .iter()
            .find(|p| p.id == source_id)
            .map_or_else(String::new, |p| p.display_name.clone());
        let target_name = all
            .iter()
            .find(|p| p.id == value)
            .map_or_else(String::new, |p| p.display_name.clone());
        let source = source_id.to_string();
        let mut panel = div()
            .debug_selector(|| "desktop-people-merge-panel".to_string())
            .mt_1()
            .p_2()
            .rounded(px(4.0))
            .bg(theme::longan_800().opacity(0.8))
            .border_1()
            .border_color(theme::longan_700())
            .flex()
            .flex_col()
            .gap_2()
            .child(label("Merge into:", LABEL_SM_PX, theme::rambutan_400()))
            .child(dropdown(
                MERGE_DROPDOWN_ID,
                "Merge into",
                &value,
                groups,
                Listbox::open_if(dropdown_open),
                on_toggle,
                on_select,
            ));
        if confirming && can_merge {
            panel = panel
                .child(
                    div()
                        .id("desktop-people-merge-confirm-text")
                        .debug_selector(|| "desktop-people-merge-confirm-text".into())
                        .role(Role::Status)
                        .child(body(
                            copy::merge_confirm(&source_name, &target_name),
                            BODY_SM_PX,
                            theme::rambutan_200(),
                        )),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            text_button(
                                "desktop-people-merge-back",
                                copy::CANCEL,
                                theme::rambutan_500(),
                                theme::rambutan_300(),
                            )
                            .on_click(cx.listener(
                                |this, _ev, _window, cx| {
                                    if let Some(merge) = &mut this.merge {
                                        merge.confirming = false;
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                        .child(
                            text_button(
                                "desktop-people-merge-commit",
                                copy::MERGE_CONFIRM_BUTTON,
                                theme::persimmon_400(),
                                theme::persimmon_200(),
                            )
                            .debug_selector(|| "desktop-people-merge-commit".into())
                            .aria_label(format!(
                                "{} {source_name} into {target_name}",
                                copy::MERGE_CONFIRM_BUTTON
                            ))
                            .on_click(cx.listener(
                                move |this, _ev, _window, cx| {
                                    let Some(target) = this
                                        .merge
                                        .as_ref()
                                        .filter(|merge| merge.confirming)
                                        .and_then(|merge| merge.target_id.clone())
                                    else {
                                        return;
                                    };
                                    this.merge = None;
                                    this.people_page.notice = None;
                                    this.people_page.pending_merge =
                                        Some((source.clone(), target.clone()));
                                    let source = source.clone();
                                    this.meetings_hub
                                        .update(cx, |hub, cx| hub.merge_people(source, target, cx));
                                },
                            )),
                        ),
                );
        } else {
            panel = panel.child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        text_button(
                            "desktop-people-merge-cancel",
                            copy::CANCEL,
                            theme::rambutan_500(),
                            theme::rambutan_300(),
                        )
                        .on_click(cx.listener(|this, _ev, _window, cx| {
                            this.merge = None;
                            cx.notify();
                        })),
                    )
                    .child(
                        text_button(
                            "desktop-people-merge-confirm",
                            "Merge",
                            theme::persimmon_400(),
                            theme::persimmon_200(),
                        )
                        .debug_selector(|| "desktop-people-merge-confirm".into())
                        .when(!can_merge, |el| el.opacity(0.4).cursor_default())
                        .when(can_merge, |el| {
                            el.on_click(cx.listener(|this, _ev, _window, cx| {
                                if let Some(merge) = &mut this.merge {
                                    merge.confirming = merge.target_id.is_some();
                                }
                                cx.notify();
                            }))
                        }),
                    ),
            );
        }
        panel.into_any_element()
    }

    /// Web `ObservationsByKind`.
    fn render_observations_by_kind(
        &self,
        card: usize,
        observations: &[PersonObservation],
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let hub = self.meetings_hub.read(cx);
        let title = |meeting_id: &str| {
            hub.meetings
                .iter()
                .find(|meeting| meeting.id == meeting_id)
                .map(|meeting| meeting.title.clone())
        };
        let mut row = 0usize;
        let mut by_kind: BTreeMap<&'static str, Vec<PersonObservation>> = BTreeMap::new();
        for obs in observations {
            by_kind
                .entry(observation_kind_label(obs.kind))
                .or_default()
                .push(obs.clone());
        }
        for items in by_kind.values_mut() {
            items.sort_by_key(|o| std::cmp::Reverse(o.created_at));
        }
        div()
            .pt_2()
            .flex()
            .flex_col()
            .gap_3()
            .children(by_kind.into_iter().map(|(kind, items)| {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(label(
                        kind.to_ascii_uppercase(),
                        LABEL_SM_PX,
                        theme::rambutan_500(),
                    ))
                    .children(items.into_iter().map(|o| {
                        row += 1;
                        // Provenance: open the meeting this observation came from.
                        let source = match title(&o.at_meeting_id) {
                            Some(title) => {
                                let meeting_id = o.at_meeting_id.clone();
                                text_button(
                                    ("desktop-observation-source", card * 10_000 + row),
                                    format!("From {title}"),
                                    theme::blueberry_200(),
                                    theme::rambutan_100(),
                                )
                                .aria_label(format!("Open source meeting {title}"))
                                .debug_selector(move || {
                                    format!("desktop-observation-source-{card}-{row}")
                                })
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    this.open_meeting_detail(meeting_id.clone(), cx)
                                }))
                                .into_any_element()
                            }
                            None => label(
                                "Source meeting is not stored",
                                LABEL_SM_PX,
                                theme::rambutan_500(),
                            )
                            .into_any_element(),
                        };
                        div()
                            .flex()
                            .flex_wrap()
                            .items_baseline()
                            .gap_2()
                            .child(
                                label(o.text, LABEL_PX, theme::rambutan_300())
                                    .line_height(relative(1.375)),
                            )
                            .child(source)
                    }))
            }))
            .into_any_element()
    }
}
