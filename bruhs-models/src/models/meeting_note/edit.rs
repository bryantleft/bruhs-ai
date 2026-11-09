use super::MeetingNote;

/// An app-owned edit session. The original snapshot protects against stale writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingNoteDraft {
    pub id: String,
    pub expected: Option<MeetingNote>,
    pub title: String,
    pub body: String,
    pub recording_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeetingNoteEditError {
    InvalidContent,
    InvalidLink,
    Conflict,
    /// The original note no longer exists; the draft must not recreate it silently.
    Removed,
    InvalidTime,
}

impl MeetingNoteEditError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::InvalidContent => "Enter a title (up to 4096 UTF-8 bytes) and notes of at most 65536 bytes. Your draft is retained.",
            Self::InvalidLink => "The linked recording identity is invalid. Your draft is retained.",
            Self::Conflict => "This note changed since editing began. Copy your draft before opening the latest note; nothing was overwritten.",
            Self::Removed => "This note was deleted elsewhere. Your draft is kept here; copy it into a new note if you still need it. Nothing was recreated.",
            Self::InvalidTime => "The note timestamp cannot advance safely. Your draft is retained.",
        }
    }
}

impl std::fmt::Display for MeetingNoteEditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for MeetingNoteEditError {}

impl MeetingNoteDraft {
    pub fn new(recording_id: Option<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            expected: None,
            title: String::new(),
            body: String::new(),
            recording_id,
        }
    }

    pub fn edit(note: &MeetingNote) -> Self {
        Self {
            id: note.id.clone(),
            expected: Some(note.clone()),
            title: note.title.clone(),
            body: note.body.clone(),
            recording_id: note.recording_id.clone(),
        }
    }

    pub fn prepare(&self, now: i64) -> Result<MeetingNote, MeetingNoteEditError> {
        if self.id.trim().is_empty()
            || self.id.len() > 128
            || !MeetingNote::valid_content(&self.title, &self.body)
        {
            return Err(MeetingNoteEditError::InvalidContent);
        }
        if !MeetingNote::valid_recording_id(self.recording_id.as_deref()) {
            return Err(MeetingNoteEditError::InvalidLink);
        }
        let (created_at, updated_at) = match &self.expected {
            Some(note) if note.id != self.id => return Err(MeetingNoteEditError::Conflict),
            Some(note)
                if note.title == self.title
                    && note.body == self.body
                    && note.recording_id == self.recording_id =>
            {
                (note.created_at, note.updated_at)
            }
            Some(note) => (
                note.created_at,
                now.max(
                    note.updated_at
                        .checked_add(1)
                        .ok_or(MeetingNoteEditError::InvalidTime)?,
                ),
            ),
            None => (now, now),
        };
        Ok(MeetingNote {
            id: self.id.clone(),
            title: self.title.clone(),
            body: self.body.clone(),
            recording_id: self.recording_id.clone(),
            created_at,
            updated_at,
        })
    }

    /// The exact current record must match, even when its timestamp did not change.
    pub fn check_current(&self, current: Option<&MeetingNote>) -> Result<(), MeetingNoteEditError> {
        if current != self.expected.as_ref() {
            return Err(if current.is_none() && self.expected.is_some() {
                MeetingNoteEditError::Removed
            } else {
                MeetingNoteEditError::Conflict
            });
        }
        Ok(())
    }

    /// Preflight before modifying the native collection. Duplicate IDs fail closed.
    pub fn apply(
        &self,
        notes: &mut Vec<MeetingNote>,
        now: i64,
    ) -> Result<MeetingNote, MeetingNoteEditError> {
        let note = self.prepare(now)?;
        let mut matches = notes
            .iter()
            .enumerate()
            .filter(|(_, note)| note.id == self.id);
        let current = matches.next();
        if matches.next().is_some() {
            return Err(MeetingNoteEditError::Conflict);
        }
        self.check_current(current.map(|(_, note)| note))?;
        if let Some((index, _)) = current {
            notes[index] = note.clone();
        } else {
            notes.insert(0, note.clone());
        }
        Ok(note)
    }
}

#[cfg(test)]
mod tests;
