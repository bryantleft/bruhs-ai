//! Read and group theme files on the background executor. A malformed file
//! does not prevent another theme from the same selection from being installed.
use super::library::{InstalledTheme, ThemeFile, MAX_THEME_BYTES};
use super::vscode::display_name;
use bruhs_ui::theme::ThemeMode;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(super) enum Selection {
    Canceled,
    Single {
        name: String,
        text: String,
    },
    Batch {
        themes: Vec<Arc<InstalledTheme>>,
        errors: Vec<String>,
    },
}

fn read(path: &Path) -> Result<String, String> {
    let mut text = String::new();
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    file.take(MAX_THEME_BYTES as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    if text.len() > MAX_THEME_BYTES {
        return Err("Theme files must be 256 KiB or smaller.".into());
    }
    Ok(text)
}

pub(super) fn read_selection(paths: Vec<PathBuf>) -> Result<Selection, String> {
    let filename = |p: &Path| {
        p.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    if paths.is_empty() {
        return Ok(Selection::Canceled);
    }
    if let [path] = paths.as_slice() {
        let name = filename(path);
        return Ok(Selection::Single {
            text: read(path).map_err(|e| format!("{name}: {e}"))?,
            name,
        });
    }
    let mut parsed = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        match read(&path).and_then(|s| InstalledTheme::parse(&s)) {
            Ok(theme) => parsed.push((path, theme)),
            Err(e) => errors.push(format!("{}: {e}", filename(&path))),
        }
    }
    Ok(Selection::Batch {
        themes: pair_variants(unique_names(parsed), |_, _| String::new())
            .into_iter()
            .map(Arc::new)
            .collect(),
        errors,
    })
}

pub(super) fn filename_label(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(display_name)
        .filter(|n| !n.is_empty())
}

pub(super) fn unique_names(parsed: Vec<(PathBuf, InstalledTheme)>) -> Vec<InstalledTheme> {
    let mut counts = BTreeMap::new();
    for (_, theme) in &parsed {
        *counts.entry(theme.file.id.clone()).or_insert(0usize) += 1;
    }
    let mut used = BTreeSet::new();
    parsed
        .into_iter()
        .map(|(path, mut theme)| {
            if counts[&theme.file.id] > 1 {
                if let Some(renamed) = filename_label(&path)
                    .filter(|name| !name.eq_ignore_ascii_case(&theme.file.name))
                    .and_then(|name| theme.relabel(name))
                {
                    theme = renamed;
                }
            }
            if used.contains(&theme.file.id) {
                for n in 2..100 {
                    if let Some(renamed) = theme.relabel(format!("{} {n}", theme.file.name)) {
                        if !used.contains(&renamed.file.id) {
                            theme = renamed;
                            break;
                        }
                    }
                }
            }
            used.insert(theme.file.id.clone());
            theme
        })
        .collect()
}

fn is_ascii_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Remove ASCII word-bounded `light` / `dark` tokens (case-insensitive),
/// returning `None` if neither token appears.
fn strip_mode_words(name: &str) -> Option<String> {
    let bytes = name.as_bytes();
    let mut out = String::with_capacity(name.len());
    let mut matched = false;
    let mut i = 0;
    while i < bytes.len() {
        let prev_word = i > 0 && is_ascii_word(bytes[i - 1]);
        if !prev_word {
            let word_len = if bytes[i..].len() >= 5
                && bytes[i..i + 5].eq_ignore_ascii_case(b"light")
                && !bytes.get(i + 5).copied().is_some_and(is_ascii_word)
            {
                Some(5)
            } else if bytes[i..].len() >= 4
                && bytes[i..i + 4].eq_ignore_ascii_case(b"dark")
                && !bytes.get(i + 4).copied().is_some_and(is_ascii_word)
            {
                Some(4)
            } else {
                None
            };
            if let Some(len) = word_len {
                matched = true;
                out.push(' ');
                i += len;
                continue;
            }
        }
        let Some(ch) = name[i..].chars().next() else {
            break;
        };
        out.push(ch);
        i += ch.len_utf8();
    }
    matched.then(|| out.split_whitespace().collect::<Vec<_>>().join(" "))
}

pub(super) fn pair_variants(
    themes: Vec<InstalledTheme>,
    paired_id: impl Fn(&str, &str) -> String,
) -> Vec<InstalledTheme> {
    let mut groups = BTreeMap::<String, Vec<usize>>::new();
    for (index, theme) in themes.iter().enumerate() {
        if !theme.file.variants.is_empty() {
            continue;
        }
        let Some(name) = strip_mode_words(&theme.file.name) else {
            continue;
        };
        if name.bytes().any(|c| c.is_ascii_alphanumeric()) {
            groups.entry(name).or_default().push(index);
        }
    }
    let mut result: Vec<_> = themes.into_iter().map(Some).collect();
    for (name, indices) in groups {
        let [first, second] = indices.as_slice() else {
            continue;
        };
        let (Some(a), Some(b)) = (result[*first].as_ref(), result[*second].as_ref()) else {
            continue;
        };
        let (a, b) = (&a.file, &b.file);
        if a.appearance == b.appearance {
            continue;
        }
        let (light, dark) = if a.appearance == ThemeMode::Light {
            (a, b)
        } else {
            (b, a)
        };
        let mut pair = ThemeFile::single(
            paired_id(&light.id, &dark.id),
            name,
            ThemeMode::Light,
            light.colors.clone(),
        );
        pair.variants.insert(ThemeMode::Dark, dark.colors.clone());
        if let Ok(theme) = InstalledTheme::try_from(pair) {
            result[*first] = Some(theme);
            result[*second] = None;
        }
    }
    result.into_iter().flatten().collect()
}
