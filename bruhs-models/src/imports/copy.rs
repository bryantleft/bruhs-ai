//! Every line of app-owned import copy, exact. Both apps and the server
//! use these, and [`ALL_COPY`] lists each one for the copy test.

// Tabs and buttons.
pub const EXPORT_TAB: &str = "Export file";
pub const CHOOSE_EXPORT: &str = "Choose export file";
pub const CHOOSE_ANOTHER: &str = "Choose another file";
pub const IMPORT_ALL: &str = "Import all";

// Where to get each export.
pub const EXPORT_HELP_CHATGPT: &str =
    "ChatGPT: Settings, Data controls, Export data. Upload the ZIP from the email.";
pub const EXPORT_HELP_CLAUDE: &str = "Claude.ai: Settings, Privacy, Export data. Upload the ZIP, or conversations-000.zip and memories-000.zip from the newer list of files.";
pub const EXPORT_HELP_GROK: &str =
    "Grok: Settings, Data controls, Download account data. Upload the ZIP.";
pub const GROK_BEST_EFFORT: &str =
    "Grok is best effort: xAI doesn't document its export, so some chats may come in incomplete.";

// Results.
pub const ALREADY_HERE: &str = "Every chat in this export is already here.";
/// The Export file tab's search field: its placeholder and its name.
pub const EXPORT_SEARCH: &str = "Search titles";
/// A search that leaves no chat to show while some are left to import.
pub const NO_MATCHING_CHATS: &str = "No chats match this search.";
pub const USE_AS_MEMORY: &str = "Use as a bruh's memory";
pub const CLAUDE_MANIFEST: &str = "This is Claude's list of export files. Download conversations-000.zip from it, then upload that file.";

// Placeholders and headings inside imported text.
pub const IMAGE_NOT_IMPORTED: &str = "[Image not imported]";
pub const AUDIO_NOT_IMPORTED: &str = "[Audio not imported]";
pub const DOCUMENT_NOT_IMPORTED: &str = "[Document not imported]";
pub const CUT_MARK: &str = "\n[Cut to fit]";
pub const IDENTITY_FROM: &str = "Identity from IDENTITY.md";
pub const PROJECT_MEMORY: &str = "Project memory:";

// Errors.
pub const NOT_AN_EXPORT: &str = "Bruhs didn't find ChatGPT, Claude.ai or Grok chats in this file.";
pub const EXPORT_TOO_LARGE: &str =
    "Exports up to 4 GB can be imported. Upload the conversations file on its own.";
pub const TOO_MANY_FILES: &str = "Choose up to 16 files at once.";
pub const ARCHIVE_REFUSED: &str =
    "This file holds more than Bruhs reads from an export. Nothing was imported.";
pub const CONVERSATION_TOO_LARGE: &str =
    "One chat in this export is over 33 MB. Nothing was imported.";
pub const DAMAGED_EXPORT: &str = "This export is damaged or cut short. Download it again.";
pub const ENCRYPTED_EXPORT: &str = "This ZIP is password protected. Unzip it first.";
pub const NO_SPACE_IMPORT: &str = "There isn't enough free disk space. Free {size} and try again.";
pub const IMPORT_BUSY: &str = "Another export is being read. Wait for it to finish.";
pub const IMPORT_GONE: &str = "That export isn't here anymore. Choose the file again.";
pub const OWNER_ONLY_IMPORT: &str = "Open Bruhs from its own address to import.";
pub const IMPORT_INVALID: &str = "That request isn't one Bruhs understands.";
pub const IMPORT_FAILED: &str = "Bruhs couldn't read this export.";
/// No answer from the server behind an import call.
pub const IMPORT_UNREACHABLE: &str =
    "Bruhs couldn't reach its server. Check that it's running, then try again.";
pub const FOLDER_MISSING: &str = "That folder doesn't exist.";
pub const FOLDER_PRIVATE: &str =
    "Bruhs doesn't open folders named for secrets, such as credentials.";

// The Move in dialog.
pub const MOVE_IN_TITLE: &str = "Move in";
pub const MOVE_IN_COMMAND: &str = "Move in...";
pub const MOVE_IN_COMMAND_HINT: &str = "OpenClaw, Hermes Agent, a prompt";
pub const BY_PROMPT_TAB: &str = "By prompt";
pub const AGENTS_PRIVACY: &str =
    "Bruhs reads only persona and memory files. It never opens .env, auth.json or other secrets.";
