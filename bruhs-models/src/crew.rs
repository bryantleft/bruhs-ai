//! Crew: the bruhs a person runs, how each looks, what each may do and
//! what each is doing now. Shared by the web app, the desktop app and the
//! background service.

use serde::{Deserialize, Serialize};

use crate::logic::dither::{BRUH_FACE_RGB, BRUH_MOUTH_RGB};
use crate::roles::ResolvedRole;
use crate::EpochMs;

mod access;
pub use access::*;
mod automation;
pub use automation::*;
mod computer;
pub use computer::*;
mod face;
pub use face::*;
mod page;
pub use page::*;
pub mod room;
mod transcript;
pub use transcript::*;
mod vault;
pub use vault::*;
#[cfg(test)]
mod wire_tests;

/// Version of the crew wire format the background service speaks.
pub const CREW_PROTOCOL: u32 = 4;

/// First version that has Stop, access and the other crew commands.
pub const CREW_COMMANDS_PROTOCOL: u32 = 1;

/// First version that has rooms.
pub const ROOMS_PROTOCOL: u32 = 2;

/// First version that has a bruh's Computer.
pub const COMPUTER_PROTOCOL: u32 = 3;

/// First version that has vault grants on a bruh.
pub const VAULT_PROTOCOL: u32 = 4;

/// Which way a bruh looks.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Look {
    Left,
    #[default]
    Front,
    Right,
}

impl Look {
    pub const ALL: [Look; 3] = [Look::Left, Look::Front, Look::Right];

    /// Horizontal gaze offset, in face widths from the centre.
    #[must_use]
    pub fn look_x(self) -> f32 {
        match self {
            Look::Left => -0.6,
            Look::Front => 0.0,
            Look::Right => 0.6,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Look::Left => "Left",
            Look::Front => "Front",
            Look::Right => "Right",
        }
    }
}

/// Face colour. Unknown values read as the default bruh yellow.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BruhColor {
    Pandan,
    Blueberry,
    Mangosteen,
    Dragonfruit,
    Lychee,
    Rambutan,
    #[default]
    #[serde(other)]
    Bruh,
}

impl BruhColor {
    pub const ALL: [Self; 7] = [
        Self::Bruh,
        Self::Pandan,
        Self::Blueberry,
        Self::Mangosteen,
        Self::Dragonfruit,
        Self::Lychee,
        Self::Rambutan,
    ];

    /// sRGB of the matching `bruhs.css` token.
    #[must_use]
    pub fn rgb(self) -> u32 {
        match self {
            Self::Bruh => BRUH_FACE_RGB,
            Self::Pandan => 0x0087_be52,
            Self::Blueberry => 0x007e_b3ff,
            Self::Mangosteen => 0x00ab_a0ec,
            Self::Dragonfruit => 0x00d6_5aad,
            Self::Lychee => 0x00df_746d,
            Self::Rambutan => 0x00c3_afa1,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Bruh => "Bruh",
            Self::Pandan => "Pandan",
            Self::Blueberry => "Blueberry",
            Self::Mangosteen => "Mangosteen",
            Self::Dragonfruit => "Dragonfruit",
            Self::Lychee => "Lychee",
            Self::Rambutan => "Rambutan",
        }
    }
}

/// Mouth colour. Unknown values read as the default bruh red.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MouthColor {
    Dragonfruit,
    Mangosteen,
    Blueberry,
    Longan,
    #[default]
    #[serde(other)]
    Bruh,
}

impl MouthColor {
    pub const ALL: [Self; 5] = [
        Self::Bruh,
        Self::Dragonfruit,
        Self::Mangosteen,
        Self::Blueberry,
        Self::Longan,
    ];

    #[must_use]
    pub fn rgb(self) -> u32 {
        match self {
            Self::Bruh => BRUH_MOUTH_RGB,
            Self::Dragonfruit => 0x00d6_5aad,
            Self::Mangosteen => 0x00ab_a0ec,
            Self::Blueberry => 0x007e_b3ff,
            Self::Longan => 0x00a3_9485,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Bruh => "Bruh",
            Self::Dragonfruit => "Dragonfruit",
            Self::Mangosteen => "Mangosteen",
            Self::Blueberry => "Blueberry",
            Self::Longan => "Longan",
        }
    }
}

/// How a bruh looks.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct BruhLook {
    #[serde(default)]
    pub look: Look,
    #[serde(default)]
    pub face: BruhColor,
    #[serde(default)]
    pub mouth: MouthColor,
}

/// How far a bruh may act without asking. Unknown values read as read only.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccessLevel {
    Ask,
    Full,
    #[default]
    #[serde(other)]
    ReadOnly,
}

impl AccessLevel {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "Read only",
            Self::Ask => "Ask first",
            Self::Full => "Full",
        }
    }
}

/// What a bruh may do.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct BruhAccess {
    #[serde(default)]
    pub level: AccessLevel,
    #[serde(default)]
    pub send_as_me: bool,
    #[serde(default)]
    pub can_deliver: bool,
}

