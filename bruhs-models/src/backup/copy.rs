//! Every string the Backup section and its routes show. Templates name their
//! values in braces (`{size}`); [`super::fill`] puts the values in.

pub const BACKUP_TITLE: &str = "Backup";
pub const BACKUP_DESCRIPTION: &str = "One encrypted file with your threads, bruhs, vault, mail and meetings. Restore it here or on another computer.";
pub const MAKE_GROUP: &str = "Make a backup";
pub const RESTORE_GROUP: &str = "Restore";
pub const KEPT_GROUP: &str = "Kept from earlier restores";

// Fields and buttons.
pub const PASSWORD: &str = "Password";
pub const PASSWORD_AGAIN: &str = "Password again";
pub const PASSWORD_HINT: &str = "At least 12 characters. Bruhs never keeps it, and nobody can open the backup without it, Bruhs included.";
pub const MAKE_BACKUP: &str = "Make backup";
pub const DOWNLOAD: &str = "Download";
pub const CHOOSE_FILE: &str = "Choose backup file";
pub const OPEN_BACKUP: &str = "Open backup";
pub const REPLACE: &str = "Replace my data";
pub const CANCEL: &str = "Cancel";
pub const PUT_BACK: &str = "Put back";
pub const DELETE: &str = "Delete";
pub const KEEP: &str = "Keep";
pub const OPEN_WEB_APP: &str = "Open the web app";
pub const RESTORE_BROWSER: &str = "Restore browser data";

// Options. `{n}` is a count of keychain items, `{size}` a byte size.
pub const KEYCHAIN_OPTION: &str = "Saved logins, keys and app tokens ({n})";
pub const AUDIO_OPTION: &str = "Meeting audio ({size})";
pub const BROWSERS_OPTION: &str = "Bruh browser sign-ins ({size})";
pub const KEYCHAIN_OPTION_HINT: &str = "Read from your keychain into the encrypted file only.";
pub const WEB_INCLUDED: &str = "This browser's data goes in too.";
pub const DESKTOP_NO_WEB: &str =
    "Browser data isn't in a backup made here. Make one from the web app to include it.";
pub const OUTSIDE_ROOTS: &str =
    "Some data lives outside Bruhs' folders and isn't in a backup: {list}.";

// Progress.
pub const JOB_COLLECTING: &str = "Reading your data: {done} of {total}";
pub const JOB_KEYCHAIN: &str = "Reading your keychain";
pub const JOB_WRITING: &str = "Writing the backup: {done} of {total}";
pub const JOB_CHECKING: &str = "Checking the backup: {done} of {total}";
pub const JOB_PREPARING: &str = "Preparing your data for this computer";
/// A step this build doesn't know, from a newer server.
pub const JOB_WORKING: &str = "Working: {done} of {total}";
pub const LOCKING_WEB: &str = "Locking this browser's data";
pub const CREATED_WEB: &str = "Backup ready: {name}, {size}.";
pub const CREATED_DESKTOP: &str = "Saved {name}, {size}.";
/// Warnings on a made backup. `{n}` counts owner files and folders left
/// out by name (a folder counts once), `{name}` is a bruh's name.
pub const SKIPPED_NAME: &str = "{n} file or folder with a name a backup can't hold was left out.";
pub const SKIPPED_NAMES: &str =
    "{n} files or folders with names a backup can't hold were left out.";
pub const BROWSER_BUSY: &str = "{name}'s browser was busy, so its sign-ins aren't in this backup.";
pub const BACKUP_STOPPED: &str = "The backup was stopped. Nothing was saved.";
/// `{n}` counts files that changed while they were read; each holds the
/// part that was there when the backup listed it (ruling B Task 6 I2).
pub const CHANGED_FILE: &str =
    "{n} file changed while Bruhs read it, so its copy may be incomplete.";
pub const CHANGED_FILES: &str =
    "{n} files changed while Bruhs read them, so their copies may be incomplete.";
/// `{n}` counts keychain items a catalog names that the keychain doesn't
/// hold (D5).
pub const MISSING_ITEM: &str =
    "{n} keychain item wasn't in your keychain, so it isn't in this backup.";
pub const MISSING_ITEMS: &str =
    "{n} keychain items weren't in your keychain, so they aren't in this backup.";
pub const BRUHS_UNCOUNTED: &str =
    "Bruhs couldn't count your bruhs, so this backup's summary shows none. They are in it.";

