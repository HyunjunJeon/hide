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
//! A body file is the authority on its board, unless the index let it go: a
//! start reads every `board-<n>.json` in the folder, so a board written just
//! before a stop whose index line was not yet written (its number is at or
//! past the index's next id) is still there; a file whose board the index
//! names no line for and whose number is below the index's next id is a
//! board that went (deleted with its session, or let go past the cap) whose
//! file could not be removed, and the start removes it rather than read it
//! back. The next id is past both the index's and the highest file's, so an
//! id is never given twice. An index of another schema, or a folder that
//! cannot be listed, is kept as it is: no board command changes anything
//! until it can be read.
//!
//! The runtime reads the folder once, when it is created, before the lock
//! exists; after that each accepted change hands the boards it changed to
//! one save thread, which holds at most one pending body per board and
//! writes off the lock (`docs/ARCHITECTURE.md`, State placement and
//! publication). A body or a delete that fails is kept and done again with
//! the next save, unless a newer one of that board came first, and once more
//! when the writer is dropped.

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
    // The boards the index let go: below its next id, with no line.
    let let_go = |id: &str| {
        index.as_ref().is_some_and(|index| {
            board_number(id) < index.next && !index.boards.iter().any(|line| line.id == id)
        })
    };
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
        if let_go(id) {
            let removed = std::fs::remove_file(entry.path());
            crate::diagnostic!(json!({
                "component": "boards", "kind": "boards.file_let_go",
                "board": id, "removed": removed.is_ok(),
                "message": removed.err().map(|error| error.to_string()),
            }));
            continue;
        }
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
/// thread and tries once more what failed, so what was queued before a stop
/// reaches the disk.
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
    /// What failed to be written or deleted, done again with the next save
    /// unless a newer one of the same board came first. Bounded by the book.
    failed: BTreeMap<String, Option<Board>>,
    /// The last index that failed to be written, tried once more on drop.
    failed_index: Option<(u64, Vec<IndexLine>)>,
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
        // Oldest first, so a newer body of a board replaces a failed one.
        let mut pending = std::mem::take(&mut slot.failed);
        pending.append(&mut slot.bodies);
        pending.extend(changes);
        slot.bodies = pending;
        slot.index = Some((next, lines));
        slot.failed_index = None;
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

impl BoardFiles {
    /// Waits for the save thread to end, when one runs.
    fn join(&self) {
        let worker = self
            .worker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(worker) = worker
            && worker.join().is_err()
        {
            crate::diagnostic!(json!({"component": "boards", "kind": "boards.save_join_failed"}));
        }
    }
}

impl Drop for BoardFiles {
    fn drop(&mut self) {
        self.join();
        let (bodies, index) = {
            let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
            (std::mem::take(&mut slot.failed), slot.failed_index.take())
        };
        if bodies.is_empty() && index.is_none() {
            return;
        }
        let (failed, index_failed) = write(&self.dir, bodies, index);
        if !failed.is_empty() || index_failed.is_some() {
            crate::diagnostic!(json!({
                "component": "boards", "kind": "boards.save_lost",
                "boards": failed.keys().collect::<Vec<_>>(),
                "index": index_failed.is_some(),
            }));
        }
    }
}

/// Bodies and deletes by board, and an index: what waits to be written, or
/// what failed to be.
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
        let (failed, index_failed) = write(dir, bodies, index);
        let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
        for (id, body) in failed {
            // A newer body or delete of that board waits already.
            if !slot.bodies.contains_key(&id) {
                slot.failed.insert(id, body);
            }
        }
        if slot.index.is_none() {
            slot.failed_index = index_failed;
        }
    }
}

