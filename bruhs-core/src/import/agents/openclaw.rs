//! An OpenClaw workspace as a persona (D10, R§4): `IDENTITY.md` for the
//! name, `SOUL.md`, `AGENTS.md` and `TOOLS.md` for the job, and
//! `USER.md`, `MEMORY.md` and the daily logs for memory. The retired and
//! internal files are listed by name only.

use super::files::{entries, planned, read_text, select, FileRead, Kind, Pick};
use super::{persona_id, shown_path, MAX_LISTED};
use crate::import::text::{shown_name, NAME_MAX_CHARS};
use bruhs_models::imports::{
    assemble_persona, identity_fields, FileUse, PersonaCandidate, PersonaFile, PersonaTexts,
    SkipReason, MAX_DAILY_LOGS,
};
use bruhs_models::logic::portable::ImportSource;
use bruhs_models::reactive::MAX_TEXT_CHARS;
use std::path::Path;

const IDENTITY: &str = "IDENTITY.md";
/// Read into the job, in this order.
const INSTRUCTIONS: [&str; 3] = ["SOUL.md", "AGENTS.md", "TOOLS.md"];
/// Read into memory, before the daily logs.
const MEMORY: [&str; 2] = ["USER.md", "MEMORY.md"];
/// The folder of daily logs, `memory/YYYY-MM-DD.md`.
const DAILY: &str = "memory";
/// Listed by name only.
const NOT_IMPORTED: [&str; 4] = ["HEARTBEAT.md", "BOOT.md", "BOOTSTRAP.md", "DREAMS.md"];
const NOT_IMPORTED_FOLDERS: [&str; 2] = ["skills", "canvas"];

/// The workspace at `dir` read as a persona, or `None` when it holds none
/// of the persona files.
pub(crate) fn persona(dir: &Path) -> Option<PersonaCandidate> {
    let (planned, past_cap) = plan(dir);
    // Only the persona files, read or skipped, make a folder a workspace.
    if planned.iter().all(|file| file.use_ == not_imported()) {
        return None;
    }
    let source = ImportSource::OpenClaw;
    let mut texts = PersonaTexts {
        source,
        fallback_name: source.label().to_string(),
        identity: None,
        instructions: Vec::new(),
        memory: Vec::new(),
    };
    let mut files = Vec::with_capacity(planned.len());
    let mut memory_chars = 0;
    // Daily logs past a full memory would be cut anyway: they aren't
    // opened, and are counted with those past `MAX_DAILY_LOGS`.
    let mut left_out = past_cap;
    for file in planned {
        if !matches!(
            file.use_,
            FileUse::Identity | FileUse::Instructions | FileUse::Memory
        ) {
            files.push(file);
            continue;
        }
        if file.use_ == FileUse::Memory && memory_chars > MAX_TEXT_CHARS {
            left_out = left_out.saturating_add(1);
            continue;
        }
        match read_text(dir, &file.name) {
            FileRead::Text(text) => {
                match file.use_ {
                    FileUse::Identity => texts.identity = Some(identity_fields(&text)),
                    FileUse::Instructions => texts.instructions.push((file.name.clone(), text)),
                    _ => {
                        memory_chars += text.chars().count();
                        texts.memory.push((file.name.clone(), text));
                    }
                }
                files.push(file);
            }
            // Gone since it was listed.
            FileRead::Missing => {}
            FileRead::Skipped(reason) => files.push(PersonaFile {
                name: file.name,
                use_: FileUse::Skipped { reason },
            }),
        }
    }
    let assembled = assemble_persona(&texts);
    Some(PersonaCandidate {
        id: persona_id(dir),
        source,
        folder: shown_path(dir),
        name: assembled.name,
        instructions: assembled.instructions,
        memory: assembled.memory,
        files,
        clipped: assembled.clipped || left_out > 0,
        dropped_identity: assembled.dropped_identity,
        left_out_logs: left_out,
    })
}

