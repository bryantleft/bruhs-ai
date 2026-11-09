use super::*;
use crate::meetings::SpeakerLinkJob;

#[cfg(test)]
mod tests;

pub(super) fn enqueue(tx: &Connection, job: &SpeakerLinkJob) -> MeetingStoreResult<()> {
    records::required(records::meeting(tx, &job.meeting_id)?, &job.meeting_id)?;
    tx.execute(
        "INSERT INTO speaker_link_jobs(meeting_id,token,attempts,due) VALUES (?1,?2,0,0)
        ON CONFLICT(meeting_id) DO UPDATE SET token=excluded.token,attempts=0,due=0
        WHERE speaker_link_jobs.token<>excluded.token",
        (&job.meeting_id, &job.token),
    )?;
    Ok(())
}

pub(super) fn next(
    connection: &Connection,
    now_ms: i64,
) -> MeetingStoreResult<Option<SpeakerLinkJob>> {
    Ok(connection.query_row(
        "SELECT meeting_id,token,attempts,due FROM speaker_link_jobs WHERE due<=?1 ORDER BY due,meeting_id LIMIT 1",
        [now_ms], |row| Ok(SpeakerLinkJob {
            meeting_id: row.get(0)?, token: row.get(1)?, attempts: row.get(2)?, due_at_ms: row.get(3)?,
        })).optional()?)
}

pub(super) fn finish(tx: &Connection, job: &SpeakerLinkJob) -> MeetingStoreResult<()> {
    tx.execute(
        "DELETE FROM speaker_link_jobs WHERE meeting_id=?1 AND token=?2",
        (&job.meeting_id, &job.token),
    )?;
    Ok(())
}

pub(super) fn retry(tx: &Connection, job: &SpeakerLinkJob, now_ms: i64) -> MeetingStoreResult<()> {
    let next = job.retry(now_ms);
    tx.execute("UPDATE speaker_link_jobs SET attempts=?1,due=?2 WHERE meeting_id=?3 AND token=?4 AND attempts=?5 AND due=?6",
        (next.attempts, next.due_at_ms, &job.meeting_id, &job.token, job.attempts, job.due_at_ms))?;
    Ok(())
}
