//! A local agent's persona becoming a bruh, and memory moved in from another
//! assistant. Imported memory never rides a `CreateBruh`: it reaches a bruh
//! only as a pending `SetMemory` proposal the owner approves (D11 to D13).

use super::lines::fill;
use super::{
    is_hidden_char, persona_lines, PersonaCandidate, CUT_MARK, EXPORT_MEMORY_FROM_TEMPLATE,
    IDENTITY_FROM, MOVE_IN_EMPTY, MOVE_IN_TOO_LONG, PASTED_ANSWER_TEMPLATE,
};
use crate::agent::DEFAULT_AGENT_MODEL;
use crate::crew::BruhDraft;
use crate::logic::portable::ImportSource;
use crate::reactive::{
    ProposalStatus, ReactiveChange, ReactiveCommand, ReactiveSnapshot, MAX_TEXT_CHARS,
};
use crate::time::EpochMs;
use std::collections::HashSet;

/// Characters a persona's name keeps.
const MAX_PERSONA_NAME_CHARS: usize = 40;

/// The `IDENTITY.md` labels Bruhs carries into a bruh's instructions, as the
/// file names them.
const NAME_LABEL: &str = "Name";
const CREATURE_LABEL: &str = "Creature";
const VIBE_LABEL: &str = "Vibe";
/// The labels left out, because bruhs use faces.
const EMOJI_LABEL: &str = "Emoji";
const AVATAR_LABEL: &str = "Avatar";

/// The fields of an OpenClaw `IDENTITY.md` (R§4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityFields {
    pub name: Option<String>,
    pub creature: Option<String>,
    pub vibe: Option<String>,
    pub emoji: Option<String>,
    pub avatar: Option<String>,
}

/// Reads `- **Label:** value`, `- Label: value` and `Label: value` lines.
/// Labels match without regard to case, and the first line with a value
/// wins. An empty value, or one that is only a parenthesised placeholder
/// such as `(pick something you like)`, is no value.
pub fn identity_fields(text: &str) -> IdentityFields {
    let mut fields = IdentityFields::default();
    for (label, value) in text.lines().filter_map(identity_line) {
        let slot = [
            (NAME_LABEL, &mut fields.name),
            (CREATURE_LABEL, &mut fields.creature),
            (VIBE_LABEL, &mut fields.vibe),
            (EMOJI_LABEL, &mut fields.emoji),
            (AVATAR_LABEL, &mut fields.avatar),
        ]
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(label));
        if let Some((_, slot)) = slot {
            if slot.is_none() {
                *slot = identity_value(value);
            }
        }
    }
    fields
}

/// One line's label and raw value, when it is shaped like a field.
fn identity_line(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    let line = line.strip_prefix('-').map_or(line, str::trim_start);
    if let Some(bold) = line.strip_prefix("**") {
        let (label, rest) = bold.split_once("**")?;
        return match label.strip_suffix(':') {
            Some(label) => Some((label.trim(), rest)),
            None => Some((label.trim(), rest.trim_start().strip_prefix(':')?)),
        };
    }
    let (label, value) = line.split_once(':')?;
    Some((label.trim(), value))
}

/// A field's value: trimmed, and out of one layer of `*` or `_` emphasis,
/// so `*Pinch*` is Pinch and `_(how do you come across?)_` a placeholder.
fn identity_value(raw: &str) -> Option<String> {
    let value = raw.trim();
    let value = ['*', '_']
        .into_iter()
        .find_map(|mark| value.strip_prefix(mark)?.strip_suffix(mark))
        .map_or(value, str::trim);
    let placeholder = value.starts_with('(') && value.ends_with(')');
    (!value.is_empty() && !placeholder).then(|| value.to_string())
}

/// A Hermes memory file's entries: split on a section sign on its own line,
/// trimmed, with empty entries and exact repeats left out, in file order.
pub fn hermes_entries(text: &str) -> Vec<String> {
    let text = text.replace("\r\n", "\n");
    let mut seen = HashSet::new();
    text.split("\n§\n")
        .map(str::trim)
        .filter(|entry| !entry.is_empty() && seen.insert(*entry))
        .map(str::to_string)
        .collect()
}

