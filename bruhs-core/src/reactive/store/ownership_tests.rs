use super::*;
#[test]
fn reactive_last_owner_unlocks_with_an_inherited_open_file_description() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactive.sqlite3");
    let (connection, owner) = open(&path).unwrap();
    let inherited = owner.0.try_clone().unwrap(); // dup retains the same open-file description as fork.
    let owner = std::sync::Arc::new(owner);
    let last = owner.clone();
    drop(connection);
    drop(owner);
    assert!(matches!(open(&path), Err(ReactiveError::Conflict(_))));
    drop(last);
    let (_connection, _owner) =
        open(&path).expect("the inherited descriptor cannot keep ownership after explicit unlock");
    drop(inherited);
}
