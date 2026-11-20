//! Copy only the selected, saved personal task into an explicit review step.
use crate::{
    state::*,
    ui::{Button, ButtonSize, ButtonVariant},
};
use bruhs_models::workbench_handoff::{WorkItemHandoff, WorkItemHandoffRequest};
use leptos::prelude::*;

#[component]
pub(super) fn TaskWorkbenchAction(item: WorkItem) -> impl IntoView {
    let saves = expect_context::<crate::task_saves::TaskSaves>();
    let CurrentModel(model) = expect_context();
    let PermissionModeSignal(permission) = expect_context();
    let ClaudeEffortSignal(effort) = expect_context();
    let ActiveProjectSignal(project) = expect_context();
    let ActiveView(active_view) = expect_context();
    let SetActiveView(set_view) = expect_context();
    let SelectedWorkItem(selected) = expect_context();
    let WorkItemsSignal(items) = expect_context();
    let WorkbenchTaskHandoff(handoff) = expect_context();
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let start = Callback::new(move |_| {
        if busy.get_untracked()
            || saves.status.get_untracked() != crate::durable_metadata::SaveState::Saved
        {
            return;
        }
        let request = WorkItemHandoffRequest {
            item: item.clone(),
            project_path: project
                .get_untracked()
                .map_or_else(String::new, |project| project.path),
            model: model.get_untracked(),
            effort: effort.get_untracked().filter(|_| {
                bruhs_models::coordination::task_effort_supported(&model.get_untracked())
            }),
            permission_mode: permission.get_untracked(),
        };
        let source_id = item.id.clone();
        let source_view = active_view.get_untracked();
        busy.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            let result = super::local_tools_panel::request::<WorkItemHandoff, _>(
                "POST",
                "/api/coordination/handoffs",
                Some(&request),
            )
            .await;
            let _ = busy.try_set(false);
            match result {
                Ok(prepared) => {
                    if selected.get_untracked().as_deref() == Some(source_id.as_str())
                        && active_view.get_untracked() == source_view
                        && items
                            .with_untracked(|items| items.iter().any(|item| item == &request.item))
                        && saves.status.get_untracked() == crate::durable_metadata::SaveState::Saved
                    {
                        handoff.set(Some(prepared.id));
                        set_view.set(AppView::Workbench);
                    }
                }
                Err(message) => {
                    let _ = error.try_set(Some(message));
                }
            }
        });
    });
    view! {
        <div class="space-y-2 border-t border-longan-800 pt-3">
            <Button variant=ButtonVariant::Outline size=ButtonSize::Small
                disabled=Signal::derive(move || busy.get() || saves.status.get() != crate::durable_metadata::SaveState::Saved)
                on_click=start>"Review agent work"</Button>
            <p class="text-body-sm text-rambutan-400">"Review this task in Workbench, then choose the project, model and access before starting. The personal task stays unchanged."</p>
            <Show when=move || busy.get()><p role="status" class="text-body-sm text-rambutan-400">"Preparing agent work…"</p></Show>
            <Show when=move || saves.status.get() != crate::durable_metadata::SaveState::Saved><p class="text-body-sm text-rambutan-400">"Save task changes before preparing agent work."</p></Show>
            {move || error.get().map(|message| view! { <p role="alert" class="text-body-sm text-lychee-300">{message}</p> })}
        </div>
    }
}
