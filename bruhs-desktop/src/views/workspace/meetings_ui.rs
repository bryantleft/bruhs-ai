//! Meetings, meeting detail, live pill, universal search, people (with the
//! voice-enroll modal), the auto-record prompt and the notice toast — 1:1
//! with `bruhs-web/src/components/{meetings_page,meeting_detail,
//! meeting_live,search_page,people_page,voice_enroll,auto_record_prompt,
//! capture_status_banner,banner}.rs`. Data comes from `MeetingsHub`; sizes
//! are the web's Tailwind values in px.

use std::collections::{hash_map::DefaultHasher, BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use bruhs_models::api::{CaptureStatus, SystemAudioPermission};
use bruhs_models::ids::{self, MeetingId};
use bruhs_models::speaker_id::ColorIndex;
use bruhs_models::{
    AppView, CaptureKind, Meeting, MeetingStatus, ObservationKind, PersonObservation,
    PersonProfile, RelationshipItemKind, RelationshipProfile, SearchDocument, SearchHit,
    SearchIndex, SearchKind, Speaker, SpeakerIdentity,
};
use gpui::prelude::*;
use gpui::*;
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Escape as InputEscape, Input, InputEvent, InputState};
use gpui_component::Disableable;

use super::{
    element_key, is_web_button_activation_key, meetings_client, redraw_on_edit, DesktopLifeTab,
    PersistedWorkspaceState, Workspace,
};
use crate::app_state::DesktopState;
use crate::views::dropdown::{
    dropdown, DropdownGroup, DropdownOption, Listbox, SelectHandler, ToggleHandler,
};
use crate::views::meetings_hub::{
    format_duration, format_elapsed, format_started_at, summary_preview, EnrollState, HubEvent,
    MeetingsHub, SEARCH_LIMIT,
};
use crate::views::web_button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use crate::{assets, theme};
use bruhs_models::EpochMs;

mod actions;
mod note_detail;
pub(super) mod note_editor;
mod notes;
pub(super) mod transcript_view;

// Web type scale on these pages.
const BODY_PX: f32 = 16.0;
const BODY_SM_PX: f32 = 14.0;
const LABEL_PX: f32 = 14.0;
const LABEL_SM_PX: f32 = 12.0;
const H4_PX: f32 = 18.0;
const DISPLAY_SM_PX: f32 = 32.0;
const BODY_FONT: &str = "Monaspace Neon";
const LABEL_FONT: &str = "Monaspace Argon";
const PAGE_MAX_WIDTH_PX: f32 = 768.0;
const LIVE_PANEL_WIDTH_PX: f32 = 480.0;
const LIVE_PANEL_MAX_HEIGHT_PX: f32 = 260.0;
const SEARCH_RESULT_PREVIEW_CHARS: usize = 120;

pub const MERGE_DROPDOWN_ID: &str = "desktop-people-merge-target";
const LOCAL_FOCUS_PRIVACY_NOTE: &str =
    "Local app name + bundle ID only · sampled every 2s · not shared-screen evidence. No \
    images, titles, URLs or AI upload.";
const SYSTEM_AUDIO_PERMISSION_HINT: &str =
    "Grant Screen Recording permission to capture the other side of meetings — open System \
    Settings → Privacy & Security → Screen Recording and enable bruhs-capture.";

/// Web `MergePanel`: the person row whose merge panel is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PersonMergePanel {
    pub(super) person_id: String,
    /// The profile picked to merge into, if any yet.
    pub(super) target_id: Option<String>,
    pub(super) dropdown_open: bool,
}

/// A transcript speaker chip: the one being renamed, or whose link menu is
/// open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SpeakerRef {
    pub(super) meeting_id: MeetingId,
    pub(super) speaker_id: ids::SpeakerId,
}

impl SpeakerRef {
    /// `None` when either is not a valid id, which no server-sent id is.
    fn new(meeting_id: &str, speaker_id: &str) -> Option<Self> {
        Some(Self {
            meeting_id: meeting_id.parse().ok()?,
            speaker_id: speaker_id.parse().ok()?,
        })
    }

    fn is(&self, meeting_id: &str, speaker_id: &str) -> bool {
        self.meeting_id.as_str() == meeting_id && self.speaker_id.as_str() == speaker_id
    }
}

fn label(text: impl Into<SharedString>, size: f32, color: Hsla) -> Div {
    div()
        .font_family(LABEL_FONT)
        // Web `text-label-sm` is tracked .025em; the wider Argon instance matches it.
        .when(size <= 12.0, |el| {
            el.font_family(theme::UI_SMALL_FONT_FAMILY)
        })
        .text_size(px(size))
        .text_color(color)
        .child(text.into())
}

fn body(text: impl Into<SharedString>, size: f32, color: Hsla) -> Div {
    div()
        .font_family(BODY_FONT)
        .text_size(px(size))
        .text_color(color)
        .child(text.into())
}

fn search_join(parts: impl IntoIterator<Item = String>) -> String {
    parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

fn search_preview(value: &str) -> String {
    let value = value.trim();
    if value.chars().count() <= SEARCH_RESULT_PREVIEW_CHARS {
        return value.to_string();
    }
    let mut preview = value
        .chars()
        .take(SEARCH_RESULT_PREVIEW_CHARS)
        .collect::<String>();
    preview.push('…');
    preview
}

fn search_result_icon(kind: SearchKind) -> &'static str {
    match kind {
        SearchKind::Thread => assets::MESSAGE_SQUARE_ICON_PATH,
        SearchKind::Task => assets::LIST_CHECKS_ICON_PATH,
        SearchKind::Project => assets::FOLDER_ICON_PATH,
        SearchKind::Event => assets::CALENDAR_ICON_PATH,
        SearchKind::SchedulingLink => assets::CALENDAR_ICON_PATH,
        SearchKind::Meeting | SearchKind::Transcript => assets::MIC_ICON_PATH,
        SearchKind::Person => assets::USERS_ICON_PATH,
        SearchKind::Document | SearchKind::Clip | SearchKind::MeetingNote => {
            assets::TOOL_FILE_ICON_PATH
        }
        SearchKind::Conversation => assets::MESSAGE_SQUARE_ICON_PATH,
        SearchKind::Finance | SearchKind::Health => assets::ACTIVITY_ICON_PATH,
    }
}

/// Web `inset-ring inset-ring-longan-700 rounded-seed px-2 py-0.5` text button.
fn ring_text_button(
    id: impl Into<ElementId>,
    text: &'static str,
    color: Hsla,
    hover: Hsla,
) -> Stateful<Div> {
    div()
        .id(id)
        .rounded(px(4.0))
        .border_1()
        .border_color(theme::longan_700())
        .px_2()
        .py(px(2.0))
        .font_family(LABEL_FONT)
        .text_size(px(LABEL_PX))
        .text_color(color)
        .tab_index(0)
        .role(Role::Button)
        .aria_label(text)
        .focus_visible(|el| el.border_color(theme::persimmon_400()))
        .cursor_pointer()
        .hover(move |el| el.text_color(hover))
        .child(text)
}

/// Web `text-label-sm text-rambutan-500 hover:text-rambutan-300` link-style button.
fn text_button(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    color: Hsla,
    hover: Hsla,
) -> Stateful<Div> {
    let text = text.into();
    div()
        .id(id)
        .font_family(LABEL_FONT)
        .text_size(px(LABEL_SM_PX))
        .text_color(color)
        .tab_index(0)
        .role(Role::Button)
        .aria_label(text.clone())
        .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
        .cursor_pointer()
        .hover(move |el| el.text_color(hover))
        .child(text)
}

fn status_label(s: MeetingStatus) -> &'static str {
    match s {
        MeetingStatus::Live => "Live",
        MeetingStatus::Ended => "Ended",
        MeetingStatus::Failed => "Failed",
    }
}

fn status_color(s: MeetingStatus) -> Hsla {
    match s {
        MeetingStatus::Live => theme::persimmon_400(),
        MeetingStatus::Ended => theme::rambutan_400(),
        MeetingStatus::Failed => theme::lychee_400(),
    }
}

/// Web `speaker_colors`: `(text, ring)` for a speaker slot.
pub fn speaker_colors(speaker_id: Option<&str>) -> (Hsla, Hsla) {
    let identity = SpeakerIdentity::resolve(speaker_id, &[]);
    match identity.color_index {
        ColorIndex::Me => (
            theme::dragonfruit_400(),
            theme::dragonfruit_500().opacity(0.6),
        ),
        ColorIndex::Them => (theme::pandan_300(), theme::pandan_500().opacity(0.6)),
        ColorIndex::Cluster(n) => match n % 4 {
            0 => (theme::pandan_300(), theme::pandan_500().opacity(0.6)),
            1 => (theme::lychee_300(), theme::lychee_500().opacity(0.6)),
            2 => (theme::blueberry_400(), theme::blueberry_500()),
            _ => (theme::durian_300(), theme::durian_500().opacity(0.6)),
        },
        ColorIndex::Unknown => (theme::rambutan_300(), theme::longan_700()),
    }
}

pub fn speaker_label(speakers: &[Speaker], id: Option<&str>) -> String {
    SpeakerIdentity::resolve(id, speakers).display
}

fn observation_kind_label(kind: ObservationKind) -> &'static str {
    match kind {
        ObservationKind::Topic => "Topics",
        ObservationKind::Decision => "Decisions",
        ObservationKind::ActionItemOwner => "Action items",
        ObservationKind::Tone => "Tone",
        ObservationKind::Quote => "Quotes",
    }
}

impl Workspace {
    fn hub(&self) -> Entity<MeetingsHub> {
        self.meetings_hub.clone()
    }

    pub(super) fn open_meeting_detail(&mut self, meeting_id: String, cx: &mut Context<'_, Self>) {
        self.meetings_hub.update(cx, |hub, cx| {
            hub.ensure_segments(&meeting_id, cx);
            hub.load_local_focus(&meeting_id);
        });
        self.active_view = AppView::MeetingDetail { meeting_id };
        self.editing_speaker = None;
        self.speaker_menu = None;
        self.close_transient_surfaces();
        self.persist_and_notify(cx);
    }

    pub(super) fn open_meetings_list(&mut self, cx: &mut Context<'_, Self>) {
        self.meetings_hub.update(cx, |hub, cx| {
            hub.refresh_meetings(cx);
            hub.refresh_capture_status(cx);
        });
        self.active_view = AppView::Meetings;
        self.close_transient_surfaces();
        self.persist_and_notify(cx);
    }

    // ── Meetings page ────────────────────────────────────────────────────