pub const NONE_FOUND_OPENCLAW: &str =
    "No OpenClaw workspace found here. Type or choose its folder.";
pub const NONE_FOUND_HERMES: &str = "No Hermes Agent home found here. Type or choose its folder.";
pub const FOLDER: &str = "Folder";
pub const LOOK_HERE: &str = "Look here";
pub const CHOOSE_FOLDER: &str = "Choose folder";
pub const MAKE_BRUH: &str = "Make this bruh";
pub const STARTS_READ_ONLY: &str = "It starts Read only, with its own folder.";
pub const MEMORY_WAITS: &str = "Its memory waits for your review on its Memory tab.";
pub const PERSONA_CLIPPED: &str = "Some text was cut to fit 16,000 characters.";
pub const MOVE_IN_HINT: &str =
    "Paste this into ChatGPT, Claude or Grok, then paste the answer here.";
pub const COPY_PROMPT: &str = "Copy prompt";
pub const COPIED: &str = "Copied";
pub const FOR_BRUH: &str = "For bruh";
pub const THEIR_ANSWER: &str = "Their answer";
pub const PROPOSE_MEMORY: &str = "Propose memory";
pub const NO_BRUHS: &str = "Make a bruh first.";
pub const MAKE_A_BRUH: &str = "Make a bruh";
pub const MOVE_IN_EMPTY: &str = "Paste the answer first.";
pub const MOVE_IN_TOO_LONG: &str =
    "That's over 16,000 characters with this bruh's memory. Shorten it first.";
pub const MOVE_IN_PROMPT: &str = "I'm moving to a new assistant. List what you know and remember about me that would help it work with me: my preferences, how I like answers written, my work and projects, the people, tools and places I mention, and my routines. Write one fact per line as a plain sentence. Leave out passwords, keys, account numbers and anything I asked you to forget.";

// The Proposed memory card.
pub const PROPOSED_MEMORY: &str = "Proposed memory";
pub const PROPOSED_HINT: &str = "Approving adds this to the bruh's instructions. Runs still waiting are cancelled, and its next run starts a new conversation.";
pub const APPROVE_MEMORY: &str = "Approve memory";
pub const DECLINE: &str = "Decline";
pub const MEMORY_CHANGED: &str =
    "This bruh's memory changed after this was proposed. Approving replaces those changes.";

// Templates the line functions fill: each `{name}` is replaced whole. A
// `{chats}` slot takes the whole counted phrase, such as "3 chats".
pub const READING_TEMPLATE: &str = "Reading the export: {done} of {files} files, {chats} so far";
/// [`READING_TEMPLATE`] when the read has one file.
pub const READING_ONE_FILE_TEMPLATE: &str = "Reading the export: {done} of 1 file, {chats} so far";
pub const BATCH_SUMMARY_TEMPLATE: &str = "{chats} from {sources}.";
// Counted sentences: `_ONE` when n is 1, else `_MANY` with `{n}` filled.
pub const SKIPPED_EMPTY_ONE: &str = "1 empty chat was left out.";
pub const SKIPPED_EMPTY_MANY: &str = "{n} empty chats were left out.";
pub const SKIPPED_UNKNOWN_ONE: &str = "1 record Bruhs didn't recognize was left out.";
pub const SKIPPED_UNKNOWN_MANY: &str = "{n} records Bruhs didn't recognize were left out.";
pub const DUPLICATES_ONE: &str = "1 chat came twice and was imported once.";
pub const DUPLICATES_MANY: &str = "{n} chats came twice and were imported once.";
pub const DROPPED_BRANCHES_ONE: &str =
    "1 other branch was left out. Each chat comes in as the branch you saw last.";
pub const DROPPED_BRANCHES_MANY: &str =
    "{n} other branches were left out. Each chat comes in as the branch you saw last.";
