use super::super::{
    import::{pair_variants, unique_names},
    vscode,
};
use super::{
    Extension, FILE_LIMIT, InstalledTheme, check_license, jsonc, manifest_themes, theme_id,
};
use serde_json::{Map, Value};
use std::{
    collections::{HashMap, HashSet},
    io::{Cursor, Read},
    path::PathBuf,
    sync::Arc,
};
use zip::ZipArchive;

type Archive<'a> = ZipArchive<Cursor<&'a [u8]>>;

fn relative_path(path: &str, parent: &str) -> Result<String, String> {
    let normalized = path.replace('\\', "/");
    if normalized.len() > 1024
        || normalized.contains('\0')
        || normalized.starts_with('/')
        || normalized.as_bytes().get(1) == Some(&b':')
    {
        return Err("Invalid path in the extension.".into());
    }
    let mut parts: Vec<_> = parent.split('/').collect();
    parts.pop();
    for part in normalized.split('/') {
        match part {
            "." | "" => {}
            ".." if parts.len() > 1 => {
                parts.pop();
            }
            ".." => return Err("Theme path escapes the extension.".into()),
            _ => parts.push(part),
        }
    }
    Ok(parts.join("/"))
}

fn read_json(zip: &mut Archive<'_>, path: &str) -> Result<Value, String> {
    let entry = zip
        .by_name(path)
        .map_err(|_| format!("Missing {path} in the extension."))?;
    if entry.size() > FILE_LIMIT as u64 {
        return Err(format!("{path} is too large."));
    }
    let mut buf = Vec::new();
    entry
        .take(FILE_LIMIT as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    if buf.len() > FILE_LIMIT {
        return Err(format!("{path} is too large."));
    }
    jsonc(&buf)
}

// Open VSX packages contribute only the workbench keys used by T3's
// extension importer. Standalone local theme files use the full converter.
fn supported_color(key: &str) -> bool {
    matches!(
        key,
        "activityBar.background"
            | "activityBarBadge.background"
            | "badge.background"
            | "button.background"
            | "button.foreground"
            | "contrastBorder"
            | "descriptionForeground"
            | "disabledForeground"
            | "dropdown.background"
            | "dropdown.border"
            | "editor.background"
            | "editor.foreground"
            | "editor.findMatchBackground"
            | "editor.findMatchForeground"
            | "editor.findMatchHighlightBackground"
            | "editor.findMatchHighlightForeground"
            | "editor.selectionBackground"
            | "editorCursor.foreground"
            | "editorError.foreground"
            | "editorGroup.border"
            | "editorPane.background"
            | "editorWarning.foreground"
            | "editorWidget.background"
            | "errorForeground"
            | "focusBorder"
            | "foreground"
            | "input.border"
            | "input.placeholderForeground"
            | "list.activeSelectionBackground"
            | "list.hoverBackground"
            | "list.inactiveSelectionBackground"
            | "menu.background"
            | "panel.background"
            | "panel.border"
            | "progressBar.background"
            | "quickInput.background"
            | "scrollbarSlider.background"
            | "sideBar.background"
            | "sideBar.border"
            | "sideBar.foreground"
            | "terminal.background"
            | "terminal.foreground"
            | "terminal.selectionBackground"
            | "terminalCursor.foreground"
            | "textCodeBlock.background"
            | "textLink.foreground"
    )
}

fn resolve_colors(
    zip: &mut Archive<'_>,
    path: &str,
    cache: &mut HashMap<String, Map<String, Value>>,
    parents: &mut Vec<String>,
) -> Result<Map<String, Value>, String> {
    if parents.len() >= 8 || parents.iter().any(|s| s == path) {
        return Err("Theme includes are cyclic or nested too deeply.".into());
    }
    if let Some(value) = cache.get(path) {
        return Ok(value.clone());
    }
    if cache.len() + parents.len() >= 320 {
        return Err("The extension references too many files.".into());
    }
    let json = read_json(zip, path)?;
    let mut colors = if let Some(include) = json["include"].as_str() {
        parents.push(path.into());
        let merged = resolve_colors(zip, &relative_path(include, path)?, cache, parents)?;
        parents.pop();
        merged
    } else {
        Map::new()
    };
    if let Some(child) = json["colors"].as_object() {
        colors.extend(
            child
                .iter()
                .filter(|(k, v)| supported_color(k) && v.as_str().is_some_and(|s| s.len() <= 128))
                .map(|(k, v)| (k.clone(), v.clone())),
        );
    }
    cache.insert(path.into(), colors.clone());
    Ok(colors)
}

// Bound the central directory before the ZIP reader allocates its index.
// Open VSX themes use ordinary single-disk VSIX archives.
fn check_directory(bytes: &[u8]) -> Result<(), String> {
    const FOOTER: usize = 22;
    let start = bytes.len().saturating_sub(FOOTER + u16::MAX as usize);
    let footer = (start..=bytes.len().saturating_sub(FOOTER))
        .rev()
        .find_map(|at| {
            let tail = bytes.get(at..)?;
            if tail.len() < FOOTER || !tail.starts_with(b"PK\x05\x06") {
                return None;
            }
            let comment = usize::from(u16::from_le_bytes([tail[20], tail[21]]));
            (FOOTER + comment == tail.len()).then_some((at, tail))
        })
        .ok_or("The extension has no readable ZIP directory.")?;
    let (end, tail) = footer;
    let count = u16::from_le_bytes([tail[10], tail[11]]);
    let length = u32::from_le_bytes([tail[12], tail[13], tail[14], tail[15]]) as usize;
    let start = u32::from_le_bytes([tail[16], tail[17], tail[18], tail[19]]) as usize;
    if count > 5000 || tail[4..8] != [0; 4] || start.checked_add(length) != Some(end) {
        return Err("The extension exceeds the supported ZIP directory limits.".into());
    }
    Ok(())
}

pub(super) fn convert(ext: &Extension, bytes: &[u8]) -> Result<Vec<Arc<InstalledTheme>>, String> {
    check_directory(bytes)?;
    let mut zip = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("Could not open the extension. {e}"))?;
    if zip.len() > 5000 {
        return Err("The extension contains too many files.".into());
    }
    let mut expanded = 0u64;
    for index in 0..zip.len() {
        let entry = zip.by_index_raw(index).map_err(|e| e.to_string())?;
        relative_path(&entry.name().map_err(|e| e.to_string())?, "extension/")?;
        expanded = expanded.saturating_add(entry.size());
        if expanded > 100 * 1024 * 1024
            || entry.size() > entry.compressed_size().saturating_mul(200)
        {
            return Err("The extension exceeds the supported package limits.".into());
        }
    }
    let manifest = read_json(&mut zip, "extension/package.json")?;
    let identity = format!(
        "{}.{}",
        manifest["publisher"].as_str().unwrap_or(""),
        manifest["name"].as_str().unwrap_or("")
    );
    if !identity.eq_ignore_ascii_case(&ext.id)
        || manifest["version"].as_str() != Some(&ext.version)
        || !check_license(&manifest, &ext.license)
    {
        return Err(
            "The package does not match the selected extension, version, or license.".into(),
        );
    }
    let themes = manifest_themes(&manifest)?;
    if themes.len() > 40 {
        return Err("The extension contains too many themes.".into());
    }
    let id = ext.id.to_lowercase();
    let mut cache = HashMap::new();
    let mut seen_paths = HashMap::<String, usize>::new();
    let mut ids = Vec::new();
    let mut parsed = Vec::new();
    for item in themes {
        let path = relative_path(
            item["path"].as_str().ok_or("Missing theme path.")?,
            "extension/",
        )?;
        let colors = resolve_colors(&mut zip, &path, &mut cache, &mut Vec::new())?;
        let label = item["label"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&ext.name);
        let mode = match item["uiTheme"].as_str() {
            Some("vs") => "light",
            Some("vs-dark") => "dark",
            Some("hc-black") => "hc-black",
            Some("hc-light") => "hc-light",
            _ => "",
        };
        let source = serde_json::json!({"colors":colors,"type":mode,"displayName":label});
        let theme = InstalledTheme::try_from(vscode::convert(&source)?)?;
        let count = seen_paths.entry(path.clone()).or_default();
        let key = if *count == 0 {
            path.clone()
        } else {
            format!("{path}\0{count}")
        };
        *count += 1;
        ids.push(theme_id(&id, &key));
        parsed.push((PathBuf::from(path), theme));
    }
    let mut themes = unique_names(parsed);
    for (theme, key) in themes.iter_mut().zip(ids) {
        theme.file.id = key;
    }
    let paired = pair_variants(themes, |a, b| {
        let mut ids = [a, b];
        ids.sort();
        theme_id(&id, &ids.join(":"))
    });
    let mut distinct = HashSet::new();
    let mut length = 0;
    let label: String = ext
        .name
        .chars()
        .take_while(|ch| {
            length += ch.len_utf16();
            length <= 48
        })
        .collect();
    paired
        .into_iter()
        .map(|mut t| {
            if !distinct.insert(t.file.id.clone()) {
                return Err("The collection contains duplicate themes.".into());
            }
            t.file.set_collection(&ext.collection, &label);
            Ok(Arc::new(t))
        })
        .collect()
}
