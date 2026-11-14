//! A coalescing native writer keeps the newest committed edit through view changes.
use super::*;
use std::{collections::BTreeMap, sync::Arc};
struct PendingSave {
    board: Arc<BoardDocument>,
    payloads: Arc<BTreeMap<String, AssetPayload>>,
}
struct State {
    version: BoardVersion,
    pending: Option<PendingSave>,
    running: bool,
    error: Option<String>,
}
pub struct BoardWriter {
    store: BoardStore,
    lease: Arc<BoardLease>,
    state: Mutex<State>,
}
#[derive(Debug, Clone)]
pub struct WriterStatus {
    pub revision: u64,
    pub busy: bool,
    pub error: Option<String>,
}
impl BoardWriter {
    pub fn new(store: BoardStore, lease: Arc<BoardLease>, version: BoardVersion) -> Self {
        Self {
            store,
            lease,
            state: Mutex::new(State {
                version,
                pending: None,
                running: false,
                error: None,
            }),
        }
    }
    pub fn authority(&self) -> Result<BoardDocument> {
        let expected = self.state.lock().map_err(|_| StoreError::Poisoned)?.version;
        let (board, version) = self.store.snapshot(&self.lease.id)?;
        if version != expected {
            return Err(BoardError::Conflict.into());
        }
        Ok(board)
    }
    pub fn status(&self) -> Result<WriterStatus> {
        let state = self.state.lock().map_err(|_| StoreError::Poisoned)?;
        Ok(WriterStatus {
            revision: state.version.revision,
            busy: state.running,
            error: state.error.clone(),
        })
    }
    /// True starts one worker. Further edits replace its pending snapshot.
    pub fn submit(
        &self,
        board: Arc<BoardDocument>,
        payloads: Arc<BTreeMap<String, AssetPayload>>,
        retry: bool,
    ) -> Result<bool> {
        board.validate()?;
        let mut state = self.state.lock().map_err(|_| StoreError::Poisoned)?;
        if board.id != self.lease.id {
            return Err(
                BoardError::Invalid("A writer cannot change board identity.".into()).into(),
            );
        }
        if board.revision <= state.version.revision {
            return Ok(false);
        }
        if state
            .pending
            .as_ref()
            .is_none_or(|pending| pending.board.revision <= board.revision)
        {
            state.pending = Some(PendingSave { board, payloads });
        }
        if state.running || state.error.is_some() && !retry {
            return Ok(false);
        }
        state.error = None;
        state.running = true;
        drop(state);
        Ok(true)
    }
    /// Run on a worker. No async cancellation point exists between queued saves.
    pub fn run(&self) -> Result<BoardVersion> {
        loop {
            let next = {
                let mut state = self.state.lock().map_err(|_| StoreError::Poisoned)?;
                match state.pending.take() {
                    Some(PendingSave { board, payloads }) => (board, payloads, state.version),
                    None => {
                        state.running = false;
                        return Ok(state.version);
                    }
                }
            };
            let (board, payloads, version) = next;
            let result = (|| {
                for asset in board.assets.values() {
                    if let Some(payload) = payloads.get(&asset.sha256) {
                        self.store.put_asset(asset, payload)?;
                    }
                }
                self.lease.save(&board, Some(version))
            })();
            let mut state = self.state.lock().map_err(|_| StoreError::Poisoned)?;
            match result {
                Ok(version) => {
                    state.version = version;
                    drop(state);
                }
                Err(error) => {
                    state.running = false;
                    state.error = Some(error.to_string());
                    if state.pending.is_none() {
                        state.pending = Some(PendingSave { board, payloads });
                    }
                    drop(state);
                    return Err(error);
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_edits_coalesce_to_the_latest_revision() {
        let dir = tempfile::tempdir().unwrap();
        let store = BoardStore::open(dir.path().to_owned()).unwrap();
        let mut board = benchmark_scene(1000).unwrap();
        let lease = Arc::new(store.lease(&board.id).unwrap());
        let version = lease.save(&board, None).unwrap();
        let writer = BoardWriter::new(store.clone(), lease, version);
        let payloads = Arc::new(BTreeMap::new());
        board.revision = 1;
        board.title = "First".into();
        assert!(writer
            .submit(Arc::new(board.clone()), payloads.clone(), false)
            .unwrap());
        board.revision = 2;
        board.title = "Latest".into();
        assert!(!writer
            .submit(Arc::new(board.clone()), payloads, false)
            .unwrap());
        writer.run().unwrap();
        assert_eq!(store.load(&board.id).unwrap().title, "Latest");
        assert_eq!(writer.status().unwrap().revision, 2);
        assert!(!writer.status().unwrap().busy);
    }
}
