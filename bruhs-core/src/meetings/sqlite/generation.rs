use super::*;
use crate::meetings::{generation as rules, GenerationReservation, SummaryUpdate};
use bruhs_models::{
    Meeting, SummaryGenerationReceipt, SummaryGenerationRequest, SummaryGenerationState,
};

#[cfg(test)]
mod tests;

pub(super) fn pending(
    connection: &Connection,
    meeting: &str,
) -> MeetingStoreResult<Option<SummaryGenerationReceipt>> {
    let id: Option<String> = connection
        .query_row(
            "SELECT id FROM summary_generations WHERE meeting_id=?1 AND pending=1",
            [meeting],
            |row| row.get(0),
        )
        .optional()?;
    id.map(|id| read(connection, &id))
        .transpose()
        .map(Option::flatten)
}

fn read(connection: &Connection, id: &str) -> MeetingStoreResult<Option<SummaryGenerationReceipt>> {
    let row: Option<(String, bool, String)> = connection
        .query_row(
            "SELECT meeting_id,pending,payload FROM summary_generations WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    row.map(|(meeting, pending, payload)| {
        let receipt: SummaryGenerationReceipt = decode(payload)?;
        if !receipt.request.valid()
            || receipt.request.request_id != id
            || receipt.request.meeting_id != meeting
            || pending != matches!(receipt.state, SummaryGenerationState::Pending)
        {
            return Err(MeetingStoreError::PersistenceFailed(
                "Generation receipt is malformed; original data was preserved.".into(),
            ));
        }
        Ok(receipt)
    })
    .transpose()
}

pub(super) fn get(
    connection: &Connection,
    meeting: &str,
    id: &str,
) -> MeetingStoreResult<Option<SummaryGenerationReceipt>> {
    Ok(read(connection, id)?.filter(|receipt| receipt.request.meeting_id == meeting))
}

fn save(connection: &Connection, receipt: &SummaryGenerationReceipt) -> MeetingStoreResult<()> {
    connection.execute(
        "UPDATE summary_generations SET pending=?2,payload=?3 WHERE id=?1",
        rusqlite::params![
            receipt.request.request_id,
            matches!(receipt.state, SummaryGenerationState::Pending),
            encode(receipt)?
        ],
    )?;
    Ok(())
}

pub(super) fn begin(
    connection: &Connection,
    request: SummaryGenerationRequest,
) -> MeetingStoreResult<GenerationReservation> {
    rules::validate(&request)?;
    records::required(
        records::meeting(connection, &request.meeting_id)?,
        &request.meeting_id,
    )?;
    if let Some(receipt) = read(connection, &request.request_id)? {
        return rules::replay(receipt, &request);
    }
    let busy: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM summary_generations WHERE meeting_id=?1 AND pending=1)",
        [&request.meeting_id],
        |row| row.get(0),
    )?;
    if busy {
        return Err(MeetingStoreError::Conflict("This meeting has an unresolved generation. Review or cancel that request before starting another.".into()));
    }
    let receipt = SummaryGenerationReceipt {
        request,
        state: SummaryGenerationState::Pending,
    };
    connection.execute(
        "INSERT INTO summary_generations(id,meeting_id,pending,payload) VALUES(?1,?2,1,?3)",
        rusqlite::params![
            receipt.request.request_id,
            receipt.request.meeting_id,
            encode(&receipt)?
        ],
    )?;
    Ok(GenerationReservation::Started)
}

pub(super) fn finish(
    connection: &Connection,
    meeting: &str,
    id: &str,
    outcome: crate::meetings::GenerationOutcome,
) -> MeetingStoreResult<SummaryGenerationReceipt> {
    let mut receipt = get(connection, meeting, id)?.ok_or_else(|| {
        MeetingStoreError::InvalidInput("Generation request was not found.".into())
    })?;
    // A delayed failure/cancel cannot downgrade an already committed receipt.
    if matches!(receipt.state, SummaryGenerationState::Pending) {
        receipt.state = outcome.state();
        save(connection, &receipt)?;
    }
    Ok(receipt)
}

pub(super) fn fence(
    connection: &Connection,
    request: SummaryGenerationRequest,
) -> MeetingStoreResult<SummaryGenerationReceipt> {
    rules::validate(&request)?;
    records::required(
        records::meeting(connection, &request.meeting_id)?,
        &request.meeting_id,
    )?;
    if let Some(receipt) = read(connection, &request.request_id)? {
        // The same identity with different options is a conflict, never a cancellation.
        rules::replay(receipt, &request)?;
        return finish(
            connection,
            &request.meeting_id,
            &request.request_id,
            crate::meetings::GenerationOutcome::Cancelled,
        );
    }
    let receipt = SummaryGenerationReceipt {
        request,
        state: SummaryGenerationState::Cancelled,
    };
    connection.execute(
        "INSERT INTO summary_generations(id,meeting_id,pending,payload) VALUES(?1,?2,0,?3)",
        rusqlite::params![
            receipt.request.request_id,
            receipt.request.meeting_id,
            encode(&receipt)?
        ],
    )?;
    Ok(receipt)
}

pub(super) fn commit(
    connection: &Connection,
    id: &str,
    update: SummaryUpdate,
) -> MeetingStoreResult<Meeting> {
    let mut receipt = get(connection, &update.expected.id, id)?.ok_or_else(|| {
        MeetingStoreError::Conflict(
            "Generation reservation is missing; nothing was replaced.".into(),
        )
    })?;
    rules::pending(&receipt, &update)?;
    let summary = update.summary.clone();
    let meeting = summaries::commit(connection, update)?;
    receipt.state = SummaryGenerationState::Succeeded { summary };
    save(connection, &receipt)?;
    Ok(meeting)
}
