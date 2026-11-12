//! Where the agents' boards live on disk: one file per board in `boards/`
//! (`hide_kit::layout::boards_dir`), beside an index that holds the next
//! board id and one line per board. A change rewrites the files of the
//! boards it changed and the index, so the record grows by files rather
//! than by one file every change rewrites (engineering #15).
//!
//! The folder is the boards' own, never `links.sqlite3` or
//! `workspace-views.json`: an older or another build that opens either
//! store at a schema it does not know sets the whole store aside, and one
//! that never knew boards never reads this folder.
//!
//! A body file is the authority on its board: a start reads every
//! `board-<n>.json` in the folder, so a board written just before a stop
//! whose index line was not yet written is still there, and a board's file
//! is deleted before the index drops its line, so a stop between the two
//! leaves no board to come back. The next id is past both the index's and
//! the highest file's, so an id is never given twice. An index of another
//! schema, or a folder that cannot be listed, is kept as it is: no board
//! command changes anything until it can be read.
//!
//! The runtime reads the folder once, when it is created, before the lock
//! exists; after that each accepted change hands the boards it changed to
//! one save thread, which holds at most one pending body per board and
//! writes off the lock (`docs/ARCHITECTURE.md`, State placement and
//! publication).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::boards::{BOARD_BYTES, Board, board_number, encode, valid_id};
use crate::latest_file::read_capped;

const INDEX_SCHEMA: u32 = 1;
/// A body larger than this was not written by a build that keeps
/// `BOARD_BYTES`, and is not read.
const BODY_CAP: u64 = 2 * BOARD_BYTES as u64;
const INDEX_CAP: u64 = 1024 * 1024;

/// One board's line in the index. It names no machine, so a core move or a
/// device's new id rewrites only the board files (`node_migration`).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct IndexLine {
    pub(crate) id: String,
    pub(crate) state: String,
    pub(crate) updated_at_unix_ms: u64,
}

impl IndexLine {
    pub(crate) fn of(board: &Board) -> Self {
        Self {
            id: board.id.clone(),
            state: board.state.name().to_owned(),
            updated_at_unix_ms: board.updated_at_unix_ms,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct Index {
    schema_version: u32,
    next: u64,
    boards: Vec<IndexLine>,
}

/// What a start found in the folder.
pub(crate) enum Restored {
    /// The next board id and every board read; none for a first start.
    Read { next: u64, boards: Vec<Board> },
    /// A folder this build cannot use, kept as it is and never written over.
    Unreadable(String),
}

/// Reads the board folder `dir`, before the runtime lock exists.
pub(crate) fn restore(dir: &Path) -> Restored {
    let index = match read_capped(&hide_kit::layout::board_index(dir), INDEX_CAP) {
        Ok(None) => None,
        Ok(Some(bytes)) => match serde_json::from_slice::<Index>(&bytes) {
            Ok(index) if index.schema_version == INDEX_SCHEMA => Some(index),
            Ok(index) => {
                return Restored::Unreadable(format!(
                    "its index has schema version {}, expected {INDEX_SCHEMA}",
                    index.schema_version
                ));
            }
            Err(error) => {
                return Restored::Unreadable(format!("its index could not be decoded: {error}"));
            }
        },
        Err(reason) => return Restored::Unreadable(format!("its index {reason}")),
    };
    let mut next = index.as_ref().map_or(1, |index| index.next).max(1);
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Restored::Read {
                next,
                boards: Vec::new(),
            };
        }
        Err(error) => return Restored::Unreadable(format!("it could not be listed: {error}")),
    };
    let mut boards = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".json")) else {
            continue;
        };
        if !valid_id(id) {
            continue;
        }
        next = next.max(board_number(id) + 1);
        match read_board(dir, id) {
            Ok(board) => boards.push(board),
            // Left on disk; its id is past `next` all the same.
            Err(reason) => crate::diagnostic!(json!({
                "component": "boards", "kind": "boards.file_unreadable",
                "board": id, "message": reason,
            })),
        }
    }
    boards.sort_by_key(|board| board_number(&board.id));
    Restored::Read { next, boards }
}