// What a backup holds, shown before a restore.
pub const PREVIEW_MADE: &str = "Made {date} on the {app} app, Bruhs {version}.";
/// Each value is a count with its noun, from the `COUNT_*` pairs.
pub const PREVIEW_COUNTS: &str = "{bruhs}, {entries}, {items}, {files}, {size}.";
pub const COUNT_BRUH: &str = "bruh";
pub const COUNT_BRUHS: &str = "bruhs";
pub const COUNT_VAULT_ENTRY: &str = "vault entry";
pub const COUNT_VAULT_ENTRIES: &str = "vault entries";
pub const COUNT_KEYCHAIN_ITEM: &str = "keychain item";
pub const COUNT_KEYCHAIN_ITEMS: &str = "keychain items";
pub const COUNT_FILE: &str = "file";
pub const COUNT_FILES: &str = "files";
pub const PREVIEW_REWRITE: &str = "Paths under {from} become {to}.";
pub const PREVIEW_OUTSIDE: &str =
    "These folders are outside the old home and stay as they are: {list}.";
pub const PREVIEW_WEB: &str = "Browser data: included.";
pub const PREVIEW_NO_WEB: &str = "No browser data in this backup.";
pub const PREVIEW_NO_KEYCHAIN: &str =
    "No keychain items in this backup. Restored vault entries need their values again.";
pub const PREVIEW_NO_AUDIO: &str = "Meeting audio isn't in this backup.";
pub const PREVIEW_NO_BROWSERS: &str = "Bruh browser sign-ins aren't in this backup.";
pub const PREVIEW_SKIPPED_LINK: &str = "{n} link was left out.";
pub const PREVIEW_SKIPPED_LINKS: &str = "{n} links were left out.";

// Restoring.
pub const REPLACE_WARNING: &str = "Restoring replaces the data on this computer. Bruhs stops, swaps the data in and keeps today's data aside until you delete it.";
pub const STOPPING: &str =
    "Bruhs is stopping to finish the restore. This page reconnects when it's back.";
pub const START_AGAIN: &str = "Start Bruhs again to finish the restore.";
pub const RESTORED: &str = "Restored. Your bruhs are paused: resume them on their pages.";
pub const BRUHS_PAUSED: &str = RESTORED;
pub const WEB_PART_WAITING: &str =
    "This backup has data for this browser. Restoring it replaces what this browser holds now.";
pub const WEB_PART_DONE: &str = "This browser's data is restored. Reload other Bruhs tabs.";
pub const NO_WEB_PART: &str = "This backup has no browser data, so this browser keeps its own.";

// Kept sets and the last backup.
pub const PRE_RESTORE_LINE: &str = "From before the restore on {date}: {size}.";
pub const DELETE_PRE_RESTORE_CONFIRM: &str =
    "Delete the data from before the restore on {date}? It can't be brought back.";
pub const PUT_BACK_CONFIRM: &str =
    "Put back the data from before the restore on {date}? Today's data is kept aside the same way.";
pub const LAST_BACKUP_LINE: &str = "Last backup: {date} from the {app} app, {size}.";

// Errors.
pub const PASSWORD_SHORT: &str = "Use at least 12 characters.";
pub const PASSWORD_LONG: &str = "Keep the password under 1 KB.";
pub const PASSWORD_CONTROL: &str = "Passwords can't contain control characters.";
pub const PASSWORD_BLANK: &str = "Use more than spaces in the password.";
pub const PASSWORDS_DIFFER: &str = "The passwords don't match.";
pub const WRONG_PASSWORD: &str = "That password doesn't open this backup, or the file was changed.";
pub const NOT_A_BACKUP: &str = "This isn't a Bruhs backup.";
pub const NEWER_BACKUP: &str =
    "This backup comes from a newer Bruhs. Update Bruhs, then try again.";
pub const NEWER_WEB_DATA: &str =
    "This browser data comes from a newer Bruhs. Update Bruhs, then try again.";
pub const UNSAFE_ENTRY: &str = "This backup holds a file Bruhs won't write. Nothing was changed.";
pub const DAMAGED: &str = "This backup is damaged. Nothing was changed.";
/// Two entries a case-insensitive or accent-insensitive volume would give
/// one name (`Notes` and `notes`, or an "é" typed two ways).
pub const NAMES_CLASH: &str =
    "This backup holds two files whose names differ only in case or accents. Nothing was changed.";
pub const ROOTS_OVERLAP: &str =
    "This backup's two data folders overlap on this computer. Nothing was changed.";
