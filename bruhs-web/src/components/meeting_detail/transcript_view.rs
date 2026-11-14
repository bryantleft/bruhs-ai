use super::transcript_projection::{TranscriptGroup, TranscriptProjection};
use super::{speaker_colors, speaker_label, SpeakerChip};
use crate::state::{MeetingSegmentsSignal, Speaker, TranscriptSegment};
use bruhs_models::num::ToIntSaturating;
use leptos::prelude::*;

#[component]
pub(super) fn TranscriptContents(
    meeting_id: String,
    speakers: Memo<Vec<Speaker>>,
    empty_label: Signal<&'static str>,
) -> impl IntoView {
    let MeetingSegmentsSignal(segments) = expect_context::<MeetingSegmentsSignal>();
    let meeting_id = StoredValue::new(meeting_id);
    let groups = RwSignal::new(Vec::<TranscriptGroup>::new());
    let mut projection = TranscriptProjection::default();
    // Run immediately for the initial view, then publish only changed rows or
    // group membership. Keep the effect alive for exactly this view's owner.
    let project = RenderEffect::new(move |_| {
        segments.with(|all| {
            let id = meeting_id.get_value();
            let source = all.get(&id).map_or(&[][..], Vec::as_slice);
            if projection.sync(source) {
                groups.set(projection.groups().to_vec());
            }
        });
    });
    on_cleanup(move || drop(project));

    view! {
        <Show when=move || groups.with(|groups| !groups.is_empty()) fallback=move || view! {
            <div class="rounded-grape inset-ring inset-ring-longan-800 bg-longan-900/40 px-3 py-2 text-body-sm text-rambutan-500">
                {move || empty_label.get()}
            </div>
        }>
            <div class="space-y-3">
                <For each=move || groups.get() key=|group| group.rows.clone()
                    children=move |group| {
                        let speaker = StoredValue::new(group.speaker_id.as_ref().map(ToString::to_string));
                        let (text_class, ring_class) = speaker.with_value(|id| speaker_colors(id.as_deref()));
                        let label = Memo::new(move |_| speakers.with(|speakers| {
                            speaker.with_value(|id| speaker_label(speakers, id.as_deref()))
                        }));
                        view! {
                            <div class="space-y-1">
                                <div class="flex items-center">
                                    // Label receipts update the chip alone; the
                                    // transcript text keeps its DOM and selection.
                                    {move || view! {
                                        <SpeakerChip meeting_id=meeting_id.get_value()
                                            speaker_id=speaker.get_value() label=label.get()
                                            text_class ring_class />
                                    }}
                                </div>
                                <div class="space-y-1 pl-3">
                                    <For each=move || group.rows.get() key=|row| row.clone()
                                        children=move |segment| view! { <TranscriptSegmentLine segment /> } />
                                </div>
                            </div>
                        }
                    } />
            </div>
        </Show>
    }
}

#[component]
fn TranscriptSegmentLine(segment: ArcRwSignal<TranscriptSegment>) -> impl IntoView {
    let id = segment.with_untracked(|segment| segment.id.clone());
    let partial = segment.clone();
    let text = segment.clone();
    view! {
        <div data-transcript-segment=id data-testid="meeting-transcript-segment"
            class=move || partial.with(|segment| {
                if segment.partial {
                    "flex items-baseline text-body-sm text-rambutan-200 leading-snug opacity-50"
                } else {
                    "flex items-baseline text-body-sm text-rambutan-200 leading-snug"
                }
            })>
            <span>{move || text.with(|segment| segment.text.clone())}</span>
            {move || segment.with(|segment| segment.speaker_confidence)
                .filter(|&confidence| confidence < 0.85)
                .map(|confidence| view! {
                    <span class="text-label-sm text-rambutan-500 ml-1">
                        {format!("{}%", (confidence * 100.0).to_u32_saturating())}
                    </span>
                })}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_line_retains_full_escaped_text_and_partial_confidence_state() {
        let owner = Owner::new();
        owner.with(|| {
            let text = format!(
                "é <script> & {} ending-quartz",
                "full saved text ".repeat(1_000)
            );
            let segment = ArcRwSignal::new(TranscriptSegment {
                id: "segment-0".into(),
                meeting_id: "meeting".into(),
                speaker_id: Some("me".into()),
                start_ms: 0,
                end_ms: 1_000,
                text,
                confidence: 0.9,
                partial: true,
                speaker_confidence: Some(0.8),
            });
            let html = untrack(|| {
                tachys::view::RenderHtml::to_html(
                    view! { <TranscriptSegmentLine segment=segment.clone() /> },
                )
            });
            assert!(html.contains("é &lt;script&gt; &amp;"));
            assert!(html.contains(&"full saved text ".repeat(1_000)));
            assert!(html.contains("ending-quartz"));
            assert!(html.contains("opacity-50"));
            assert!(html.contains("80%"));
            assert!(html.contains("data-transcript-segment=\"segment-0\""));
            segment.update(|segment| {
                segment.text.push_str(" final");
                segment.partial = false;
                segment.speaker_confidence = Some(0.95);
            });
            let html = untrack(|| {
                tachys::view::RenderHtml::to_html(view! { <TranscriptSegmentLine segment /> })
            });
            assert!(html.contains("ending-quartz final"));
            assert!(!html.contains("opacity-50"));
            assert!(!html.contains("80%"));
            assert!(html.contains("data-transcript-segment=\"segment-0\""));
        });
    }
}