/// Board `id`'s file, checked to hold that board.
fn read_board(dir: &Path, id: &str) -> Result<Board, String> {
    let bytes =
        read_capped(&hide_kit::layout::board_file(dir, id), BODY_CAP)?.ok_or("is missing")?;
    let board: Board =
        serde_json::from_slice(&bytes).map_err(|error| format!("could not be decoded: {error}"))?;
    if board.id != id {
        return Err(format!("holds {}, not {id}", board.id));
    }
    Ok(board)
}

/// The writer of the board folder: one save thread, started when there is
/// something to write and ended when there is not, with at most one
/// pending body per board and the newest index. Dropping it waits for the
/// thread, so what was queued before a stop reaches the disk.
pub(crate) struct BoardFiles {
    dir: PathBuf,
    slot: Arc<Mutex<Slot>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Default)]
struct Slot {
    /// Each board's newest body to write, or none for a board whose file
    /// goes.
    bodies: BTreeMap<String, Option<Board>>,
    /// The next id and the index lines to write after them.
    index: Option<(u64, Vec<IndexLine>)>,
    active: bool,
}

impl BoardFiles {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            slot: Arc::default(),
            worker: Mutex::new(None),
        }
    }

    /// Queues the boards `changes` names (a body to write, or none to
    /// delete its file) and the index after them. A body still waiting
    /// when a newer one of the same board arrives is replaced.
    pub(crate) fn save(
        &self,
        changes: impl IntoIterator<Item = (String, Option<Board>)>,
        next: u64,
        lines: Vec<IndexLine>,
    ) {
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        slot.bodies.extend(changes);
        slot.index = Some((next, lines));
        if slot.active {
            return;
        }
        let mut worker = self.worker.lock().unwrap_or_else(PoisonError::into_inner);
        // The previous thread has cleared `active` and is returning.
        if let Some(finished) = worker.take() {
            let _ = finished.join();
        }
        let shared = Arc::clone(&self.slot);
        let dir = self.dir.clone();
        match std::thread::Builder::new()
            .name("hide-board-save".to_owned())
            .spawn(move || write_pending(&shared, &dir))
        {
            Ok(handle) => {
                slot.active = true;
                *worker = Some(handle);
            }
            Err(error) => crate::diagnostic!(json!({
                "component": "boards", "kind": "boards.save_failed",
                "message": format!("the save thread could not start: {error}"),
            })),
        }
    }
}

impl Drop for BoardFiles {
    fn drop(&mut self) {
        let worker = self
            .worker
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(worker) = worker
            && worker.join().is_err()
        {
            crate::diagnostic!(json!({"component": "boards", "kind": "boards.save_join_failed"}));
        }
    }
}

type Pending = (
    BTreeMap<String, Option<Board>>,
    Option<(u64, Vec<IndexLine>)>,
);

fn write_pending(slot: &Mutex<Slot>, dir: &Path) {
    loop {
        let (bodies, index): Pending = {
            let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
            if slot.bodies.is_empty() && slot.index.is_none() {
                slot.active = false;
                return;
            }
            (std::mem::take(&mut slot.bodies), slot.index.take())
        };
        if let Err(reason) = write(dir, bodies, index) {
            crate::diagnostic!(json!({
                "component": "boards", "kind": "boards.save_failed",
                "path": dir.to_string_lossy(), "message": reason,
            }));
        }
    }
}