/// What a bruh is doing now. Unknown values read as asleep.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BruhStatus {
    Working,
    NeedsYou,
    Paused,
    Off,
    #[default]
    #[serde(other)]
    Asleep,
}

impl BruhStatus {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Asleep => "Asleep",
            Self::Working => "Working",
            Self::NeedsYou => "Needs you",
            Self::Paused => "Paused",
            Self::Off => "Off",
        }
    }
}

/// Stored payload of a bruh's profile.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct BruhProfile {
    #[serde(default)]
    pub look: BruhLook,
    #[serde(default)]
    pub access: BruhAccess,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub paused_at: Option<EpochMs>,
    /// The owner turned the bruh's Computer on. Set only by
    /// `ReactiveCommand::SetComputer`; see [`computer_on`] for whether it runs.
    #[serde(default)]
    pub computer: bool,
    /// The Global vault entries granted to the bruh, sorted and deduped.
    /// Set only by `ReactiveCommand::SetVault`; see [`usable_by`].
    #[serde(default)]
    pub vault: Vec<String>,
}

/// A bruh as the rail and pages show it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BruhState {
    pub agent_id: String,
    pub name: String,
    pub look: BruhLook,
    pub access: BruhAccess,
    #[serde(default)]
    pub effort: Option<String>,
    pub status: BruhStatus,
    #[serde(default)]
    pub next_wake_ms: Option<EpochMs>,
    #[serde(default)]
    pub unread: usize,
    /// The owner turned the bruh's Computer on.
    #[serde(default)]
    pub computer: bool,
    /// The Global vault entries granted to the bruh.
    #[serde(default)]
    pub vault: Vec<String>,
}

/// Status by priority: off, paused, needs you, working, asleep.
/// `waiting_on_owner` is an open ask or approval, or a run awaiting review.
#[must_use]
pub fn status_for(
    enabled: bool,
    paused: bool,
    waiting_on_owner: bool,
    running: bool,
) -> BruhStatus {
    if !enabled {
        BruhStatus::Off
    } else if paused {
        BruhStatus::Paused
    } else if waiting_on_owner {
        BruhStatus::NeedsYou
    } else if running {
        BruhStatus::Working
    } else {
        BruhStatus::Asleep
    }
}

fn status_rank(status: BruhStatus) -> u8 {
    match status {
        BruhStatus::NeedsYou => 0,
        BruhStatus::Working => 1,
        BruhStatus::Asleep => 2,
        BruhStatus::Paused => 3,
        BruhStatus::Off => 4,
    }
}

/// Rail order: needs you, working, asleep, paused, off; then name; then id.
pub fn rail_order(states: &mut [BruhState]) {
    states.sort_by(|a, b| {
        status_rank(a.status)
            .cmp(&status_rank(b.status))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.agent_id.cmp(&b.agent_id))
    });
}

/// The short line under a bruh's name.
#[must_use]
pub fn status_line(status: BruhStatus, wake_label: Option<&str>) -> String {
    match (status, wake_label) {
        (BruhStatus::Asleep, Some(wake)) => format!("Wakes {wake}"),
        _ => status.label().to_string(),
    }
}

/// Whether a service speaks this app's crew protocol.
#[must_use]
pub fn service_is_current(protocol: u32) -> bool {
    protocol >= CREW_PROTOCOL
}

/// Whether a service understands the crew commands (Stop, access, chat).
#[must_use]
pub fn crew_commands_ready(protocol: u32) -> bool {
    protocol >= CREW_COMMANDS_PROTOCOL
}

/// Whether a service understands rooms.
#[must_use]
pub fn rooms_ready(protocol: u32) -> bool {
    protocol >= ROOMS_PROTOCOL
}

/// Whether a service understands a bruh's Computer.
#[must_use]
pub fn computer_ready(protocol: u32) -> bool {
    protocol >= COMPUTER_PROTOCOL
}

/// Which older service the person should update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceNotice {
    ServingOld,
    InstalledOld,
}

impl ServiceNotice {
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::ServingOld => {
                "The background service runs an older Bruhs. Update it so every crew feature works."
            }
            Self::InstalledOld => {
                "The installed background service is older than this app. Update it so every crew feature keeps working after you quit."
            }
        }
    }
}

#[must_use]
pub fn service_notice(
    serving_protocol: u32,
    installed_protocol: Option<u32>,
) -> Option<ServiceNotice> {
    if !service_is_current(serving_protocol) {
        Some(ServiceNotice::ServingOld)
    } else if installed_protocol.is_some_and(|p| !service_is_current(p)) {
        Some(ServiceNotice::InstalledOld)
    } else {
        None
    }
}

/// Where an approval a bruh asked for stands.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    #[default]
    Waiting,
    Approved,
    Denied,
    Expired,
}

/// A tool call a bruh paused on until the owner decides.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub tool: String,
    pub summary: String,
    pub fingerprint: String,
    #[serde(default)]
    pub state: ApprovalState,
    /// The whole call when the summary can't show it, cut at 4 KB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Starting values for a new bruh.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BruhDraft {
    pub name: String,
    pub instructions: String,
    pub model: String,
    pub effort: Option<String>,
}

