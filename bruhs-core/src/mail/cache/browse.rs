#[cfg(test)]
mod tests;

use super::{identity::MailboxId, CacheError, CachedThread, MailCache};
use bruhs_models::{
    connections::ServiceProvider, mail::*, EpochMs, InboxCard, InboxFilter, INBOX_PAGE_SIZE,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

pub(super) struct Projection {
    pub card: String,
    pub fields: String,
    pub message_at_ms: i64,
    pub unread: bool,
    pub archived: bool,
    pub snoozed_until_ms: Option<i64>,
}

pub(super) fn projection(record: &CachedThread) -> Result<Projection, CacheError> {
    let conversation = &record.conversation;
    let card = record.connected_mail.as_ref().map_or_else(
        || InboxCard::from(conversation),
        |metadata| connected_mail_card(conversation, metadata),
    );
    let mut fields = vec![conversation.subject.to_lowercase()];
    for participant in &conversation.participants {
        fields.extend([
            participant.display_name.to_lowercase(),
            participant.address.to_lowercase(),
        ]);
    }
    fields.extend(
        conversation
            .messages
            .iter()
            .map(|message| message.body.to_lowercase()),
    );
    if let Some(metadata) = &record.connected_mail {
        for message in &metadata.messages {
            fields.extend(
                message
                    .attachments
                    .iter()
                    .map(|attachment| attachment.file_name.to_lowercase()),
            );
            fields.extend(
                message
                    .envelope
                    .headers
                    .iter()
                    .map(|header| header.value.to_lowercase()),
            );
        }
    }
    Ok(Projection {
        fields: serde_json::to_string(&fields).map_err(|_| CacheError::InvalidInput)?,
        message_at_ms: card.message_at_ms,
        card: serde_json::to_string(&card).map_err(|_| CacheError::InvalidInput)?,
        unread: conversation.unread,
        archived: conversation.archived,
        snoozed_until_ms: conversation.snoozed_until_ms,
    })
}

pub(super) fn migrate(connection: &Connection) -> Result<(), CacheError> {
    connection.execute_batch("ALTER TABLE threads ADD COLUMN card TEXT NOT NULL DEFAULT ''; ALTER TABLE threads ADD COLUMN search_fields TEXT NOT NULL DEFAULT '[]'; ALTER TABLE threads ADD COLUMN message_at_ms INTEGER NOT NULL DEFAULT 0; ALTER TABLE threads ADD COLUMN unread INTEGER NOT NULL DEFAULT 0; ALTER TABLE threads ADD COLUMN archived INTEGER NOT NULL DEFAULT 0; ALTER TABLE threads ADD COLUMN snoozed_until_ms INTEGER; CREATE INDEX mail_threads_browse ON threads(mailbox,message_at_ms DESC,id);")?;
    let mut statement = connection.prepare("SELECT mailbox,id,payload FROM threads")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (mailbox, id, payload) = row?;
        let record: CachedThread =
            serde_json::from_str(&payload).map_err(|_| CacheError::CorruptRecord)?;
        let projection = projection(&record)?;
        connection.execute("UPDATE threads SET card=?1,search_fields=?2,message_at_ms=?3,unread=?4,archived=?5,snoozed_until_ms=?6 WHERE mailbox=?7 AND id=?8",params![projection.card,projection.fields,projection.message_at_ms,projection.unread,projection.archived,projection.snoozed_until_ms,mailbox,id])?;
    }
    Ok(())
}

