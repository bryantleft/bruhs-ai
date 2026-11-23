use super::*;
use bruhs_ui::theme::{Palette, ThemeMode};
use serde_json::Value;

pub(super) const MAX_THEME_BYTES: usize = 262_144;

pub(super) type Colors = BTreeMap<String, String>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ThemeFile {
    version: u8,
    #[serde(default)]
    pub(super) id: String,
    pub(super) name: String,
    pub(super) appearance: ThemeMode,
    pub(super) colors: Colors,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) variants: BTreeMap<ThemeMode, Colors>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(try_from = "ThemeFile")]
pub(super) struct InstalledTheme {
    pub(super) file: ThemeFile,
    palettes: BTreeMap<ThemeMode, InstalledPalette>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct InstalledPalette {
    pub(super) palette: Palette,
    pub(super) images: super::background::Styles,
    pub(super) roles: bruhs_ui::theme::ColorRoles,
}

impl Serialize for InstalledTheme {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.file.serialize(serializer)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(from = "SavedLibrary")]
pub(in crate::views::workspace) struct ThemeLibrary {
    pub(super) themes: Vec<Arc<InstalledTheme>>,
    pub(super) light: Option<String>,
    pub(super) dark: Option<String>,
}

// Recover the rest of the workspace even when an individual saved theme is damaged.
#[derive(Default, Deserialize)]
#[serde(default)]
struct SavedLibrary {
    themes: Vec<Value>,
    light: Option<String>,
    dark: Option<String>,
}

impl From<SavedLibrary> for ThemeLibrary {
    fn from(saved: SavedLibrary) -> Self {
        let mut library = Self::default();
        for value in saved.themes {
            match serde_json::from_value::<InstalledTheme>(value) {
                Ok(theme) if library.find(&theme.file.id).is_none() => {
                    library.themes.push(Arc::new(theme))
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "Ignoring an invalid saved theme"),
            }
        }
        for (mode, id) in [
            (ThemeMode::Light, saved.light),
            (ThemeMode::Dark, saved.dark),
        ] {
            if id
                .as_deref()
                .and_then(|id| library.find(id))
                .and_then(|t| t.palette(mode))
                .is_some()
            {
                library.select(mode, id);
            }
        }
        library
    }
}

impl ThemeLibrary {
    /// Preserve install order, placing each collection at its first member.
    pub(super) fn card_groups(&self) -> Vec<Vec<Arc<InstalledTheme>>> {
        let mut groups = Vec::new();
        let mut positions = BTreeMap::new();
        for t in &self.themes {
            let index = match t.file.collection() {
                Some(id) => *positions.entry(id).or_insert(groups.len()),
                None => groups.len(),
            };
            if index == groups.len() {
                groups.push(Vec::new());
            }
            groups[index].push(t.clone());
        }
        groups
    }

    pub(super) fn collection(&self, id: &str) -> Vec<Arc<InstalledTheme>> {
        self.themes
            .iter()
            .filter(|t| t.file.collection() == Some(id))
            .cloned()
            .collect()
    }

    pub(super) fn replace_collection(
        &mut self,
        id: &str,
        expected: &[Arc<InstalledTheme>],
        incoming: Vec<Arc<InstalledTheme>>,
    ) -> Result<(), String> {
        if self.collection(id) != expected {
            return Err(
                "Your installed themes changed while this package was downloading. Try again."
                    .into(),
            );
        }
        if incoming.is_empty()
            || incoming.iter().any(|t| {
                t.file.collection() != Some(id)
                    || self
                        .find(&t.file.id)
                        .is_some_and(|old| old.file.collection() != Some(id))
            })
        {
            return Err("One of these theme IDs already belongs to another theme.".into());
        }
        let at = self
            .themes
            .iter()
            .position(|t| t.file.collection() == Some(id))
            .unwrap_or(self.themes.len());
        self.themes.retain(|t| t.file.collection() != Some(id));
        self.themes.splice(at..at, incoming);
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            if self
                .selection(mode)
                .is_some_and(|id| self.find(id).and_then(|t| t.palette(mode)).is_none())
            {
                self.select(mode, None);
            }
        }
        Ok(())
    }

