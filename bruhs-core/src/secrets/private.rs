//! The folders that hold the owner's saved logins, which no bruh run may
//! read: the vault catalog, the keychains, 1Password's and the Linux
//! keyring's own folders, and both data roots' `backups` folders (Backup
//! D21), where backups, staged restores and pre-restore sets sit.

use std::path::{Path, PathBuf};
use std::sync::Once;

/// The macOS keychains folder, under the home folder.
const KEYCHAINS: [&str; 2] = ["Library", "Keychains"];

/// The folders a bruh run never reaches, for `home`, the credentials
/// folder `catalog` and the data roots' `backups` folders: the catalog,
/// `~/Library/Keychains`, 1Password's group container, `~/.config/op`,
/// `~/.op`, `~/.local/share/keyrings`, then each of `backups` once.
pub(crate) fn private_folders(home: &Path, catalog: &Path, backups: &[PathBuf]) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = std::iter::once(catalog.to_path_buf())
        .chain(home_folders(home))
        .collect();
    for folder in backups {
        if !folders.contains(folder) {
            folders.push(folder.clone());
        }
    }
    folders
}

/// The login stores under `home`.
fn home_folders(home: &Path) -> [PathBuf; 5] {
    [
        home.join(KEYCHAINS[0]).join(KEYCHAINS[1]),
        home.join("Library")
            .join("Group Containers")
            .join("2BUA8C4S2C.com.1password"),
        home.join(".config").join("op"),
        home.join(".op"),
        home.join(".local").join("share").join("keyrings"),
    ]
}

/// [`private_folders`] for this account, with both data roots' `backups`
/// folders. A folder that can't be named (no home folder, no data folder)
/// is left out, and that is logged once.
pub(crate) fn vault_private_folders() -> Vec<PathBuf> {
    use crate::backup::{RootKey, Roots};
    static MISSING: Once = Once::new();
    let home = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());
    let catalog = super::storage::catalog_directory().ok();
    let backups: Vec<PathBuf> = Roots::from_env()
        .map(|roots| {
            vec![
                roots.backups(RootKey::Core),
                roots.backups(RootKey::Desktop),
            ]
        })
        .unwrap_or_default();
    if home.is_none() || catalog.is_none() || backups.is_empty() {
        MISSING.call_once(|| {
            tracing::warn!(
                home = home.is_some(),
                catalog = catalog.is_some(),
                backups = !backups.is_empty(),
                "Some vault folders have no path, so bruh runs aren't kept out of them"
            );
        });
    }
    match (home, catalog) {
        (Some(home), Some(catalog)) => private_folders(&home, &catalog, &backups),
        (home, catalog) => catalog
            .into_iter()
            .chain(home.iter().flat_map(|home| home_folders(home)))
            .chain(backups)
            .collect(),
    }
}

/// Whether Claude's deny rules may name `folder`. Its bash sandbox reads
/// them too, and the Security framework reads the keychains folder in
/// process, so denying it would break git and gh logins in a Full bruh's
/// shell. That folder keeps only the resolved-path check. Its files are
/// encrypted, and `security` asks the owner before it reads an item Bruhs
/// saved (other apps' items may not ask).
pub(crate) fn deny_rule_may_name(folder: &Path) -> bool {
    let mut tail = folder.components().rev().take(2);
    let last = tail.next().map(|part| part.as_os_str().to_string_lossy());
    let parent = tail.next().map(|part| part.as_os_str().to_string_lossy());
    !matches!(
        (parent, last),
        (Some(parent), Some(last))
            if parent.eq_ignore_ascii_case(KEYCHAINS[0]) && last.eq_ignore_ascii_case(KEYCHAINS[1])
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_private_folders_are_the_vault_and_the_login_stores() {
        let folders = private_folders(Path::new("/Users/a"), Path::new("/data/credentials"), &[]);
        let expected = [
            "/data/credentials",
            "/Users/a/Library/Keychains",
            "/Users/a/Library/Group Containers/2BUA8C4S2C.com.1password",
            "/Users/a/.config/op",
            "/Users/a/.op",
            "/Users/a/.local/share/keyrings",
        ]
        .map(PathBuf::from);
        assert_eq!(folders, expected);
        let named: Vec<_> = folders
            .iter()
            .filter(|folder| deny_rule_may_name(folder))
            .collect();
        assert_eq!(named.len(), 5);
        assert!(!named.contains(&&PathBuf::from("/Users/a/Library/Keychains")));
        assert!(!deny_rule_may_name(Path::new(
            "/private/x/library/KEYCHAINS"
        )));
        assert!(deny_rule_may_name(Path::new("/x/Keychains")));
    }

    /// Both data roots' `backups` folders join the private folders, and a
    /// bruh run's deny rules name each of them (Backup D21). The resolved
    /// path check is `reactive::approval`'s test of the same name.
    #[test]
    fn bruh_runs_are_kept_out_of_backups() {
        let backups = ["/data/core/backups", "/data/desktop/backups"].map(PathBuf::from);
        let folders = private_folders(
            Path::new("/Users/a"),
            Path::new("/data/core/credentials"),
            &backups,
        );
        for folder in &backups {
            assert!(folders.contains(folder), "{folder:?}");
            assert!(deny_rule_may_name(folder), "{folder:?}");
        }
        let named: Vec<_> = folders
            .iter()
            .filter(|folder| deny_rule_may_name(folder))
            .cloned()
            .collect();
        let rules: Vec<String> = crate::agent::hidden_folder_rules(&named).collect();
        for rule in [
            "Read(//data/core/backups/**)",
            "Edit(//data/core/backups/**)",
            "Read(//data/desktop/backups/**)",
            "Edit(//data/desktop/backups/**)",
        ] {
            assert!(rules.iter().any(|have| have == rule), "{rule}");
        }
        // One root (Linux) names its folder once.
        let one = private_folders(
            Path::new("/Users/a"),
            Path::new("/data/credentials"),
            &[
                PathBuf::from("/data/backups"),
                PathBuf::from("/data/backups"),
            ],
        );
        let once = one
            .iter()
            .filter(|folder| **folder == Path::new("/data/backups"))
            .count();
        assert_eq!(once, 1);
        // This account's list names this account's roots.
        if let Some(roots) = crate::backup::Roots::from_env() {
            let live = vault_private_folders();
            for root in [
                crate::backup::RootKey::Core,
                crate::backup::RootKey::Desktop,
            ] {
                assert!(live.contains(&roots.backups(root)), "{root:?}");
            }
        }
    }
}
