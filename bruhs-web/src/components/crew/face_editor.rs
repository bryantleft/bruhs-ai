//! Choose how a bruh looks: which way it faces, its face and mouth colours,
//! with a live preview and a still preview of every state.

use bruhs_models::crew::{
    face_view, BruhColor, BruhLook, BruhStatus, FaceView, Look, MouthColor, EDITOR_FACE_PX,
    STATE_PREVIEW_PX,
};
use leptos::prelude::*;

use crate::ui::BrailleBruh;

/// Every state in the order the previews show them.
const STATES: [BruhStatus; 5] = [
    BruhStatus::Asleep,
    BruhStatus::Working,
    BruhStatus::NeedsYou,
    BruhStatus::Paused,
    BruhStatus::Off,
];

const SEGMENT: &str = "bruhs-control inline-flex min-h-control flex-1 items-center justify-center rounded-seed px-3 text-label-sm transition-colors focus-visible:outline-2 focus-visible:outline-persimmon-400";
const SWATCH: &str = "bruhs-control inline-flex size-control items-center justify-center rounded-orb transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-persimmon-400";

/// A face drawn still: the look as chosen, or the state's expression without
/// its motion.
pub(super) fn still_view(look: &BruhLook, status: BruhStatus) -> FaceView {
    face_view(look, status, 0, true)
}

fn css_rgb(rgb: u32) -> String {
    format!("background-color:#{rgb:06x}")
}

/// One swatch row, a radio group named `label`.
fn swatches<T: Copy + PartialEq + Send + Sync + 'static>(
    label: &'static str,
    options: &'static [T],
    name: fn(T) -> &'static str,
    rgb: fn(T) -> u32,
    selected: Signal<T>,
    choose: Callback<T>,
) -> impl IntoView {
    view! {
        <div role="radiogroup" aria-label=label class="flex min-w-0 flex-wrap gap-2">
            {options.iter().copied().map(|option| view! {
                <button
                    type="button"
                    role="radio"
                    aria-label=name(option)
                    title=name(option)
                    aria-checked=move || (selected.get() == option).to_string()
                    class=move || format!("{SWATCH} {}", if selected.get() == option { "inset-ring-2 inset-ring-rambutan-100" } else { "inset-ring inset-ring-longan-700 hover:inset-ring-rambutan-400" })
                    on:click=move |_| choose.run(option)
                >
                    <span aria-hidden="true" class="size-control-xs rounded-orb" style=css_rgb(rgb(option))></span>
                </button>
            }).collect_view()}
        </div>
    }
}

#[component]
pub fn FaceEditor(look: RwSignal<BruhLook>) -> impl IntoView {
    let preview = Memo::new(move |_| look.with(|look| still_view(look, BruhStatus::Working)));
    view! {
        <div class="flex min-w-0 flex-col gap-4 sm:flex-row">
            <div class="flex shrink-0 justify-center">
                <BrailleBruh size=f64::from(EDITOR_FACE_PX) view=Signal::from(preview) />
            </div>
            <div class="min-w-0 flex-1 space-y-3">
                <div class="space-y-1">
                    <span id="bruh-look-label" class="text-label-sm text-rambutan-300">"Look"</span>
                    <div role="radiogroup" aria-labelledby="bruh-look-label" class="flex min-w-0 gap-1 rounded-grape bg-longan-900 p-0.5 inset-ring inset-ring-longan-800">
                        {Look::ALL.into_iter().map(|option| view! {
                            <button
                                type="button"
                                role="radio"
                                aria-checked=move || (look.with(|look| look.look) == option).to_string()
                                class=move || format!("{SEGMENT} {}", if look.with(|look| look.look) == option { "bg-longan-700 text-rambutan-100" } else { "text-rambutan-400 hover:text-rambutan-200" })
                                on:click=move |_| look.update(|look| look.look = option)
                            >{option.label()}</button>
                        }).collect_view()}
                    </div>
                </div>
                <div class="space-y-1">
                    <span class="text-label-sm text-rambutan-300">"Face colour"</span>
                    {swatches(
                        "Face colour",
                        &BruhColor::ALL,
                        BruhColor::label,
                        BruhColor::rgb,
                        Signal::derive(move || look.with(|look| look.face)),
                        Callback::new(move |face| look.update(|look| look.face = face)),
                    )}
                </div>
                <div class="space-y-1">
                    <span class="text-label-sm text-rambutan-300">"Mouth colour"</span>
                    {swatches(
                        "Mouth colour",
                        &MouthColor::ALL,
                        MouthColor::label,
                        MouthColor::rgb,
                        Signal::derive(move || look.with(|look| look.mouth)),
                        Callback::new(move |mouth| look.update(|look| look.mouth = mouth)),
                    )}
                </div>
                <ul aria-label="How it looks in each state" class="m-0 flex min-w-0 list-none flex-wrap gap-3 p-0">
                    {STATES.into_iter().map(|status| {
                        let view = Memo::new(move |_| look.with(|look| still_view(look, status)));
                        view! {
                            <li class="flex flex-col items-center gap-1">
                                <BrailleBruh size=f64::from(STATE_PREVIEW_PX) view=Signal::from(view) />
                                <span class="text-label-sm text-rambutan-500">{status.label()}</span>
                            </li>
                        }
                    }).collect_view()}
                </ul>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preview_shows_the_chosen_look_awake_and_still() {
        let look = BruhLook {
            look: Look::Right,
            face: BruhColor::Pandan,
            mouth: MouthColor::Longan,
        };
        let view = still_view(&look, BruhStatus::Working);
        assert!(!view.animates);
        assert_eq!(view.style.look_x, Look::Right.look_x());
        assert_eq!(view.style.face_rgb, BruhColor::Pandan.rgb());
        assert_eq!(view.style.eyes, bruhs_models::crew::Eyes::Open);
        assert!(STATES
            .iter()
            .all(|status| !still_view(&look, *status).animates));
    }

    #[test]
    fn swatches_paint_the_token_colour() {
        assert_eq!(css_rgb(0x0087_be52), "background-color:#87be52");
    }
}