pub const OVER_LIMIT_ONE: &str = "1 chat over the 50,000 limit was left out.";
pub const OVER_LIMIT_MANY: &str = "{n} chats over the 50,000 limit were left out.";
pub const SKIPPED_FILES_ONE: &str = "1 file Bruhs couldn't read was left out.";
pub const SKIPPED_FILES_MANY: &str = "{n} files Bruhs couldn't read were left out.";
pub const CUT_CHATS_ONE: &str = "1 chat was too long and came in cut short.";
pub const CUT_CHATS_MANY: &str = "{n} chats were too long and came in cut short.";
pub const LOGS_LEFT_OUT_ONE: &str = "1 older daily log was left out.";
pub const LOGS_LEFT_OUT_MANY: &str = "{n} older daily logs were left out.";
pub const ROW_DETAIL_TEMPLATE: &str = "{label}, {when}, transcript";
pub const BEST_EFFORT_SUFFIX: &str = ", best effort";
pub const IMPORTING_TEMPLATE: &str = "Importing {done} of {total}";
pub const IMPORTED_TEMPLATE: &str = "Imported {chats} from {label}.";
pub const MEMORY_FOUND_TEMPLATE: &str = "This export holds {label}'s memory of you.";
pub const LOOKED_IN_TEMPLATE: &str = "Looked in: {list}.";
pub const INSTRUCTIONS_FROM_TEMPLATE: &str = "Instructions from {files}.";
pub const MEMORY_FROM_TEMPLATE: &str = "Memory from {files}.";
pub const NOT_IMPORTED_TEMPLATE: &str = "Not imported: {files}.";
pub const SKIPPED_LINK_TEMPLATE: &str = "Skipped {name}: it's a link.";
pub const SKIPPED_TOO_LARGE_TEMPLATE: &str = "Skipped {name}: it's over 262 KB.";
pub const SKIPPED_NOT_A_FILE_TEMPLATE: &str = "Skipped {name}: it isn't a file.";
pub const SKIPPED_UNREADABLE_TEMPLATE: &str = "Skipped {name}: Bruhs couldn't read it.";
// `_ONE` for one field, else `_MANY` with the fields joined by "and".
pub const IDENTITY_DROPPED_ONE: &str = "{fields} isn't carried over: bruhs use faces.";
pub const IDENTITY_DROPPED_MANY: &str = "{fields} aren't carried over: bruhs use faces.";
pub const MEMORY_PROPOSED_TEMPLATE: &str = "Proposed. Approve it on {name}'s Memory tab.";
pub const ATTACHMENT_TEMPLATE: &str = "Attachment not imported: {name}";
pub const ATTACHED_TEMPLATE: &str = "Attached {name}:";
pub const CUT_SHORT_ONE: &str = "[Cut short: 1 more character]";
pub const CUT_SHORT_MANY: &str = "[Cut short: {n} more characters]";
pub const PASTED_ANSWER_TEMPLATE: &str = "Pasted answer, {date}";
pub const EXPORT_MEMORY_FROM_TEMPLATE: &str = "{label} export";
pub const ROWS_SHOWN_TEMPLATE: &str = "Showing {shown} of {total}. Search to find the rest.";

// Final review, server.
/// A chat a live export batch or a local tool no longer has.
pub const IMPORT_CHAT_GONE: &str = "That chat isn't here anymore.";
/// Refusal for applying, or approving, a change to a bruh's memory from a
/// page that isn't the owner's (final review B I1).
pub const OWNER_ONLY_MEMORY: &str = "Open Bruhs from its own address to change a bruh's memory.";

// Final review, readers.
/// The last item of a names list cut to its first few: "and {n} more".
pub const NAMES_MORE_TEMPLATE: &str = "{n} more";
/// Refusal for a picked export file inside a folder named for secrets.
pub const FILE_PRIVATE: &str =
    "Bruhs doesn't open files in folders named for secrets, such as credentials.";

