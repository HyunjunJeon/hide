//! The runtime half of a session's boards: the book of every board the core
//! keeps, the `hide board` commands, each applied under the lock as one
//! change and handed to the board folder's save thread (`board_store.rs`),
//! who may change or read a board, the limits, and the boards going with
//! the session that drew them.
//!
//! A board is changed only by the pane-bound caller whose native session
//! drew it, as a plan is (`runtime/plans.rs`, `caller_session`); it is read
//! by that session, by any agent in the board's checkout, and by any
//! ancestor of the drawing session, so a mission's Conductor reads the
//! boards its workers draw in their own worktrees.

use std::collections::HashSet;
use std::path::Path;

use serde_json::{Value, json};

use super::Runtime;
use crate::board_store::{self, BoardFiles, IndexLine, Restored};
use crate::boards::{
    self, BOOK_LIMIT, Board, BoardCheckout, BoardCommand, BoardState, OPEN_PER_SESSION, Remover,
};
use crate::coordination::refusal;
use crate::links::ledger::SessionIdentity;
use crate::workspace_control::{Caller, Context, Query};

/// Parents an ancestor read walks up at most, so a ledger whose parents
/// loop cannot hold the lock.
const LINEAGE_HOPS: usize = 64;

#[derive(Default)]
pub(crate) struct BoardRuntime {
    /// Every board the core keeps, by id. Bounded by `BOOK_LIMIT`.
    book: std::collections::BTreeMap<String, Board>,
    /// The number the next board's id takes.
    next: u64,
    /// The folder's writer; none for a runtime with no state folder, which
    /// keeps its boards in memory only.
    files: Option<BoardFiles>,
    /// Why the folder could not be read: no command runs until it can be,
    /// so nothing is written over the boards it holds.
    unreadable: Option<String>,
}

impl BoardRuntime {
    /// The boards of the state folder `state_dir`, read once when the
    /// runtime is created, before its lock exists.
    pub(crate) fn restore(state_dir: Option<&Path>) -> Self {
        let Some(state_dir) = state_dir else {
            return Self {
                next: 1,
                ..Self::default()
            };
        };
        let dir = hide_kit::layout::boards_dir(state_dir);
        match board_store::restore(&dir) {
            Restored::Read { next, boards } => Self {
                book: boards
                    .into_iter()
                    .map(|board| (board.id.clone(), board))
                    .collect(),
                next,
                files: Some(BoardFiles::new(dir)),
                unreadable: None,
            },
            Restored::Unreadable(reason) => {
                crate::diagnostic!(json!({
                    "component": "boards", "kind": "boards.restore_failed",
                    "path": dir.to_string_lossy(), "message": reason,
                }));
                Self {
                    next: 1,
                    unreadable: Some(reason),
                    ..Self::default()
                }
            }
        }
    }

    /// Every board the core keeps, deleted stubs and kept boards included,
    /// for a reader that only reads (`hide board`, Hide AI's investigation).
    pub(crate) fn boards(&self) -> impl Iterator<Item = &Board> {
        self.book.values()
    }

    pub(crate) fn get(&self, id: &str) -> Option<&Board> {
        self.book.get(id)
    }
}

/// What a `hide board` command answers: an answer made under the lock, or
/// the boards a `show` prints, copied under the lock and drawn as Markdown
/// or as their records after it is released (`BoardTurn::answer`), since a
/// bare `show` prints every board of the session.
pub(crate) enum BoardTurn {
    Answered { answer: Value, changed: bool },
    ShowOne { board: Box<Board>, json: bool },
    ShowAll { boards: Vec<Board>, json: bool },
}

impl BoardTurn {
    /// The command's answer and whether a board changed; a `show` is drawn
    /// here, off the lock.
    pub(crate) fn answer(self) -> (Value, bool) {
        match self {
            Self::Answered { answer, changed } => (answer, changed),
            Self::ShowOne { board, json } => (shown(&board, json), false),
            Self::ShowAll { boards, json } => {
                let shown: Vec<Value> = boards.iter().map(|board| shown(board, json)).collect();
                (json!({"boards": shown}), false)
            }
        }
    }
}

/// Whether `session` is the one `machine` and `id` name.
fn names(session: &SessionIdentity, (machine, id): &(String, String)) -> bool {
    session.device_id == *machine && session.session_id == *id
}

