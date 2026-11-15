//! The way back from a meeting or conversation opened from a profile.

use leptos::prelude::*;

use crate::state::{AppView, ReturnToPersonSignal, SelectedPersonSignal, SetActiveView};

/// Opens a profile's linked records and remembers which profile they were
/// opened from. Built during component setup, where contexts resolve.
#[derive(Clone, Copy)]
pub struct PersonOpener {
    back: Option<ReturnToPersonSignal>,
    set_active_view: SetActiveView,
}

impl PersonOpener {
    pub fn from_context() -> Self {
        Self {
            back: use_context::<ReturnToPersonSignal>(),
            set_active_view: expect_context::<SetActiveView>(),
        }
    }

    pub fn open(self, person_id: &str, view: AppView) {
        if let Some(ReturnToPersonSignal(back)) = self.back {
            back.set(Some(person_id.to_string()));
        }
        self.set_active_view.0.set(view);
    }
}

/// "Back to People", shown only while the view was opened from a profile.
/// Returns to People with that profile open; leaving any other way forgets it.
#[component]
pub fn BackToPeople() -> impl IntoView {
    let back = use_context::<ReturnToPersonSignal>();
    let person = StoredValue::new(back.and_then(|ReturnToPersonSignal(back)| back.get_untracked()));
    on_cleanup(move || {
        if let Some(ReturnToPersonSignal(back)) = back {
            back.try_set(None);
        }
    });
    let SetActiveView(set_active_view) = expect_context::<SetActiveView>();
    let selected = use_context::<SelectedPersonSignal>();
    person.get_value().map(|person_id| {
        view! {
            <button
                type="button"
                data-people-back=""
                class="bruhs-control min-h-[32px] rounded-seed px-2 py-1.5 text-label text-blueberry-300 hover:bg-longan-800 hover:text-blueberry-200 cursor-pointer"
                on:click=move |_| {
                    if let Some(SelectedPersonSignal(selected)) = selected {
                        selected.set(Some(person_id.clone()));
                    }
                    set_active_view.set(AppView::People);
                }
            >
                {bruhs_models::people_copy::BACK_TO_PEOPLE}
            </button>
        }
    })
}