    pub(in crate::views::workspace) fn empty(&self) -> bool {
        self.themes.is_empty()
    }
    pub(in crate::views::workspace) fn has_selection(&self) -> bool {
        self.light.is_some() || self.dark.is_some()
    }
    pub(in crate::views::workspace) fn reset_selection(&mut self) {
        self.light = None;
        self.dark = None;
    }
    pub(super) fn find(&self, id: &str) -> Option<&Arc<InstalledTheme>> {
        self.themes.iter().find(|theme| theme.file.id == id)
    }
    pub(super) fn selection(&self, mode: ThemeMode) -> Option<&str> {
        match mode {
            ThemeMode::Light => self.light.as_deref(),
            ThemeMode::Dark => self.dark.as_deref(),
        }
    }
    pub(super) fn select(&mut self, mode: ThemeMode, id: Option<String>) {
        match mode {
            ThemeMode::Light => self.light = id,
            ThemeMode::Dark => self.dark = id,
        }
    }
    pub(super) fn forget_selection(&mut self, id: &str) {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            if self.selection(mode) == Some(id) {
                self.select(mode, None);
            }
        }
    }
}

pub(super) const ROLES: &str = "canvas chrome toolbar toolbarForeground toolbarBorder toolbarControl toolbarControlForeground toolbarControlHover surface surfaceRaised surfaceOverlay text textMuted border input focus accent accentForeground secondary secondaryForeground muted mutedForeground placeholder secondaryLabel iconMuted error errorForeground errorSurface warning warningForeground warningSurface update updateForeground updateSurface accentSurface accentSurfaceForeground messageSurface messageForeground messageAction messageActionForeground messageActionHover codeBackground codeForeground searchMatchBackground searchMatchForeground searchMatchActiveBackground searchMatchActiveForeground sidebar sidebarForeground sidebarMutedForeground sidebarControlSurface sidebarRowHover sidebarRowActive sidebarRowSelected sidebarBorder terminalBackground terminalForeground terminalCursor terminalSelection terminalScrollbar terminalScrollbarHover";

pub(super) fn reserved(id: &str) -> bool {
    matches!(
        id,
        "bruhs" | "system" | "light" | "dark" | "grove" | "ocean" | "ember" | "iris"
    )
}

pub(super) fn identifier(name: &str) -> String {
    let name = name.trim().to_lowercase();
    let slug: String = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(48)
        .collect();
    if !slug.is_empty() {
        return slug;
    }
    let hash = name.encode_utf16().fold(2166136261u32, |hash, c| {
        (hash ^ u32::from(c)).wrapping_mul(16777619)
    });
    let mut n = hash;
    let mut digits = Vec::new();
    loop {
        if let Some(ch) = char::from_digit(n % 36, 36) {
            digits.push(ch);
        }
        n /= 36;
        if n == 0 {
            break;
        }
    }
    format!("theme-{}", digits.into_iter().rev().collect::<String>())
}

