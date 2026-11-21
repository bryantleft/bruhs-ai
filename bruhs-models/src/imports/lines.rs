//! The app-owned lines both apps show for a read, an export row, a persona
//! and a memory proposal. Each fills one template from [`super::ALL_COPY`];
//! names, paths and other user values are placed as they are.

use super::{
    plural, BatchCounts, FileUse, ImportBatch, PersonaCandidate, SkipReason, ATTACHED_TEMPLATE,
    ATTACHMENT_TEMPLATE, BATCH_SUMMARY_TEMPLATE, BEST_EFFORT_SUFFIX, CUT_SHORT_MANY, CUT_SHORT_ONE,
    DROPPED_BRANCHES_MANY, DROPPED_BRANCHES_ONE, DUPLICATES_MANY, DUPLICATES_ONE,
    IDENTITY_DROPPED_MANY, IDENTITY_DROPPED_ONE, IMPORTED_TEMPLATE, IMPORTING_TEMPLATE,
    INSTRUCTIONS_FROM_TEMPLATE, LOOKED_IN_TEMPLATE, MEMORY_FOUND_TEMPLATE, MEMORY_FROM_TEMPLATE,
    MEMORY_PROPOSED_TEMPLATE, MEMORY_WAITS, NOT_IMPORTED_TEMPLATE, OVER_LIMIT_MANY, OVER_LIMIT_ONE,
    PERSONA_CLIPPED, READING_ONE_FILE_TEMPLATE, READING_TEMPLATE, ROWS_SHOWN_TEMPLATE,
    ROW_DETAIL_TEMPLATE, SKIPPED_EMPTY_MANY, SKIPPED_EMPTY_ONE, SKIPPED_LINK_TEMPLATE,
    SKIPPED_NOT_A_FILE_TEMPLATE, SKIPPED_TOO_LARGE_TEMPLATE, SKIPPED_UNKNOWN_MANY,
    SKIPPED_UNKNOWN_ONE, SKIPPED_UNREADABLE_TEMPLATE, STARTS_READ_ONLY,
};
use crate::logic::portable::{ImportCandidate, ImportSource};
use crate::time::{format_relative_ms, EpochMs, RelativeTimeStyle};