/// What a scan read from one workspace or home, before it is assembled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaTexts {
    pub source: ImportSource,
    /// The Hermes profile name, else the source's label.
    pub fallback_name: String,
    pub identity: Option<IdentityFields>,
    /// (file label, text), in priority order.
    pub instructions: Vec<(String, String)>,
    /// (file label, text), in priority order.
    pub memory: Vec<(String, String)>,
}

/// A persona's name, job and memory, each within [`MAX_TEXT_CHARS`] and
/// free of NUL, so the store takes them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssembledPersona {
    pub name: String,
    pub instructions: String,
    pub memory: String,
    /// Some text was cut to fit.
    pub clipped: bool,
    /// Labels of the identity fields left out, such as "Emoji".
    pub dropped_identity: Vec<String>,
}

/// The instructions are an identity block (`Identity from IDENTITY.md`, then
/// a line for each of Name, Creature and Vibe that is set) and then each
/// instruction file under `## <file>`. The memory is each memory file under
/// `## <source label>: <file>`. Each is cut to [`MAX_TEXT_CHARS`].
pub fn assemble_persona(texts: &PersonaTexts) -> AssembledPersona {
    let identity = texts.identity.clone().unwrap_or_default();
    let mut identity_lines = Vec::new();
    if let Some(name) = clean_name(identity.name.as_deref()) {
        identity_lines.push(format!("{NAME_LABEL}: {name}"));
    }
    for (label, value) in [
        (CREATURE_LABEL, &identity.creature),
        (VIBE_LABEL, &identity.vibe),
    ] {
        if let Some(value) = value {
            identity_lines.push(format!("{label}: {value}"));
        }
    }
    let mut sections = Vec::with_capacity(texts.instructions.len() + 1);
    if !identity_lines.is_empty() {
        // No label: the block's first line is its own heading.
        let block = format!("{IDENTITY_FROM}\n{}", identity_lines.join("\n"));
        sections.push((String::new(), block));
    }
    sections.extend(texts.instructions.iter().cloned());
    let (instructions, instructions_cut) = fit_sections(
        &sections,
        |label| {
            if label.is_empty() {
                String::new()
            } else {
                format!("## {label}")
            }
        },
        MAX_TEXT_CHARS,
    );
    let source = texts.source.label();
    let (memory, memory_cut) = fit_sections(
        &texts.memory,
        |label| format!("## {source}: {label}"),
        MAX_TEXT_CHARS,
    );
    let dropped_identity = [
        (EMOJI_LABEL, &identity.emoji),
        (AVATAR_LABEL, &identity.avatar),
    ]
    .into_iter()
    .filter(|(_, value)| value.is_some())
    .map(|(label, _)| label.to_string())
    .collect();
    AssembledPersona {
        name: persona_name(identity.name.as_deref(), &texts.fallback_name),
        instructions,
        memory,
        clipped: instructions_cut || memory_cut,
        dropped_identity,
    }
}

/// The shared cut rule: each non-empty section, as its heading (when there
/// is one), a blank line and its trimmed text less any NUL, joined by blank
/// lines. Whole sections go in while they fit `budget` characters; the first that
/// doesn't is cut at a character to end with [`CUT_MARK`] inside the budget,
/// and the ones after it are dropped. The flag says whether anything was cut.
fn fit_sections(
    sections: &[(String, String)],
    heading: impl Fn(&str) -> String,
    budget: usize,
) -> (String, bool) {
    let mut out = String::new();
    let mut used = 0;
    for (label, text) in sections {
        // A NUL is invisible and makes the store refuse the whole text.
        let text = text.replace('\0', "");
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let head = heading(label);
        let mut piece = String::new();
        if !out.is_empty() {
            piece.push_str("\n\n");
        }
        if !head.is_empty() {
            piece.push_str(&head);
            piece.push_str("\n\n");
        }
        piece.push_str(text);
        out.push_str(&piece);
        used += piece.chars().count();
        if used > budget {
            let keep = budget.saturating_sub(CUT_MARK.chars().count());
            let mut cut = take_chars(&out, keep).trim_end().to_string();
            cut.push_str(CUT_MARK);
            return (cut, true);
        }
    }
    (out, false)
}

