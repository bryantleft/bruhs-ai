//! A Hermes Agent home, or one of its profiles, as a persona (D10, R§5):
//! `SOUL.md` for the job, and `memories/USER.md` then `memories/MEMORY.md`
//! for memory, split on `§` and deduplicated. Skills, cron jobs and
//! sessions are listed by name only.

use super::files::{entries, planned, read_text, FileRead, Kind};
use super::{persona_id, shown_path, MAX_LISTED};
use bruhs_models::imports::{
    assemble_persona, hermes_entries, FileUse, PersonaCandidate, PersonaFile, PersonaTexts,
    SkipReason,
};
use bruhs_models::logic::portable::ImportSource;
use std::collections::HashSet;
use std::path::Path;

const SOUL: &str = "SOUL.md";
const MEMORIES: &str = "memories";
/// Read into memory, in this order.
const MEMORY_FILES: [&str; 2] = ["USER.md", "MEMORY.md"];
/// Listed by name only.
const NOT_IMPORTED_FOLDERS: [&str; 3] = ["skills", "cron", "sessions"];

/// The Hermes home `home` (`profile` `None`), or its profile `profile`,
/// read as a persona; `None` when it has neither `SOUL.md` nor
/// `memories/`. Every file opens by its name under `home`, so a linked
/// `profiles`, profile or `memories` folder is never gone through.
pub(crate) fn persona(home: &Path, profile: Option<&str>) -> Option<PersonaCandidate> {
    let (folder, prefix) = match profile {
        Some(name) => {
            if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
                return None;
            }
            (
                home.join("profiles").join(name),
                format!("profiles/{name}/"),
            )
        }
        None => (home.to_path_buf(), String::new()),
    };
    let listed = entries(&folder, MAX_LISTED);
    let kind_of = |name: &str| {
        listed
            .iter()
            .find(|(listed, _)| listed == name)
            .map(|(_, kind)| *kind)
    };
    let (soul, memories) = (kind_of(SOUL), kind_of(MEMORIES));
    if soul.is_none() && memories.is_none() {
        return None;
    }
    let source = ImportSource::Hermes;
    let mut texts = PersonaTexts {
        source,
        fallback_name: profile.unwrap_or(source.label()).to_string(),
        identity: None,
        instructions: Vec::new(),
        memory: Vec::new(),
    };
    let mut files = Vec::new();
    let read = |name: &str, use_: FileUse, files: &mut Vec<PersonaFile>| match read_text(
        home,
        &format!("{prefix}{name}"),
    ) {
        FileRead::Text(text) => {
            files.push(file(name.to_string(), use_));
            Some(text)
        }
        FileRead::Missing => None,
        FileRead::Skipped(reason) => {
            files.push(file(name.to_string(), FileUse::Skipped { reason }));
            None
        }
    };
    if let Some(kind) = soul {
        match planned(kind, FileUse::Instructions) {
            FileUse::Instructions => {
                if let Some(text) = read(SOUL, FileUse::Instructions, &mut files) {
                    texts.instructions.push((SOUL.to_string(), text));
                }
            }
            skipped => files.push(file(SOUL.to_string(), skipped)),
        }
    }
    match memories {
        Some(Kind::Folder) => {
            // An entry already in USER.md isn't repeated from MEMORY.md.
            let mut seen = HashSet::new();
            for name in MEMORY_FILES {
                let name = format!("{MEMORIES}/{name}");
                if let Some(text) = read(&name, FileUse::Memory, &mut files) {
                    let kept: Vec<String> = hermes_entries(&text)
                        .into_iter()
                        .filter(|entry| seen.insert(entry.clone()))
                        .collect();
                    texts.memory.push((name, kept.join("\n\n")));
                }
            }
        }
        Some(kind) => files.push(file(
            format!("{MEMORIES}/"),
            FileUse::Skipped {
                reason: if kind == Kind::Link {
                    SkipReason::Link
                } else {
                    SkipReason::NotAFile
                },
            },
        )),
        None => {}
    }
    for name in NOT_IMPORTED_FOLDERS {
        if kind_of(name).is_some() {
            files.push(file(
                format!("{name}/"),
                FileUse::Skipped {
                    reason: SkipReason::NotImported,
                },
            ));
        }
    }
    let assembled = assemble_persona(&texts);
    Some(PersonaCandidate {
        id: persona_id(&folder),
        source,
        folder: shown_path(&folder),
        name: assembled.name,
        instructions: assembled.instructions,
        memory: assembled.memory,
        files,
        clipped: assembled.clipped,
        dropped_identity: assembled.dropped_identity,
        left_out_logs: 0,
    })
}

fn file(name: String, use_: FileUse) -> PersonaFile {
    PersonaFile { name, use_ }
}
