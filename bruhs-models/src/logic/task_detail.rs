//! The task detail's dependency picker and delete confirmation, shared by
//! web (`task_detail.rs`) and desktop (`task_forms_ui.rs`). Moved from the
//! web task detail, which held the only picker and confirmation; desktop
//! used to cycle through candidates, clear them all, and delete at once.

use crate::WorkItemPlanning;

/// The picker's first row. Choosing it adds nothing.
pub const ADD_DEPENDENCY_LABEL: &str = "Add dependency…";
/// The picker's accessible name.
pub const ADD_DEPENDENCY_PICKER_LABEL: &str = "Add dependency";
/// A dependency whose task no longer exists.
pub const MISSING_DEPENDENCY_LABEL: &str = "Missing task";
/// The tooltip and action of a dependency chip.
pub const REMOVE_DEPENDENCY_LABEL: &str = "Remove dependency";
/// The delete confirmation's second line.
pub const DELETE_TASK_NOTE: &str = "This can't be undone.";
/// The delete confirmation's accessible name.
pub const DELETE_TASK_DIALOG_LABEL: &str = "Confirm task deletion";

/// The delete confirmation's question.
pub fn delete_task_prompt(title: &str) -> String {
    format!("Delete task \"{title}\"?")
}

/// The tasks `current_id` can depend on: every other task, in list order.
/// `tasks` are each task's id and title.
pub fn dependency_candidates<'a>(
    current_id: &str,
    tasks: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<(&'a str, &'a str)> {
    tasks
        .into_iter()
        .filter(|(id, _)| *id != current_id)
        .collect()
}

/// The title a dependency chip shows: its task's title, or
/// [`MISSING_DEPENDENCY_LABEL`] once that task is gone.
pub fn dependency_title<'a>(
    dependency_id: &str,
    tasks: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> &'a str {
    tasks
        .into_iter()
        .find(|(id, _)| *id == dependency_id)
        .map_or(MISSING_DEPENDENCY_LABEL, |(_, title)| title)
}

impl WorkItemPlanning {
    /// Adds `dependency_id` once. The picker's empty first row adds
    /// nothing. Returns whether the list changed.
    pub fn add_dependency(&mut self, dependency_id: &str) -> bool {
        if dependency_id.is_empty() || self.dependency_ids.iter().any(|id| id == dependency_id) {
            return false;
        }
        self.dependency_ids.push(dependency_id.to_string());
        true
    }

    /// Removes `dependency_id` and keeps the others in order. Returns
    /// whether the list changed.
    pub fn remove_dependency(&mut self, dependency_id: &str) -> bool {
        let before = self.dependency_ids.len();
        self.dependency_ids.retain(|id| id != dependency_id);
        self.dependency_ids.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::{
        delete_task_prompt, dependency_candidates, dependency_title, MISSING_DEPENDENCY_LABEL,
    };
    use crate::WorkItemPlanning;

    const TASKS: [(&str, &str); 3] = [("a", "Write"), ("b", "Review"), ("c", "Ship")];

    #[test]
    fn candidates_are_every_other_task_in_list_order() {
        assert_eq!(
            dependency_candidates("b", TASKS),
            [("a", "Write"), ("c", "Ship")]
        );
        assert_eq!(dependency_candidates("missing", TASKS).len(), 3);
    }

    #[test]
    fn a_chip_names_its_task_or_says_it_is_missing() {
        assert_eq!(dependency_title("c", TASKS), "Ship");
        assert_eq!(dependency_title("gone", TASKS), MISSING_DEPENDENCY_LABEL);
    }

    #[test]
    fn adding_ignores_the_placeholder_and_duplicates() {
        let mut planning = WorkItemPlanning::default();
        assert!(!planning.add_dependency(""));
        assert!(planning.add_dependency("a"));
        assert!(!planning.add_dependency("a"));
        assert!(planning.add_dependency("c"));
        assert_eq!(planning.dependency_ids, ["a", "c"]);
    }

    #[test]
    fn removing_one_dependency_keeps_the_others_in_order() {
        let mut planning = WorkItemPlanning {
            dependency_ids: vec!["a".into(), "b".into(), "c".into()],
            ..WorkItemPlanning::default()
        };
        assert!(planning.remove_dependency("b"));
        assert_eq!(planning.dependency_ids, ["a", "c"]);
        assert!(!planning.remove_dependency("b"));
    }

    #[test]
    fn the_delete_question_quotes_the_title() {
        assert_eq!(delete_task_prompt("Ship v2"), "Delete task \"Ship v2\"?");
    }
}