/// A draft from a role snapshot; a role with no model keeps the fallback.
#[must_use]
pub fn draft_from_role(role: &ResolvedRole, fallback_model: &str) -> BruhDraft {
    BruhDraft {
        name: role.name.clone(),
        instructions: role.instructions.clone(),
        model: role
            .model
            .clone()
            .unwrap_or_else(|| fallback_model.to_string()),
        effort: role.effort.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(name: &str, status: BruhStatus) -> BruhState {
        BruhState {
            agent_id: format!("id-{name}"),
            name: name.to_string(),
            look: BruhLook::default(),
            access: BruhAccess::default(),
            effort: None,
            status,
            next_wake_ms: None,
            unread: 0,
            computer: false,
            vault: Vec::new(),
        }
    }

    #[test]
    fn defaults_are_todays_bruh_read_only_and_quiet() {
        let look = BruhLook::default();
        assert_eq!(
            (look.look, look.face, look.mouth),
            (Look::Front, BruhColor::Bruh, MouthColor::Bruh)
        );
        assert_eq!(look.face.rgb(), BRUH_FACE_RGB);
        assert_eq!(look.mouth.rgb(), BRUH_MOUTH_RGB);
        let access = BruhAccess::default();
        assert_eq!(access.level, AccessLevel::ReadOnly);
        assert!(!access.send_as_me && !access.can_deliver);
        let empty: BruhProfile = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, BruhProfile::default());
    }

    #[test]
    fn persimmon_is_never_a_bruh_colour_and_unknown_values_fall_back() {
        assert!(BruhColor::ALL
            .iter()
            .all(|c| c.label() != "Persimmon" && c.rgb() != 0x00e6_974d));
        assert_eq!(
            serde_json::from_str::<BruhColor>("\"persimmon\"").unwrap(),
            BruhColor::Bruh
        );
        assert_eq!(
            serde_json::from_str::<AccessLevel>("\"root\"").unwrap(),
            AccessLevel::ReadOnly
        );
        let future: BruhLook = serde_json::from_str(r#"{"face":"pandan","sparkle":true}"#).unwrap();
        assert_eq!(future.face, BruhColor::Pandan);
    }

    #[test]
    fn status_priority_is_off_paused_needs_you_working_asleep() {
        assert_eq!(status_for(false, true, true, true), BruhStatus::Off);
        assert_eq!(status_for(true, true, true, true), BruhStatus::Paused);
        assert_eq!(status_for(true, false, true, true), BruhStatus::NeedsYou);
        assert_eq!(status_for(true, false, false, true), BruhStatus::Working);
        assert_eq!(status_for(true, false, false, false), BruhStatus::Asleep);
    }

    #[test]
    fn the_rail_puts_bruhs_that_need_you_first_then_by_name() {
        let mut states = vec![
            state("zed", BruhStatus::NeedsYou),
            state("Amy", BruhStatus::Asleep),
            state("bo", BruhStatus::Working),
            state("al", BruhStatus::Off),
            state("Cy", BruhStatus::Asleep),
        ];
        rail_order(&mut states);
        let names: Vec<&str> = states.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["zed", "bo", "Amy", "Cy", "al"]);
    }

    #[test]
    fn status_lines_are_short_labels() {
        assert_eq!(
            status_line(BruhStatus::Asleep, Some("08:00")),
            "Wakes 08:00"
        );
        assert_eq!(status_line(BruhStatus::Asleep, None), "Asleep");
        assert_eq!(
            status_line(BruhStatus::NeedsYou, Some("08:00")),
            "Needs you"
        );
    }

    #[test]
    fn an_older_service_is_noticed_whether_it_serves_or_waits() {
        assert_eq!(service_notice(0, None), Some(ServiceNotice::ServingOld));
        assert_eq!(
            service_notice(CREW_PROTOCOL, Some(0)),
            Some(ServiceNotice::InstalledOld)
        );
        assert_eq!(service_notice(CREW_PROTOCOL, Some(CREW_PROTOCOL)), None);
        assert_eq!(service_notice(CREW_PROTOCOL, None), None);
        assert!(!service_is_current(0) && service_is_current(CREW_PROTOCOL));
    }

    #[test]
    fn a_role_snapshot_fills_a_draft_and_keeps_the_fallback_model() {
        let role = ResolvedRole {
            id: "reviewer".into(),
            name: "Reviewer".into(),
            use_for: "PRs".into(),
            model: None,
            effort: Some("high".into()),
            access: None,
            instructions: "Review PRs.".into(),
        };
        let draft = draft_from_role(&role, "claude-sonnet-4-6");
        assert_eq!(draft.model, "claude-sonnet-4-6");
        assert_eq!(draft.effort.as_deref(), Some("high"));
        assert_eq!(draft.name, role.name);
        assert_eq!(draft.instructions, "Review PRs.");
    }
}