    /// Web `MeetingsPage`.
    pub(super) fn render_meetings_page(&self, cx: &Context<'_, Self>) -> AnyElement {
        let hub = self.meetings_hub.read(cx);
        let running = hub.capture_status.is_running();
        // Borrowed for the frame: the list can be long and redraws are frequent.
        let meetings = &hub.meetings;
        let permission = match &hub.capture_status {
            CaptureStatus::Running(running) => Some(running.system_audio_permission),
            CaptureStatus::NotRunning => None,
        };
        let record_disabled = !running;
        div()
            .id("desktop-meetings-page")
            .debug_selector(|| "desktop-meetings-page".to_string())
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme::longan_950())
            .text_color(theme::rambutan_300())
            .child(
                crate::views::page_header::page_header(
                    "Meetings",
                    Some(
                        if running {
                            "Capture daemon ready"
                        } else {
                            "Capture daemon not running"
                        }
                        .into(),
                    ),
                )
                .child(
                    web_button(
                        "desktop-meetings-record",
                        WebButtonVariant::Primary,
                        WebButtonSize::Small,
                        ButtonState::disabled_if(record_disabled),
                    )
                    .aria_label("Record a meeting")
                    .when(!record_disabled, |el| {
                        el.on_click(cx.listener(|this, _ev, _window, cx| {
                            this.meetings_hub.update(cx, |hub, cx| hub.record_now(cx));
                        }))
                    })
                    .child("Record"),
                ),
            )
            .child(
                div()
                    .px_6()
                    .pt_3()
                    .children(self.render_capture_status_banner(permission)),
            )
            .child(self.render_meeting_notes(cx))
            .child(if meetings.is_empty() {
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .px_8()
                    .text_center()
                    .child(body("No meetings yet", BODY_PX, theme::rambutan_300()))
                    .child(
                        body(
                            if running {
                                "Press + Record to start one manually."
                            } else {
                                "Recording needs the bruhs-capture daemon. Start it, and Record turns on here."
                            },
                            BODY_SM_PX,
                            theme::rambutan_500(),
                        )
                        .max_w(px(448.0))
                        .text_center(),
                    )
                    .into_any_element()
            } else {
                div()
                    .id("desktop-meetings-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(
                        meetings
                            .iter()
                            .enumerate()
                            .map(|(index, meeting)| self.render_meeting_row(index, meeting, cx)),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }

    /// Web `CaptureStatusBanner`: only for `denied` / `not_determined`.
    fn render_capture_status_banner(
        &self,
        permission: Option<SystemAudioPermission>,
    ) -> Option<AnyElement> {
        let permission = match permission? {
            SystemAudioPermission::Denied => "denied",
            SystemAudioPermission::NotDetermined => "not_determined",
            _ => return None,
        };
        Some(
            div()
                .debug_selector(|| "desktop-capture-status-banner".to_string())
                .rounded(px(4.0))
                .bg(theme::lychee_600().opacity(0.25))
                .border_1()
                .border_color(theme::lychee_500().opacity(0.6))
                .px_3()
                .py_2()
                .font_family(LABEL_FONT)
                .text_size(px(LABEL_PX))
                .text_color(theme::rambutan_300())
                .child(format!(
                    "System audio capture is {permission}. {SYSTEM_AUDIO_PERMISSION_HINT}"
                ))
                .into_any_element(),
        )
    }

    fn render_meeting_row(
        &self,
        index: usize,
        meeting: &Meeting,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let id_for_click = meeting.id.clone();
        let is_live = meeting.status == MeetingStatus::Live;
        let duration = match meeting.status {
            MeetingStatus::Live => None,
            _ => meeting
                .ended_at_ms
                .map(|end| format_duration(end - meeting.started_at_ms)),
        };
        let action_items = meeting.summary.as_ref().map_or(0, |s| s.action_items.len());
        let preview = meeting
            .summary
            .as_ref()
            .filter(|s| !s.tldr.is_empty())
            .map(|s| summary_preview(&s.tldr, 80));
        let has_thread = meeting.thread_id.is_some();
        let key = element_key(&meeting.id);
        div()
            .id(("desktop-meeting-row", key))
            .debug_selector(move || format!("desktop-meeting-row-{index}"))
            .w_full()
            .flex()
            .items_start()
            .justify_between()
            .px_6()
            .py_3()
            .border_b_1()
            .border_color(theme::longan_800())
            .tab_index(0)
            .role(Role::Button)
            .aria_label(meeting.title.clone())
            .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
            .cursor_pointer()
            .hover(|el| el.bg(theme::longan_900().opacity(0.6)))
            .on_click(cx.listener(move |this, _ev, _window, cx| {
                this.open_meeting_detail(id_for_click.clone(), cx);
            }))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .pr_4()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                body(meeting.title.clone(), BODY_SM_PX, theme::rambutan_100())
                                    .min_w_0()
                                    .whitespace_nowrap()
                                    .overflow_hidden(),
                            )
                            .when(has_thread, |el| {
                                el.child(label("💬", LABEL_PX, theme::rambutan_400()))
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .flex_wrap()
                            .child(label(
                                format_started_at(meeting.started_at_ms),
                                LABEL_PX,
                                theme::rambutan_500(),
                            ))
                            .when(is_live, |el| {
                                el.child(label("live now", LABEL_PX, theme::persimmon_400()))
                            })
                            .children(duration.map(|d| label(d, LABEL_PX, theme::rambutan_500())))
                            .when(action_items > 0, |el| {
                                el.child(label(
                                    format!("📋 {action_items}"),
                                    LABEL_PX,
                                    theme::rambutan_400(),
                                ))
                            }),
                    )
                    .children(
                        preview.map(|p| label(p, LABEL_PX, theme::rambutan_500()).truncate()),
                    ),
            )
            .child(
                label(
                    status_label(meeting.status),
                    LABEL_PX,
                    status_color(meeting.status),
                )
                .flex_shrink_0(),
            )
            .into_any_element()
    }

    // ── Meeting detail ───────────────────────────────────────────────────

    /// Web `MeetingDetailPage`.
    pub(super) fn render_meeting_detail(
        &self,
        meeting_id: &str,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        div()
            .id("desktop-meeting-detail")
            .debug_selector(|| "desktop-meeting-detail".to_string())
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .bg(theme::longan_950())
            .text_color(theme::rambutan_300())
            .flex()
            .flex_col()
            .child(self.render_local_focus(meeting_id, cx))
            .child(self.render_transcript_list(meeting_id, true, cx))
            .into_any_element()
    }

    fn render_local_focus(&self, meeting_id: &str, cx: &Context<'_, Self>) -> AnyElement {
        let hub = self.meetings_hub.read(cx);
        let enabled = hub.local_focus.enabled_for(meeting_id);
        let live = hub.live_meeting_id.as_deref() == Some(meeting_id);
        let rows = hub.local_focus.rows.get(meeting_id);
        let title = hub.local_focus.current_label(meeting_id);
        let key = meeting_id.to_owned();
        let keyboard_key = key.clone();
        div()
            .id("desktop-local-focus")
            .debug_selector(|| "desktop-local-focus".into())
            .flex_shrink_0()
            .max_h(px(200.0))
            .overflow_y_scroll()
            .px_6()
            .py_3()
            .border_b_1()
            .border_color(theme::longan_800())
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(label(title, LABEL_SM_PX, theme::rambutan_300()))
                    .when(live, |el| {
                        el.child(
                            web_button(
                                "desktop-local-focus-toggle",
                                WebButtonVariant::Outline,
                                WebButtonSize::Small,
                                ButtonState::disabled_if(hub.capture_action_in_flight()),
                            )
                            .debug_selector(|| "desktop-local-focus-toggle".into())
                            .min_h(px(44.0))
                            .min_w(px(44.0))
                            .aria_label(if enabled {
                                "Turn off local app context"
                            } else {
                                "Enable local app context for this meeting"
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.meetings_hub.update(cx, |hub, cx| {
                                    if hub.live_meeting_id.as_ref() == Some(&key) {
                                        hub.toggle_local_focus(cx);
                                    }
                                });
                            }))
                            .on_key_down(cx.listener(
                                move |this, event: &KeyDownEvent, window, cx| {
                                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                        window.prevent_default();
                                        cx.stop_propagation();
                                        this.meetings_hub.update(cx, |hub, cx| {
                                            if hub.live_meeting_id.as_ref() == Some(&keyboard_key) {
                                                hub.toggle_local_focus(cx);
                                            }
                                        });
                                    }
                                },
                            ))
                            .child(if enabled {
                                "Turn off app context"
                            } else {
                                "Enable app context"
                            }),
                        )
                    }),
            )
            .child(label(
                LOCAL_FOCUS_PRIVACY_NOTE,
                LABEL_SM_PX,
                theme::rambutan_500(),
            ))
            .when(live && !hub.local_focus.feedback.is_empty(), |el| {
                el.child(
                    label(
                        hub.local_focus.feedback.clone(),
                        LABEL_SM_PX,
                        theme::rambutan_400(),
                    )
                    .id("desktop-local-focus-status")
                    .role(Role::Status),
                )
            })
            .when_some(hub.local_focus.load_errors.get(meeting_id), |el, error| {
                el.child(
                    label(error.clone(), LABEL_SM_PX, theme::lychee_400())
                        .id("desktop-local-focus-load-error")
                        .role(Role::Status),
                )
            })
            .when_some(rows.filter(|rows| !rows.is_empty()), |el, rows| {
                el.child(label(
                    format!(
                        "Latest {} of {} saved app changes · screenshot unavailable",
                        rows.len().min(10),
                        rows.len()
                    ),
                    LABEL_SM_PX,
                    theme::rambutan_500(),
                ))
                .children(rows.iter().rev().take(10).map(|row| {
                    label(
                        format!(
                            "{} · {}",
                            format_started_at(
                                EpochMs::saturating_from_i64(row.observed_at_ms).as_f64()
                            ),
                            row.label()
                        ),
                        LABEL_SM_PX,
                        theme::rambutan_400(),
                    )
                }))
            })
            .into_any_element()
    }

    fn render_meeting_header(&self, meeting_id: &str, cx: &Context<'_, Self>) -> AnyElement {
        let hub = self.meetings_hub.read(cx);
        let meeting = hub.meeting(meeting_id);
        let title = meeting.map_or_else(|| "Meeting".to_string(), |m| m.title.clone());
        let empty = meeting.is_some() && hub.segments.get(meeting_id).is_none_or(Vec::is_empty);

        let column = div()
            .id("desktop-meeting-header")
            .when_some(self.transcript_section_focus(false), |el, focus| {
                el.track_focus(&focus)
            })
            .w_full()
            .max_w(px(PAGE_MAX_WIDTH_PX))
            .px_6()
            .pt_4()
            .pb(px(if empty { 0.0 } else { 8.0 }))
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        ring_text_button(
                            "desktop-meeting-back",
                            "← Back",
                            theme::rambutan_400(),
                            theme::rambutan_200(),
                        )
                        .debug_selector(|| "desktop-meeting-back".to_string())
                        .on_click(cx.listener(|this, _ev, _window, cx| {
                            this.open_meetings_list(cx);
                        })),
                    )
                    .child(body(title, H4_PX, theme::rambutan_100())),
            );

        let column = column.when(meeting.is_some(), |el| {
            el.child(self.linked_note_button(meeting_id, cx))
        });

        let mut transcript = div().child(
            label("Transcript", LABEL_SM_PX, theme::rambutan_400()).when(empty, |el| el.mb_2()),
        );
        if empty {
            transcript = transcript.child(self.render_muted_box(
                "No transcript yet — segments appear after Stop while Whisper processes the audio.",
            ));
        }
        column.child(transcript).into_any_element()
    }

    fn render_meeting_footer(&self, meeting_id: &str, cx: &Context<'_, Self>) -> AnyElement {
        let hub = self.meetings_hub.read(cx);
        let meeting = hub.meeting(meeting_id);
        let action_status = hub.meeting_action_status(meeting_id);
        let resummarize_busy = action_status.as_ref().is_some_and(|status| status.busy);
        let action_blocked = hub.meeting_action_blocked(meeting_id);
        let meeting_id_owned = meeting_id.to_string();
        let mut column = div()
            .id("desktop-meeting-footer")
            .when_some(self.transcript_section_focus(true), |el, focus| {
                el.track_focus(&focus)
            })
            .w_full()
            .max_w(px(PAGE_MAX_WIDTH_PX))
            .px_6()
            .pt_6()
            .pb_4()
            .flex()
            .flex_col()
            .gap_6();

        // Summary
        if let Some(m) = &meeting {
            let resummarize = Some(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(self.meeting_action_button(
                        "desktop-meeting-check-server-requests",
                        "Check server requests",
                        meeting_id,
                        actions::MeetingButtonAction::DiscoverPending,
                        action_blocked,
                        cx,
                    ))
                    .child(self.meeting_action_button(
                        "desktop-meeting-resummarize",
                        if m.summary.is_some() {
                            "Re-summarize"
                        } else {
                            "Generate summary"
                        },
                        meeting_id,
                        actions::MeetingButtonAction::Generate,
                        action_blocked,
                        cx,
                    ))
                    .into_any_element(),
            );
            let summary_body: AnyElement = match &m.summary {
                None => self.render_muted_box(if resummarize_busy {
                    "Meeting request in progress…"
                } else {
                    "Generate a summary from the saved transcript."
                }),
                Some(s) => {
                    let mut el = div().flex().flex_col().gap_2().child(
                        div()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(theme::longan_800())
                            .bg(theme::longan_900().opacity(0.3))
                            .px_3()
                            .py_2()
                            .font_family(BODY_FONT)
                            .text_size(px(BODY_SM_PX))
                            .text_color(theme::rambutan_200())
                            .child(s.tldr.clone()),
                    );
                    if !s.action_items.is_empty() {
                        el = el.child(
                            div()
                                .child(
                                    label("Action items", LABEL_PX, theme::rambutan_500())
                                        .mb(px(2.0)),
                                )
                                .child(div().flex().flex_col().gap(px(2.0)).children(
                                    s.action_items.iter().map(|a| {
                                        let mut row = div()
                                            .flex()
                                            .font_family(LABEL_FONT)
                                            .text_size(px(LABEL_PX))
                                            .text_color(theme::rambutan_300())
                                            .child(format!("• {}", a.text));
                                        if let Some(due) = &a.due_hint {
                                            row = row.child(
                                                div()
                                                    .text_color(theme::rambutan_500())
                                                    .child(format!(" — {due}")),
                                            );
                                        }
                                        row
                                    }),
                                )),
                        );
                    }
                    if !s.decisions.is_empty() {
                        el = el.child(
                            div()
                                .child(
                                    label("Decisions", LABEL_PX, theme::rambutan_500()).mb(px(2.0)),
                                )
                                .child(div().flex().flex_col().gap(px(2.0)).children(
                                    s.decisions.iter().map(|d| {
                                        label(format!("• {d}"), LABEL_PX, theme::rambutan_300())
                                    }),
                                )),
                        );
                    }
                    if !s.topics.is_empty() {
                        el = el.child(
                            div()
                                .child(label("Topics", LABEL_PX, theme::rambutan_500()).mb(px(2.0)))
                                .children(s.topics.iter().map(|t| {
                                    div()
                                        .mb_1()
                                        .child(label(
                                            t.title.clone(),
                                            LABEL_PX,
                                            theme::rambutan_200(),
                                        ))
                                        .child(
                                            div().ml_2().flex().flex_col().gap(px(2.0)).children(
                                                t.bullets.iter().map(|b| {
                                                    label(
                                                        format!("– {b}"),
                                                        LABEL_SM_PX,
                                                        theme::rambutan_400(),
                                                    )
                                                }),
                                            ),
                                        )
                                })),
                        );
                    }
                    el.into_any_element()
                }
            };
            column = column.child(self.render_section("Summary", resummarize, summary_body));
            if let Some(status) = &action_status {
                column = column.child(self.render_meeting_action_status(meeting_id, status, cx));
            }

            // Observation extraction
            let skip = m.skip_observations;
            let has_summary = m.summary.is_some();
            let reextract_disabled = skip
                || !has_summary
                || action_blocked
                || !hub.prefs.personality.extract_observations;
            let hub_for_skip = self.hub();
            let mid_skip = meeting_id_owned.clone();
            let observation = div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Checkbox::new("desktop-meeting-skip-observations")
                                .checked(skip)
                                .disabled(action_blocked)
                                .on_click(move |_checked, _window, cx| {
                                    let mid = mid_skip.clone();
                                    hub_for_skip.update(cx, |hub, cx| {
                                        hub.set_skip_observations(mid, !skip, cx)
                                    });
                                }),
                        )
                        .child(label(
                            "Skip observation extraction for this meeting",
                            LABEL_PX,
                            theme::rambutan_400(),
                        )),
                )
                .child(self.meeting_action_button(
                    "desktop-meeting-reextract",
                    "Re-extract observations",
                    meeting_id,
                    actions::MeetingButtonAction::Reextract,
                    reextract_disabled,
                    cx,
                ))
                .into_any_element();
            column = column.child(self.render_section("Observation extraction", None, observation));

            // Chat
            let mid_chat = meeting_id_owned;
            let chat = div()
                .id("desktop-meeting-chat")
                .debug_selector(|| "desktop-meeting-chat".to_string())
                .rounded(px(4.0))
                .border_1()
                .border_color(theme::longan_700())
                .px_2()
                .py_1()
                .font_family(LABEL_FONT)
                .text_size(px(LABEL_PX))
                .text_color(theme::rambutan_200())
                .tab_index(0)
                .role(Role::Button)
                .aria_label("Chat with this meeting")
                .focus_visible(|el| el.border_color(theme::persimmon_400()))
                .cursor_pointer()
                .hover(|el| el.border_color(theme::persimmon_500()))
                .on_click(cx.listener(move |this, _ev, _window, cx| {
                    let mid = mid_chat.clone();
                    this.meetings_hub
                        .update(cx, |hub, cx| hub.open_meeting_chat(mid, cx));
                }))
                .child("Chat with this meeting")
                .into_any_element();
            column = column.child(self.render_section(
                "Chat",
                None,
                div().flex().child(chat).into_any_element(),
            ));
        }

        column.into_any_element()
    }

    fn render_section(
        &self,
        title: &'static str,
        action: Option<AnyElement>,
        body_el: AnyElement,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .mb_2()
                    .child(label(title, LABEL_SM_PX, theme::rambutan_400()))
                    .children(action),
            )
            .child(body_el)
            .into_any_element()
    }

    fn render_muted_box(&self, text: &'static str) -> AnyElement {
        div()
            .rounded(px(8.0))
            .border_1()
            .border_color(theme::longan_800())
            .bg(theme::longan_900().opacity(0.4))
            .px_3()
            .py_2()
            .font_family(BODY_FONT)
            .text_size(px(BODY_SM_PX))
            .text_color(theme::rambutan_500())
            .child(text)
            .into_any_element()
    }

    /// Web `SpeakerChip`: click renames inline, `▾` links clustered
    /// speakers to a person.
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the web SpeakerChip's props one to one"
    )]
    fn render_speaker_chip(
        &self,
        meeting_id: String,
        speaker_id: Option<String>,
        group_index: usize,
        label_text: String,
        text_color: Hsla,
        ring_color: Hsla,
        people: &[PersonProfile],
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let Some(sid) = speaker_id else {
            return div()
                .rounded(px(4.0))
                .border_1()
                .border_color(ring_color)
                .px_2()
                .py(px(2.0))
                .opacity(0.6)
                .font_family(LABEL_FONT)
                .text_size(px(LABEL_PX))
                .text_color(text_color)
                .child(label_text)
                .into_any_element();
        };
        let editing = self
            .editing_speaker
            .as_ref()
            .is_some_and(|speaker| speaker.is(&meeting_id, &sid))
            && self.transcript_editor_is_at(group_index);
        if editing {
            return div()
                .debug_selector(move || format!("desktop-speaker-editor-{group_index}"))
                .w(px(120.0))
                .rounded(px(4.0))
                .border_1()
                .border_color(ring_color)
                .px_1()
                .capture_action(cx.listener(|this, _: &InputEscape, _window, cx| {
                    this.cancel_speaker_rename(cx);
                    cx.stop_propagation();
                }))
                .child(
                    Input::new(&self.speaker_rename_input)
                        .appearance(false)
                        .aria_label("Speaker name"),
                )
                .into_any_element();
        }
        let is_linkable = sid.starts_with("them_") || sid == "them";
        let menu_open = self
            .speaker_menu
            .as_ref()
            .is_some_and(|speaker| speaker.is(&meeting_id, &sid));
        let speaker_ref = SpeakerRef::new(&meeting_id, &sid);
        let (speaker_edit, label_edit) = (speaker_ref.clone(), label_text.clone());
        let speaker_menu = speaker_ref;
        div()
            .relative()
            .flex()
            .items_center()
            .gap(px(2.0))
            .child(
                div()
                    .id(("desktop-speaker-chip", group_index))
                    .debug_selector(move || format!("desktop-speaker-chip-{group_index}"))
                    .rounded(px(4.0))
                    .border_1()
                    .border_color(ring_color)
                    .px_2()
                    .py(px(2.0))
                    .font_family(LABEL_FONT)
                    .text_size(px(LABEL_PX))
                    .text_color(text_color)
                    .tab_index(0)
                    .role(Role::Button)
                    .aria_label(format!("Rename speaker {label_text}"))
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.opacity(0.8))
                    .on_click(cx.listener(move |this, _ev, window, cx| {
                        if let Some(speaker) = speaker_edit.clone() {
                            this.begin_speaker_rename(
                                speaker,
                                label_edit.clone(),
                                group_index,
                                window,
                                cx,
                            );
                        }
                    }))
                    .child(label_text),
            )
            .when(is_linkable, |el| {
                el.child(
                    div()
                        .id(("desktop-speaker-link", group_index))
                        .debug_selector(move || format!("desktop-speaker-link-{group_index}"))
                        .px(px(2.0))
                        .font_family(LABEL_FONT)
                        .text_size(px(LABEL_SM_PX))
                        .text_color(text_color)
                        .opacity(0.6)
                        .tab_index(0)
                        .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
                        .role(Role::Button)
                        .aria_label("Link to person...")
                        .cursor_pointer()
                        .hover(|el| el.opacity(1.0))
                        .on_click(cx.listener(move |this, _ev, _window, cx| {
                            this.speaker_menu = if this.speaker_menu == speaker_menu {
                                None
                            } else {
                                speaker_menu.clone()
                            };
                            cx.notify();
                        }))
                        .child("▾"),
                )
            })
            .when(menu_open, |el| {
                el.child(self.render_speaker_link_menu(meeting_id.clone(), sid.clone(), people, cx))
            })
            .into_any_element()
    }

    fn render_speaker_link_menu(
        &self,
        meeting_id: String,
        speaker_id: String,
        people: &[PersonProfile],
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let mut menu = div()
            .absolute()
            .top(relative(1.0))
            .left_0()
            .mt_1()
            .occlude()
            .min_w(px(160.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(theme::longan_700())
            .bg(theme::longan_900())
            .py_1()
            .flex()
            .flex_col()
            .child(
                label("Link to person...", LABEL_SM_PX, theme::rambutan_500())
                    .px_2()
                    .pb_1(),
            );
        if people.is_empty() {
            menu = menu.child(
                label("No profiles yet", LABEL_PX, theme::rambutan_500())
                    .px_2()
                    .py_1(),
            );
        } else {
            for (i, person) in people.iter().enumerate() {
                let (mid, sid) = (meeting_id.clone(), speaker_id.clone());
                let (pid, name) = (person.id.clone(), person.display_name.clone());
                menu = menu.child(
                    div()
                        .id(("desktop-speaker-link-person", i))
                        .w_full()
                        .px_2()
                        .py_1()
                        .font_family(LABEL_FONT)
                        .text_size(px(LABEL_PX))
                        .text_color(theme::rambutan_200())
                        .tab_index(0)
                        .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
                        .role(Role::Button)
                        .aria_label(format!("Link to {}", person.display_name))
                        .cursor_pointer()
                        .hover(|el| el.bg(theme::longan_800()))
                        .on_click(cx.listener(move |this, _ev, _window, cx| {
                            this.speaker_menu = None;
                            let (mid, sid, pid, name) =
                                (mid.clone(), sid.clone(), pid.clone(), name.clone());
                            this.meetings_hub
                                .update(cx, |hub, cx| hub.link_speaker(mid, sid, pid, name, cx));
                        }))
                        .child(person.display_name.clone()),
                );
            }
        }
        menu.child(
            div()
                .id("desktop-speaker-link-cancel")
                .w_full()
                .px_2()
                .py_1()
                .mt_1()
                .font_family(LABEL_FONT)
                .text_size(px(LABEL_PX))
                .text_color(theme::rambutan_500())
                .tab_index(0)
                .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
                .role(Role::Button)
                .aria_label("Cancel")
                .cursor_pointer()
                .hover(|el| el.bg(theme::longan_800()))
                .on_click(cx.listener(|this, _ev, _window, cx| {
                    this.speaker_menu = None;
                    cx.notify();
                }))
                .child("Cancel"),
        )
        .into_any_element()
    }

    fn begin_speaker_rename(
        &mut self,
        speaker: SpeakerRef,
        current: String,
        group_index: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.speaker_menu = None;
        self.editing_speaker = Some(speaker);
        self.pin_transcript_editor(Some(group_index), cx);
        self.speaker_rename_input.update(cx, |input, cx| {
            input.set_value(current, window, cx);
        });
        self.speaker_rename_input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        cx.notify();
    }

    /// Enter in the speaker input: commit; Escape: cancel.
    pub(super) fn commit_speaker_rename(&mut self, cx: &mut Context<'_, Self>) {
        self.pin_transcript_editor(None, cx);
        let Some(SpeakerRef {
            meeting_id,
            speaker_id,
        }) = self.editing_speaker.take()
        else {
            return;
        };
        let label = self.speaker_rename_input.read(cx).value().to_string();
        if !label.trim().is_empty() {
            self.meetings_hub.update(cx, |hub, cx| {
                hub.rename_speaker(
                    meeting_id.into_inner(),
                    speaker_id.into_inner(),
                    label.trim().to_string(),
                    cx,
                )
            });
        }
        cx.notify();
    }

    pub(super) fn cancel_speaker_rename(&mut self, cx: &mut Context<'_, Self>) {
        self.pin_transcript_editor(None, cx);
        self.editing_speaker = None;
        cx.notify();
    }

    // ── Live pill ────────────────────────────────────────────────────────

    pub(super) fn open_meeting_dock(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.meetings_hub.read(cx).live_meeting().is_none() {
            return;
        }
        if let Some(dock) = self.meeting_dock {
            // Re-entering from the workspace also recovers a pill left on a
            // disconnected monitor. Do not reuse unverified screen coordinates.
            let _ = dock.update(cx, |_, dock, _| dock.remove_window());
        }
        match crate::views::meeting_dock::open(self.meetings_hub.clone(), window, cx) {
            Ok(dock) => self.meeting_dock = Some(dock),
            Err(error) => {
                tracing::warn!("could not open the meeting dock: {error}");
                self.meetings_hub.update(cx, |hub, cx| {
                    hub.show_notice(
                        format!("Could not open the meeting dock: {error}"),
                        true,
                        cx,
                    )
                });
            }
        }
    }

    /// Web `MeetingLive`: bottom-centre pill (dot · elapsed · levels ·
    /// title · Stop) with the live segments above it.
    pub(super) fn render_live_meeting_pill(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let hub = self.meetings_hub.read(cx);
        let meeting = hub.live_meeting()?;
        let has_segments = hub
            .segments
            .get(&meeting.id)
            .is_some_and(|segments| !segments.is_empty());
        let elapsed = format_elapsed((EpochMs::now().as_f64() - meeting.started_at_ms).max(0.0));
        let mut stack = div()
            .absolute()
            .bottom(px(16.0))
            .left_0()
            .right_0()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .occlude();
        if has_segments {
            stack = stack.child(
                div()
                    .id("desktop-live-segments")
                    .w(px(LIVE_PANEL_WIDTH_PX))
                    .max_h(px(LIVE_PANEL_MAX_HEIGHT_PX))
                    .rounded(px(8.0))
                    .bg(theme::longan_900().opacity(0.95))
                    .border_1()
                    .border_color(theme::longan_700())
                    .px_3()
                    .py_2()
                    .child(self.render_transcript_list(&meeting.id, false, cx)),
            );
        }
        let levels = hub.audio_meter.clone();
        stack = stack.child(
            div()
                .debug_selector(|| "desktop-live-pill".to_string())
                .flex()
                .items_center()
                .gap_3()
                .rounded(px(8.0))
                .bg(theme::longan_900().opacity(0.95))
                .border_1()
                .border_color(theme::longan_700())
                .px_4()
                .py(px(10.0))
                .child(
                    div()
                        .size(px(8.0))
                        .rounded_full()
                        .bg(theme::persimmon_500()),
                )
                .child(
                    label(elapsed, LABEL_PX, theme::persimmon_400())
                        .font(theme::tabular_system_font()),
                )
                .child(levels)
                .child(
                    web_button(
                        "desktop-live-dock",
                        WebButtonVariant::Ghost,
                        WebButtonSize::Default,
                        ButtonState::Enabled,
                    )
                    .aria_label("Compact meeting dock")
                    .on_click(cx.listener(|this, _, window, cx| this.open_meeting_dock(window, cx)))
                    .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(&ev.keystroke.key) {
                            cx.stop_propagation();
                            this.open_meeting_dock(window, cx);
                        }
                    }))
                    .child("Dock"),
                )
                .child(body(
                    meeting.title.clone(),
                    BODY_SM_PX,
                    theme::rambutan_100(),
                ))
                .child(
                    web_button(
                        "desktop-live-stop",
                        WebButtonVariant::Ghost,
                        WebButtonSize::Default,
                        ButtonState::Enabled,
                    )
                    .aria_label("Stop recording")
                    .on_click(cx.listener(|this, _ev, _window, cx| {
                        this.meetings_hub.update(cx, |hub, cx| hub.stop_live(cx));
                    }))
                    .child("Stop"),
                ),
        );
        Some(stack.into_any_element())
    }

    // ── Search page ──────────────────────────────────────────────────────

    fn build_universal_search_index(&self, cx: &Context<'_, Self>) -> SearchIndex {
        let hub = self.meetings_hub.read(cx);
        let mut documents = Vec::new();
        let project_names = self
            .projects
            .iter()
            .map(|project| (project.id.as_str(), project.name.as_str()))
            .collect::<HashMap<_, _>>();
        let mut project_task_counts = HashMap::<&str, usize>::new();
        for item in &self.work_items {
            if let Some(project_id) = item.project_id.as_deref() {
                *project_task_counts.entry(project_id).or_default() += 1;
            }
        }

        documents.extend(self.threads.iter().map(|thread| {
            let chat = thread.chat.read(cx);
            SearchDocument::new(
                &thread.id,
                SearchKind::Thread,
                &chat.thread_title,
                &thread.project_name,
                search_join([
                    thread.branch.clone().unwrap_or_default(),
                    thread.project_path.clone(),
                ]),
                thread.updated_at_ms.as_millis(),
            )
        }));
        documents.extend(self.work_items.iter().map(|item| {
            SearchDocument::new(
                &item.id,
                SearchKind::Task,
                &item.title,
                item.project_id
                    .as_deref()
                    .and_then(|id| project_names.get(id).copied())
                    .unwrap_or("No project"),
                search_join([
                    item.description.clone().unwrap_or_default(),
                    item.due_date.clone().unwrap_or_default(),
                    item.status.label().to_string(),
                    item.priority.label().to_string(),
                    item.planning.cycle.clone().unwrap_or_default(),
                    item.planning.waiting_on.clone().unwrap_or_default(),
                    item.planning
                        .recurrence
                        .map_or_else(String::new, |rule| rule.label().to_string()),
                ]),
                item.updated_at.as_millis(),
            )
        }));
        documents.extend(self.projects.iter().map(|project| {
            let task_count = project_task_counts
                .get(project.id.as_str())
                .copied()
                .unwrap_or_default();
            SearchDocument::new(
                &project.id,
                SearchKind::Project,
                &project.name,
                format!(
                    "{task_count} task{}",
                    if task_count == 1 { "" } else { "s" }
                ),
                "",
                0,
            )
        }));
        documents.extend(self.calendar_events.iter().map(|event| {
            SearchDocument::new(
                &event.id,
                SearchKind::Event,
                &event.title,
                if event.all_day {
                    "All-day event"
                } else {
                    "Calendar event"
                },
                event.notes.clone().unwrap_or_default(),
                event.start_at.and_utc().timestamp_millis(),
            )
        }));
        documents.extend(self.scheduling_preferences.links.iter().map(|link| {
            SearchDocument::new(
                &link.id,
                SearchKind::SchedulingLink,
                &link.title,
                format!("{} minutes · #/book/{}", link.duration_minutes, link.slug),
                format!(
                    "{} hour notice · {} day window",
                    link.minimum_notice_hours, link.booking_window_days
                ),
                self.scheduling_preferences.updated_at_ms,
            )
        }));
        documents.extend(self.life_context.finance.accounts.iter().map(|account| {
            SearchDocument::new(
                &account.id,
                SearchKind::Finance,
                &account.name,
                format!("{} account", account.kind.label()),
                account.institution.clone().unwrap_or_default(),
                account.updated_at_ms,
            )
        }));
        documents.extend(
            self.life_context
                .finance
                .transactions
                .iter()
                .map(|transaction| {
                    SearchDocument::new(
                        &transaction.id,
                        SearchKind::Finance,
                        &transaction.merchant,
                        &transaction.category,
                        transaction.note.clone().unwrap_or_default(),
                        transaction.occurred_at_ms,
                    )
                }),
        );
        documents.extend(self.life_context.finance.budgets.iter().map(|budget| {
            SearchDocument::new(
                &budget.id,
                SearchKind::Finance,
                &budget.category,
                "Monthly budget",
                "",
                self.life_context.finance.updated_at_ms,
            )
        }));
        documents.extend(
            self.life_context
                .finance
                .income_streams
                .iter()
                .map(|income| {
                    SearchDocument::new(
                        &income.id,
                        SearchKind::Finance,
                        &income.name,
                        "Income stream",
                        "",
                        self.life_context.finance.updated_at_ms,
                    )
                }),
        );
        documents.extend(self.life_context.finance.holdings.iter().map(|holding| {
            SearchDocument::new(
                &holding.id,
                SearchKind::Finance,
                &holding.symbol,
                "Holding",
                &holding.name,
                holding.updated_at_ms,
            )
        }));
        documents.extend(self.life_context.health.workouts.iter().map(|workout| {
            SearchDocument::new(
                &workout.id,
                SearchKind::Health,
                &workout.activity,
                "Workout",
                format!("{} minutes", workout.duration_minutes),
                workout.occurred_at_ms,
            )
        }));
        documents.extend(self.life_context.health.sleep.iter().map(|sleep| {
            SearchDocument::new(
                &sleep.id,
                SearchKind::Health,
                "Sleep log",
                format!("{} minutes", sleep.duration_minutes),
                format!("{}% quality", sleep.quality_percent),
                sleep.ended_at_ms,
            )
        }));
        documents.extend(self.life_context.health.recovery.iter().map(|recovery| {
            SearchDocument::new(
                &recovery.id,
                SearchKind::Health,
                "Recovery log",
                format!("{}%", recovery.score_percent),
                recovery.note.clone().unwrap_or_default(),
                recovery.recorded_at_ms,
            )
        }));
        documents.extend(self.life_context.health.habits.iter().map(|habit| {
            SearchDocument::new(
                &habit.id,
                SearchKind::Health,
                &habit.title,
                "Habit",
                format!("{} days/week", habit.target_days_per_week),
                self.life_context.health.updated_at_ms,
            )
        }));
        documents.extend(hub.google_events.iter().map(|event| {
            SearchDocument::new(
                &event.id,
                SearchKind::Event,
                &event.title,
                "Google Calendar",
                search_join(
                    event
                        .notes
                        .clone()
                        .into_iter()
                        .chain(event.location.clone())
                        .chain(event.attendees.iter().map(|attendee| {
                            attendee
                                .display_name
                                .clone()
                                .unwrap_or_else(|| attendee.email.clone())
                        })),
                ),
                event.updated_at,
            )
        }));
        documents.extend(hub.meetings.iter().map(|meeting| {
            let summary = meeting
                .summary
                .as_ref()
                .map_or_else(String::new, |summary| {
                    search_join(
                        std::iter::once(summary.tldr.clone())
                            .chain(summary.decisions.iter().cloned())
                            .chain(summary.action_items.iter().map(|item| item.text.clone()))
                            .chain(summary.topics.iter().flat_map(|topic| {
                                std::iter::once(topic.title.clone())
                                    .chain(topic.bullets.iter().cloned())
                            })),
                    )
                });
            SearchDocument::new(
                &meeting.id,
                SearchKind::Meeting,
                &meeting.title,
                match meeting.status {
                    MeetingStatus::Live => "Live meeting",
                    MeetingStatus::Ended => "Meeting",
                    MeetingStatus::Failed => "Interrupted meeting",
                },
                summary,
                meeting.updated_at,
            )
        }));
        documents.extend(self.meeting_notes.iter().map(SearchDocument::from));
        documents.extend(hub.people.iter().map(|person| {
            SearchDocument::new(
                &person.id,
                SearchKind::Person,
                &person.display_name,
                format!(
                    "{} observation{}",
                    person.observations.len(),
                    if person.observations.len() == 1 {
                        ""
                    } else {
                        "s"
                    }
                ),
                search_join(
                    person
                        .aliases
                        .iter()
                        .cloned()
                        .chain(person.observations.iter().map(|item| item.text.clone())),
                ),
                person.updated_at,
            )
        }));
        documents.extend(self.captures.iter().filter_map(|record| {
            if record.target_id.is_some() {
                return None;
            }
            let (kind, subtitle) = match record.kind {
                CaptureKind::Note => (SearchKind::Document, "Captured note"),
                CaptureKind::MessageDraft => (SearchKind::Conversation, "Message draft"),
                CaptureKind::Task | CaptureKind::Event | CaptureKind::Contact => return None,
            };
            Some(SearchDocument::new(
                &record.id,
                kind,
                &record.title,
                subtitle,
                &record.body,
                record.updated_at_ms,
            ))
        }));
        documents.extend(self.inbox_conversations.iter().map(|conversation| {
            SearchDocument::new(
                &conversation.id,
                SearchKind::Conversation,
                &conversation.subject,
                format!(
                    "{} · {}",
                    conversation.source.label(),
                    conversation
                        .participants
                        .iter()
                        .map(|participant| participant.display_name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                search_join(
                    conversation
                        .messages
                        .iter()
                        .map(|message| message.body.clone()),
                ),
                conversation.updated_at_ms,
            )
        }));
        documents.extend(self.knowledge_documents.iter().map(|document| {
            SearchDocument::new(
                &document.id,
                SearchKind::Document,
                &document.title,
                document
                    .tags
                    .iter()
                    .map(|tag| format!("#{tag}"))
                    .collect::<Vec<_>>()
                    .join(" "),
                search_join(
                    std::iter::once(document.body_markdown.clone())
                        .chain(document.links.iter().map(|link| link.label.clone())),
                ),
                document.updated_at_ms,
            )
        }));
        documents.extend(self.source_clips.iter().map(|clip| {
            SearchDocument::new(
                &clip.id,
                SearchKind::Clip,
                &clip.title,
                &clip.source_url,
                search_join([
                    clip.excerpt.clone(),
                    clip.source_name.clone(),
                    clip.author.clone(),
                    clip.tags.join(" "),
                ]),
                clip.updated_at_ms,
            )
        }));

        let meeting_metadata = hub
            .meetings
            .iter()
            .map(|meeting| {
                (
                    meeting.id.as_str(),
                    (meeting.title.as_str(), meeting.updated_at),
                )
            })
            .collect::<HashMap<_, _>>();
        let mut seen_segments = HashSet::new();
        for segment in hub
            .segments
            .values()
            .flatten()
            .filter(|segment| !segment.partial)
        {
            if !seen_segments.insert(segment.id.as_str()) {
                continue;
            }
            let (title, updated_at) = meeting_metadata
                .get(segment.meeting_id.as_str())
                .copied()
                .unwrap_or(("Unknown meeting", 0));
            documents.push(SearchDocument::new(
                &segment.meeting_id,
                SearchKind::Transcript,
                title,
                "Meeting transcript",
                &segment.text,
                updated_at,
            ));
        }

        SearchIndex::new(documents)
    }

    pub(super) fn universal_search(
        &self,
        query: &str,
        cx: &Context<'_, Self>,
    ) -> (usize, Vec<SearchHit>) {
        let mut revision = DefaultHasher::new();
        self.meetings_hub
            .read(cx)
            .search_revision()
            .hash(&mut revision);
        self.threads.len().hash(&mut revision);
        self.work_items.len().hash(&mut revision);
        self.projects.len().hash(&mut revision);
        self.calendar_events.len().hash(&mut revision);
        self.scheduling_preferences
            .updated_at_ms
            .hash(&mut revision);
        self.life_context.finance.updated_at_ms.hash(&mut revision);
        self.life_context.health.updated_at_ms.hash(&mut revision);
        self.captures.len().hash(&mut revision);
        self.meeting_notes.len().hash(&mut revision);
        for document in &self.knowledge_documents {
            document.id.hash(&mut revision);
            document.updated_at_ms.hash(&mut revision);
        }
        for clip in &self.source_clips {
            clip.id.hash(&mut revision);
            clip.updated_at_ms.hash(&mut revision);
        }
        let source_revision = revision.finish();
        let needs_rebuild = self
            .universal_search_index
            .borrow()
            .as_ref()
            .is_none_or(|(cached, _)| *cached != source_revision);
        if needs_rebuild {
            let index = self.build_universal_search_index(cx);
            *self.universal_search_index.borrow_mut() = Some((source_revision, index));
        }
        let cache = self.universal_search_index.borrow();
        let Some((_, index)) = cache.as_ref() else {
            return (0, Vec::new());
        };
        (index.len(), index.search(query, SEARCH_LIMIT))
    }

    pub(super) fn open_universal_search_hit(
        &mut self,
        hit: SearchHit,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        use super::search_record_detail::SelectedSearchRecord;

        self.selected_search_record = None;
        match hit.document.kind {
            SearchKind::Thread => self.select_thread_by_id(&hit.document.id, cx),
            SearchKind::Task => {
                self.active_view = AppView::Tasks;
                self.selected_project_id = None;
                self.close_transient_surfaces();
                self.select_work_item_by_id(&hit.document.id, window, cx);
            }
            SearchKind::Project => {
                self.selected_project_id = Some(hit.document.id);
                self.selected_work_item_id = None;
                self.active_view = AppView::Tasks;
                self.close_transient_surfaces();
                self.persist_and_notify(cx);
            }
            SearchKind::Event => {
                if self
                    .calendar_events
                    .iter()
                    .any(|event| event.id == hit.document.id)
                {
                    self.open_event_edit(hit.document.id, window, cx);
                } else {
                    self.open_search_record(
                        SelectedSearchRecord::ExternalEvent(hit.document.id),
                        window,
                        cx,
                    );
                }
            }
            SearchKind::SchedulingLink => {
                self.open_search_record(
                    SelectedSearchRecord::SchedulingLink(hit.document.id),
                    window,
                    cx,
                );
            }
            SearchKind::Meeting | SearchKind::Transcript => {
                self.open_meeting_detail(hit.document.id, cx);
            }
            SearchKind::MeetingNote => self.open_local_meeting_note(hit.document.id, window, cx),
            SearchKind::Person => {
                self.activate_sidebar_view(AppView::People, window, cx);
                self.expanded_person_id = Some(hit.document.id);
                cx.notify();
            }
            SearchKind::Document => {
                if self
                    .knowledge_documents
                    .iter()
                    .any(|document| document.id == hit.document.id)
                {
                    self.select_knowledge_document(&hit.document.id, window, cx);
                } else {
                    self.open_capture_record(hit.document.id, window, cx);
                }
            }
            SearchKind::Conversation => {
                if self
                    .inbox_conversations
                    .iter()
                    .any(|conversation| conversation.id == hit.document.id)
                {
                    self.select_inbox_conversation(&hit.document.id, window, cx);
                } else {
                    self.open_capture_record(hit.document.id, window, cx);
                }
            }
            SearchKind::Clip => self.select_source_clip(&hit.document.id, window, cx),
            SearchKind::Finance => {
                self.open_search_record(
                    SelectedSearchRecord::Life(bruhs_models::SelectedLifeRecord {
                        section: DesktopLifeTab::Finance,
                        id: hit.document.id,
                    }),
                    window,
                    cx,
                );
            }
            SearchKind::Health => {
                self.open_search_record(
                    SelectedSearchRecord::Life(bruhs_models::SelectedLifeRecord {
                        section: DesktopLifeTab::Health,
                        id: hit.document.id,
                    }),
                    window,
                    cx,
                );
            }
        }
    }

    /// Web `SearchPage`: ranked local results across every hydrated domain.
    pub(super) fn render_search_page(&self, window: &Window, cx: &Context<'_, Self>) -> AnyElement {
        let search_focused = self.search_query_input.focus_handle(cx).is_focused(window);
        let query = self.search_query_input.read(cx).value().to_string();
        let (indexed_count, results) = self.universal_search(&query, cx);
        let transcripts_loading = self.meetings_hub.read(cx).search_loading;

        let status: AnyElement = if query.trim().is_empty() {
            div()
                .rounded(px(8.0))
                .border_1()
                .border_color(theme::longan_800())
                .bg(theme::longan_900().opacity(0.3))
                .px_4()
                .py_3()
                .child(body(
                    "Find anything local instantly. Results open the exact record—no extra navigation.",
                    BODY_SM_PX,
                    theme::rambutan_500(),
                ))
                .into_any_element()
        } else if results.is_empty() {
            body(
                format!("No results for “{}”.", query.trim()),
                BODY_SM_PX,
                theme::rambutan_500(),
            )
            .into_any_element()
        } else {
            div()
                .id("desktop-search-results")
                .role(Role::ListBox)
                .aria_label("Search results")
                .flex()
                .flex_col()
                .gap_1()
                .children(results.into_iter().enumerate().map(|(index, hit)| {
                    let title = hit.document.title.clone();
                    let subtitle = hit.document.subtitle.clone();
                    let preview = search_preview(&hit.document.body);
                    let kind = hit.document.kind.label();
                    let icon = search_result_icon(hit.document.kind);
                    let aria_label = if subtitle.is_empty() {
                        format!("{kind}: {title}")
                    } else {
                        format!("{kind}: {title}, {subtitle}")
                    };
                    div()
                        .id(("desktop-search-hit", index))
                        .debug_selector(move || format!("desktop-search-hit-{index}"))
                        .w_full()
                        .rounded(px(8.0))
                        .px_3()
                        .py(px(10.0))
                        .flex()
                        .items_start()
                        .gap_3()
                        .tab_index(0)
                        .role(Role::ListBoxOption)
                        .aria_label(aria_label)
                        .cursor_pointer()
                        .focus_visible(|element| {
                            element.border_2().border_color(theme::persimmon_400())
                        })
                        .hover(|element| {
                            element
                                .bg(theme::longan_900().opacity(0.7))
                                .border_1()
                                .border_color(theme::longan_700())
                        })
                        .on_click(cx.listener(move |this, _event, window, cx| {
                            this.open_universal_search_hit(hit.clone(), window, cx);
                        }))
                        .child(
                            div()
                                .mt(px(2.0))
                                .size(px(28.0))
                                .flex_shrink_0()
                                .rounded(px(4.0))
                                .border_1()
                                .border_color(theme::ring_card())
                                .bg(theme::longan_800().opacity(0.7))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    svg()
                                        .path(icon)
                                        .size(px(14.0))
                                        .text_color(theme::rambutan_400()),
                                ),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            label(title, LABEL_PX, theme::rambutan_100())
                                                .min_w_0()
                                                .truncate(),
                                        )
                                        .child(label(kind, LABEL_SM_PX, theme::rambutan_500())),
                                )
                                .when(!subtitle.is_empty(), |element| {
                                    element.child(
                                        label(subtitle, LABEL_SM_PX, theme::blueberry_400())
                                            .truncate(),
                                    )
                                })
                                .when(!preview.is_empty(), |element| {
                                    element.child(
                                        body(preview, BODY_SM_PX, theme::rambutan_500()).truncate(),
                                    )
                                }),
                        )
                }))
                .into_any_element()
        };

        div()
            .id("desktop-search-page")
            .debug_selector(|| "desktop-search-page".to_string())
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll()
            .bg(theme::longan_950())
            .text_color(theme::rambutan_300())
            .child(
                div()
                    .w_full()
                    .max_w(px(PAGE_MAX_WIDTH_PX))
                    .mx_auto()
                    .px_6()
                    .py_4()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        ring_text_button(
                                            "desktop-search-back",
                                            "← Chat",
                                            theme::rambutan_400(),
                                            theme::rambutan_200(),
                                        )
                                        .on_click(
                                            cx.listener(|this, _event, window, cx| {
                                                this.activate_sidebar_view(
                                                    AppView::Chat,
                                                    window,
                                                    cx,
                                                );
                                            }),
                                        ),
                                    )
                                    .child(body(
                                        "Search everything",
                                        BODY_PX,
                                        theme::rambutan_100(),
                                    )),
                            )
                            .child(label(
                                format!("{indexed_count} indexed"),
                                LABEL_SM_PX,
                                theme::rambutan_500(),
                            )),
                    )
                    .child(
                        div()
                            .w_full()
                            .h(px(40.0))
                            .rounded(px(8.0))
                            .bg(theme::control_surface())
                            .map(|element| {
                                if search_focused {
                                    element
                                        .border_2()
                                        .border_color(theme::persimmon_400())
                                        .px(px(11.0))
                                } else {
                                    element.border_1().border_color(theme::ring_card()).px_3()
                                }
                            })
                            .flex()
                            .items_center()
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&self.search_query_input)
                                        .appearance(false)
                                        .aria_label("Search everything"),
                                ),
                            )
                            .when(!query.is_empty(), |element| {
                                element.child(
                                    div()
                                        .id("desktop-search-clear")
                                        .debug_selector(|| "desktop-search-clear".to_string())
                                        .size(px(16.0))
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .cursor_pointer()
                                        .role(Role::Button)
                                        .aria_label("Clear search")
                                        .tab_index(0)
                                        .focus_visible(|element| {
                                            element.border_1().border_color(theme::persimmon_400())
                                        })
                                        .on_click(cx.listener(|this, _event, window, cx| {
                                            this.search_query_input.update(cx, |input, cx| {
                                                input.set_value("", window, cx)
                                            });
                                            cx.notify();
                                        }))
                                        .child(
                                            svg()
                                                .path(assets::CLOSE_ICON_PATH)
                                                .size(px(12.0))
                                                .text_color(theme::rambutan_100()),
                                        ),
                                )
                            }),
                    )
                    .child(status)
                    .when(transcripts_loading, |element| {
                        element.child(label(
                            "Adding transcript history…",
                            LABEL_SM_PX,
                            theme::rambutan_500(),
                        ))
                    }),
            )
            .into_any_element()
    }

    // ── People page ──────────────────────────────────────────────────────

    fn load_relationship_inputs(
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
        self.relationship_item_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.relationship_item_kind = RelationshipItemKind::Interaction;
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
        let company = self
            .relationship_company_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let role = self
            .relationship_role_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let email = self
            .relationship_email_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let phone = self
            .relationship_phone_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let location = self
            .relationship_location_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let notes = self
            .relationship_notes_input
            .read(cx)
            .value()
            .trim()
            .to_string();
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

    fn cycle_relationship_item_kind(&mut self, cx: &mut Context<'_, Self>) {
        self.relationship_item_kind = match self.relationship_item_kind {
            RelationshipItemKind::Interaction => RelationshipItemKind::Commitment,
            RelationshipItemKind::Commitment => RelationshipItemKind::Reminder,
            RelationshipItemKind::Reminder => RelationshipItemKind::Note,
            RelationshipItemKind::Note => RelationshipItemKind::Interaction,
        };
        cx.notify();
    }

    fn add_relationship_item(
        &mut self,
        person_id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let text = self.relationship_item_input.read(cx).value().to_string();
        let now = EpochMs::now().as_millis();
        let mut item = bruhs_models::RelationshipItem::new(self.relationship_item_kind, text, now);
        if item.text.is_empty() {
            return;
        }
        if matches!(
            item.kind,
            RelationshipItemKind::Commitment | RelationshipItemKind::Reminder
        ) {
            item.due_at_ms = Some(now + 86_400_000);
        }
        self.relationship_profile_mut(person_id).add_item(item, now);
        self.relationship_item_input
            .update(cx, |input, cx| input.set_value("", window, cx));
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

    /// Web `PeoplePage`.
    pub(super) fn render_people_page(&self, cx: &Context<'_, Self>) -> AnyElement {
        let hub = self.meetings_hub.read(cx);
        let mut people = hub.people.clone();
        people.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
        let enroll = hub.enroll.clone();
        let mut page = div()
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
        if hub.needs_people_retry() {
            page = page.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(label(
                        if hub.pending_people_count() == 0 {
                            "Contact refresh needs attention. Local records are retained.".into()
                        } else { format!(
                            "{} contact(s) awaiting sync or merge recovery. Check local save status.",
                            hub.pending_people_count()
                        ) },
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
        if people.is_empty() {
            page = page.child(label(
                "No people yet. Profiles populate after your first meeting summary completes.",
                LABEL_PX,
                theme::rambutan_500(),
            ));
        } else {
            let all = people.clone();
            page = page.child(
                div().flex().flex_col().gap_2().children(
                    people
                        .into_iter()
                        .enumerate()
                        .map(|(i, p)| self.render_person_card(i, p, &all, cx)),
                ),
            );
        }
        if let Some((person_id, state)) = enroll {
            let name = hub
                .people
                .iter()
                .find(|p| p.id == person_id)
                .map_or_else(String::new, |p| p.display_name.clone());
            page = page.child(self.render_enroll_modal(name, state, cx));
        }
        page.into_any_element()
    }

    fn render_person_card(
        &self,
        index: usize,
        profile: PersonProfile,
        all: &[PersonProfile],
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let id = profile.id.clone();
        let expanded = self.expanded_person_id.as_deref() == Some(id.as_str());
        let renaming = self.renaming_person_id.as_deref() == Some(id.as_str());
        let merge_open = self
            .merge
            .as_ref()
            .is_some_and(|merge| merge.person_id == id);
        let tracked = profile.tracked;
        let obs_count = profile.observations.len();
        let display_name = profile.display_name.clone();
        let expand_label = format!("Expand {display_name}");

        let id_toggle = id.clone();
        let (id_rename, name_rename) = (id.clone(), display_name.clone());
        let id_track = id.clone();
        let id_merge = id.clone();
        let id_enroll = id.clone();

        let name_slot: AnyElement = if renaming {
            div()
                .w_full()
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
            body(display_name, BODY_SM_PX, theme::rambutan_200()).into_any_element()
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
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .id(("desktop-person-expand", index))
                            .debug_selector(move || format!("desktop-person-expand-{index}"))
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            // Web renders this expander as a <button>.
                            .role(Role::Button)
                            .aria_label(expand_label)
                            .aria_expanded(expanded)
                            .tab_index(0)
                            .focus_visible(|el| el.border_1().border_color(theme::persimmon_400()))
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
                                cx.notify();
                            }))
                            .child(name_slot)
                            .when(!tracked, |el| {
                                el.child(
                                    div()
                                        .rounded(px(4.0))
                                        .bg(theme::longan_800())
                                        .border_1()
                                        .border_color(theme::longan_600())
                                        .px_1()
                                        .py(px(2.0))
                                        .font_family(LABEL_FONT)
                                        .text_size(px(LABEL_SM_PX))
                                        .text_color(theme::rambutan_300())
                                        .child("UNTRACKED"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
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
                                    if tracked { "track" } else { "untrack" },
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
                                .on_click(cx.listener(
                                    move |this, _ev, _window, cx| {
                                        let pid = id_track.clone();
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
                                .on_click(cx.listener(
                                    move |this, _ev, _window, cx| {
                                        let pid = id_enroll.clone();
                                        this.meetings_hub
                                            .update(cx, |hub, cx| hub.open_enroll(pid, cx));
                                    },
                                )),
                            ),
                    ),
            );
        if merge_open {
            card = card.child(self.render_merge_panel(&id, all, cx));
        }
        if expanded {
            card = card
                .child(self.render_relationship_panel(&id, cx))
                .child(self.render_observations_by_kind(&profile.observations));
        }
        card.into_any_element()
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
            .map(|(index, item)| {
                let person_id = person_id.to_string();
                let item_id = item.id.clone();
                let person_id_key = person_id.clone();
                let item_id_key = item_id.clone();
                let complete = item.completed_at_ms.is_some();
                div()
                    .id(("relationship-item", index))
                    .debug_selector(move || format!("desktop-relationship-item-{index}"))
                    .role(Role::Button)
                    .aria_label(format!("Toggle {}", item.text))
                    .tab_index(0)
                    .cursor_pointer()
                    .rounded(px(4.0))
                    .px_2()
                    .py_1()
                    .hover(|element| element.bg(theme::longan_800().opacity(0.7)))
                    .focus_visible(|element| {
                        element.border_1().border_color(theme::persimmon_400())
                    })
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.toggle_relationship_item(&person_id, &item_id, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            this.toggle_relationship_item(&person_id_key, &item_id_key, cx);
                        }
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
                            .child(item.kind.label()),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let suggestions = bruhs_models::suggested_follow_ups(
            &self.relationships,
            &self.inbox_conversations,
            EpochMs::now().as_millis(),
        )
        .into_iter()
        .filter(|suggestion| suggestion.person_id == person_id)
        .map(|suggestion| {
            div()
                .text_size(px(11.0))
                .text_color(theme::persimmon_300())
                .child(format!("→ {}", suggestion.reason))
                .into_any_element()
        })
        .collect::<Vec<_>>();

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
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        web_button(
                            ("relationship-kind", element_key(person_id)),
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child(self.relationship_item_kind.label())
                        .on_click(cx.listener(
                            |this, _event, _window, cx| this.cycle_relationship_item_kind(cx),
                        )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.relationship_item_input)),
                    )
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

    /// Web `MergePanel`.
    fn render_merge_panel(
        &self,
        source_id: &str,
        all: &[PersonProfile],
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
                }
                cx.notify();
            });
        });
        let (value, dropdown_open) = self.merge.as_ref().map_or((String::new(), false), |merge| {
            (
                merge.target_id.clone().unwrap_or_default(),
                merge.dropdown_open,
            )
        });
        let can_merge = !value.is_empty();
        let source = source_id.to_string();
        div()
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
            ))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        text_button(
                            "desktop-people-merge-cancel",
                            "Cancel",
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
                        .when(!can_merge, |el| el.opacity(0.4).cursor_default())
                        .when(can_merge, |el| {
                            el.on_click(cx.listener(move |this, _ev, _window, cx| {
                                let Some(target) = this
                                    .merge
                                    .as_ref()
                                    .and_then(|merge| merge.target_id.clone())
                                else {
                                    return;
                                };
                                this.merge = None;
                                let source = source.clone();
                                this.meetings_hub
                                    .update(cx, |hub, cx| hub.merge_people(source, target, cx));
                            }))
                        }),
                    ),
            )
            .into_any_element()
    }

    /// Web `ObservationsByKind`.
    fn render_observations_by_kind(&self, observations: &[PersonObservation]) -> AnyElement {
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
                        label(format!("• {}", o.text), LABEL_PX, theme::rambutan_300())
                            .line_height(relative(1.375))
                    }))
            }))
            .into_any_element()
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

    pub(super) fn commit_person_rename(&mut self, cx: &mut Context<'_, Self>) {
        let Some(person_id) = self.renaming_person_id.take() else {
            return;
        };
        let name = self.person_rename_input.read(cx).value().to_string();
        if !name.trim().is_empty() {
            self.meetings_hub.update(cx, |hub, cx| {
                hub.rename_person(person_id, name.trim().to_string(), cx)
            });
        }
        cx.notify();
    }

    pub(super) fn cancel_person_rename(&mut self, cx: &mut Context<'_, Self>) {
        self.renaming_person_id = None;
        cx.notify();
    }

    /// Web `VoiceEnrollModal`.
    fn render_enroll_modal(
        &self,
        person_name: String,
        state: EnrollState,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let busy = matches!(state, EnrollState::Recording | EnrollState::Uploading);
        let status: AnyElement = match &state {
            EnrollState::Idle => {
                label("Click Record to begin.", LABEL_PX, theme::rambutan_300()).into_any_element()
            }
            EnrollState::Countdown(n) => div()
                .flex()
                .flex_col()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .font_family("Monaspace Xenon")
                        .text_size(px(DISPLAY_SM_PX))
                        .line_height(relative(1.0))
                        .text_color(theme::persimmon_400())
                        .child(n.to_string()),
                )
                .child(label("Get ready…", LABEL_PX, theme::rambutan_400()))
                .into_any_element(),
            EnrollState::Recording => div()
                .flex()
                .flex_col()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .size(px(10.0))
                                .rounded_full()
                                .bg(theme::persimmon_500()),
                        )
                        .child(body("Recording 5s…", BODY_SM_PX, theme::persimmon_400())),
                )
                .child(label(
                    "Speak clearly and naturally.",
                    LABEL_SM_PX,
                    theme::rambutan_300(),
                ))
                .into_any_element(),
            EnrollState::Uploading => {
                label("Uploading…", LABEL_PX, theme::rambutan_400()).into_any_element()
            }
            EnrollState::Success => body(
                "✓ Voice enrolled successfully",
                BODY_SM_PX,
                theme::pandan_400(),
            )
            .into_any_element(),
            EnrollState::Error(message) => label(message.clone(), LABEL_PX, theme::lychee_400())
                .text_center()
                .into_any_element(),
        };
        let is_idle = state == EnrollState::Idle;
        let is_error = matches!(state, EnrollState::Error(_));
        div()
            .id("desktop-enroll-overlay")
            .debug_selector(|| "desktop-enroll-overlay".to_string())
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .occlude()
            .bg(theme::longan_950().opacity(0.8))
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .on_click(cx.listener(|this, _ev, _window, cx| {
                this.meetings_hub.update(cx, |hub, cx| hub.close_enroll(cx));
            }))
            .child(
                div()
                    .id("desktop-enroll-dialog")
                    .role(Role::Dialog)
                    .aria_label("Enroll voice")
                    .w_full()
                    .max_w(px(384.0))
                    .rounded(px(8.0))
                    .bg(theme::longan_900())
                    .border_1()
                    .border_color(theme::longan_700())
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .on_click(cx.listener(|_this, _ev, _window, cx| cx.stop_propagation()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                label("Enroll voice", H4_PX, theme::rambutan_100())
                                    .font_weight(FontWeight::MEDIUM),
                            )
                            .child(
                                text_button(
                                    "desktop-enroll-close",
                                    "✕",
                                    theme::rambutan_500(),
                                    theme::rambutan_300(),
                                )
                                .text_size(px(LABEL_PX))
                                .on_click(cx.listener(
                                    |this, _ev, _window, cx| {
                                        this.meetings_hub
                                            .update(cx, |hub, cx| hub.close_enroll(cx));
                                    },
                                )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .font_family(LABEL_FONT)
                            .text_size(px(LABEL_PX))
                            .text_color(theme::rambutan_400())
                            .child("Record a clean 5-second voice sample for ")
                            .child(div().text_color(theme::rambutan_200()).child(person_name))
                            .child(
                                ". Speak naturally. This replaces any existing voice reference.",
                            ),
                    )
                    .child(
                        div()
                            .min_h(px(60.0))
                            .py_3()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap_3()
                            .child(status),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .when(!busy, |el| {
                                el.child(
                                    div()
                                        .id("desktop-enroll-cancel")
                                        .px_3()
                                        .py_1()
                                        .rounded(px(4.0))
                                        .font_family(LABEL_FONT)
                                        .text_size(px(LABEL_PX))
                                        .text_color(theme::rambutan_400())
                                        .tab_index(0)
                                        .focus_visible(|el| {
                                            el.border_1().border_color(theme::persimmon_400())
                                        })
                                        .role(Role::Button)
                                        .aria_label("Cancel")
                                        .cursor_pointer()
                                        .hover(|el| el.text_color(theme::rambutan_200()))
                                        .on_click(cx.listener(|this, _ev, _window, cx| {
                                            this.meetings_hub
                                                .update(cx, |hub, cx| hub.close_enroll(cx));
                                        }))
                                        .child("Cancel"),
                                )
                            })
                            .when(is_idle, |el| {
                                el.child(
                                    div()
                                        .id("desktop-enroll-record")
                                        .debug_selector(|| "desktop-enroll-record".to_string())
                                        .px_3()
                                        .py_1()
                                        .rounded(px(4.0))
                                        .bg(theme::persimmon_600())
                                        .font_family(LABEL_FONT)
                                        .text_size(px(LABEL_PX))
                                        .text_color(gpui::white())
                                        .tab_index(0)
                                        .focus_visible(|el| {
                                            el.border_1().border_color(theme::persimmon_400())
                                        })
                                        .role(Role::Button)
                                        .aria_label("Record a 5 second sample")
                                        .cursor_pointer()
                                        .hover(|el| el.bg(theme::persimmon_500()))
                                        .on_click(cx.listener(|this, _ev, _window, cx| {
                                            this.meetings_hub
                                                .update(cx, |hub, cx| hub.start_enrollment(cx));
                                        }))
                                        .child("Record (5s)"),
                                )
                            })
                            .when(is_error, |el| {
                                el.child(
                                    div()
                                        .id("desktop-enroll-retry")
                                        .px_3()
                                        .py_1()
                                        .rounded(px(4.0))
                                        .bg(theme::longan_800())
                                        .font_family(LABEL_FONT)
                                        .text_size(px(LABEL_PX))
                                        .text_color(theme::rambutan_200())
                                        .tab_index(0)
                                        .focus_visible(|el| {
                                            el.border_1().border_color(theme::persimmon_400())
                                        })
                                        .role(Role::Button)
                                        .aria_label("Try again")
                                        .cursor_pointer()
                                        .hover(|el| el.bg(theme::longan_700()))
                                        .on_click(cx.listener(|this, _ev, _window, cx| {
                                            this.meetings_hub
                                                .update(cx, |hub, cx| hub.enroll_reset(cx));
                                        }))
                                        .child("Try again"),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    // ── Auto-record prompt ───────────────────────────────────────────────

    /// Web `AutoRecordPromptModal`.
    pub(super) fn render_auto_record_prompt(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let hub = self.meetings_hub.read(cx);
        let event = hub.pending_auto_record.clone()?;
        let countdown = hub.prompt_countdown.max(0);
        let paused = hub.prompt_paused;
        Some(
            div()
                .id("desktop-auto-record-overlay")
                .debug_selector(|| "desktop-auto-record-overlay".to_string())
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .bottom_0()
                .occlude()
                .bg(theme::longan_950().opacity(0.7))
                .flex()
                .items_start()
                .justify_center()
                .pt(px(128.0))
                .on_click(cx.listener(|this, _ev, _window, cx| {
                    this.meetings_hub
                        .update(cx, |hub, cx| hub.prompt_skip_once(cx));
                }))
                .child(
                    div()
                        .id("desktop-auto-record-dialog")
                        .role(Role::Dialog)
                        .aria_label("Record this meeting?")
                        .w_full()
                        .max_w(px(448.0))
                        .rounded(px(14.0))
                        .bg(theme::longan_800())
                        .border_1()
                        .border_color(theme::ring_modal())
                        .p_4()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .on_click(cx.listener(|_this, _ev, _window, cx| cx.stop_propagation()))
                        .child(
                            label("Record this meeting?", H4_PX, theme::rambutan_100())
                                .font_weight(FontWeight::MEDIUM),
                        )
                        .child(
                            body(event.title, BODY_SM_PX, theme::rambutan_300())
                                .line_height(relative(1.375)),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    label(
                                        if paused {
                                            "Auto-record paused".to_string()
                                        } else {
                                            format!("Auto-record in {countdown}s")
                                        },
                                        LABEL_PX,
                                        theme::rambutan_400(),
                                    )
                                    .font(theme::tabular_system_font()),
                                )
                                .child(
                                    text_button(
                                        "desktop-auto-record-pause",
                                        if paused { "Resume" } else { "Pause" },
                                        theme::rambutan_300(),
                                        theme::rambutan_100(),
                                    )
                                    .rounded(px(4.0))
                                    .px(px(6.0))
                                    .py(px(2.0))
                                    .hover(|el| el.bg(theme::longan_700().opacity(0.5)))
                                    .on_click(cx.listener(
                                        |this, _ev, _window, cx| {
                                            this.meetings_hub
                                                .update(cx, |hub, cx| hub.prompt_toggle_pause(cx));
                                        },
                                    )),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_2()
                                .pt_1()
                                .child(
                                    web_button(
                                        "desktop-auto-record-record",
                                        WebButtonVariant::Primary,
                                        WebButtonSize::Small,
                                        ButtonState::Enabled,
                                    )
                                    .aria_label("Record")
                                    .on_click(cx.listener(|this, _ev, _window, cx| {
                                        this.meetings_hub
                                            .update(cx, |hub, cx| hub.prompt_record(cx));
                                    }))
                                    .child("Record"),
                                )
                                .child(
                                    web_button(
                                        "desktop-auto-record-skip",
                                        WebButtonVariant::Outline,
                                        WebButtonSize::Small,
                                        ButtonState::Enabled,
                                    )
                                    .aria_label("Skip once")
                                    .on_click(cx.listener(|this, _ev, _window, cx| {
                                        this.meetings_hub
                                            .update(cx, |hub, cx| hub.prompt_skip_once(cx));
                                    }))
                                    .child("Skip once"),
                                )
                                .child(
                                    web_button(
                                        "desktop-auto-record-always-skip",
                                        WebButtonVariant::Ghost,
                                        WebButtonSize::Small,
                                        ButtonState::Enabled,
                                    )
                                    .aria_label("Always skip this event")
                                    .on_click(cx.listener(|this, _ev, _window, cx| {
                                        this.meetings_hub
                                            .update(cx, |hub, cx| hub.prompt_always_skip(cx));
                                    }))
                                    .child("Always skip this event"),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }

    // ── Notice toast ─────────────────────────────────────────────────────

    /// Web `BannerStack`: a bounded notice rail below page content.
    pub(super) fn render_hub_notice(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let notice = self.meetings_hub.read(cx).notice.clone()?;
        let (bg, ring, fg) = if notice.warning {
            (
                theme::lychee_500().opacity(0.15),
                theme::lychee_400().opacity(0.4),
                theme::lychee_100(),
            )
        } else {
            (
                theme::longan_800(),
                gpui::white().opacity(0.1),
                theme::rambutan_200(),
            )
        };
        Some(
            div()
                .id("desktop-notification-rail")
                .debug_selector(|| "desktop-notification-rail".into())
                .role(Role::Status)
                .aria_label("Notifications")
                .min_h_0()
                .max_h(px(f32::from(window.viewport_size().height) * 0.3))
                .flex_shrink_0()
                .overflow_y_scroll()
                .px_3()
                .py_2()
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("desktop-hub-notice")
                        .debug_selector(|| "desktop-hub-notice".to_string())
                        .occlude()
                        .w_full()
                        .min_w_0()
                        .max_w(px(480.0))
                        .flex_shrink_0()
                        .rounded(px(8.0))
                        .bg(bg)
                        .border_1()
                        .border_color(ring)
                        .px_3()
                        .py_2()
                        .flex()
                        .items_center()
                        .gap_3()
                        .font_family(LABEL_FONT)
                        .text_size(px(LABEL_PX))
                        .text_color(fg)
                        .child(div().min_w_0().flex_1().child(notice.message))
                        .child(
                            text_button(
                                "desktop-hub-notice-dismiss",
                                "Dismiss",
                                fg,
                                theme::rambutan_100(),
                            )
                            .debug_selector(|| "desktop-hub-notice-dismiss".into())
                            .min_h(px(44.0))
                            .min_w(px(44.0))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .on_click(cx.listener(|this, _ev, window, cx| {
                                this.workspace_focus_handle.focus(window, cx);
                                this.meetings_hub
                                    .update(cx, |hub, cx| hub.dismiss_notice(cx));
                            }))
                            .on_key_down(cx.listener(
                                |this, event: &KeyDownEvent, window, cx| {
                                    if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                        window.prevent_default();
                                        cx.stop_propagation();
                                        this.workspace_focus_handle.focus(window, cx);
                                        this.meetings_hub
                                            .update(cx, |hub, cx| hub.dismiss_notice(cx));
                                    }
                                },
                            )),
                        ),
                )
                .into_any_element(),
        )
    }
}

/// The relationship editor, universal search and rename fields.
pub(super) struct PeopleInputs {
    pub(super) relationship_company_input: Entity<InputState>,
    pub(super) relationship_role_input: Entity<InputState>,
    pub(super) relationship_email_input: Entity<InputState>,
    pub(super) relationship_phone_input: Entity<InputState>,
    pub(super) relationship_location_input: Entity<InputState>,
    pub(super) relationship_notes_input: Entity<InputState>,
    pub(super) relationship_item_input: Entity<InputState>,
    pub(super) search_query_input: Entity<InputState>,
    pub(super) person_rename_input: Entity<InputState>,
    pub(super) speaker_rename_input: Entity<InputState>,
}

impl Workspace {
    /// The meetings / people hub, restored from the saved caches and started.
    pub(super) fn new_meetings_hub(
        persisted: Option<&PersistedWorkspaceState>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Entity<MeetingsHub> {
        let meeting_prefs = persisted
            .and_then(|state| state.meeting_prefs.clone())
            .unwrap_or_default();
        let desktop_runtime = cx.global::<DesktopState>().runtime.clone();
        let meetings_hub = cx.new(|cx| {
            let mut hub = MeetingsHub::new(meetings_client(), desktop_runtime, meeting_prefs, cx);
            if let Some(state) = persisted {
                hub.restore_people(state.people_cache.clone());
                hub.restore_meetings(state.meeting_cache.clone());
                hub.restore_action_checkpoints(state.meeting_actions.clone());
            }
            hub
        });
        subscriptions.push(cx.observe(&meetings_hub, |this, _hub, cx| {
            if let Some(view) = this.meeting_transcript_view.borrow().as_ref() {
                view.invalidate_footer();
            }
            cx.notify();
        }));
        subscriptions.push(cx.subscribe_in(
            &meetings_hub,
            window,
            |this: &mut Self, _hub, event: &HubEvent, window, cx| match event {
                HubEvent::TranscriptChanged { meeting_id, range } => {
                    let focus = this.speaker_rename_input.read(cx).focus_handle(cx);
                    for slot in [&this.meeting_transcript_view, &this.live_transcript_view] {
                        if let Some(view) = slot.borrow_mut().as_mut() {
                            view.invalidate(meeting_id, range.as_ref());
                            if range.is_none() {
                                view.rebase_editor(this.meetings_hub.read(cx), &focus);
                            }
                        }
                    }
                }
                HubEvent::PrefsChanged | HubEvent::MeetingsChanged => this.persist(cx),
                HubEvent::PeopleChanged => {
                    this.repair_retired_person_references(cx);
                    this.persist(cx);
                }
                HubEvent::PersonMergeResolved(redirect) => {
                    if let Err(error) = this.reconcile_person_merge(redirect, cx) {
                        this.meetings_hub
                            .update(cx, |hub, cx| hub.report_person_merge_conflict(error, cx));
                    } else {
                        this.repair_retired_person_references(cx);
                    }
                }
                HubEvent::PersonMergesResolved(redirects) => {
                    this.reconcile_person_merges(redirects, cx);
                }
                HubEvent::RefreshPeople => {
                    this.repair_retired_person_references(cx);
                    let referenced_ids = this.referenced_person_ids(cx);
                    this.meetings_hub.update(cx, |hub, cx| {
                        hub.refresh_people_reconciled(referenced_ids, cx)
                    });
                }
                HubEvent::ShowMeeting { meeting_id } => {
                    this.open_meeting_detail(meeting_id.clone(), cx);
                }
                HubEvent::EnterMeetingDock => this.open_meeting_dock(window, cx),
                HubEvent::CalendarConflict { local, remote } => {
                    let already_pending =
                        this.scheduling_preferences
                            .conflicts
                            .iter()
                            .any(|conflict| {
                                conflict.event_id == local.id && conflict.resolution.is_none()
                            });
                    if !already_pending {
                        this.scheduling_preferences.conflicts.push(
                            bruhs_models::SchedulingConflict {
                                id: uuid::Uuid::new_v4().to_string(),
                                event_id: local.id.clone(),
                                event_title: local.title.clone(),
                                local_start_at_ms: local.start_at_ms,
                                local_end_at_ms: local.end_at_ms,
                                remote_start_at_ms: remote.start_at_ms,
                                remote_end_at_ms: remote.end_at_ms,
                                detected_at_ms: EpochMs::now().as_millis(),
                                resolution: None,
                            },
                        );
                        this.scheduling_preferences.updated_at_ms = EpochMs::now().as_millis();
                        this.persist_and_notify(cx);
                    }
                }
                HubEvent::OpenMeetingChat { meeting_title, .. } => {
                    this.open_meeting_chat_thread(meeting_title.clone(), window, cx)
                }
            },
        ));
        meetings_hub.update(cx, |hub, cx| hub.start(cx));
        meetings_hub
    }
    /// The relationship editor, universal search and person / speaker rename fields.
    pub(super) fn new_people_inputs(
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> PeopleInputs {
        let relationship_company_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Company"));
        subscriptions.push(redraw_on_edit(&relationship_company_input, window, cx));
        let relationship_role_input = cx.new(|cx| InputState::new(window, cx).placeholder("Role"));
        subscriptions.push(redraw_on_edit(&relationship_role_input, window, cx));
        let relationship_email_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Email"));
        subscriptions.push(redraw_on_edit(&relationship_email_input, window, cx));
        let relationship_phone_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Phone"));
        subscriptions.push(redraw_on_edit(&relationship_phone_input, window, cx));
        let relationship_location_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Location"));
        subscriptions.push(redraw_on_edit(&relationship_location_input, window, cx));
        let relationship_notes_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Private notes"));
        subscriptions.push(redraw_on_edit(&relationship_notes_input, window, cx));
        let relationship_item_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Interaction, commitment, reminder, or note…")
        });
        subscriptions.push(redraw_on_edit(&relationship_item_input, window, cx));
        let search_query_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search tasks, events, people, meetings, threads…")
        });
        subscriptions.push(cx.subscribe_in(
            &search_query_input,
            window,
            |this: &mut Self, _input, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.meetings_hub
                        .update(cx, |hub, cx| hub.ensure_search_cache(cx));
                    cx.notify();
                }
            },
        ));
        let person_rename_input = cx.new(|cx| InputState::new(window, cx));
        subscriptions.push(cx.subscribe_in(
            &person_rename_input,
            window,
            |this: &mut Self, _input, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.commit_person_rename(cx);
                }
            },
        ));
        let speaker_rename_input = cx.new(|cx| InputState::new(window, cx));
        subscriptions.push(cx.subscribe_in(
            &speaker_rename_input,
            window,
            |this: &mut Self, _input, ev: &InputEvent, _window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.commit_speaker_rename(cx);
                }
            },
        ));
        PeopleInputs {
            relationship_company_input,
            relationship_role_input,
            relationship_email_input,
            relationship_phone_input,
            relationship_location_input,
            relationship_notes_input,
            relationship_item_input,
            search_query_input,
            person_rename_input,
            speaker_rename_input,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{speaker_colors, speaker_label, status_label};
    use bruhs_models::{MeetingStatus, Speaker};

    #[test]
    fn speaker_labels_and_colors_follow_the_shared_grammar() {
        let speakers = vec![Speaker {
            id: "them_1".into(),
            label: "Alice".into(),
            person_id: None,
            color: None,
        }];
        assert_eq!(speaker_label(&speakers, Some("them_1")), "Alice");
        assert_eq!(speaker_label(&speakers, None), "Unknown");
        assert_ne!(speaker_colors(Some("me")).0, speaker_colors(Some("them")).0);
        assert_eq!(status_label(MeetingStatus::Live), "Live");
    }
}