impl Runtime {
    /// Runs a `hide board` command from `caller`'s pane on `device`.
    /// Answers the command's answer and whether the book changed, or the
    /// boards a `show` prints.
    pub(crate) fn board_command(
        &mut self,
        device: &str,
        caller: &str,
        expected: &Context,
        command: BoardCommand,
    ) -> Result<BoardTurn, String> {
        if !command.valid() {
            return Err(refusal("board_invalid", "board: not a board id"));
        }
        let context = self
            .workspace_control_query(device, caller, Query::Info)
            .map_err(|refusal| refusal.reason.to_owned())?
            .context;
        if context != *expected {
            return Err("caller_context_changed".into());
        }
        let Caller::Pane(pane) = Caller::parse(caller) else {
            return Err("agent_pane_required".into());
        };
        let pane = crate::address::named_pane_id(self.node.as_str(), device, pane);
        let actor = self
            .delivery_observations
            .get(&pane)
            .ok_or("agent_pane_required")?
            .actor
            .clone();
        let own = self
            .caller_session(device, &pane, &actor)
            .ok_or("board_session_unknown")?;
        if self.boards.unreadable.is_some() {
            return Err("board_store_unavailable".into());
        }
        let now = super::memory::now_ms();
        let (answer, changed) = match command {
            BoardCommand::New { title, blocks } => {
                let id = format!("board-{}", self.boards.next.max(1));
                let checkout = BoardCheckout {
                    device_id: context.device_id.clone(),
                    path: context.checkout_path.clone(),
                };
                let board = boards::apply_new(id, own.clone(), checkout, &title, &blocks, now)?;
                let evicted = self.make_board_room(&own)?;
                self.boards.next = boards::board_number(&board.id) + 1;
                let answer = json!({"board": boards::summary(&board)});
                let mut changes = evicted;
                changes.push((board.id.clone(), Some(board)));
                self.keep_boards(changes);
                (answer, true)
            }
            BoardCommand::Set {
                board,
                blocks,
                title,
                revision,
            } => {
                let current = self.own_board(&own, &board)?;
                boards::check_changeable(current, revision)?;
                match boards::apply_set(current, title.as_deref(), &blocks, now)? {
                    Some(next) => {
                        let answer = json!({"board": boards::summary(&next)});
                        self.keep_boards(vec![(board, Some(next))]);
                        (answer, true)
                    }
                    None => (json!({"board": boards::summary(current)}), false),
                }
            }
            BoardCommand::Edit {
                board,
                edits,
                revision,
            } => {
                let current = self.own_board(&own, &board)?;
                boards::check_changeable(current, revision)?;
                match boards::apply_edits(current, &edits, now)? {
                    Some(next) => {
                        let answer = json!({"board": boards::summary(&next)});
                        self.keep_boards(vec![(board, Some(next))]);
                        (answer, true)
                    }
                    None => (json!({"board": boards::summary(current)}), false),
                }
            }
            BoardCommand::Show {
                board: Some(id),
                json,
            } => {
                let board = self.boards.get(&id).ok_or("board_unknown")?;
                if !self.may_read(&own, &context, board) {
                    return Err("board_not_yours".into());
                }
                if matches!(board.state, BoardState::Deleted { .. }) {
                    return Err("board_deleted".into());
                }
                return Ok(BoardTurn::ShowOne {
                    board: Box::new(board.clone()),
                    json,
                });
            }
            BoardCommand::Show { board: None, json } => {
                let boards: Vec<Board> = self
                    .boards
                    .boards()
                    .filter(|board| board.owner == own)
                    .filter(|board| !matches!(board.state, BoardState::Deleted { .. }))
                    .cloned()
                    .collect();
                return Ok(BoardTurn::ShowAll { boards, json });
            }
            BoardCommand::List {} => {
                let lines: Vec<Value> = self
                    .boards
                    .boards()
                    .filter(|board| board.owner == own)
                    .map(|board| {
                        json!({
                            "id": board.id,
                            "title": board.title,
                            "state": board.state.name(),
                            "revision": board.revision,
                            "updated_at_unix_ms": board.updated_at_unix_ms,
                        })
                    })
                    .collect();
                (json!({"boards": lines}), false)
            }
            BoardCommand::Delete { board } => {
                let current = self.own_board(&own, &board)?;
                if matches!(current.state, BoardState::Deleted { .. }) {
                    return Err("board_deleted".into());
                }
                let stub = boards::deleted(current, Remover::Agent, now);
                let answer = json!({"board": boards::summary(&stub)});
                self.keep_boards(vec![(board, Some(stub))]);
                (answer, true)
            }
        };
        if changed {
            crate::diagnostic!(json!({
                "component": "boards", "kind": "boards.changed",
                "pane_id": pane, "session_id": own.session_id,
                "board": answer["board"]["id"], "revision": answer["board"]["revision"],
            }));
        }
        Ok(BoardTurn::Answered { answer, changed })
    }

    /// Board `id`, when the caller's session `own` drew it.
    fn own_board(&self, own: &SessionIdentity, id: &str) -> Result<&Board, String> {
        let board = self.boards.get(id).ok_or("board_unknown")?;
        if board.owner != *own {
            return Err("board_not_yours".into());
        }
        Ok(board)
    }

    /// Who reads a board: the session that drew it, an agent in its
    /// checkout, and an ancestor of the drawing session in its lineage, in
    /// any checkout or on any machine.
    fn may_read(&self, own: &SessionIdentity, context: &Context, board: &Board) -> bool {
        board.owner == *own
            || (board.checkout.device_id == context.device_id
                && board.checkout.path == context.checkout_path)
            || self.session_is_ancestor(own, &board.owner)
    }

