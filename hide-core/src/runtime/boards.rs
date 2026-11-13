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

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Serialize;
use serde_json::{Value, json};

use super::Runtime;
use crate::board_store::{self, BoardFiles, IndexLine, Restored};
use crate::boards::{
    self, BOOK_LIMIT, Board, BoardCheckout, BoardCommand, BoardState, Body, OPEN_PER_SESSION,
    Remover, Tone, ViewHint,
};
use crate::coordination::refusal;
use crate::links::ledger::SessionIdentity;
use crate::view_layout::Display;
use crate::workspace_control::{Action, Caller, Context, Query};

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
    /// Each board the front Workspace shows, by its revision and state, as
    /// `rest.boards` was last made from: the projection is made again only
    /// when this moves.
    shown: Option<Vec<(String, u64, &'static str)>>,
    /// The stamp `rest.boards` carries, moved each time it is made again.
    generation: u64,
    /// The View layouts' generation the board records' View hints were last
    /// read from.
    hinted: Option<u64>,
    /// Set while the start puts the boards' Views back, so their hints are
    /// read once, after the last.
    restoring: bool,
}

/// `rest.boards`: the boards a display of the front Workspace's View layout
/// shows, at most one per display, drawn by the board View from what the
/// core holds (board design, section 3, The wire).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct BoardsSnapshot {
    /// Each shared with the frame before when its board did not move, so a
    /// change to one board builds that board's item alone.
    pub items: Vec<std::sync::Arc<BoardView>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BoardView {
    pub id: String,
    pub title: String,
    /// `open`, `kept` or `deleted` (`contracts/snapshot-wire-enums.json`:
    /// `board_state`).
    pub state: &'static str,
    pub revision: u64,
    pub updated_at_unix_ms: u64,
    pub blocks: Vec<BoardBlockView>,
}

/// One block as the board View draws it: its body as the export writes it
/// (`boards::block_markdown`, which the kept file and Hide AI read too),
/// and the little the frame adds.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BoardBlockView {
    pub id: String,
    /// `contracts/snapshot-wire-enums.json`: `board_block_kind`.
    pub kind: &'static str,
    /// A callout's tone (`board_callout_tone`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
    /// The lines a quote shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote: Option<QuoteRef>,
    pub changed_at: u64,
    pub markdown: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct QuoteRef {
    pub path: String,
    pub start: u32,
    pub end: u32,
}