/// Every constant above.
pub const ALL_COPY: &[&str] = &[
    EXPORT_TAB,
    CHOOSE_EXPORT,
    CHOOSE_ANOTHER,
    IMPORT_ALL,
    EXPORT_HELP_CHATGPT,
    EXPORT_HELP_CLAUDE,
    EXPORT_HELP_GROK,
    GROK_BEST_EFFORT,
    ALREADY_HERE,
    EXPORT_SEARCH,
    NO_MATCHING_CHATS,
    USE_AS_MEMORY,
    CLAUDE_MANIFEST,
    IMAGE_NOT_IMPORTED,
    AUDIO_NOT_IMPORTED,
    DOCUMENT_NOT_IMPORTED,
    CUT_MARK,
    IDENTITY_FROM,
    PROJECT_MEMORY,
    NOT_AN_EXPORT,
    EXPORT_TOO_LARGE,
    TOO_MANY_FILES,
    ARCHIVE_REFUSED,
    CONVERSATION_TOO_LARGE,
    DAMAGED_EXPORT,
    ENCRYPTED_EXPORT,
    NO_SPACE_IMPORT,
    IMPORT_BUSY,
    IMPORT_GONE,
    OWNER_ONLY_IMPORT,
    IMPORT_INVALID,
    IMPORT_FAILED,
    IMPORT_UNREACHABLE,
    FOLDER_MISSING,
    FOLDER_PRIVATE,
    MOVE_IN_TITLE,
    MOVE_IN_COMMAND,
    MOVE_IN_COMMAND_HINT,
    BY_PROMPT_TAB,
    AGENTS_PRIVACY,
    NONE_FOUND_OPENCLAW,
    NONE_FOUND_HERMES,
    FOLDER,
    LOOK_HERE,
    CHOOSE_FOLDER,
    MAKE_BRUH,
    STARTS_READ_ONLY,
    MEMORY_WAITS,
    PERSONA_CLIPPED,
    MOVE_IN_HINT,
    COPY_PROMPT,
    COPIED,
    FOR_BRUH,
    THEIR_ANSWER,
    PROPOSE_MEMORY,
    NO_BRUHS,
    MAKE_A_BRUH,
    MOVE_IN_EMPTY,
    MOVE_IN_TOO_LONG,
    MOVE_IN_PROMPT,
    PROPOSED_MEMORY,
    PROPOSED_HINT,
    APPROVE_MEMORY,
    DECLINE,
    MEMORY_CHANGED,
    READING_TEMPLATE,
    READING_ONE_FILE_TEMPLATE,
    BATCH_SUMMARY_TEMPLATE,
    SKIPPED_EMPTY_ONE,
    SKIPPED_EMPTY_MANY,
    SKIPPED_UNKNOWN_ONE,
    SKIPPED_UNKNOWN_MANY,
    DUPLICATES_ONE,
    DUPLICATES_MANY,
    DROPPED_BRANCHES_ONE,
    DROPPED_BRANCHES_MANY,
    OVER_LIMIT_ONE,
    OVER_LIMIT_MANY,
    SKIPPED_FILES_ONE,
    SKIPPED_FILES_MANY,
    CUT_CHATS_ONE,
    CUT_CHATS_MANY,
    LOGS_LEFT_OUT_ONE,
    LOGS_LEFT_OUT_MANY,
    ROW_DETAIL_TEMPLATE,
    BEST_EFFORT_SUFFIX,
    IMPORTING_TEMPLATE,
    IMPORTED_TEMPLATE,
    MEMORY_FOUND_TEMPLATE,
    LOOKED_IN_TEMPLATE,
    INSTRUCTIONS_FROM_TEMPLATE,
    MEMORY_FROM_TEMPLATE,
    NOT_IMPORTED_TEMPLATE,
    SKIPPED_LINK_TEMPLATE,
    SKIPPED_TOO_LARGE_TEMPLATE,
    SKIPPED_NOT_A_FILE_TEMPLATE,
    SKIPPED_UNREADABLE_TEMPLATE,
    IDENTITY_DROPPED_ONE,
    IDENTITY_DROPPED_MANY,
    MEMORY_PROPOSED_TEMPLATE,
    ATTACHMENT_TEMPLATE,
    ATTACHED_TEMPLATE,
    CUT_SHORT_ONE,
    CUT_SHORT_MANY,
    PASTED_ANSWER_TEMPLATE,
    EXPORT_MEMORY_FROM_TEMPLATE,
    ROWS_SHOWN_TEMPLATE,
    // Final review, server.
    IMPORT_CHAT_GONE,
    OWNER_ONLY_MEMORY,
    // Final review, readers.
    NAMES_MORE_TEMPLATE,
    FILE_PRIVATE,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_follows_the_ui_standard() {
        for text in ALL_COPY {
            assert!(
                !text.contains(" - ")
                    && !text.contains("--")
                    && !text.contains('—')
                    && !text.contains('–')
                    && !text.contains('\u{2212}'),
                "{text}"
            );
            assert!(
                !text.contains('•') && !text.contains('·') && !text.trim_start().starts_with("- "),
                "{text}"
            );
        }
        assert!(ALL_COPY.iter().all(|text| !text.contains("recognise")));
        assert_eq!(EXPORT_SEARCH, "Search titles");
        assert_eq!(NO_MATCHING_CHATS, "No chats match this search.");
        assert!(
            MOVE_IN_PROMPT.contains("one fact per line") && MOVE_IN_PROMPT.contains("passwords")
        );
    }
}