pub const NO_SPACE: &str = "There isn't enough free disk space. Free {size} and try again.";
/// More files and folders than a restore takes (B Task 7 I1).
pub const BACKUP_TOO_MANY: &str =
    "This backup would hold more files than a restore can take. Move some files out of Bruhs' folders, then try again.";
/// `keychain.json` over the restore's cap (B Task 7 N1).
pub const KEYCHAIN_TOO_LARGE: &str =
    "Your saved logins and keys are too large for one backup. Leave them out or remove some, then try again.";
pub const FILE_CHANGED: &str = "A file changed while Bruhs was reading it. Try again.";
/// A file in Bruhs' own folders whose name a restore would refuse (B Task 5
/// I2 ruling). Owner files with such names are left out and counted.
pub const DATA_NAME_REFUSED: &str =
    "A file in Bruhs' own folders has a name a backup can't hold, so no backup was made.";
/// A data root whose own path isn't UTF-8 (B Task 5 ruling).
pub const ROOT_NAME_REFUSED: &str =
    "A Bruhs data folder has a name a backup can't hold, so no backup was made.";
pub const KEYCHAIN_NEEDS_YOU: &str =
    "Your keychain needs you. Allow access on this computer, or leave out saved logins and keys.";
pub const KEYCHAIN_ITEM_REFUSED: &str = "That keychain item isn't one of Bruhs' own.";
pub const KEYCHAIN_READ_FAILED: &str = "Bruhs couldn't read its keychain items. Try again.";
pub const KEYCHAIN_SET_UNREADABLE: &str = "The saved keychain items could not be opened.";
pub const BACKUP_BUSY: &str = "A backup or restore is already running.";
pub const RESTORE_PENDING: &str = "A restore is waiting to finish. Start Bruhs again to finish it.";
pub const RESTORE_CANT_MOVE: &str =
    "Bruhs can't move some of its data. Check the folder's permissions, then restore again.";
pub const RESTORE_CANT_MAP: &str = "Bruhs couldn't fit this backup to this computer's folders.";
pub const NOTHING_TO_PUT_BACK: &str =
    "This set only holds files Bruhs kept aside during a restore that didn't finish. There is nothing to put back.";
pub const PARKED_STAYED: &str =
    "Some files Bruhs kept aside during a restore that didn't finish stayed in the set you put back.";
pub const RESTORE_ROLLED_BACK: &str =
    "The restore couldn't finish, so your data was put back as it was. Nothing was replaced.";
pub const CONFIRM_REPLACE: &str = "Confirm that your data will be replaced.";
pub const RECORDING_FIRST: &str = "Stop the meeting recording first.";
pub const QUIT_DESKTOP_FIRST: &str =
    "Quit the Bruhs desktop app first, or restore from the desktop app.";
pub const SERVICE_TOO_OLD: &str =
    "The background service runs an older Bruhs. Update it in Settings, then restore.";
pub const DESTINATION_EXISTS: &str = "A file with that name is already there.";
pub const DESTINATION_INSIDE: &str = "Save the backup outside Bruhs' own folders.";
pub const DESTINATION_MISSING: &str = "That folder doesn't exist.";
pub const DESTINATION_NAME_LONG: &str = "Use a shorter file name.";
pub const SOURCE_MISSING: &str = "That backup file isn't there anymore.";
/// Ruling F28 caps a browser's plaintext at 256 MiB.
pub const WEB_TOO_LARGE: &str =
    "This browser's data is over 256 MB. Make the backup from the desktop app.";
pub const WEB_UNSUPPORTED: &str = "This browser holds data Bruhs can't put in a backup: {store}.";
pub const OWNER_ONLY_BACKUP: &str = "Open Bruhs from its own address to make or restore a backup.";
pub const UNSUPPORTED_PLATFORM: &str = "Backup runs on macOS and Linux.";
pub const INVALID_REQUEST: &str = "That request isn't one Bruhs understands.";
pub const GONE: &str = "That backup isn't here anymore.";
/// A web part upload sent as anything but raw bytes.
pub const WEB_PART_TYPE: &str = "Send the browser's part as raw bytes.";