/// The first `n` characters of `text`.
fn take_chars(text: &str, n: usize) -> &str {
    text.char_indices()
        .nth(n)
        .map_or(text, |(at, _)| &text[..at])
}

/// `raw` without hidden characters, trimmed and cut to
/// [`MAX_PERSONA_NAME_CHARS`], when anything is left.
fn clean_name(raw: Option<&str>) -> Option<String> {
    let kept: String = raw?.chars().filter(|c| !is_hidden_char(*c)).collect();
    let name = take_chars(kept.trim(), MAX_PERSONA_NAME_CHARS).trim_end();
    (!name.is_empty()).then(|| name.to_string())
}

/// IDENTITY's name when it has one, else `fallback` (the Hermes profile
/// name or the source's label), each cleaned the same way.
pub fn persona_name(raw: Option<&str>, fallback: &str) -> String {
    clean_name(raw)
        .or_else(|| clean_name(Some(fallback)))
        .unwrap_or_default()
}

/// The Create bruh dialog's starting values for a persona. The memory stays
/// out: it is proposed once the bruh exists ([`after_created`]).
pub fn draft_from_persona(persona: &PersonaCandidate, fallback_model: &str) -> BruhDraft {
    BruhDraft {
        name: persona.name.clone(),
        instructions: persona.instructions.clone(),
        model: fallback_model.to_string(),
        effort: None,
    }
}

/// A persona as the Create bruh dialog takes it: the draft, the memory to
/// propose after creation, and the preview lines shown under the job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPrefill {
    pub draft: BruhDraft,
    pub memory: String,
    pub lines: Vec<String>,
}

/// The prefill for **Make this bruh**, on [`DEFAULT_AGENT_MODEL`].
pub fn prefill_from_persona(persona: &PersonaCandidate) -> ImportPrefill {
    ImportPrefill {
        draft: draft_from_persona(persona, DEFAULT_AGENT_MODEL),
        memory: persona.memory.clone(),
        lines: persona_lines(persona),
    }
}

/// What to send once the bruh made from `prefill` appears as `agent_id`:
/// its memory as a proposal, or nothing when it has none.
pub fn after_created(prefill: &ImportPrefill, agent_id: &str) -> Option<ReactiveCommand> {
    (!prefill.memory.trim().is_empty())
        .then(|| propose_memory(agent_id.to_string(), prefill.memory.clone()))
}

/// The Move in dialog's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveInTab {
    OpenClaw,
    Hermes,
    ByPrompt,
}

/// How the Move in dialog opens: on a tab, and for By prompt, optionally
/// with the answer area filled and the heading it goes under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveInStart {
    pub tab: MoveInTab,
    pub text: Option<String>,
    pub from: Option<String>,
}

/// `current` memory with `pasted` added under `## <from>`. The pasted text
/// loses its control and format characters except newline and tab, and is
/// trimmed. Nothing to add is [`MOVE_IN_EMPTY`]; a result over
/// [`MAX_TEXT_CHARS`] characters is [`MOVE_IN_TOO_LONG`].
pub fn moved_in_memory(current: &str, pasted: &str, from: &str) -> Result<String, &'static str> {
    // A line or paragraph separator keeps its break as a newline.
    let kept: String = pasted
        .chars()
        .map(|c| {
            if matches!(c, '\u{2028}' | '\u{2029}') {
                '\n'
            } else {
                c
            }
        })
        .filter(|c| matches!(c, '\n' | '\t') || !is_hidden_char(*c))
        .collect();
    let text = kept.trim();
    if text.is_empty() {
        return Err(MOVE_IN_EMPTY);
    }
    let base = current.trim_end();
    let memory = if base.is_empty() {
        format!("## {from}\n\n{text}")
    } else {
        format!("{base}\n\n## {from}\n\n{text}")
    };
    if memory.chars().count() > MAX_TEXT_CHARS {
        return Err(MOVE_IN_TOO_LONG);
    }
    Ok(memory)
}