pub(super) fn native_palette(
    mode: ThemeMode,
    colors: &Colors,
    css: &str,
) -> Result<InstalledPalette, String> {
    if colors.is_empty() {
        return Err("Add at least one color role.".into());
    }
    let mut parsed = BTreeMap::new();
    for (role, literal) in colors {
        if !ROLES.split_ascii_whitespace().any(|key| key == role) {
            return Err(format!("Unknown theme color role “{role}”."));
        }
        let rgba = csscolorparser::parse(literal)
            .map_err(|_| format!("Enter a literal CSS color for {role}."))?
            .to_rgba8();
        parsed.insert(role.clone(), rgba);
    }
    let images=super::stylesheet::apply(css, mode, &mut parsed);
    let mut palette = *ThemeFamily::Bruhs.palette(mode);
    let resolve = |roles: &[&str], fallback: u32, canvas: u32| {
        let Some([r, g, b, a]) = roles.iter().find_map(|role| parsed.get(*role).copied()) else {
            return fallback;
        };
        let channel = |value: u8, shift: u32| {
            (u32::from(value) * u32::from(a) + ((canvas >> shift) & 255) * u32::from(255 - a) + 127)
                / 255
        };
        channel(r, 16) << 16 | channel(g, 8) << 8 | channel(b, 0)
    };
    let canvas = resolve(&["canvas"], palette.surface[0], palette.surface[0]);
    palette.surface[0] = canvas;
    for (value, roles) in palette.surface[1..].iter_mut().zip([
        &["sidebar", "surface", "chrome"][..],
        &["surfaceRaised", "surface"][..],
        &["border", "surfaceOverlay"][..],
        &["input", "border"][..],
    ]) {
        *value = resolve(roles, *value, canvas);
    }
    for (value, roles) in palette.text.iter_mut().zip([
        &["text"][..],
        &["secondaryForeground", "text"],
        &["codeForeground", "text"],
        &["textMuted", "mutedForeground"],
        &["placeholder", "textMuted"],
        &["iconMuted", "textMuted"],
    ]) {
        *value = resolve(roles, *value, canvas);
    }
    for (value, roles) in palette.accent.iter_mut().zip([
        &["updateForeground", "accent"][..],
        &["accentSurfaceForeground", "accent"],
        &["accentSurface", "accent"],
        &["focus", "accent"],
        &["accent"],
        &["messageActionHover", "accent"],
    ]) {
        *value = resolve(roles, *value, canvas);
    }
    Ok(InstalledPalette {
        images,
        palette,
        roles: Arc::new(parsed),
    })
}

impl TryFrom<ThemeFile> for InstalledTheme {
    type Error = String;
    fn try_from(mut file: ThemeFile) -> Result<Self, String> {
        if file.version != 1 {
            return Err("This theme needs version 1.".into());
        }
        file.name = file.name.trim().to_owned();
        if file.name.is_empty() || file.name.encode_utf16().count() > 48 {
            return Err("Use a theme name of 1 to 48 characters.".into());
        }
        if file.id.is_empty() {
            file.id = identifier(&file.name);
        }
        if file.id.len() > 48
            || !file.id.as_bytes()[0].is_ascii_alphanumeric()
            || !file
                .id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        {
            return Err(
                "Use a theme ID of up to 48 lowercase letters, numbers and hyphens.".into(),
            );
        }
        if reserved(&file.id) {
            return Err(format!("“{}” is reserved for a built-in theme.", file.id));
        }
        if let Some(css) = file.extra.get("css") {
            if !css
                .as_str()
                .is_some_and(|s| s.encode_utf16().count() <= 50000)
            {
                return Err("Theme CSS must be text of at most 50,000 characters.".into());
            }
        }
        if let Some(collection) = file.extra.get("collection") {
            let valid_id = collection["id"].as_str().is_some_and(|s| {
                !s.is_empty()
                    && s.len() <= 128
                    && s.as_bytes()[0].is_ascii_alphanumeric()
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b".:-".contains(&b))
            });
            let valid_name = collection["label"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty() && s.trim().encode_utf16().count() <= 48);
            if !valid_id || !valid_name {
                return Err("Enter a valid theme collection ID and label.".into());
            }
        }
        let mut palettes = BTreeMap::from([(
            file.appearance,
            native_palette(file.appearance, &file.colors, file.css())?,
        )]);
        for (&mode, colors) in &file.variants {
            if mode == file.appearance {
                return Err("The alternate palette must use the other appearance.".into());
            }
            palettes.insert(mode, native_palette(mode, colors, file.css())?);
        }
        Ok(Self { file, palettes })
    }
}