    /// Whether `ancestor` is a parent, or a parent's parent, of the agent
    /// whose session is `of`, as the delivery ledger's registrations say.
    fn session_is_ancestor(&self, ancestor: &SessionIdentity, of: &SessionIdentity) -> bool {
        let Ok(ledger) = self.delivery_state() else {
            return false;
        };
        let mut current = ledger
            .agents
            .iter()
            .rev()
            .find(|record| record.machine == of.device_id && record.session == of.session_id);
        for _ in 0..LINEAGE_HOPS {
            let Some(parent) = current.and_then(|record| record.parent.as_deref()) else {
                return false;
            };
            current = ledger
                .agents
                .iter()
                .rev()
                .find(|record| record.id == parent);
            if current.is_some_and(|record| {
                record.machine == ancestor.device_id && record.session == ancestor.session_id
            }) {
                return true;
            }
        }
        false
    }

    /// Room for a new board of `own`: refused when the session keeps
    /// `OPEN_PER_SESSION` open boards; with the book full, the board to let
    /// go (a deleted stub first, then a kept board, then an open one whose
    /// session runs in no pane, each the one updated longest ago), or a
    /// refusal when every board belongs to an agent on screen.
    fn make_board_room(
        &self,
        own: &SessionIdentity,
    ) -> Result<Vec<(String, Option<Board>)>, String> {
        let open = self
            .boards
            .book
            .values()
            .filter(|board| board.owner == *own && board.state == BoardState::Open)
            .count();
        if open >= OPEN_PER_SESSION {
            return Err(refusal(
                "board_limit",
                &format!("this session has {open} open boards, at most {OPEN_PER_SESSION}"),
            ));
        }
        if self.boards.book.len() < BOOK_LIMIT {
            return Ok(Vec::new());
        }
        let live: HashSet<SessionIdentity> = self
            .live_sessions()
            .into_iter()
            .map(|(_, session)| session)
            .collect();
        let rank = |board: &Board| match board.state {
            BoardState::Deleted { .. } => Some(0),
            BoardState::Kept { .. } => Some(1),
            BoardState::Open if !live.contains(&board.owner) => Some(2),
            BoardState::Open => None,
        };
        let oldest = self
            .boards
            .book
            .values()
            // Boards updated in the same millisecond go in the order drawn.
            .filter_map(|board| {
                let order = (
                    rank(board)?,
                    board.updated_at_unix_ms,
                    boards::board_number(&board.id),
                );
                Some((order, board))
            })
            .min_by_key(|(order, _)| *order)
            .map(|(_, board)| board.id.clone())
            .ok_or_else(|| {
                refusal(
                    "board_limit",
                    &format!(
                        "Hide keeps {BOOK_LIMIT} boards and each belongs to an agent on screen"
                    ),
                )
            })?;
        crate::diagnostic!(json!({
            "component": "boards", "kind": "boards.evicted",
            "board": oldest, "limit": BOOK_LIMIT,
        }));
        Ok(vec![(oldest, None)])
    }

    /// Puts `changes` in the book (a board, or none for one that goes) and
    /// hands them to the save thread with the index after them.
    fn keep_boards(&mut self, changes: Vec<(String, Option<Board>)>) {
        for (id, board) in &changes {
            match board {
                Some(board) => {
                    self.boards.book.insert(id.clone(), board.clone());
                }
                None => {
                    self.boards.book.remove(id);
                }
            }
        }
        if let Some(files) = &self.boards.files {
            let lines = self.boards.book.values().map(IndexLine::of).collect();
            files.save(changes, self.boards.next.max(1), lines);
        }
    }

    /// Drops the boards of sessions that were deleted or pruned, as their
    /// plans go (`drop_plans_of`); their files are deleted by the save
    /// thread. Answers whether any went.
    pub(crate) fn drop_boards_of(&mut self, sessions: &[(String, String)]) -> bool {
        let gone: Vec<(String, Option<Board>)> = self
            .boards
            .book
            .values()
            .filter(|board| sessions.iter().any(|session| names(&board.owner, session)))
            .map(|board| (board.id.clone(), None))
            .collect();
        if gone.is_empty() {
            return false;
        }
        crate::diagnostic!(json!({
            "component": "boards", "kind": "boards.dropped_with_sessions",
            "boards": gone.iter().map(|(id, _)| id).collect::<Vec<_>>(),
        }));
        self.keep_boards(gone);
        true
    }
}

/// A board as `hide board show` prints it: its line and its Markdown, or
/// with `--json` its record.
fn shown(board: &Board, json: bool) -> Value {
    if json {
        json!({"board": board})
    } else {
        json!({"board": boards::summary(board), "markdown": boards::markdown(board)})
    }
}