/// `template` with each `{slot}` replaced by its value, in one pass, so a
/// value that itself holds `{name}` is placed as it is. A slot with no
/// value is kept.
pub(super) fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let value = after.find('}').and_then(|close| {
            let slot = &after[..close];
            values
                .iter()
                .find(|(name, _)| *name == slot)
                .map(|(_, value)| (*value, close))
        });
        match value {
            Some((value, close)) => {
                out.push_str(value);
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `n` as a wire-sized count for [`plural`].
fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

fn chats(n: u64) -> String {
    plural(n, "chat", "chats")
}

/// "A", "A and B", "A, B and C".
fn and_list<S: AsRef<str>>(items: &[S]) -> String {
    match items {
        [] => String::new(),
        [only] => only.as_ref().to_string(),
        [head @ .., last] => {
            let head: Vec<&str> = head.iter().map(AsRef::as_ref).collect();
            format!("{} and {}", head.join(", "), last.as_ref())
        }
    }
}

/// A counted sentence: `one` when `n` is 1, else `many` with `{n}` filled.
fn counted(n: u32, one: &str, many: &str) -> String {
    if n == 1 {
        one.to_string()
    } else {
        fill(many, &[("n", &n.to_string())])
    }
}

/// While a read runs: "Reading the export: 1 of 2 files, 40 chats so far".
pub fn reading_line(done: u32, files: u32, chats_so_far: u32) -> String {
    fill(
        if files == 1 {
            READING_ONE_FILE_TEMPLATE
        } else {
            READING_TEMPLATE
        },
        &[
            ("done", &done.to_string()),
            ("files", &files.to_string()),
            ("chats", &chats(u64::from(chats_so_far))),
        ],
    )
}

/// A ready batch's summary: what it holds and from where, then a line for
/// each count that isn't zero, then the batch's warnings.
pub fn batch_lines(batch: &ImportBatch) -> Vec<String> {
    let BatchCounts {
        conversations,
        skipped_empty,
        skipped_unknown,
        duplicates,
        dropped_branches,
        over_limit,
        files_read: _,
    } = batch.counts;
    let mut lines = Vec::new();
    let sources: Vec<&str> = ImportSource::EXPORTS
        .into_iter()
        .filter(|source| batch.candidates.iter().any(|c| c.source == *source))
        .map(ImportSource::label)
        .collect();
    if conversations > 0 && !sources.is_empty() {
        lines.push(fill(
            BATCH_SUMMARY_TEMPLATE,
            &[
                ("chats", &chats(u64::from(conversations))),
                ("sources", &and_list(&sources)),
            ],
        ));
    }
    let counts = [
        (skipped_empty, SKIPPED_EMPTY_ONE, SKIPPED_EMPTY_MANY),
        (skipped_unknown, SKIPPED_UNKNOWN_ONE, SKIPPED_UNKNOWN_MANY),
        (duplicates, DUPLICATES_ONE, DUPLICATES_MANY),
        (
            dropped_branches,
            DROPPED_BRANCHES_ONE,
            DROPPED_BRANCHES_MANY,
        ),
        (over_limit, OVER_LIMIT_ONE, OVER_LIMIT_MANY),
    ];
    for (n, one, many) in counts {
        if n > 0 {
            lines.push(counted(n, one, many));
        }
    }
    lines.extend(batch.warnings.iter().cloned());
    lines
}

/// An export row's second line: "ChatGPT, 2d, transcript", and
/// ", best effort" after a Grok chat's.
pub fn export_row_detail(candidate: &ImportCandidate, now: EpochMs) -> String {
    let when = format_relative_ms(candidate.updated_at, now, RelativeTimeStyle::Compact);
    let mut line = fill(
        ROW_DETAIL_TEMPLATE,
        &[("label", candidate.source.label()), ("when", &when)],
    );
    if candidate.source == ImportSource::Grok {
        line.push_str(BEST_EFFORT_SUFFIX);
    }
    line
}

/// While Import all runs: "Importing 3 of 100".
pub fn importing_line(done: usize, total: usize) -> String {
    fill(
        IMPORTING_TEMPLATE,
        &[("done", &done.to_string()), ("total", &total.to_string())],
    )
}

/// After an import: "Imported 4 chats from Claude.ai."
pub fn imported_line(n: usize, source: ImportSource) -> String {
    fill(
        IMPORTED_TEMPLATE,
        &[("chats", &chats(count(n))), ("label", source.label())],
    )
}

/// Above **Use as a bruh's memory**.
pub fn memory_found_line(source: ImportSource) -> String {
    fill(MEMORY_FOUND_TEMPLATE, &[("label", source.label())])
}

/// Where an agent scan looked: "Looked in: ~/clawd and ~/openclaw."
pub fn looked_in_line(paths: &[String]) -> String {
    fill(LOOKED_IN_TEMPLATE, &[("list", &and_list(paths))])
}

/// A persona's preview, in order: where its instructions and memory come
/// from, that the memory waits for review, the files left out and why, the
/// identity fields not carried, whether text was cut, and how it starts.
pub fn persona_lines(persona: &PersonaCandidate) -> Vec<String> {
    let names = |pick: &dyn Fn(FileUse) -> bool| -> Vec<&str> {
        persona
            .files
            .iter()
            .filter(|file| pick(file.use_))
            .map(|file| file.name.as_str())
            .collect()
    };
    let mut lines = Vec::new();
    let instructions = names(&|use_| matches!(use_, FileUse::Identity | FileUse::Instructions));
    if !instructions.is_empty() {
        lines.push(fill(
            INSTRUCTIONS_FROM_TEMPLATE,
            &[("files", &and_list(&instructions))],
        ));
    }
    let memory = names(&|use_| use_ == FileUse::Memory);
    if !memory.is_empty() {
        lines.push(fill(MEMORY_FROM_TEMPLATE, &[("files", &and_list(&memory))]));
    }
    if !persona.memory.trim().is_empty() {
        lines.push(MEMORY_WAITS.to_string());
    }
    let not_imported = names(&|use_| {
        use_ == FileUse::Skipped {
            reason: SkipReason::NotImported,
        }
    });
    if !not_imported.is_empty() {
        lines.push(fill(
            NOT_IMPORTED_TEMPLATE,
            &[("files", &and_list(&not_imported))],
        ));
    }
    for file in &persona.files {
        let FileUse::Skipped { reason } = file.use_ else {
            continue;
        };
        let template = match reason {
            SkipReason::NotImported => continue,
            SkipReason::Link => SKIPPED_LINK_TEMPLATE,
            SkipReason::TooLarge => SKIPPED_TOO_LARGE_TEMPLATE,
            SkipReason::NotAFile => SKIPPED_NOT_A_FILE_TEMPLATE,
            SkipReason::Unreadable => SKIPPED_UNREADABLE_TEMPLATE,
        };
        lines.push(fill(template, &[("name", &file.name)]));
    }
    let dropped = &persona.dropped_identity;
    if !dropped.is_empty() {
        let template = if dropped.len() == 1 {
            IDENTITY_DROPPED_ONE
        } else {
            IDENTITY_DROPPED_MANY
        };
        lines.push(fill(template, &[("fields", &and_list(dropped))]));
    }
    if persona.clipped {
        lines.push(PERSONA_CLIPPED.to_string());
    }
    lines.push(STARTS_READ_ONLY.to_string());
    lines
}

/// After **Propose memory**: "Proposed. Approve it on Scout's Memory tab."
pub fn memory_proposed_line(name: &str) -> String {
    fill(MEMORY_PROPOSED_TEMPLATE, &[("name", name)])
}

/// The placeholder for an attachment whose bytes stay behind.
pub fn attachment_line(name: &str) -> String {
    fill(ATTACHMENT_TEMPLATE, &[("name", name)])
}

/// The heading above an attachment's extracted text.
pub fn attached_line(name: &str) -> String {
    fill(ATTACHED_TEMPLATE, &[("name", name)])
}

/// Where a long text part was cut: "[Cut short: 12 more characters]".
pub fn cut_short_line(more: usize) -> String {
    if more == 1 {
        CUT_SHORT_ONE.to_string()
    } else {
        fill(CUT_SHORT_MANY, &[("n", &more.to_string())])
    }
}

/// Under a list cut to [`super::MAX_ROWS_SHOWN`] rows.
pub fn rows_shown_line(shown: usize, total: usize) -> String {
    fill(
        ROWS_SHOWN_TEMPLATE,
        &[("shown", &shown.to_string()), ("total", &total.to_string())],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imports::{PersonaFile, CLAUDE_MANIFEST};

    fn candidate(source: ImportSource, id: &str, updated_at: i64) -> ImportCandidate {
        ImportCandidate {
            source,
            id: id.into(),
            title: id.into(),
            cwd: None,
            updated_at: EpochMs::saturating_from_i64(updated_at),
            provider: None,
            session_id: None,
            model: None,
        }
    }

    fn file(name: &str, use_: FileUse) -> PersonaFile {
        PersonaFile {
            name: name.into(),
            use_,
        }
    }

    fn skipped(name: &str, reason: SkipReason) -> PersonaFile {
        file(name, FileUse::Skipped { reason })
    }

    #[test]
    fn lines_read_plainly() {
        assert_eq!(
            reading_line(1, 2, 40),
            "Reading the export: 1 of 2 files, 40 chats so far"
        );
        assert_eq!(
            reading_line(0, 1, 1),
            "Reading the export: 0 of 1 file, 1 chat so far"
        );
        assert_eq!(importing_line(3, 100), "Importing 3 of 100");
        assert_eq!(
            imported_line(1, ImportSource::ChatGpt),
            "Imported 1 chat from ChatGPT."
        );
        assert_eq!(
            imported_line(4, ImportSource::ClaudeAi),
            "Imported 4 chats from Claude.ai."
        );
        assert_eq!(
            memory_found_line(ImportSource::ClaudeAi),
            "This export holds Claude.ai's memory of you."
        );
        assert_eq!(
            looked_in_line(&["~/clawd".into(), "~/openclaw".into()]),
            "Looked in: ~/clawd and ~/openclaw."
        );
        assert_eq!(
            memory_proposed_line("Scout"),
            "Proposed. Approve it on Scout's Memory tab."
        );
        assert_eq!(
            attachment_line("plan.pdf"),
            "Attachment not imported: plan.pdf"
        );
        assert_eq!(attached_line("notes.txt"), "Attached notes.txt:");
        assert_eq!(cut_short_line(12), "[Cut short: 12 more characters]");
        assert_eq!(cut_short_line(1), "[Cut short: 1 more character]");
        assert_eq!(
            rows_shown_line(500, 12_000),
            "Showing 500 of 12000. Search to find the rest."
        );
        // A user's text is placed once and never read as a slot.
        assert_eq!(attached_line("{name}"), "Attached {name}:");
    }

    #[test]
    fn batch_lines_summarise_then_count_then_warn() {
        let batch = ImportBatch {
            candidates: vec![
                candidate(ImportSource::Grok, "g", 1),
                candidate(ImportSource::ChatGpt, "a", 1),
                candidate(ImportSource::ChatGpt, "b", 1),
            ],
            counts: BatchCounts {
                conversations: 3,
                skipped_empty: 1,
                dropped_branches: 2,
                ..Default::default()
            },
            warnings: vec![CLAUDE_MANIFEST.into()],
            best_effort: true,
            ..Default::default()
        };
        assert_eq!(
            batch_lines(&batch),
            vec![
                "3 chats from ChatGPT and Grok.".to_string(),
                "1 empty chat was left out.".to_string(),
                "2 other branches were left out. Each chat comes in as the branch you saw last."
                    .to_string(),
                CLAUDE_MANIFEST.to_string(),
            ]
        );
        let every = ImportBatch {
            candidates: vec![
                candidate(ImportSource::ClaudeAi, "c", 1),
                candidate(ImportSource::Grok, "g", 1),
                candidate(ImportSource::ChatGpt, "a", 1),
            ],
            counts: BatchCounts {
                conversations: 1,
                skipped_empty: 2,
                skipped_unknown: 1,
                duplicates: 2,
                dropped_branches: 1,
                over_limit: 7,
                files_read: 3,
            },
            ..Default::default()
        };
        assert_eq!(
            batch_lines(&every),
            vec![
                "1 chat from ChatGPT, Claude.ai and Grok.".to_string(),
                "2 empty chats were left out.".to_string(),
                SKIPPED_UNKNOWN_ONE.to_string(),
                "2 chats came twice and were imported once.".to_string(),
                DROPPED_BRANCHES_ONE.to_string(),
                "7 chats over the 50,000 limit were left out.".to_string(),
            ]
        );
        assert!(batch_lines(&ImportBatch::default()).is_empty());
    }

    #[test]
    fn export_rows_say_where_and_when() {
        let now = EpochMs::saturating_from_i64(3 * 86_400_000);
        assert_eq!(
            export_row_detail(&candidate(ImportSource::ChatGpt, "a", 86_400_000), now),
            "ChatGPT, 2d, transcript"
        );
        assert_eq!(
            export_row_detail(&candidate(ImportSource::Grok, "g", 3 * 86_400_000), now),
            "Grok, now, transcript, best effort"
        );
    }

    #[test]
    fn persona_lines_follow_the_preview_order() {
        let persona = PersonaCandidate {
            id: "p1".into(),
            source: ImportSource::OpenClaw,
            folder: "/home/example/clawd".into(),
            name: "Pinch".into(),
            instructions: "Be kind.".into(),
            memory: "## OpenClaw: USER.md\n\nTea".into(),
            files: vec![
                file("IDENTITY.md", FileUse::Identity),
                file("SOUL.md", FileUse::Instructions),
                file("AGENTS.md", FileUse::Instructions),
                file("USER.md", FileUse::Memory),
                file("memory/2026-01-26.md", FileUse::Memory),
                skipped("HEARTBEAT.md", SkipReason::NotImported),
                skipped("skills/", SkipReason::NotImported),
                skipped("TOOLS.md", SkipReason::Link),
                skipped("MEMORY.md", SkipReason::TooLarge),
                skipped("memory/2026-01-27.md", SkipReason::NotAFile),
                skipped("memory/2026-01-28.md", SkipReason::Unreadable),
            ],
            clipped: true,
            dropped_identity: vec!["Emoji".into(), "Avatar".into()],
        };
        assert_eq!(
            persona_lines(&persona),
            vec![
                "Instructions from IDENTITY.md, SOUL.md and AGENTS.md.".to_string(),
                "Memory from USER.md and memory/2026-01-26.md.".to_string(),
                MEMORY_WAITS.to_string(),
                "Not imported: HEARTBEAT.md and skills/.".to_string(),
                "Skipped TOOLS.md: it's a link.".to_string(),
                "Skipped MEMORY.md: it's over 262 KB.".to_string(),
                "Skipped memory/2026-01-27.md: it isn't a file.".to_string(),
                "Skipped memory/2026-01-28.md: Bruhs couldn't read it.".to_string(),
                "Emoji and Avatar aren't carried over: bruhs use faces.".to_string(),
                PERSONA_CLIPPED.to_string(),
                STARTS_READ_ONLY.to_string(),
            ]
        );
        let bare = PersonaCandidate {
            memory: String::new(),
            files: vec![file("SOUL.md", FileUse::Instructions)],
            clipped: false,
            dropped_identity: vec!["Emoji".into()],
            ..persona
        };
        assert_eq!(
            persona_lines(&bare),
            vec![
                "Instructions from SOUL.md.".to_string(),
                "Emoji isn't carried over: bruhs use faces.".to_string(),
                STARTS_READ_ONLY.to_string(),
            ]
        );
    }
}