/// Writes the changed bodies, deletes the files of boards that went, then
/// replaces the index. Each failure is logged, the rest are still written,
/// and what failed is answered so the writer keeps it: the bodies and
/// deletes by board, and the index.
fn write(
    dir: &Path,
    bodies: BTreeMap<String, Option<Board>>,
    index: Option<(u64, Vec<IndexLine>)>,
) -> Pending {
    let failure = |what: &str, message: String| {
        crate::diagnostic!(json!({
            "component": "boards", "kind": "boards.save_failed",
            "path": dir.to_string_lossy(), "what": what, "message": message,
        }));
    };
    if let Err(error) = hide_platform::fs::private::create_dir_all(dir) {
        failure("folder", error.to_string());
        return (bodies, index);
    }
    let mut failed = BTreeMap::new();
    for (id, body) in bodies {
        let path = hide_kit::layout::board_file(dir, &id);
        let written = match &body {
            Some(board) => encode(board).and_then(|bytes| replace(&path, &bytes)),
            None => match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(format!("could not be deleted: {error}")),
            },
        };
        if let Err(reason) = written {
            failure(&id, reason);
            failed.insert(id, body);
        }
    }
    let Some((next, mut boards)) = index else {
        return (failed, None);
    };
    boards.sort_by_key(|line| board_number(&line.id));
    let encoded = serde_json::to_vec(&Index {
        schema_version: INDEX_SCHEMA,
        next,
        boards: boards.clone(),
    })
    .map_err(|error| format!("could not be encoded: {error}"))
    .and_then(|bytes| replace(&hide_kit::layout::board_index(dir), &bytes));
    match encoded {
        Ok(()) => (failed, None),
        Err(reason) => {
            failure("index", reason);
            (failed, Some((next, boards)))
        }
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

    fn revision_on_disk(dir: &Path, id: &str) -> Option<u64> {
        let bytes = std::fs::read(hide_kit::layout::board_file(dir, id)).ok()?;
        serde_json::from_slice::<Board>(&bytes)
            .ok()
            .map(|board| board.revision)
    }

    /// A folder where a board's file goes makes its write fail on every
    /// system.
    fn block_file(dir: &Path, id: &str) -> PathBuf {
        let path = hide_kit::layout::board_file(dir, id);
        std::fs::create_dir_all(path.join("held")).unwrap();
        path
    }

    #[test]
    fn a_failed_write_is_written_again_with_the_next_save() {
        let state = tempfile::tempdir().unwrap();
        let dir = state.path().join("boards");
        let blocked = block_file(&dir, "board-1");
        let files = BoardFiles::new(dir.clone());
        files.save(
            [("board-1".to_owned(), Some(board(1)))],
            2,
            vec![IndexLine::of(&board(1))],
        );
        files.join();
        assert_eq!(revision_on_disk(&dir, "board-1"), None);
        std::fs::remove_dir_all(&blocked).unwrap();
        files.save(
            [("board-2".to_owned(), Some(board(2)))],
            3,
            vec![IndexLine::of(&board(1)), IndexLine::of(&board(2))],
        );
        files.join();
        assert_eq!(revision_on_disk(&dir, "board-1"), Some(1));
        drop(files);
        assert_eq!(
            read(&dir),
            (3, vec!["board-1".to_owned(), "board-2".to_owned()])
        );
    }

    #[test]
    fn a_newer_body_replaces_a_failed_one() {
        let state = tempfile::tempdir().unwrap();
        let dir = state.path().join("boards");
        let blocked = block_file(&dir, "board-1");
        let files = BoardFiles::new(dir.clone());
        files.save(
            [("board-1".to_owned(), Some(board(1)))],
            2,
            vec![IndexLine::of(&board(1))],
        );
        files.join();
        std::fs::remove_dir_all(&blocked).unwrap();
        let mut newer = board(1);
        newer.revision = 2;
        files.save(
            [("board-1".to_owned(), Some(newer.clone()))],
            2,
            vec![IndexLine::of(&newer)],
        );
        drop(files);
        assert_eq!(revision_on_disk(&dir, "board-1"), Some(2));
    }

    #[test]
    fn a_failed_delete_is_done_again_and_its_board_never_comes_back() {
        let state = tempfile::tempdir().unwrap();
        let dir = state.path().join("boards");
        let blocked = block_file(&dir, "board-1");
        let files = BoardFiles::new(dir.clone());
        files.save(
            [
                ("board-1".to_owned(), None),
                ("board-2".to_owned(), Some(board(2))),
            ],
            3,
            vec![IndexLine::of(&board(2))],
        );
        files.join();
        assert!(blocked.exists(), "the delete failed");
        // A start before the delete is done again does not read it back.
        assert_eq!(read(&dir), (3, vec!["board-2".to_owned()]));
        // The folder in the way goes, and the board's old file is there.
        std::fs::remove_dir_all(&blocked).unwrap();
        std::fs::write(&blocked, encode(&board(1)).unwrap()).unwrap();
        files.save([], 3, vec![IndexLine::of(&board(2))]);
        drop(files);
        assert!(!blocked.exists(), "the delete was done again");
    }

    #[test]
    fn a_file_the_index_let_go_is_removed_and_one_past_its_next_id_is_kept() {
        let state = tempfile::tempdir().unwrap();
        let dir = state.path().join("boards");
        std::fs::create_dir_all(&dir).unwrap();
        for n in [1, 2, 5] {
            std::fs::write(
                hide_kit::layout::board_file(&dir, &format!("board-{n}")),
                encode(&board(n)).unwrap(),
            )
            .unwrap();
        }
        std::fs::write(
            hide_kit::layout::board_index(&dir),
            serde_json::to_vec(&Index {
                schema_version: INDEX_SCHEMA,
                next: 5,
                boards: vec![IndexLine::of(&board(2))],
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            read(&dir),
            (6, vec!["board-2".to_owned(), "board-5".to_owned()])
        );
        assert!(!hide_kit::layout::board_file(&dir, "board-1").exists());
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