/// The heading for a pasted answer: "Pasted answer, 2026-11-21".
pub fn moved_in_from(date: &str) -> String {
    fill(PASTED_ANSWER_TEMPLATE, &[("date", date)])
}

/// The heading for memory from an export: "Claude.ai export".
pub fn export_memory_from(source: ImportSource) -> String {
    fill(EXPORT_MEMORY_FROM_TEMPLATE, &[("label", source.label())])
}

/// The only way imported memory travels: a proposal the owner decides.
pub fn propose_memory(agent_id: String, memory: String) -> ReactiveCommand {
    ReactiveCommand::Propose {
        change: ReactiveChange::SetMemory { agent_id, memory },
    }
}

/// Whether `proposed` doesn't start with the `current` memory (its trailing
/// whitespace aside, as [`moved_in_memory`] drops it), meaning approving
/// would replace a change made after it was proposed (D12). A deletion at
/// the end of the memory, or clearing it, still leaves a prefix and isn't
/// caught; the card shows the full proposed text.
pub fn memory_proposal_drops(current: &str, proposed: &str) -> bool {
    !proposed.starts_with(current.trim_end())
}

/// A pending `SetMemory` proposal for one bruh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingMemory<'a> {
    pub proposal_id: &'a str,
    pub memory: &'a str,
    pub created_at: EpochMs,
}