impl BoardView {
    fn of(board: &Board) -> Self {
        Self {
            id: board.id.clone(),
            title: board.title.clone(),
            state: board.state.name(),
            revision: board.revision,
            updated_at_unix_ms: board.updated_at_unix_ms,
            blocks: board
                .blocks
                .iter()
                .map(|block| BoardBlockView {
                    id: block.id.clone(),
                    kind: block.body.kind(),
                    tone: match &block.body {
                        Body::Callout { tone, .. } => Some(*tone),
                        _ => None,
                    },
                    quote: match &block.body {
                        Body::Quote {
                            path, start, end, ..
                        } => Some(QuoteRef {
                            path: path.clone(),
                            start: *start,
                            end: *end,
                        }),
                        _ => None,
                    },
                    changed_at: block.changed_at,
                    markdown: boards::block_markdown(&block.body),
                })
                .collect(),
        }
    }
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
                ..Self::default()
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
            BoardCommand::New {
                title,
                blocks,
                no_open,
            } => {
                let id = format!("board-{}", self.boards.next.max(1));
                let checkout = BoardCheckout {
                    device_id: context.device_id.clone(),
                    path: context.checkout_path.clone(),
                };
                let board = boards::apply_new(id, own.clone(), checkout, &title, &blocks, now)?;
                let evicted = self.make_board_room(&own)?;
                self.boards.next = boards::board_number(&board.id) + 1;
                let mut answer = json!({"board": boards::summary(&board)});
                let id = board.id.clone();
                let mut changes = evicted;
                changes.push((id.clone(), Some(board)));
                self.keep_boards(changes);
                // The board is drawn whatever its View does: a View that
                // cannot open now says why, and `hide board open` opens it
                // later.
                if !no_open {
                    answer["view"] = match self.open_board_view(device, caller, expected, &id) {
                        Ok(view) => view,
                        Err(refused) => json!({
                            "open": false,
                            "reason": refused.reason,
                            "next_action": boards::next_action(refused.reason),
                        }),
                    };
                }
                (answer, true)
            }
            // The drawing session shows its board, and so does an ancestor
            // of it, from its own pane: as with any View a delegated child
            // opened, the ancestor shows it to the operator by opening it
            // again, so it lands in the ancestor's checkout as the
            // ancestor's to look at. An agent that only shares the checkout
            // reads it with `show`.
            BoardCommand::Open { board } => {
                let current = self.boards.get(&board).ok_or("board_unknown")?;
                if current.owner != own && !self.session_is_ancestor(&own, &current.owner) {
                    return Err("board_not_yours".into());
                }
                match &current.state {
                    BoardState::Open => {}
                    BoardState::Deleted { .. } => return Err("board_deleted".into()),
                    BoardState::Kept { kept_path, .. } => {
                        return Err(refusal("board_kept", &format!("kept as {kept_path}")));
                    }
                }
                let summary = boards::summary(current);
                let view = self
                    .open_board_view(device, caller, expected, &board)
                    .map_err(|refused| refused.reason.to_owned())?;
                (json!({"board": summary, "view": view}), true)
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
                        let mut answer = json!({"board": boards::summary(&next)});
                        let retitled = (next.title != current.title).then(|| next.title.clone());
                        self.keep_boards(vec![(board.clone(), Some(next))]);
                        if let Some(title) = retitled {
                            self.retitle_board_views(&board, &title);
                        }
                        answer["view"] = self.board_view_line(&board);
                        (answer, true)
                    }
                    None => {
                        let mut answer = json!({"board": boards::summary(current)});
                        answer["view"] = self.board_view_line(&board);
                        (answer, false)
                    }
                }
            }
            BoardCommand::Edit {
                board,
                edits,
                revision,
            } => {
                let current = self.own_board(&own, &board)?;
                boards::check_changeable(current, revision)?;
                // An edit never touches the View's unseen record: a board the
                // operator looked at stays seen, and its View draws the edit
                // from `rest.boards` in place (board design, section 3).
                match boards::apply_edits(current, &edits, now)? {
                    Some(next) => {
                        let mut answer = json!({"board": boards::summary(&next)});
                        self.keep_boards(vec![(board.clone(), Some(next))]);
                        answer["view"] = self.board_view_line(&board);
                        (answer, true)
                    }
                    None => {
                        let mut answer = json!({"board": boards::summary(current)});
                        answer["view"] = self.board_view_line(&board);
                        (answer, false)
                    }
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
                            "view": self.board_view_line(&board.id),
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

    /// Makes `rest.boards` again when a board the front Workspace shows, or
    /// which boards it shows, changed since it was last made: one walk of
    /// that layout's displays (at most `MAX_VIEW_DISPLAYS`) and a book
    /// lookup each per frame; a frame with nothing new copies no board.
    pub(super) fn refresh_boards(&mut self) {
        if self.boards.book.is_empty() && self.snapshot.boards.items.is_empty() {
            return;
        }
        let shown: Vec<(String, u64, &'static str)> = self
            .front_workspace_key()
            .and_then(|key| self.view_layout_of(&key))
            .map(|layout| {
                let mut shown: Vec<(String, u64, &'static str)> = Vec::new();
                for display in layout.displays() {
                    let Some(id) = display.board.as_deref() else {
                        continue;
                    };
                    if shown.iter().any(|(known, _, _)| known == id) {
                        continue;
                    }
                    let (revision, state) = self
                        .boards
                        .get(id)
                        .map_or((0, "gone"), |board| (board.revision, board.state.name()));
                    shown.push((id.to_owned(), revision, state));
                }
                shown
            })
            .unwrap_or_default();
        if self.boards.shown.as_ref() == Some(&shown) {
            return;
        }
        // A board whose revision and state did not move keeps the item the
        // last frame built; only a board that moved is drawn again.
        let items = shown
            .iter()
            .filter_map(|(id, revision, state)| {
                let kept = self.snapshot.boards.items.iter().find(|item| {
                    item.id == *id && item.revision == *revision && item.state == *state
                });
                match kept {
                    Some(item) => Some(std::sync::Arc::clone(item)),
                    None => self
                        .boards
                        .get(id)
                        .map(|board| std::sync::Arc::new(BoardView::of(board))),
                }
            })
            .collect();
        self.boards.shown = Some(shown);
        self.boards.generation += 1;
        self.snapshot.boards = BoardsSnapshot { items };
        self.snapshot.boards_generation = Some(self.boards.generation);
    }

    /// Keeps each open board's View hint as its View is now, read from the
    /// layouts each time they are saved (`persist_workspace_views`), whether
    /// or not a window reads them: a View opened, seen, moved or closed by any
    /// path (an agent, the operator, the cap, a closed pane) is one change of
    /// the layouts' generation. A hint that moved is handed to the board
    /// folder's save thread; it moves no revision and publishes nothing.
    pub(super) fn sync_board_views(&mut self) {
        if self.boards.restoring {
            return;
        }
        let Some(generation) = self.workspace_views.as_ref().map(|store| store.generation) else {
            return;
        };
        if self.boards.hinted == Some(generation) {
            return;
        }
        self.boards.hinted = Some(generation);
        if self.boards.book.is_empty() || self.boards.unreadable.is_some() {
            return;
        }
        // Only the View in the board's own checkout is kept: one an ancestor
        // opened in its own checkout is not put back after a restart.
        let mut placed: HashMap<String, ViewHint> = HashMap::new();
        for (key, layout) in self.every_view_layout() {
            for area in layout.areas() {
                for display in &area.displays {
                    let Some(id) = display.board.as_deref() else {
                        continue;
                    };
                    let home = self.boards.get(id).is_some_and(|board| {
                        board.checkout.device_id == key.0 && board.checkout.path == key.1
                    });
                    if !home {
                        continue;
                    }
                    placed.entry(id.to_owned()).or_insert_with(|| ViewHint {
                        area_id: area.id.clone(),
                        opener: display.opener.clone(),
                        openers: display.openers.clone(),
                        operator: display.operator,
                        unseen: display.unseen.clone(),
                    });
                }
            }
        }
        let changes: Vec<(String, Option<Board>)> = self
            .boards
            .book
            .values()
            .filter(|board| board.state == BoardState::Open)
            .filter_map(|board| {
                let hint = placed.remove(&board.id);
                (board.view != hint).then(|| {
                    (
                        board.id.clone(),
                        Some(Board {
                            view: hint,
                            ..board.clone()
                        }),
                    )
                })
            })
            .collect();
        if !changes.is_empty() {
            self.keep_boards(changes);
        }
    }

    /// Puts each open board's View back where its hint says it was, once,
    /// when the runtime is made, after the saved layouts are read: at the end
    /// of its area, behind what that area shows, with its openers and its
    /// unseen record, so a board still to look at waits in Needs You again.
    /// A gone area gives way to the checkout's active one; a full Workspace
    /// keeps the board closed, for `hide board open`.
    pub(super) fn restore_board_views(&mut self) {
        let hinted: Vec<(String, String, (String, String), ViewHint)> = self
            .boards
            .book
            .values()
            .filter(|board| board.state == BoardState::Open)
            .filter_map(|board| {
                Some((
                    board.id.clone(),
                    board.title.clone(),
                    (
                        board.checkout.device_id.clone(),
                        board.checkout.path.clone(),
                    ),
                    board.view.clone()?,
                ))
            })
            .collect();
        // Each placement saves the layouts; the hints are read again once
        // every board is back, not between two of them.
        self.boards.restoring = true;
        for (id, title, key, hint) in hinted {
            let placed = self.change_view_layout(&key, |layout, _| {
                if layout
                    .displays()
                    .any(|display| display.board.as_deref() == Some(id.as_str()))
                {
                    return Ok(((), false));
                }
                let area = layout
                    .area(&hint.area_id)
                    .map_or_else(|| layout.active_area().id.clone(), |area| area.id.clone());
                let mut display = layout.new_board_display(&id, &title);
                display.opener = hint.opener.clone();
                display.openers = hint.openers.clone();
                display.operator = hint.operator;
                display.unseen = hint.unseen.clone();
                layout.append(&area, display)?;
                Ok(((), true))
            });
            if let Err(error) = placed {
                crate::diagnostic!(json!({
                    "component": "boards", "kind": "boards.view_restore_failed",
                    "board": id, "device": key.0, "error": error.kind(),
                }));
            }
        }
        self.boards.restoring = false;
        self.sync_board_views();
    }

    /// Opens board `id`'s View from the caller's pane through the View
    /// action every agent open takes, so it lands where `hide file open`
    /// lands, behind the front when the caller is not the pane in front,
    /// wears its opener's mark and waits in Needs You until the operator
    /// looks. Answers the View's line for the command's answer.
    fn open_board_view(
        &mut self,
        device: &str,
        caller: &str,
        expected: &Context,
        id: &str,
    ) -> Result<Value, crate::workspace_control::Refusal> {
        let request_id = format!("{}-board-{id}", super::unix_milliseconds());
        let action = Action::OpenBoard {
            board: id.to_owned(),
        };
        let opened =
            self.workspace_control_action(device, caller, expected, &request_id, action, Ok(None))?;
        // The View this open showed, in the caller's checkout, though the
        // board may be shown in another checkout too.
        let key = (
            opened.context.device_id.clone(),
            opened.context.checkout_path.clone(),
        );
        let unseen = self
            .view_layout_of(&key)
            .and_then(|layout| layout.display(&opened.view_id))
            .is_some_and(|display| display.unseen.is_some());
        Ok(json!({"id": opened.view_id, "open": true, "unseen": unseen}))
    }

    /// The View that shows board `id`: the one in the board's own checkout,
    /// else one an ancestor opened in its own.
    fn board_display(&self, id: &str) -> Option<&Display> {
        let home = self.boards.get(id).map(|board| {
            (
                board.checkout.device_id.clone(),
                board.checkout.path.clone(),
            )
        });
        home.as_ref()
            .and_then(|home| self.view_layout_of(home))
            .and_then(|layout| shown_board(layout, id))
            .or_else(|| {
                self.every_view_layout()
                    .find_map(|(_, layout)| shown_board(layout, id))
            })
    }

    /// Board `id`'s View as an answer names it: its id and whether the
    /// operator has still to look at it, or that it has none open.
    fn board_view_line(&self, id: &str) -> Value {
        match self.board_display(id) {
            Some(display) => json!({
                "id": display.id,
                "open": true,
                "unseen": display.unseen.is_some(),
            }),
            None => json!({"open": false}),
        }
    }

    /// The title board `id`'s Views carry, after `hide board set` changed
    /// it, so its tab and its row in Needs You name it as it is now.
    fn retitle_board_views(&mut self, id: &str, title: &str) {
        let keys: Vec<_> = self
            .every_view_layout()
            .filter(|(_, layout)| {
                layout
                    .displays()
                    .any(|display| display.board.as_deref() == Some(id))
            })
            .map(|(key, _)| key)
            .collect();
        for key in keys {
            let retitled = self.change_view_layout(&key, |layout, _| {
                let ids: Vec<String> = layout
                    .displays()
                    .filter(|display| display.board.as_deref() == Some(id))
                    .map(|display| display.id.clone())
                    .collect();
                for view in &ids {
                    if let Some(display) = layout.display_mut(view) {
                        display.title = Some(title.to_owned());
                    }
                }
                Ok(((), !ids.is_empty()))
            });
            if let Err(error) = retitled {
                crate::diagnostic!(json!({
                    "component": "boards", "kind": "boards.view_retitle_failed",
                    "board": id, "device": key.0, "error": error.kind(),
                }));
            }
        }
        self.refresh_unseen_rows();
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
    /// session runs in no pane and whose View is closed, each the one
    /// updated longest ago), or a refusal when every board belongs to an
    /// agent on screen or is on the operator's screen.
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
            BoardState::Open
                if !live.contains(&board.owner) && self.board_display(&board.id).is_none() =>
            {
                Some(2)
            }
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

/// The display of `layout` that shows board `id`.
fn shown_board<'a>(layout: &'a crate::view_layout::Layout, id: &str) -> Option<&'a Display> {
    layout
        .displays()
        .find(|display| display.board.as_deref() == Some(id))
}

/// A board as `hide board show` prints it: its line and its Markdown, or
/// with `--json` its record.
fn shown(board: &Board, json: bool) -> Value {
    if json {
        // Where its View is is the screen's, not what the agent drew.
        let mut record = json!(board);
        if let Some(record) = record.as_object_mut() {
            record.remove("view");
        }
        json!({"board": record})
    } else {
        json!({"board": boards::summary(board), "markdown": boards::markdown(board)})
    }
}