impl InstalledTheme {
    pub(super) fn relabel(&self, name: String) -> Option<Self> {
        let mut file = self.file.clone();
        file.name = name;
        file.id.clear();
        file.extra.remove("collection");
        Self::try_from(file).ok()
    }

    pub(super) fn keep_collection(&mut self, previous: &Self) {
        if let Some(collection) = previous.file.extra.get("collection") {
            self.file
                .extra
                .insert("collection".into(), collection.clone());
        }
    }

    pub(super) fn installed_palette(&self, mode: ThemeMode) -> Option<&InstalledPalette> {
        self.palettes.get(&mode)
    }
    pub(super) fn palette(&self, mode: ThemeMode) -> Option<&Palette> {
        self.palettes.get(&mode).map(|p| &p.palette)
    }
    pub(super) fn parse(json: &str) -> Result<Self, String> {
        if json.len() > MAX_THEME_BYTES {
            return Err("Theme files must be 256 KiB or smaller.".into());
        }
        // T3's native schema uses strict JSON. VS Code additionally accepts
        // line comments, block comments and trailing commas.
        let value: Value =
            serde_json_lenient::from_str(json).map_err(|e| format!("Invalid theme JSON. {e}"))?;
        let file: ThemeFile = if super::vscode::recognizes(&value) {
            super::vscode::convert(&value)?
        } else {
            serde_json::from_str(json).map_err(|e| format!("Invalid theme JSON. {e}"))?
        };
        Self::try_from(file)
    }
    pub(super) fn json(&self) -> String {
        format!(
            "{}\n",
            serde_json::to_string_pretty(&self.file).expect("theme JSON")
        )
    }
}

