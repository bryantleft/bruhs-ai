//! Board, catalog and raster writes commit atomically with revision comparison.
use super::{
    from_js,
    metadata::{finish, LocalTransaction, SaveError},
    open_db, to_js, STORE_BOARDS, STORE_BOARD_ASSETS, STORE_BOARD_INDEX,
};
use bruhs_models::whiteboard::*;
use idb::TransactionMode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wasm_bindgen::JsValue;

#[derive(Serialize, Deserialize)]
struct Record {
    id: String,
    json: String,
}
fn record<T: Serialize>(id: &str, value: &T) -> Result<JsValue, SaveError> {
    to_js(&Record {
        id: id.into(),
        json: serde_json::to_string(value).map_err(|_| SaveError::Corrupt)?,
    })
    .map_err(Into::into)
}
fn decode<T: serde::de::DeserializeOwned>(raw: JsValue, id: &str) -> Result<T, SaveError> {
    let row: Record = from_js(raw)?;
    if row.id != id {
        return Err(SaveError::Corrupt);
    }
    serde_json::from_str(&row.json).map_err(|_| SaveError::Corrupt)
}
fn valid(board: &BoardDocument) -> Result<(), SaveError> {
    board.validate().map_err(|_| {
        SaveError::Rejected("The board is invalid. Export a recovery copy before changing it.")
    })
}
pub async fn list(project_path: &str) -> Result<Vec<BoardSummary>, SaveError> {
    let db = open_db().await?;
    let tx =
        LocalTransaction::new(db.transaction(&[STORE_BOARD_INDEX], TransactionMode::ReadOnly)?);
    let result = async {
        let rows = tx
            .object_store(STORE_BOARD_INDEX)?
            .get_all(None, None)?
            .await?;
        let mut boards = Vec::new();
        for raw in rows {
            let row: Record = from_js(raw)?;
            let summary: BoardSummary =
                serde_json::from_str(&row.json).map_err(|_| SaveError::Corrupt)?;
            if summary.id != row.id {
                return Err(SaveError::Corrupt);
            }
            if summary.project_path == project_path {
                boards.push(summary);
            }
        }
        boards.sort_by(|a, b| {
            b.updated_at_ms
                .cmp(&a.updated_at_ms)
                .then(a.title.cmp(&b.title))
        });
        Ok(boards)
    }
    .await;
    finish(tx, result).await
}
pub async fn load(id: &str) -> Result<PortableBoard, SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(
        &[STORE_BOARDS, STORE_BOARD_ASSETS],
        TransactionMode::ReadOnly,
    )?);
    let result = async {
        let raw = tx
            .object_store(STORE_BOARDS)?
            .get(JsValue::from_str(id))?
            .await?
            .ok_or(SaveError::Unavailable)?;
        let board: BoardDocument = decode(raw, id)?;
        valid(&board)?;
        if board.id != id {
            return Err(SaveError::Corrupt);
        }
        let mut payloads = BTreeMap::new();
        let assets = tx.object_store(STORE_BOARD_ASSETS)?;
        for asset in board.assets.values() {
            let raw = assets
                .get(JsValue::from_str(&asset.sha256))?
                .await?
                .ok_or(SaveError::Corrupt)?;
            let payload: AssetPayload = decode(raw, &asset.sha256)?;
            payload.decode(asset).map_err(|_| SaveError::Corrupt)?;
            payloads.insert(asset.sha256.clone(), payload);
        }
        Ok(PortableBoard { board, payloads })
    }
    .await;
    finish(tx, result).await
}
pub async fn save(
    board: &BoardDocument,
    expected: Option<u64>,
    payloads: &BTreeMap<String, AssetPayload>,
) -> Result<(), SaveError> {
    valid(board)?;
    let json = serde_json::to_string(board).map_err(|_| SaveError::Corrupt)?;
    if u64::try_from(json.len()).unwrap_or(u64::MAX) > MAX_BOARD_BYTES {
        return Err(SaveError::Rejected(
            "The board exceeds the local content budget. Export and split it into smaller boards.",
        ));
    }
    let encoded = to_js(&Record {
        id: board.id.clone(),
        json,
    })?;
    let summary = record(&board.id, &board.summary())?;
    // Decode incoming assets before opening the transaction, never between IDB requests.
    let mut incoming = BTreeMap::new();
    for asset in board.assets.values() {
        if let Some(payload) = payloads.get(&asset.sha256) {
            payload
                .decode(asset)
                .map_err(|_| SaveError::Rejected("Image payload validation failed."))?;
            incoming.insert(asset.sha256.clone(), record(&asset.sha256, payload)?);
        }
    }
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(
        &[STORE_BOARDS, STORE_BOARD_INDEX, STORE_BOARD_ASSETS],
        TransactionMode::ReadWrite,
    )?);
    let result = async {
        let store = tx.object_store(STORE_BOARDS)?;
        let current = store.get(JsValue::from_str(&board.id))?.await?;
        match (current, expected) {
            (None, None) => {}
            (Some(raw), Some(expected)) => {
                let current: BoardDocument = decode(raw, &board.id)?;
                valid(&current)?;
                if current.project_path != board.project_path
                    || current.revision != expected
                    || board.revision <= expected
                {
                    return Err(SaveError::Conflict);
                }
            }
            _ => return Err(SaveError::Conflict),
        }
        let assets = tx.object_store(STORE_BOARD_ASSETS)?;
        for asset in board.assets.values() {
            if let Some(encoded) = incoming.get(&asset.sha256) {
                assets.put(encoded, None)?.await?;
            } else if assets
                .get(JsValue::from_str(&asset.sha256))?
                .await?
                .is_none()
            {
                return Err(SaveError::Rejected(
                    "A local image payload is missing. Restore it before saving.",
                ));
            }
        }
        store.put(&encoded, None)?.await?;
        tx.object_store(STORE_BOARD_INDEX)?
            .put(&summary, None)?
            .await?;
        Ok(())
    }
    .await;
    finish(tx, result).await
}