/// The labels of the `IDENTITY.md` fields Bruhs knows, in file order,
/// never a value.
#[cfg(test)]
pub(crate) fn identity_labels(dir: &Path) -> Vec<String> {
    const KNOWN: [&str; 6] = ["Name", "Creature", "Vibe", "Emoji", "Avatar", "Theme"];
    let FileRead::Text(text) = read_text(dir, IDENTITY) else {
        return Vec::new();
    };
    let mut labels: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix('-').map_or(line, str::trim_start);
        let line = line.strip_prefix("**").unwrap_or(line);
        let Some((label, _)) = line.split_once(':') else {
            continue;
        };
        let label = label.trim().trim_end_matches('*').trim();
        if let Some(known) = KNOWN.iter().find(|known| known.eq_ignore_ascii_case(label)) {
            if !labels.iter().any(|seen| seen == known) {
                labels.push((*known).to_string());
            }
        }
    }
    labels
}

/// The workspace's files by name and use, in reading order, opening no
/// file: the persona files, then those listed as not imported. Only the
/// names-only live check calls it on its own.
#[cfg(test)]
pub(crate) fn inventory(dir: &Path) -> Vec<PersonaFile> {
    plan(dir).0
}

/// [`inventory`], and how many daily logs past `MAX_DAILY_LOGS` it left
/// out (the oldest).
fn plan(dir: &Path) -> (Vec<PersonaFile>, u32) {
    let listed = entries(dir, MAX_LISTED);
    let kind_of = |name: &str| {
        listed
            .iter()
            .find(|(listed, _)| listed == name)
            .map(|(_, kind)| *kind)
    };
    let mut files = Vec::new();
    let mut past_cap = 0;
    let persona_files = std::iter::once((IDENTITY, FileUse::Identity))
        .chain(INSTRUCTIONS.map(|name| (name, FileUse::Instructions)))
        .chain(MEMORY.map(|name| (name, FileUse::Memory)));
    for (name, use_) in persona_files {
        if let Some(kind) = kind_of(name) {
            files.push(file(name.to_string(), planned(kind, use_)));
        }
    }
    let mut other_logs = Vec::new();
    match kind_of(DAILY) {
        Some(Kind::Folder) => {
            let daily = dir.join(DAILY);
            let (mut logs, matched) = select(&daily, MAX_DAILY_LOGS, is_daily_log, Pick::Last);
            past_cap = u32::try_from(matched.saturating_sub(logs.len())).unwrap_or(u32::MAX);
            logs.reverse();
            let (rest, _) = select(&daily, MAX_LISTED, |name| !is_daily_log(name), Pick::First);
            for (name, kind) in logs {
                files.push(file(
                    format!("{DAILY}/{name}"),
                    planned(kind, FileUse::Memory),
                ));
            }
            for (name, kind) in rest {
                let slash = if kind == Kind::Folder { "/" } else { "" };
                if let Some(name) = listed_only(&name) {
                    other_logs.push(file(format!("{DAILY}/{name}{slash}"), not_imported()));
                }
            }
        }
        Some(Kind::Link) => {
            files.push(file(
                format!("{DAILY}/"),
                FileUse::Skipped {
                    reason: SkipReason::Link,
                },
            ));
        }
        _ => {}
    }
    for name in NOT_IMPORTED {
        if kind_of(name).is_some() {
            files.push(file(name.to_string(), not_imported()));
        }
    }
    for (name, _) in &listed {
        if name.ends_with(".dev.md") {
            if let Some(name) = listed_only(name) {
                files.push(file(name, not_imported()));
            }
        }
    }
    for name in NOT_IMPORTED_FOLDERS {
        if kind_of(name).is_some() {
            files.push(file(format!("{name}/"), not_imported()));
        }
    }
    files.extend(other_logs);
    (files, past_cap)
}

fn file(name: String, use_: FileUse) -> PersonaFile {
    PersonaFile { name, use_ }
}

/// A name listed but never opened, as one line the preview can show:
/// hidden characters dropped and cut short (the persona files' own names
/// are fixed ASCII).
fn listed_only(name: &str) -> Option<String> {
    let shown = shown_name(name, NAME_MAX_CHARS);
    (!shown.is_empty()).then_some(shown)
}

fn not_imported() -> FileUse {
    FileUse::Skipped {
        reason: SkipReason::NotImported,
    }
}

/// `YYYY-MM-DD.md`.
fn is_daily_log(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 13
        && name.ends_with(".md")
        && bytes[..10].iter().enumerate().all(|(at, byte)| {
            if at == 4 || at == 7 {
                *byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        })
}