impl ThemeFile {
    pub(super) fn css(&self) -> &str {
        self.extra
            .get("css")
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    pub(super) fn set_css(&mut self, css: &str) {
        if css.trim().is_empty() {
            self.extra.remove("css");
        } else {
            self.extra
                .insert("css".into(), Value::String(css.to_owned()));
        }
    }

    pub(super) fn managed(&self) -> bool {
        self.extra.get("managed").and_then(Value::as_bool) == Some(true)
    }

    pub(super) fn collection(&self) -> Option<&str> {
        self.extra.get("collection")?.get("id")?.as_str()
    }

    pub(super) fn collection_label(&self) -> Option<&str> {
        self.extra.get("collection")?.get("label")?.as_str()
    }

    pub(super) fn set_collection(&mut self, id: &str, label: &str) {
        self.extra.insert(
            "collection".into(),
            serde_json::json!({"id":id,"label":label}),
        );
    }

    pub(super) fn single(id: String, name: String, appearance: ThemeMode, colors: Colors) -> Self {
        Self {
            version: 1,
            id,
            name,
            appearance,
            colors,
            variants: BTreeMap::new(),
            extra: BTreeMap::new(),
        }
    }

    pub(super) fn duplicate(&self) -> Self {
        let mut file = self.clone();
        file.name = self.copy_name();
        file.id.clear();
        file.extra.remove("collection");
        file.extra.remove("managed");
        file
    }

    pub(super) fn copy_name(&self) -> String {
        let mut prefix = String::new();
        let mut length = 0;
        for ch in self.name.chars() {
            length += ch.len_utf16();
            if length > 43 {
                break;
            }
            prefix.push(ch);
        }
        format!("{prefix} copy")
    }

    pub(super) fn colors_for(&self, mode: ThemeMode) -> Option<&Colors> {
        if self.appearance == mode {
            Some(&self.colors)
        } else {
            self.variants.get(&mode)
        }
    }

    pub(super) fn set_colors(&mut self, mode: ThemeMode, colors: Colors) {
        if self.appearance == mode {
            self.colors = colors;
        } else {
            self.variants.insert(mode, colors);
        }
    }

    pub(super) fn seed(name: &str, light: &Palette, dark: &Palette) -> Self {
        let colors = |p: &Palette| {
            [
                ("canvas", p.surface[0]),
                ("sidebar", p.surface[1]),
                ("surfaceRaised", p.surface[2]),
                ("border", p.surface[3]),
                ("input", p.surface[4]),
                ("text", p.text[0]),
                ("secondaryForeground", p.text[1]),
                ("codeForeground", p.text[2]),
                ("textMuted", p.text[3]),
                ("placeholder", p.text[4]),
                ("iconMuted", p.text[5]),
                ("updateForeground", p.accent[0]),
                ("accentSurfaceForeground", p.accent[1]),
                ("accentSurface", p.accent[2]),
                ("terminalCursor", p.accent[3]),
                ("messageActionHover", p.accent[5]),
                ("focus", p.accent[3]),
                ("accent", p.accent[4]),
            ]
            .into_iter()
            .map(|(k, v)| (k.into(), format!("#{v:06x}")))
            .collect()
        };
        Self {
            version: 1,
            id: identifier(name),
            name: name.into(),
            appearance: ThemeMode::Light,
            colors: colors(light),
            variants: BTreeMap::from([(ThemeMode::Dark, colors(dark))]),
            extra: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{InstalledTheme, ThemeFamily, ThemeFile, ThemeLibrary, ThemeMode};

    #[test]
    fn theme_file_round_trips_both_builtin_palettes() {
        for &family in ThemeFamily::ALL.iter() {
            let light = family.palette(ThemeMode::Light);
            let dark = family.palette(ThemeMode::Dark);
            let theme = InstalledTheme::try_from(ThemeFile::seed("Example", light, dark)).unwrap();
            let restored = InstalledTheme::parse(&theme.json()).unwrap();
            assert_eq!(restored.palette(ThemeMode::Light), Some(light));
            assert_eq!(restored.palette(ThemeMode::Dark), Some(dark));
        }
    }

    #[test]
    fn damaged_theme_leaves_other_saved_themes_usable() {
        let saved = serde_json::json!({"themes":[null, {"version":1, "name":"Aurora","appearance":"dark",
            "colors":{"canvas":"red"}}], "light":"aurora", "dark":"aurora"});
        let library: ThemeLibrary = serde_json::from_value(saved).unwrap();
        assert_eq!(library.themes.len(), 1);
        assert_eq!(library.selection(ThemeMode::Dark), Some("aurora"));
        assert_eq!(library.selection(ThemeMode::Light), None);
    }
    #[test]
    fn community_collection_refresh_preserves_local_edits_and_unrelated_selections() {
        use std::sync::Arc;
        let theme = |name: &str, mode: ThemeMode, color: &str| {
            let mut file = ThemeFile::single(
                name.into(),
                name.into(),
                mode,
                [(String::from("canvas"), String::from(color))].into(),
            );
            file.set_collection("open-vsx:studio.harbor", "Harbor");
            Arc::new(InstalledTheme::try_from(file).unwrap())
        };
        let collection = "open-vsx:studio.harbor";
        let dark = theme("night", ThemeMode::Dark, "#122331");
        let light = theme("day", ThemeMode::Light, "#fdfbf8");
        let mut lib = ThemeLibrary::default();
        lib.replace_collection(collection, &[], vec![dark, light.clone()])
            .unwrap();
        lib.select(ThemeMode::Dark, Some("night".into()));
        lib.select(ThemeMode::Light, Some("day".into()));
        let initial = lib.collection(collection);
        let edited = theme("night", ThemeMode::Dark, "#161410");
        lib.themes[0] = edited;
        let before = lib.clone();
        assert!(lib
            .replace_collection(collection, &initial, vec![light.clone()])
            .is_err());
        assert_eq!(lib, before);
        let current = lib.collection(collection);
        lib.replace_collection(collection, &current, vec![light])
            .unwrap();
        assert_eq!(lib.selection(ThemeMode::Light), Some("day"));
        assert_eq!(lib.selection(ThemeMode::Dark), None);
    }
}