/// Every pending `SetMemory` proposal for `agent_id`, newest first.
pub fn pending_memory_proposals<'a>(
    snapshot: &'a ReactiveSnapshot,
    agent_id: &str,
) -> Vec<PendingMemory<'a>> {
    let mut pending: Vec<PendingMemory<'a>> = snapshot
        .proposals
        .iter()
        .filter(|proposal| proposal.status == ProposalStatus::Pending)
        .filter_map(|proposal| match &proposal.change {
            ReactiveChange::SetMemory {
                agent_id: id,
                memory,
            } if id == agent_id => Some(PendingMemory {
                proposal_id: &proposal.id,
                memory,
                created_at: proposal.created_at,
            }),
            _ => None,
        })
        .collect();
    pending.sort_by_key(|p| std::cmp::Reverse(p.created_at));
    pending
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crew::{create_command, BruhAccess, BruhLook};
    use crate::reactive::{ProposalStatus, ReactiveChange, ReactiveCommand, ReactiveProposal};

    fn persona(memory: &str) -> PersonaCandidate {
        PersonaCandidate {
            id: "p1".into(),
            source: ImportSource::OpenClaw,
            folder: "/home/example/clawd".into(),
            name: "Pinch".into(),
            instructions: "Be kind.".into(),
            memory: memory.into(),
            files: Vec::new(),
            clipped: false,
            dropped_identity: Vec::new(),
            left_out_logs: 0,
        }
    }

    #[test]
    fn identity_reads_the_template_and_drops_placeholders() {
        let text = "# IDENTITY\n- **Name:** Pinch\n- **Creature:** a tidy crab\n- **Vibe:** (pick something you like)\n- emoji: 🦀\nAvatar: avatars/pinch.png\n";
        let id = identity_fields(text);
        assert_eq!(id.name.as_deref(), Some("Pinch"));
        assert_eq!(id.creature.as_deref(), Some("a tidy crab"));
        assert_eq!(id.vibe, None);
        assert_eq!(id.emoji.as_deref(), Some("🦀"));
        assert_eq!(id.avatar.as_deref(), Some("avatars/pinch.png"));
        let id = identity_fields("- **Name:**\n- NAME: https://example.com/a:b\n- Theme: dark\n");
        assert_eq!(id.name.as_deref(), Some("https://example.com/a:b"));
        assert_eq!(
            identity_fields("Just prose: no labels\n"),
            IdentityFields::default()
        );
        let id = identity_fields(
            "- **Name:** *Pinch*\n- **Vibe:** _(how do you come across?)_\n- **Creature:** *(pick)*\n- Emoji: __\n",
        );
        assert_eq!(id.name.as_deref(), Some("Pinch"));
        assert_eq!((id.vibe, id.creature, id.emoji), (None, None, None));
    }

    #[test]
    fn hermes_entries_split_on_the_section_sign() {
        assert_eq!(
            hermes_entries("Likes tea\n§\nWorks in Lisbon\nOn weekdays\n§\nLikes tea\n§\n"),
            vec![
                "Likes tea".to_string(),
                "Works in Lisbon\nOn weekdays".to_string()
            ]
        );
        assert!(hermes_entries("\n§\n  \n§\n").is_empty());
    }

    #[test]
    fn personas_assemble_in_order_and_cut_to_fit() {
        let texts = PersonaTexts {
            source: ImportSource::OpenClaw,
            fallback_name: "OpenClaw".into(),
            identity: Some(IdentityFields {
                name: Some(" Pin\u{202e}ch ".into()),
                creature: Some("a crab".into()),
                emoji: Some("🦀".into()),
                ..Default::default()
            }),
            instructions: vec![
                ("SOUL.md".into(), "Be kind.".into()),
                ("AGENTS.md".into(), "x".repeat(MAX_TEXT_CHARS)),
            ],
            memory: vec![("USER.md".into(), "Prefers short answers.".into())],
        };
        let p = assemble_persona(&texts);
        assert_eq!(p.name, "Pinch");
        assert!(p.instructions.starts_with(
            "Identity from IDENTITY.md\nName: Pinch\nCreature: a crab\n\n## SOUL.md\n\nBe kind."
        ));
        assert!(
            p.clipped
                && p.instructions.chars().count() <= MAX_TEXT_CHARS
                && p.instructions.ends_with(CUT_MARK)
        );
        assert!(p.instructions.contains("\n\n## AGENTS.md\n\nxxx"));
        assert_eq!(p.memory, "## OpenClaw: USER.md\n\nPrefers short answers.");
        assert_eq!(p.dropped_identity, vec!["Emoji".to_string()]);
        assert_eq!(persona_name(None, "Hermes Agent"), "Hermes Agent");
        assert_eq!(persona_name(Some(&"n".repeat(60)), "x").chars().count(), 40);
        assert_eq!(persona_name(Some(" \u{200b}\u{7} "), "work"), "work");
        // Bidi isolates and the Arabic letter mark reorder text: dropped.
        assert_eq!(
            persona_name(Some("\u{2066}Sc\u{2067}out\u{2068}\u{2069}\u{61c}"), "x"),
            "Scout"
        );
        assert_eq!(persona_name(Some("\u{2069}\u{61c}"), "work"), "work");
    }

    #[test]
    fn cuts_land_on_characters_and_later_sections_drop() {
        let sections = vec![
            ("A.md".to_string(), "ab".to_string()),
            ("B.md".to_string(), "🦀".repeat(40)),
            ("C.md".to_string(), "never".to_string()),
        ];
        let budget = 40;
        let (text, cut) = fit_sections(&sections, |label| format!("## {label}"), budget);
        assert!(cut && text.ends_with(CUT_MARK) && text.chars().count() <= budget);
        assert!(text.starts_with("## A.md\n\nab\n\n## B.md\n\n🦀") && !text.contains("C.md"));
        let (whole, cut) = fit_sections(&sections[..1], |label| format!("## {label}"), budget);
        assert_eq!((whole.as_str(), cut), ("## A.md\n\nab", false));
        let empty = PersonaTexts {
            source: ImportSource::Hermes,
            fallback_name: "work".into(),
            identity: None,
            instructions: vec![("SOUL.md".into(), "  \n".into())],
            memory: Vec::new(),
        };
        let nul = PersonaTexts {
            source: ImportSource::OpenClaw,
            fallback_name: "OpenClaw".into(),
            identity: Some(IdentityFields {
                creature: Some("a\0crab".into()),
                ..Default::default()
            }),
            instructions: vec![("SOUL.md".into(), "Be\0 kind.".into())],
            memory: vec![("USER.md".into(), "\0Tea\0".into())],
        };
        let p = assemble_persona(&nul);
        assert_eq!(
            p.instructions,
            "Identity from IDENTITY.md\nCreature: acrab\n\n## SOUL.md\n\nBe kind."
        );
        assert_eq!(p.memory, "## OpenClaw: USER.md\n\nTea");
        let p = assemble_persona(&empty);
        assert_eq!(
            (p.name.as_str(), p.instructions.as_str(), p.memory.as_str()),
            ("work", "", "")
        );
        assert!(!p.clipped && p.dropped_identity.is_empty());
    }

    #[test]
    fn imported_memory_only_travels_as_a_proposal() {
        let persona = persona("## OpenClaw: USER.md\n\nTea");
        let draft = draft_from_persona(&persona, "claude-sonnet-x");
        assert_eq!(
            (draft.model.as_str(), draft.effort.as_deref()),
            ("claude-sonnet-x", None)
        );
        let ReactiveCommand::CreateBruh { agent, access, .. } =
            create_command("b1".into(), &draft, BruhLook::default(), None)
        else {
            panic!()
        };
        assert_eq!(
            (agent.memory.as_str(), agent.instructions.as_str()),
            ("", "Be kind.")
        );
        assert_eq!(access, BruhAccess::default());
        assert!(
            matches!(propose_memory("b1".into(), persona.memory.clone()),
            ReactiveCommand::Propose { change: ReactiveChange::SetMemory { agent_id, memory } } if agent_id == "b1" && memory == persona.memory)
        );
    }

    #[test]
    fn a_prefill_sends_memory_only_after_the_bruh_exists() {
        let prefill = prefill_from_persona(&persona("## OpenClaw: USER.md\n\nTea"));
        assert_eq!(prefill.draft.name, "Pinch");
        assert_eq!(prefill.draft.instructions, "Be kind.");
        assert_eq!(prefill.draft.model, crate::agent::DEFAULT_AGENT_MODEL);
        assert_eq!(prefill.memory, "## OpenClaw: USER.md\n\nTea");
        assert_eq!(
            prefill.lines,
            persona_lines(&persona("## OpenClaw: USER.md\n\nTea"))
        );
        assert_eq!(
            after_created(&prefill, "b1"),
            Some(propose_memory("b1".into(), prefill.memory.clone()))
        );
        assert_eq!(
            after_created(&prefill_from_persona(&persona("  ")), "b1"),
            None
        );
        let start = MoveInStart {
            tab: MoveInTab::ByPrompt,
            text: Some("Tea".into()),
            from: Some(export_memory_from(ImportSource::ClaudeAi)),
        };
        // Clearing the answer keeps the tab and heading it was opened with.
        let cleared = MoveInStart {
            text: None,
            ..start.clone()
        };
        assert_eq!(cleared.tab, MoveInTab::ByPrompt);
        assert_eq!(cleared.from.as_deref(), Some("Claude.ai export"));
        assert_ne!(cleared, start);
        assert_ne!(MoveInTab::OpenClaw, MoveInTab::Hermes);
        assert_eq!(moved_in_from("2026-11-21"), "Pasted answer, 2026-11-21");
    }

    #[test]
    fn moving_in_appends_and_checks_the_budget() {
        assert_eq!(
            moved_in_memory("", "  Likes tea\u{7}\n", "Pasted answer, 2026-11-21").unwrap(),
            "## Pasted answer, 2026-11-21\n\nLikes tea"
        );
        assert_eq!(
            moved_in_memory("Old\n", "New", "X").unwrap(),
            "Old\n\n## X\n\nNew"
        );
        assert_eq!(
            moved_in_memory("", "a\tb\r\nc\u{200b}\u{feff}d", "X").unwrap(),
            "## X\n\na\tb\ncd"
        );
        // Isolates and the letter mark go; line and paragraph separators
        // keep their break as a newline.
        assert_eq!(
            moved_in_memory("", "a\u{2066}b\u{2069}\u{61c}\u{2028}c\u{2029}d\te", "X").unwrap(),
            "## X\n\nab\nc\nd\te"
        );
        assert_eq!(moved_in_memory("Old", "  ", "X"), Err(MOVE_IN_EMPTY));
        assert_eq!(
            moved_in_memory("Old", "\u{202e}\u{7}", "X"),
            Err(MOVE_IN_EMPTY)
        );
        assert_eq!(
            moved_in_memory(&"o".repeat(MAX_TEXT_CHARS), "New", "X"),
            Err(MOVE_IN_TOO_LONG)
        );
        // Counted in characters, not bytes: 4-byte crabs that fit by count pass.
        let crabs = "🦀".repeat(MAX_TEXT_CHARS - "## X\n\n".len());
        assert!(moved_in_memory("", &crabs, "X").is_ok());
    }

    fn proposal(
        id: &str,
        change: ReactiveChange,
        status: ProposalStatus,
        at: i64,
    ) -> ReactiveProposal {
        ReactiveProposal {
            id: id.into(),
            change,
            status,
            created_at: EpochMs::saturating_from_i64(at),
        }
    }

    fn set_memory(agent: &str, memory: &str) -> ReactiveChange {
        ReactiveChange::SetMemory {
            agent_id: agent.into(),
            memory: memory.into(),
        }
    }

    #[test]
    fn stale_proposals_are_flagged() {
        assert!(!memory_proposal_drops("Old", "Old\n\n## X\n\nNew"));
        assert!(memory_proposal_drops(
            "Old and edited",
            "Old\n\n## X\n\nNew"
        ));
        assert!(!memory_proposal_drops("", "## X\n\nNew"));
        assert!(!memory_proposal_drops("Old  \n", "Old\n\n## X\n\nNew"));
        // The known miss (D12): a deletion at the end of the memory, or
        // clearing it, still leaves a prefix of the proposal, so it isn't
        // flagged. The card shows the full proposed text instead.
        let proposed = "Likes tea. Lives in Lisbon.\n\n## X\n\nNew";
        assert!(!memory_proposal_drops("Likes tea.", proposed));
        assert!(!memory_proposal_drops("", proposed));

        let agent = crate::reactive::ReactiveAgent {
            id: "b1".into(),
            name: "Pinch".into(),
            instructions: String::new(),
            memory: String::new(),
            model: "m".into(),
            project_path: String::new(),
            enabled: true,
        };
        let snapshot = ReactiveSnapshot {
            proposals: vec![
                proposal(
                    "older",
                    set_memory("b1", "one"),
                    ProposalStatus::Pending,
                    10,
                ),
                proposal("other", set_memory("b2", "x"), ProposalStatus::Pending, 30),
                proposal("done", set_memory("b1", "y"), ProposalStatus::Approved, 40),
                proposal(
                    "save",
                    ReactiveChange::SaveAgent { agent },
                    ProposalStatus::Pending,
                    50,
                ),
                proposal(
                    "newer",
                    set_memory("b1", "two"),
                    ProposalStatus::Pending,
                    20,
                ),
            ],
            ..Default::default()
        };
        let pending = pending_memory_proposals(&snapshot, "b1");
        assert_eq!(
            pending,
            vec![
                PendingMemory {
                    proposal_id: "newer",
                    memory: "two",
                    created_at: EpochMs::saturating_from_i64(20),
                },
                PendingMemory {
                    proposal_id: "older",
                    memory: "one",
                    created_at: EpochMs::saturating_from_i64(10),
                },
            ]
        );
        assert!(pending_memory_proposals(&snapshot, "b3").is_empty());
    }
}