/// Every constant above, for the copy rule test.
pub const ALL_COPY: &[&str] = &[
    BACKUP_TITLE,
    BACKUP_DESCRIPTION,
    MAKE_GROUP,
    RESTORE_GROUP,
    KEPT_GROUP,
    PASSWORD,
    PASSWORD_AGAIN,
    PASSWORD_HINT,
    MAKE_BACKUP,
    DOWNLOAD,
    CHOOSE_FILE,
    OPEN_BACKUP,
    REPLACE,
    CANCEL,
    PUT_BACK,
    DELETE,
    KEEP,
    OPEN_WEB_APP,
    RESTORE_BROWSER,
    KEYCHAIN_OPTION,
    AUDIO_OPTION,
    BROWSERS_OPTION,
    KEYCHAIN_OPTION_HINT,
    WEB_INCLUDED,
    DESKTOP_NO_WEB,
    OUTSIDE_ROOTS,
    JOB_COLLECTING,
    JOB_KEYCHAIN,
    JOB_WRITING,
    JOB_CHECKING,
    JOB_PREPARING,
    JOB_WORKING,
    LOCKING_WEB,
    CREATED_WEB,
    CREATED_DESKTOP,
    SKIPPED_NAME,
    SKIPPED_NAMES,
    BROWSER_BUSY,
    BACKUP_STOPPED,
    CHANGED_FILE,
    CHANGED_FILES,
    MISSING_ITEM,
    MISSING_ITEMS,
    BRUHS_UNCOUNTED,
    PREVIEW_MADE,
    PREVIEW_COUNTS,
    COUNT_BRUH,
    COUNT_BRUHS,
    COUNT_VAULT_ENTRY,
    COUNT_VAULT_ENTRIES,
    COUNT_KEYCHAIN_ITEM,
    COUNT_KEYCHAIN_ITEMS,
    COUNT_FILE,
    COUNT_FILES,
    PREVIEW_REWRITE,
    PREVIEW_OUTSIDE,
    PREVIEW_WEB,
    PREVIEW_NO_WEB,
    PREVIEW_NO_KEYCHAIN,
    PREVIEW_NO_AUDIO,
    PREVIEW_NO_BROWSERS,
    PREVIEW_SKIPPED_LINK,
    PREVIEW_SKIPPED_LINKS,
    REPLACE_WARNING,
    STOPPING,
    START_AGAIN,
    RESTORED,
    BRUHS_PAUSED,
    WEB_PART_WAITING,
    WEB_PART_DONE,
    NO_WEB_PART,
    PRE_RESTORE_LINE,
    DELETE_PRE_RESTORE_CONFIRM,
    PUT_BACK_CONFIRM,
    LAST_BACKUP_LINE,
    PASSWORD_SHORT,
    PASSWORD_LONG,
    PASSWORD_CONTROL,
    PASSWORD_BLANK,
    PASSWORDS_DIFFER,
    WRONG_PASSWORD,
    NOT_A_BACKUP,
    NEWER_BACKUP,
    NEWER_WEB_DATA,
    UNSAFE_ENTRY,
    DAMAGED,
    NAMES_CLASH,
    ROOTS_OVERLAP,
    NO_SPACE,
    BACKUP_TOO_MANY,
    KEYCHAIN_TOO_LARGE,
    FILE_CHANGED,
    DATA_NAME_REFUSED,
    ROOT_NAME_REFUSED,
    KEYCHAIN_NEEDS_YOU,
    KEYCHAIN_ITEM_REFUSED,
    KEYCHAIN_READ_FAILED,
    KEYCHAIN_SET_UNREADABLE,
    BACKUP_BUSY,
    RESTORE_PENDING,
    RESTORE_CANT_MOVE,
    RESTORE_CANT_MAP,
    RESTORE_ROLLED_BACK,
    NOTHING_TO_PUT_BACK,
    PARKED_STAYED,
    CONFIRM_REPLACE,
    RECORDING_FIRST,
    QUIT_DESKTOP_FIRST,
    SERVICE_TOO_OLD,
    DESTINATION_EXISTS,
    DESTINATION_INSIDE,
    DESTINATION_MISSING,
    DESTINATION_NAME_LONG,
    SOURCE_MISSING,
    WEB_TOO_LARGE,
    WEB_UNSUPPORTED,
    OWNER_ONLY_BACKUP,
    UNSUPPORTED_PLATFORM,
    INVALID_REQUEST,
    GONE,
    WEB_PART_TYPE,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_follows_the_rules() {
        for text in ALL_COPY {
            assert!(
                !text.contains(" - ")
                    && !text.contains('—')
                    && !text.contains('–')
                    && !text.contains('·'),
                "{text}"
            );
            assert!(!text.starts_with("- ") && !text.contains('•'), "{text}");
        }
    }

    #[test]
    fn all_copy_lists_every_constant() {
        let declared = include_str!("copy.rs")
            .lines()
            .filter(|line| line.starts_with("pub const ") && line.contains(": &str"))
            .count();
        assert_eq!(ALL_COPY.len(), declared);
    }
}