/// Writes the changed bodies, deletes the files of boards that went, then
/// replaces the index. A body that fails is logged and the rest are still
/// written; the next change of that board writes it again.
fn write(
    dir: &Path,
    bodies: BTreeMap<String, Option<Board>>,
    index: Option<(u64, Vec<IndexLine>)>,
) -> Result<(), String> {
    hide_platform::fs::private::create_dir_all(dir)
        .map_err(|error| format!("the folder could not be prepared: {error}"))?;
    let mut failed = Vec::new();
    for (id, body) in &bodies {
        let path = hide_kit::layout::board_file(dir, id);
        let written = match body {
            Some(board) => encode(board).and_then(|bytes| replace(&path, &bytes)),
            None => match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(format!("could not be deleted: {error}")),
            },
        };
        if let Err(reason) = written {
            failed.push(format!("{id}: {reason}"));
        }
    }
    if let Some((next, mut boards)) = index {
        boards.sort_by_key(|line| board_number(&line.id));
        let bytes = serde_json::to_vec(&Index {
            schema_version: INDEX_SCHEMA,
            next,
            boards,
        })
        .map_err(|error| format!("the index could not be encoded: {error}"))?;
        replace(&hide_kit::layout::board_index(dir), &bytes)
            .map_err(|reason| format!("index: {reason}"))?;
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("; "))
    }
}

fn replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    hide_platform::fs::atomic::write_file(path, bytes, hide_platform::fs::Access::Private)
        .map(|_| ())
        .map_err(|error| format!("could not be replaced: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boards::{BoardCheckout, BoardState};
    use crate::links::ledger::SessionIdentity;

    fn board(n: u64) -> Board {
        Board {
            id: format!("board-{n}"),
            owner: SessionIdentity {
                device_id: "mac".into(),
                agent: "claude".into(),
                session_id: "s1".into(),
            },
            checkout: BoardCheckout {
                device_id: "mac".into(),
                path: "/work/app".into(),
            },
            title: format!("Board {n}"),
            blocks: Vec::new(),
            next_block: 1,
            revision: 1,
            created_at_unix_ms: n,
            updated_at_unix_ms: n,
            state: BoardState::Open,
        }
    }

    fn read(dir: &Path) -> (u64, Vec<String>) {
        match restore(dir) {
            Restored::Read { next, boards } => {
                (next, boards.into_iter().map(|board| board.id).collect())
            }
            Restored::Unreadable(reason) => panic!("the folder is read: {reason}"),
        }
    }

    #[test]
    fn a_first_start_finds_no_board_and_ids_start_at_one() {
        let state = tempfile::tempdir().unwrap();
        assert_eq!(read(&state.path().join("boards")), (1, Vec::new()));
    }

    #[test]
    fn saved_boards_come_back_and_a_deleted_file_does_not() {
        let state = tempfile::tempdir().unwrap();
        let dir = state.path().join("boards");
        {
            let files = BoardFiles::new(dir.clone());
            files.save(
                [
                    ("board-1".to_owned(), Some(board(1))),
                    ("board-2".to_owned(), Some(board(2))),
                ],
                3,
                vec![IndexLine::of(&board(1)), IndexLine::of(&board(2))],
            );
            files.save(
                [("board-1".to_owned(), None)],
                3,
                vec![IndexLine::of(&board(2))],
            );
        }
        assert_eq!(read(&dir), (3, vec!["board-2".to_owned()]));
    }

    #[test]
    fn a_body_whose_index_line_was_never_written_still_counts() {
        let state = tempfile::tempdir().unwrap();
        let dir = state.path().join("boards");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            hide_kit::layout::board_file(&dir, "board-7"),
            encode(&board(7)).unwrap(),
        )
        .unwrap();
        assert_eq!(read(&dir), (8, vec!["board-7".to_owned()]));
    }

    #[test]
    fn an_index_of_another_schema_is_kept_unread() {
        let state = tempfile::tempdir().unwrap();
        let dir = state.path().join("boards");
        std::fs::create_dir_all(&dir).unwrap();
        let index = hide_kit::layout::board_index(&dir);
        std::fs::write(&index, br#"{"schema_version":2,"next":4,"boards":[]}"#).unwrap();
        assert!(matches!(restore(&dir), Restored::Unreadable(_)));
    }
}
