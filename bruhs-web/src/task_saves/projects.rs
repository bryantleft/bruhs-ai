use super::*;

impl TaskSaves {
    pub fn finish_loading_projects(self, loaded: Vec<Project>) -> Vec<Project> {
        let mut merged = Vec::new();
        self.initial_projects
            .update_value(|edits| merged = edits.merge_by(loaded, |record| &record.id));
        merged
    }

    pub fn edit_project(self, expected: Option<Project>, draft: Project) {
        self.initial_projects
            .update_value(|edits| edits.edit(&draft.id, Some(&draft)));
        self.queue.update(|queue| {
            queue.edit_record(
                Kind::Project,
                draft.id.clone(),
                expected.map(Record::Project),
                Some(Record::Project(draft)),
            );
        });
        self.refresh_status();
        self.save();
    }

    /// Preserve one original and one tombstone for every member of the
    /// confirmed cascade, including earlier failed/in-flight task edits.
    pub fn delete_project(self, project: Project, tasks: Vec<WorkItem>) {
        self.initial_projects
            .update_value(|edits| edits.edit(&project.id, None));
        self.initial_edits.update_value(|edits| {
            for task in &tasks {
                edits.edit(&task.id, None);
            }
        });
        self.queue.update(|queue| {
            let mut ids = vec![(Kind::Project, project.id.clone())];
            queue.edit_record(
                Kind::Project,
                project.id.clone(),
                Some(Record::Project(project)),
                None,
            );
            for task in tasks {
                ids.push((Kind::Task, task.id.clone()));
                queue.edit(task.id.clone(), Some(Record::Task(task)), None);
            }
            queue.group_records(&ids);
        });
        self.refresh_status();
        self.save();
    }
}