fn mailbox(connection: &Connection, id: &str) -> Result<(MailboxId, i64), CacheError> {
    let (provider, revision): (String, i64) = connection
        .query_row(
            "SELECT provider,revision FROM mailboxes WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or(CacheError::UnknownMailbox)?;
    Ok((MailboxId::from_cache_record(&provider, id)?, revision))
}

// All substitutions below are this fixed predicate, never caller SQL.
const MATCHING: &str = "mailbox=?1 AND CASE ?2 WHEN 0 THEN archived=0 AND (snoozed_until_ms IS NULL OR snoozed_until_ms<=?3) WHEN 1 THEN unread<>0 AND archived=0 WHEN 2 THEN snoozed_until_ms>?3 WHEN 3 THEN archived<>0 ELSE 1 END AND (?4='' OR EXISTS(SELECT 1 FROM json_each(search_fields) WHERE instr(value,?4)>0))";

impl MailCache {
    pub fn mailboxes(&self) -> Result<Vec<MailboxSummary>, CacheError> {
        let mut statement=self.connection.prepare("SELECT id,provider,connection_id,revision,COALESCE((SELECT json_extract(CASE WHEN json_valid(value) THEN value ELSE '{}' END,'$.account') FROM cursors WHERE mailbox=mailboxes.id AND json_type(CASE WHEN json_valid(value) THEN value ELSE '{}' END,'$.account')='text' LIMIT 1),(SELECT json_extract(payload,'$.account_label') FROM threads WHERE mailbox=mailboxes.id ORDER BY message_at_ms DESC LIMIT 1),''),(SELECT count(*) FROM threads WHERE mailbox=mailboxes.id),(SELECT count(*) FROM drafts WHERE mailbox=mailboxes.id AND state='editing') FROM mailboxes ORDER BY provider,id")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?;
        rows.map(|row| {
            let (id, provider, last_connection_id, revision, account_label, threads, drafts) = row?;
            let threads = u64::try_from(threads).map_err(|_| CacheError::CorruptRecord)?;
            let drafts = u64::try_from(drafts).map_err(|_| CacheError::CorruptRecord)?;
            let checked = MailboxId::from_cache_record(&provider, &id)?;
            let provider: ServiceProvider = checked.provider();
            Ok(MailboxSummary {
                mailbox_id: id,
                provider,
                last_connection_id,
                account_label,
                revision,
                threads,
                drafts,
            })
        })
        .collect()
    }

    pub fn browse(
        &mut self,
        mailbox_id: &str,
        query: &MailBrowseQuery,
    ) -> Result<MailThreadsPage, CacheError> {
        if query.host_id != self.host.as_str() {
            return Err(CacheError::HostChanged);
        }
        let requested_offset = query
            .page
            .checked_mul(u64::try_from(INBOX_PAGE_SIZE).map_err(|_| CacheError::InvalidInput)?)
            .and_then(|offset| i64::try_from(offset).ok())
            .ok_or(CacheError::InvalidInput)?;
        let filter = match query.filter {
            InboxFilter::Inbox => 0,
            InboxFilter::Unread => 1,
            InboxFilter::Snoozed => 2,
            InboxFilter::Archived => 3,
            InboxFilter::All => 4,
        };
        let search = query.query.trim().to_lowercase();
        let now = EpochMs::now().as_millis();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let (_, revision) = mailbox(&tx, mailbox_id)?;
        if query
            .expected_revision
            .is_some_and(|expected| expected != revision)
        {
            return Err(CacheError::RevisionConflict);
        }
        let count: i64 = tx.query_row(
            &format!("SELECT count(*) FROM threads WHERE {MATCHING}"),
            params![mailbox_id, filter, now, search],
            |row| row.get(0),
        )?;
        let count = u64::try_from(count).map_err(|_| CacheError::CorruptRecord)?;
        let page_size = u64::try_from(INBOX_PAGE_SIZE).map_err(|_| CacheError::InvalidInput)?;
        let page = query.page.min(count.saturating_sub(1) / page_size);
        let offset = if page == query.page {
            requested_offset
        } else {
            i64::try_from(page * page_size).map_err(|_| CacheError::InvalidInput)?
        };
        let rows = {
            let mut statement=tx.prepare(&format!("SELECT card,external_id FROM threads WHERE {MATCHING} ORDER BY message_at_ms DESC,id LIMIT ?5 OFFSET ?6"))?;
            let rows = statement.query_map(
                params![
                    mailbox_id,
                    filter,
                    now,
                    search,
                    i64::try_from(INBOX_PAGE_SIZE).map_err(|_| CacheError::InvalidInput)?,
                    offset
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )?;
            rows.map(|row| {
                let (card, external_id) = row?;
                Ok(MailThreadRow {
                    card: serde_json::from_str(&card).map_err(|_| CacheError::CorruptRecord)?,
                    external_id,
                })
            })
            .collect::<Result<Vec<_>, CacheError>>()?
        };
        tx.commit()?;
        Ok(MailThreadsPage {
            host_id: self.host.as_str().to_owned(),
            mailbox_id: mailbox_id.to_owned(),
            revision,
            page,
            total_matches: count,
            rows,
        })
    }

    pub fn connected_thread(
        &self,
        host: &str,
        mailbox_id: &str,
        external_id: &str,
    ) -> Result<Option<String>, CacheError> {
        if host != self.host.as_str() {
            return Err(CacheError::HostChanged);
        }
        let (mailbox, _) = mailbox(&self.connection, mailbox_id)?;
        self.thread(&mailbox.thread(external_id)?)
    }
}
