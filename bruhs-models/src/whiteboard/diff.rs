use super::*;
/// Exact document edits for an immutable owner preview. Identities never move.
pub fn board_diff(
    before: &BoardDocument,
    after: &BoardDocument,
) -> BoardResult<Vec<BoardOperation>> {
    before.validate()?;
    after.validate()?;
    if before.id != after.id || before.project_path != after.project_path {
        return Err(BoardError::Invalid(
            "A preview cannot replace a board's identity or working folder.".into(),
        ));
    }
    let mut ops = Vec::new();
    if before.title != after.title {
        ops.push(BoardOperation::Title {
            title: after.title.clone(),
        });
    }
    if before.background != after.background {
        ops.push(BoardOperation::Background {
            color: after.background,
        });
    }
    if before.agent_read != after.agent_read {
        ops.push(BoardOperation::AgentRead {
            allowed: after.agent_read,
        });
    }
    for (id, source) in &after.sources {
        if before.sources.get(id) != Some(source) {
            ops.push(BoardOperation::Source {
                source: source.clone(),
            });
        }
    }
    for (id, asset) in &after.assets {
        if before.assets.get(id) != Some(asset) {
            ops.push(BoardOperation::Asset {
                asset: asset.clone(),
            });
        }
    }
    for (id, shape) in &after.shapes {
        if before.shapes.get(id) != Some(shape) {
            ops.push(BoardOperation::shape(shape.clone()));
        }
    }
    ops.extend(
        before
            .shapes
            .keys()
            .filter(|id| !after.shapes.contains_key(*id))
            .cloned()
            .map(|id| BoardOperation::Delete { id }),
    );
    ops.extend(
        before
            .sources
            .keys()
            .filter(|id| !after.sources.contains_key(*id))
            .cloned()
            .map(|id| BoardOperation::RemoveSource { id }),
    );
    ops.extend(
        before
            .assets
            .keys()
            .filter(|id| !after.assets.contains_key(*id))
            .cloned()
            .map(|id| BoardOperation::RemoveAsset { id }),
    );
    Ok(ops)
}
