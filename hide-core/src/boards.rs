//! A session's boards: what an agent draws for the operator with `hide
//! board`, kept by the core as a record of its own, keyed by the native
//! session that drew it, as a plan is (`plans.rs`).
//!
//! A board is a title and a list of blocks: an explanation, a callout (a
//! decision is one), a table, code the agent writes, code quoted from a file
//! of the checkout, one Mermaid diagram, or a divider. This module owns the
//! model, the rules each block is held to (a refusal names the field that
//! broke one, such as `blocks[2].rows[3]`), the commands that change a board,
//! applied all or none, and the one Markdown export the screen, a kept file
//! and Hide AI share (`markdown`). The runtime keeps the book and who may
//! change what (`runtime/boards.rs`); the folder is `board_store.rs`'s.
//!
//! The core never reads a file for a board: a quote's lines are read by the
//! `hide` command in the agent's pane, on the machine that holds the file,
//! and the core only checks what it was sent.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::links::ledger::SessionIdentity;

/// Characters of a board's title, and of a block's title.
pub const TITLE_CHARS: usize = 120;
/// Blocks one board holds.
pub const BLOCK_LIMIT: usize = 60;
/// Bytes of one board as it is saved, which bounds it on disk and on the
/// wire.
pub const BOARD_BYTES: usize = 64 * 1024;
/// Why a board's View shows nothing: its record is a deleted stub, or went
/// with its session. `hide view status` prints it as the View's reason.
pub const DELETED_REASON: &str = "board_deleted";
/// Operations one `hide board edit` applies, as one revision.
pub const EDIT_LIMIT: usize = 20;
/// Open boards one session keeps at once.
pub const OPEN_PER_SESSION: usize = 16;
/// Records the core keeps, deleted and kept ones counted; past it the
/// oldest that no agent on screen owns is let go (`runtime/boards.rs`).
pub const BOOK_LIMIT: usize = 256;
/// Lines one quote holds.
pub const QUOTE_LINES: usize = 80;
/// Bytes of a file the `hide` command reads a quote from.
pub const QUOTE_FILE_BYTES: u64 = 1024 * 1024;

const SECTION_TEXT_CHARS: usize = 4000;
const CALLOUT_TEXT_CHARS: usize = 1000;
const TABLE_COLUMNS: usize = 8;
const TABLE_ROWS: usize = 40;
const CELL_CHARS: usize = 200;
const CODE_CHARS: usize = 8000;
const CODE_LINES: usize = 120;
const CODE_LINE_CHARS: usize = 240;
const MERMAID_CHARS: usize = 8000;
const NOTE_CHARS: usize = 1000;
const LANGUAGE_CHARS: usize = 32;

/// The refusals whose reason carries a detail (the field to fix, the
/// current revision, the kept file), which the daemon adds to the next
/// action (`coordination::split_refusal`).
pub const DETAILED: &[&str] = &[
    "board_invalid",
    "board_too_large",
    "board_limit",
    "board_moved",
    "board_kept",
    "quote_refused",
];

/// A callout's tone; a decision the agent took or asks for is `decision`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Info,
    Decision,
    Risk,
}

/// What one block shows. The agent writes it as JSON tagged by `type`; a
/// quote's `lines` are filled by the `hide` command from the file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Body {
    /// An explanation: its text is Markdown.
    Section { title: String, text: String },
    Callout {
        tone: Tone,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        text: String,
    },
    Table {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        columns: Vec<String>,
        #[serde(default)]
        rows: Vec<Vec<String>>,
    },
    /// Code the agent writes itself.
    Code {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
        code: String,
    },
    /// Lines `start..=end` (from 1) of a file of the checkout, as they were
    /// when the board was drawn, so the board keeps showing what was
    /// discussed after the file changes.
    Quote {
        path: String,
        start: u32,
        end: u32,
        #[serde(default)]
        lines: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// One diagram, as Mermaid source.
    Diagram {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        mermaid: String,
    },
    /// A rule between groups of blocks; it carries no content.
    Divider {},
}

impl Body {
    /// The block's kind as the wire names it.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Section { .. } => "section",
            Self::Callout { .. } => "callout",
            Self::Table { .. } => "table",
            Self::Code { .. } => "code",
            Self::Quote { .. } => "quote",
            Self::Diagram { .. } => "diagram",
            Self::Divider {} => "divider",
        }
    }
}

/// One block of a board. Its id is `b<n>`, from the board's `next_block`,
/// and is never reused, so a later reference to a block (a comment, a
/// reader's place) cannot name another one.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Block {
    pub id: String,
    /// The board's revision at which this block last changed.
    pub changed_at: u64,
    #[serde(flatten)]
    pub body: Body,
}

/// Who deleted a board.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Remover {
    Agent,
    Operator,
}

/// Where a board stands. A kept board is read-only and its file is the one
/// to change; a deleted board is a stub whose blocks are gone, kept so a
/// later command learns it was deleted rather than never drawn.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum BoardState {
    Open,
    Kept {
        kept_path: String,
        kept_at_unix_ms: u64,
    },
    Deleted {
        deleted_by: Remover,
        deleted_at_unix_ms: u64,
    },
}

impl BoardState {
    /// The state as the wire names it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Kept { .. } => "kept",
            Self::Deleted { .. } => "deleted",
        }
    }
}

/// The checkout a board was drawn in: the machine and its path there.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct BoardCheckout {
    pub device_id: String,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Board {
    /// `board-<n>`, from the store's index, never reused.
    pub id: String,
    /// The native session that drew it, which alone changes it.
    pub owner: SessionIdentity,
    pub checkout: BoardCheckout,
    pub title: String,
    pub blocks: Vec<Block>,
    /// The number the next block's id takes; it only grows.
    pub next_block: u32,
    /// Moves by one with each accepted command that changed the board.
    pub revision: u64,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    #[serde(flatten)]
    pub state: BoardState,
    /// Where its View is, kept so a restart puts it back (design section 3,
    /// Restart): a board display is never stored in `workspace-views.json`.
    /// Absent while it has no View. It is no part of what was drawn: it
    /// moves no revision, and a board's size and the wire leave it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewHint>,
}

/// Where a board's View was and what the operator had of it: the checkout
/// and area it was in, who opened it and whether it was still to look at.
/// Written from the View layouts when they change, read once at a start.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ViewHint {
    pub device_id: String,
    pub checkout_path: String,
    pub area_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opener: Option<crate::view_feedback::Opener>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub openers: Vec<crate::view_feedback::Opener>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub operator: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::view_feedback::unseen_or_unreadable"
    )]
    pub unseen: Option<crate::view_feedback::Unseen>,
}

/// The number in a board id, or 0 for a string that is not one.
pub fn board_number(id: &str) -> u64 {
    id.strip_prefix("board-")
        .and_then(|number| number.parse::<u64>().ok())
        .filter(|number| *number > 0 && id == format!("board-{number}"))
        .unwrap_or(0)
}

/// Whether `id` is a board id: `board-<n>` with `n` from 1, as written.
pub fn valid_id(id: &str) -> bool {
    board_number(id) > 0
}

/// One `hide board` command. The blocks and edits travel as JSON values so
/// the core, the one place that checks them, names the field a refusal is
/// about.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "verb", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoardCommand {
    /// Draws a board and, unless `no_open`, opens its View as `hide file
    /// open` opens a file.
    New {
        title: String,
        blocks: Vec<Value>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        no_open: bool,
    },
    /// Brings the board's View forward, or makes it again in the caller's
    /// checkout; like any agent's open, a board already seen is news again.
    Open {
        board: String,
    },
    /// Replaces the title (when given) and every block.
    Set {
        board: String,
        blocks: Vec<Value>,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        revision: Option<u64>,
    },
    Edit {
        board: String,
        edits: Vec<Value>,
        #[serde(default)]
        revision: Option<u64>,
    },
    /// One board, or every board of the caller's session.
    Show {
        #[serde(default)]
        board: Option<String>,
        /// The record instead of its Markdown.
        #[serde(default)]
        json: bool,
    },
    List {},
    Delete {
        board: String,
    },
}

impl BoardCommand {
    /// The bounds a daemon checks before the command reaches the core: a
    /// board id where one is named is a key (one an agent could type; the
    /// core answers `board_unknown` for one it does not keep). The title,
    /// the blocks and the edits are the core's to check, so their refusal
    /// names the field.
    pub fn valid(&self) -> bool {
        let key = |id: &str| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control);
        match self {
            Self::New { .. } | Self::List {} | Self::Show { board: None, .. } => true,
            Self::Set { board, .. }
            | Self::Edit { board, .. }
            | Self::Open { board }
            | Self::Delete { board }
            | Self::Show {
                board: Some(board), ..
            } => key(board),
        }
    }
}

/// One operation of `hide board edit`: the reference model's five.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    /// Puts `block` after `after`, or at the end without it.
    Insert {
        block: Value,
        #[serde(default)]
        after: Option<String>,
    },
    /// Changes some fields of one block; its type stays.
    Update {
        id: String,
        changes: Map<String, Value>,
    },
    /// Puts a whole new block in its place, keeping its id.
    Replace {
        id: String,
        block: Value,
    },
    Remove {
        id: String,
    },
    /// Puts the block after `after`, or at the end without it.
    Move {
        id: String,
        #[serde(default)]
        after: Option<String>,
    },
}

/// A refusal with the field it is about, joined as the daemon splits it.
fn refused(code: &str, detail: impl std::fmt::Display) -> String {
    crate::coordination::refusal(code, &detail.to_string())
}

fn invalid(at: &str, why: impl std::fmt::Display) -> String {
    refused("board_invalid", format!("{at}: {why}"))
}

/// Line ends as one `\n`, so a board written on Windows exports the same.
fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// A title: one line, not blank, at most `TITLE_CHARS` characters.
fn check_title(at: &str, title: &str) -> Result<String, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err(invalid(at, "is empty"));
    }
    if title.chars().any(char::is_control) {
        return Err(invalid(at, "is one line without control characters"));
    }
    let chars = title.chars().count();
    if chars > TITLE_CHARS {
        return Err(invalid(
            at,
            format!("{chars} characters, at most {TITLE_CHARS}"),
        ));
    }
    Ok(title.to_owned())
}

fn check_optional_title(at: &str, title: Option<&str>) -> Result<Option<String>, String> {
    title.map(|title| check_title(at, title)).transpose()
}

/// Text of several lines: not blank, at most `limit` characters, and no
/// control character but a line break or a tab.
fn check_text(at: &str, text: &str, limit: usize, blank: bool) -> Result<String, String> {
    let text = normalize(text);
    if !blank && text.trim().is_empty() {
        return Err(invalid(at, "is empty"));
    }
    if text
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(invalid(at, "holds a control character"));
    }
    let chars = text.chars().count();
    if chars > limit {
        return Err(invalid(at, format!("{chars} characters, at most {limit}")));
    }
    Ok(text)
}

/// A table cell or column name: one line of at most `CELL_CHARS`.
fn check_cell(at: &str, cell: &str) -> Result<String, String> {
    if cell.chars().any(char::is_control) {
        return Err(invalid(at, "a cell is one line without control characters"));
    }
    let chars = cell.chars().count();
    if chars > CELL_CHARS {
        return Err(invalid(
            at,
            format!("{chars} characters, at most {CELL_CHARS}"),
        ));
    }
    Ok(cell.to_owned())
}

/// Why a quote's path is refused, or none: it must name a file below the
/// checkout's root, as every machine spells it (`/` between names), with no
/// `..`, and no name of it may be a credential file, a folder named for
/// secrets or Git's own folder. The rule is the suggestion investigation's
/// (`readable_path`, which this module depends on), so every path a board
/// quotes is one Hide AI may read. The `hide` command refuses such a path
/// before it reads anything (`quote_refused`), and the core refuses it
/// again, so a board never carries a credential past this guard.
pub fn quote_path_refused(path: &str) -> Option<String> {
    match crate::suggestions::investigation::reads::readable_path(path) {
        Ok(()) => None,
        Err("credential_file") => Some("names a credential file, a secrets folder or .git".into()),
        Err(_) => Some(format!(
            "is not a path of at most {} characters below the checkout's root, with / between names and no . or .. names",
            crate::suggestions::policy::READ_PATH_CHARS
        )),
    }
}

/// A block's body as the agent wrote it at `at`, checked and with its text
/// normalized.
pub(crate) fn parse_body(at: &str, value: &Value) -> Result<Body, String> {
    let body: Body = serde_json::from_value(value.clone()).map_err(|error| invalid(at, error))?;
    check_body(at, body)
}

fn check_body(at: &str, body: Body) -> Result<Body, String> {
    let field = |name: &str| format!("{at}.{name}");
    Ok(match body {
        Body::Section { title, text } => Body::Section {
            title: check_title(&field("title"), &title)?,
            text: check_text(&field("text"), &text, SECTION_TEXT_CHARS, false)?,
        },
        Body::Callout { tone, title, text } => Body::Callout {
            tone,
            title: check_optional_title(&field("title"), title.as_deref())?,
            text: check_text(&field("text"), &text, CALLOUT_TEXT_CHARS, false)?,
        },
        Body::Table {
            title,
            columns,
            rows,
        } => {
            let title = check_optional_title(&field("title"), title.as_deref())?;
            if columns.is_empty() || columns.len() > TABLE_COLUMNS {
                return Err(invalid(
                    &field("columns"),
                    format!(
                        "{} columns, a table has 1 to {TABLE_COLUMNS}",
                        columns.len()
                    ),
                ));
            }
            if rows.len() > TABLE_ROWS {
                return Err(invalid(
                    &field("rows"),
                    format!("{} rows, at most {TABLE_ROWS}", rows.len()),
                ));
            }
            let columns = columns
                .iter()
                .enumerate()
                .map(|(index, cell)| check_cell(&format!("{at}.columns[{index}]"), cell))
                .collect::<Result<Vec<_>, _>>()?;
            let mut checked = Vec::with_capacity(rows.len());
            for (index, row) in rows.iter().enumerate() {
                let at_row = format!("{at}.rows[{index}]");
                if row.len() != columns.len() {
                    return Err(invalid(
                        &at_row,
                        format!(
                            "{} cells, the table has {} columns",
                            row.len(),
                            columns.len()
                        ),
                    ));
                }
                checked.push(
                    row.iter()
                        .enumerate()
                        .map(|(cell, text)| check_cell(&format!("{at_row}[{cell}]"), text))
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
            Body::Table {
                title,
                columns,
                rows: checked,
            }
        }
        Body::Code {
            title,
            language,
            code,
        } => {
            let title = check_optional_title(&field("title"), title.as_deref())?;
            let language = language
                .map(|language| {
                    let language = language.trim().to_owned();
                    let fits = !language.is_empty()
                        && language.chars().count() <= LANGUAGE_CHARS
                        && language
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || "+#._-".contains(c));
                    if fits {
                        Ok(language)
                    } else {
                        Err(invalid(
                            &field("language"),
                            format!(
                                "is one word of at most {LANGUAGE_CHARS} letters, digits or +#._-"
                            ),
                        ))
                    }
                })
                .transpose()?;
            let code = check_text(&field("code"), &code, CODE_CHARS, false)?;
            let lines = code.lines().count();
            if lines > CODE_LINES {
                return Err(invalid(
                    &field("code"),
                    format!("{lines} lines, at most {CODE_LINES}"),
                ));
            }
            if let Some((index, line)) = code
                .lines()
                .enumerate()
                .find(|(_, line)| line.chars().count() > CODE_LINE_CHARS)
            {
                return Err(invalid(
                    &field("code"),
                    format!(
                        "line {} is {} characters, at most {CODE_LINE_CHARS}",
                        index + 1,
                        line.chars().count()
                    ),
                ));
            }
            Body::Code {
                title,
                language,
                code,
            }
        }
        Body::Quote {
            path,
            start,
            end,
            lines,
            note,
        } => {
            if let Some(why) = quote_path_refused(&path) {
                return Err(refused(
                    "quote_refused",
                    format!("{}: {why}", field("path")),
                ));
            }
            if start == 0 || end < start {
                return Err(invalid(
                    &field("end"),
                    format!("lines {start} to {end}: start is 1 or more and end is start or more"),
                ));
            }
            let count = (end - start) as usize + 1;
            if count > QUOTE_LINES {
                return Err(invalid(
                    &field("end"),
                    format!("{count} lines, a quote holds at most {QUOTE_LINES}"),
                ));
            }
            if lines.is_empty() {
                return Err(invalid(
                    &field("lines"),
                    "are missing; hide board reads them from the file",
                ));
            }
            if lines.len() != count {
                return Err(invalid(
                    &field("lines"),
                    format!("{} lines for lines {start} to {end}", lines.len()),
                ));
            }
            let lines = lines
                .iter()
                .enumerate()
                .map(|(index, line)| {
                    let line = line.trim_end_matches(['\r', '\n']);
                    if line.chars().any(|c| c.is_control() && c != '\t') {
                        Err(invalid(
                            &format!("{at}.lines[{index}]"),
                            "holds a control character",
                        ))
                    } else {
                        Ok(line.to_owned())
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            let note = note
                .map(|note| check_text(&field("note"), &note, NOTE_CHARS, false))
                .transpose()?;
            Body::Quote {
                path,
                start,
                end,
                lines,
                note,
            }
        }
        Body::Diagram { title, mermaid } => Body::Diagram {
            title: check_optional_title(&field("title"), title.as_deref())?,
            mermaid: check_text(&field("mermaid"), &mermaid, MERMAID_CHARS, false)?,
        },
        Body::Divider {} => Body::Divider {},
    })
}

/// The board as saved, which `BOARD_BYTES` bounds.
pub(crate) fn encode(board: &Board) -> Result<Vec<u8>, String> {
    serde_json::to_vec(board).map_err(|error| format!("the board could not be encoded: {error}"))
}

fn check_size(board: &Board) -> Result<(), String> {
    if board.blocks.len() > BLOCK_LIMIT {
        return Err(refused(
            "board_too_large",
            format!(
                "blocks: {} blocks, a board holds at most {BLOCK_LIMIT}",
                board.blocks.len()
            ),
        ));
    }
    // Where its View is is not what the agent drew.
    let bytes = match &board.view {
        None => encode(board)?.len(),
        Some(_) => encode(&Board {
            view: None,
            ..board.clone()
        })?
        .len(),
    };
    if bytes > BOARD_BYTES {
        return Err(refused(
            "board_too_large",
            format!("board: {bytes} bytes, a board holds at most {BOARD_BYTES}"),
        ));
    }
    Ok(())
}

/// The blocks `values` give, each with an id: a value that names an id of
/// `previous` keeps that block's id (and its `changed_at` when nothing in it
/// changed); the rest take new ids from `next_block`.
fn assign_blocks(
    previous: &[Block],
    values: &[Value],
    next_block: &mut u32,
    revision: u64,
) -> Result<Vec<Block>, String> {
    if values.len() > BLOCK_LIMIT {
        return Err(refused(
            "board_too_large",
            format!(
                "blocks: {} blocks, a board holds at most {BLOCK_LIMIT}",
                values.len()
            ),
        ));
    }
    let mut blocks: Vec<Block> = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let at = format!("blocks[{index}]");
        let mut value = value.clone();
        let named = match value.as_object_mut().map(|fields| fields.remove("id")) {
            None => return Err(invalid(&at, "a block is a JSON object")),
            Some(None) => None,
            Some(Some(Value::String(id))) => Some(id),
            Some(Some(_)) => return Err(invalid(&format!("{at}.id"), "is a block id such as b3")),
        };
        let body = parse_body(&at, &value)?;
        let block = match named {
            Some(id) => {
                let Some(before) = previous.iter().find(|block| block.id == id) else {
                    return Err(invalid(
                        &format!("{at}.id"),
                        format!("no block {id} on this board"),
                    ));
                };
                if blocks.iter().any(|block| block.id == id) {
                    return Err(invalid(&format!("{at}.id"), format!("{id} is named twice")));
                }
                let changed_at = if before.body == body {
                    before.changed_at
                } else {
                    revision
                };
                Block {
                    id,
                    changed_at,
                    body,
                }
            }
            None => new_block(next_block, revision, body),
        };
        blocks.push(block);
    }
    Ok(blocks)
}

fn new_block(next_block: &mut u32, revision: u64, body: Body) -> Block {
    let id = format!("b{next_block}");
    *next_block += 1;
    Block {
        id,
        changed_at: revision,
        body,
    }
}

/// The board `hide board new` draws: revision 1, its blocks `b1`, `b2`, …
pub(crate) fn apply_new(
    id: String,
    owner: SessionIdentity,
    checkout: BoardCheckout,
    title: &str,
    values: &[Value],
    now: u64,
) -> Result<Board, String> {
    let title = check_title("title", title)?;
    let mut next_block = 1;
    let blocks = assign_blocks(&[], values, &mut next_block, 1)?;
    let board = Board {
        id,
        owner,
        checkout,
        title,
        blocks,
        next_block,
        revision: 1,
        created_at_unix_ms: now,
        updated_at_unix_ms: now,
        state: BoardState::Open,
        view: None,
    };
    check_size(&board)?;
    Ok(board)
}

/// Refuses a command on a board that cannot change, or one named at a
/// revision it has moved past.
pub(crate) fn check_changeable(board: &Board, revision: Option<u64>) -> Result<(), String> {
    match &board.state {
        BoardState::Open => {}
        BoardState::Deleted { .. } => return Err("board_deleted".into()),
        BoardState::Kept { kept_path, .. } => {
            return Err(refused("board_kept", format!("its file is {kept_path}")));
        }
    }
    match revision {
        Some(revision) if revision != board.revision => Err(refused(
            "board_moved",
            format!(
                "the board is at revision {}, not {revision}",
                board.revision
            ),
        )),
        _ => Ok(()),
    }
}

/// `next` as the board's next revision when it differs from `board`, or
/// none when the command changed nothing.
fn settle(board: &Board, mut next: Board, now: u64) -> Result<Option<Board>, String> {
    if next.title == board.title && next.blocks == board.blocks {
        return Ok(None);
    }
    next.revision = board.revision + 1;
    next.updated_at_unix_ms = now;
    check_size(&next)?;
    Ok(Some(next))
}

/// `hide board set`: the title (when given) and every block replaced, the
/// board's id and revision line kept. None when nothing changed.
pub(crate) fn apply_set(
    board: &Board,
    title: Option<&str>,
    values: &[Value],
    now: u64,
) -> Result<Option<Board>, String> {
    let mut next = board.clone();
    if let Some(title) = title {
        next.title = check_title("title", title)?;
    }
    next.blocks = assign_blocks(
        &board.blocks,
        values,
        &mut next.next_block,
        board.revision + 1,
    )?;
    settle(board, next, now)
}

/// The fields of a quote its lines are read from: an update that changes
/// one of them without new lines is refused, since the core never reads
/// the file.
const QUOTE_SOURCE: [&str; 3] = ["path", "start", "end"];

/// `hide board edit`: every operation applied in order, or none. None when
/// they changed nothing.
pub(crate) fn apply_edits(
    board: &Board,
    edits: &[Value],
    now: u64,
) -> Result<Option<Board>, String> {
    if edits.is_empty() {
        return Err(invalid("edits", "is empty"));
    }
    if edits.len() > EDIT_LIMIT {
        return Err(refused(
            "board_too_large",
            format!(
                "edits: {} edits, a command applies at most {EDIT_LIMIT}",
                edits.len()
            ),
        ));
    }
    let revision = board.revision + 1;
    let mut next = board.clone();
    for (index, value) in edits.iter().enumerate() {
        let at = format!("edits[{index}]");
        let edit: Edit =
            serde_json::from_value(value.clone()).map_err(|error| invalid(&at, error))?;
        apply_edit(&mut next, &at, edit, revision)?;
    }
    settle(board, next, now)
}

fn position(board: &Board, at: &str, id: &str) -> Result<usize, String> {
    board
        .blocks
        .iter()
        .position(|block| block.id == id)
        .ok_or_else(|| invalid(at, format!("no block {id} on this board")))
}

/// Where a block goes: after `after`, or at the end.
fn place(board: &Board, at: &str, after: Option<&str>) -> Result<usize, String> {
    match after {
        Some(after) => Ok(position(board, &format!("{at}.after"), after)? + 1),
        None => Ok(board.blocks.len()),
    }
}

fn apply_edit(board: &mut Board, at: &str, edit: Edit, revision: u64) -> Result<(), String> {
    match edit {
        Edit::Insert { block, after } => {
            if block.get("id").is_some() {
                return Err(invalid(
                    &format!("{at}.block.id"),
                    "an inserted block takes a new id",
                ));
            }
            let body = parse_body(&format!("{at}.block"), &block)?;
            let index = place(board, at, after.as_deref())?;
            if board.blocks.len() >= BLOCK_LIMIT {
                return Err(refused(
                    "board_too_large",
                    format!("{at}: a board holds at most {BLOCK_LIMIT} blocks"),
                ));
            }
            let block = new_block(&mut board.next_block, revision, body);
            board.blocks.insert(index, block);
        }
        Edit::Update { id, changes } => {
            let index = position(board, &format!("{at}.id"), &id)?;
            let current = &board.blocks[index];
            if let Some(kind) = changes.get("type")
                && kind.as_str() != Some(current.body.kind())
            {
                return Err(invalid(
                    &format!("{at}.changes.type"),
                    "a block keeps its type; replace it to change the type",
                ));
            }
            if changes.contains_key("id") {
                return Err(invalid(&format!("{at}.changes.id"), "a block keeps its id"));
            }
            let mut merged = match serde_json::to_value(&current.body) {
                Ok(Value::Object(fields)) => fields,
                _ => return Err(invalid(at, "the block could not be read")),
            };
            if matches!(current.body, Body::Quote { .. })
                && QUOTE_SOURCE.iter().any(|key| changes.contains_key(*key))
                && !changes.contains_key("lines")
            {
                return Err(invalid(
                    &format!("{at}.changes"),
                    "a quote's path, start or end changes with replace, which reads its lines again",
                ));
            }
            for (key, value) in changes {
                // `null` takes an optional field away, as the reference does.
                if value.is_null() {
                    merged.remove(&key);
                } else {
                    merged.insert(key, value);
                }
            }
            let body = parse_body(&format!("{at}.changes"), &Value::Object(merged))?;
            let block = &mut board.blocks[index];
            if block.body != body {
                block.body = body;
                block.changed_at = revision;
            }
        }
        Edit::Replace { id, block } => {
            let index = position(board, &format!("{at}.id"), &id)?;
            if block
                .get("id")
                .is_some_and(|named| named.as_str() != Some(id.as_str()))
            {
                return Err(invalid(
                    &format!("{at}.block.id"),
                    "a replaced block keeps its id",
                ));
            }
            let mut block = block;
            if let Some(fields) = block.as_object_mut() {
                fields.remove("id");
            }
            let body = parse_body(&format!("{at}.block"), &block)?;
            let block = &mut board.blocks[index];
            if block.body != body {
                block.body = body;
                block.changed_at = revision;
            }
        }
        Edit::Remove { id } => {
            let index = position(board, &format!("{at}.id"), &id)?;
            board.blocks.remove(index);
        }
        Edit::Move { id, after } => {
            if after.as_deref() == Some(id.as_str()) {
                return Err(invalid(
                    &format!("{at}.after"),
                    "a block cannot be placed after itself",
                ));
            }
            let index = position(board, &format!("{at}.id"), &id)?;
            // The block leaves first, so `after` is found where it is now;
            // a refusal drops the whole draft (`apply_edits`).
            let block = board.blocks.remove(index);
            let index = place(board, at, after.as_deref())?;
            board.blocks.insert(index, block);
        }
    }
    Ok(())
}

/// The stub a deleted board leaves: its id, owner, checkout and title, and
/// who deleted it when.
pub(crate) fn deleted(board: &Board, by: Remover, now: u64) -> Board {
    Board {
        blocks: Vec::new(),
        revision: board.revision + 1,
        updated_at_unix_ms: now,
        state: BoardState::Deleted {
            deleted_by: by,
            deleted_at_unix_ms: now,
        },
        // A stub is never put back on screen.
        view: None,
        ..board.clone()
    }
}

/// The board as one line of a command's answer: its id, title, revision,
/// state, and each block's id and type.
pub fn summary(board: &Board) -> Value {
    serde_json::json!({
        "id": board.id,
        "title": board.title,
        "revision": board.revision,
        "state": board.state.name(),
        "blocks": board
            .blocks
            .iter()
            .map(|block| serde_json::json!({"id": block.id, "type": block.body.kind()}))
            .collect::<Vec<_>>(),
    })
}

/// The longest run of backticks in `text`.
fn longest_ticks(text: &str) -> usize {
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest
}

/// A fence of backticks longer than any run of backticks in `text`, and
/// at least three.
fn fence(text: &str) -> String {
    "`".repeat((longest_ticks(text) + 1).max(3))
}

fn fenced(language: &str, text: &str) -> String {
    let fence = fence(text);
    format!("{fence}{language}\n{text}\n{fence}")
}

/// `text` as inline code, with enough backticks around it that none inside
/// ends it.
fn inline_code(text: &str) -> String {
    if !text.contains('`') {
        return format!("`{text}`");
    }
    let ticks = "`".repeat(longest_ticks(text) + 1);
    format!("{ticks} {text} {ticks}")
}

/// The fence language a quoted file's extension names, or none.
fn language_of(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path);
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "rs" => "rust",
        "ts" | "mts" | "cts" => "ts",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" => "js",
        "jsx" => "jsx",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "sh" | "bash" | "zsh" => "sh",
        "ps1" | "psm1" => "powershell",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" => "markdown",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "sql" => "sql",
        "xml" => "xml",
        "zig" => "zig",
        "lua" => "lua",
        "dart" => "dart",
        _ => "",
    }
}

/// A table cell as one GFM cell: `|` escaped.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

fn table_row(cells: &[String]) -> String {
    format!(
        "| {} |",
        cells
            .iter()
            .map(|text| cell(text))
            .collect::<Vec<_>>()
            .join(" | ")
    )
}

/// Text with no blank line before or after it.
fn trimmed(text: &str) -> &str {
    text.trim_matches('\n').trim_end()
}

/// One block as Markdown (design section 4, The export, and its
/// "Optional titles and notes"): parts of one block are separated by one
/// blank line, and the block has no leading or trailing blank line.
pub fn block_markdown(body: &Body) -> String {
    let titled = |title: &Option<String>, rest: String| match title {
        Some(title) => format!("**{title}**\n\n{rest}"),
        None => rest,
    };
    match body {
        Body::Section { title, text } => format!("## {title}\n\n{}", trimmed(text)),
        Body::Callout { tone, title, text } => {
            let marker = match tone {
                Tone::Info => "NOTE",
                Tone::Decision => "IMPORTANT",
                Tone::Risk => "WARNING",
            };
            let mut lines = vec![format!("> [!{marker}]")];
            if let Some(title) = title {
                lines.push(format!("> **{title}**"));
                lines.push(">".to_owned());
            }
            lines.extend(trimmed(text).lines().map(|line| {
                if line.is_empty() {
                    ">".to_owned()
                } else {
                    format!("> {line}")
                }
            }));
            lines.join("\n")
        }
        Body::Table {
            title,
            columns,
            rows,
        } => {
            let mut lines = vec![
                table_row(columns),
                format!("|{}", " --- |".repeat(columns.len())),
            ];
            lines.extend(rows.iter().map(|row| table_row(row)));
            titled(title, lines.join("\n"))
        }
        Body::Code {
            title,
            language,
            code,
        } => titled(
            title,
            fenced(language.as_deref().unwrap_or_default(), trimmed(code)),
        ),
        Body::Quote {
            path,
            start,
            end,
            lines,
            note,
        } => {
            let mut parts = vec![
                inline_code(&format!("{path}:{start}-{end}")),
                fenced(language_of(path), &lines.join("\n")),
            ];
            if let Some(note) = note {
                parts.push(trimmed(note).to_owned());
            }
            parts.join("\n\n")
        }
        Body::Diagram { title, mermaid } => titled(title, fenced("mermaid", trimmed(mermaid))),
        Body::Divider {} => "---".to_owned(),
    }
}

/// The whole board as Markdown: its title as `# <title>`, then each block,
/// one blank line between them, ending with a line break. What a kept file
/// holds and Hide AI reads.
pub fn markdown(board: &Board) -> String {
    let mut parts = vec![format!("# {}", board.title)];
    parts.extend(board.blocks.iter().map(|block| block_markdown(&block.body)));
    let mut text = parts.join("\n\n");
    text.push('\n');
    text
}

/// The refusal's next step, which a refusal gives an agent as a command it
/// can run, as `plans::next_action` does.
pub fn next_action(code: &str) -> &'static str {
    match code {
        "board_invalid" => {
            "Fix the block where the reason says (hide board help lists each block's fields and limits), then run the command again"
        }
        "board_too_large" => {
            "A board holds at most 60 blocks and 64 KiB, and an edit at most 20 operations; shorten the blocks or split the board, then run the command again"
        }
        "board_limit" => {
            "This session keeps at most 16 open boards and Hide 256 in all; delete one you no longer need (hide board delete <id>), then run the command again"
        }
        "board_unknown" => {
            "No board has that id; run hide board list for your boards' ids, then run the command again with one of them"
        }
        "board_not_yours" => {
            "A board is changed only by the session that drew it; draw your own with hide board new, or send its agent a letter (hide request send <agent> ...)"
        }
        "board_deleted" => {
            "That board was deleted and cannot change; draw a new one with hide board new if it is still needed"
        }
        "board_kept" => {
            "The operator kept that board as a file; edit that file instead, as the reason names it"
        }
        "board_moved" => {
            "The board changed since that revision; read it with hide board show <id> --json, then run the command again with its revision"
        }
        "board_store_unavailable" => {
            "Hide cannot read or write its board record right now; go on with the work and run the same command again in a minute"
        }
        "board_session_unknown" => {
            "Hide does not know this pane's agent session yet; run the same command again after your next message"
        }
        "view_limit" => {
            "The operator's Workspace holds as many Views as it can; the board is kept, so run hide board open <id> once a View is closed"
        }
        "views_unavailable" => {
            "No Hide window shows this checkout's Views; the board is kept, so run hide board open <id> once one does"
        }
        "agent_pane_required" => "Run hide board from an agent's own Hide pane",
        "caller_context_changed" => {
            "This pane moved to another checkout; run hide workspace info, then run the command again"
        }
        "quote_refused" => {
            "Quote a file inside your checkout, by its path relative to the checkout's root, that is not a credential file (.env, keys, secrets) or in .git"
        }
        "quote_unreadable" => {
            "That file or line range cannot be read; check the path (relative to the checkout's root) and start and end, then run the command again"
        }
        _ => "Check the hide board command (hide board help) and run it again",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn owner() -> SessionIdentity {
        SessionIdentity {
            device_id: "mac".into(),
            agent: "claude".into(),
            session_id: "s1".into(),
        }
    }

    fn checkout() -> BoardCheckout {
        BoardCheckout {
            device_id: "mac".into(),
            path: "/work/app".into(),
        }
    }

    fn board(blocks: Value) -> Board {
        apply_new(
            "board-1".into(),
            owner(),
            checkout(),
            "Login flow",
            blocks.as_array().unwrap(),
            10,
        )
        .unwrap()
    }

    fn code(refusal: &str) -> (&str, Option<&str>) {
        crate::coordination::split_refusal(refusal)
    }

    fn md(block: Value) -> String {
        block_markdown(&parse_body("blocks[0]", &block).unwrap())
    }

    #[test]
    fn a_new_board_numbers_its_blocks_from_b1_at_revision_one() {
        let board = board(json!([
            {"type": "section", "title": "Why", "text": "Because."},
            {"type": "divider"},
        ]));
        assert_eq!(board.revision, 1);
        assert_eq!(board.next_block, 3);
        assert_eq!(
            board
                .blocks
                .iter()
                .map(|block| (block.id.as_str(), block.changed_at))
                .collect::<Vec<_>>(),
            [("b1", 1), ("b2", 1)]
        );
        let saved = encode(&board).unwrap();
        let read: Board = serde_json::from_slice(&saved).unwrap();
        assert_eq!(read, board, "the record reads back as it was written");
        let wire: Value = serde_json::from_slice(&saved).unwrap();
        assert_eq!(
            (&wire["state"], &wire["blocks"][0]["type"]),
            (&json!("open"), &json!("section"))
        );
    }

    #[test]
    fn a_refusal_names_the_field_that_broke_the_rule() {
        let refusal = apply_new(
            "board-1".into(),
            owner(),
            checkout(),
            "T",
            &[
                json!({"type": "divider"}),
                json!({"type": "divider"}),
                json!({"type": "table", "columns": ["a", "b"], "rows": [["1", "2"], ["1", "2"], ["1", "2"], ["1", "2", "3"]]}),
            ],
            1,
        )
        .unwrap_err();
        assert_eq!(
            code(&refusal),
            (
                "board_invalid",
                Some("blocks[2].rows[3]: 3 cells, the table has 2 columns")
            )
        );
        let unknown = apply_new(
            "board-1".into(),
            owner(),
            checkout(),
            "T",
            &[json!({"type": "section", "title": "A", "text": "x", "titel": "typo"})],
            1,
        )
        .unwrap_err();
        let (kind, detail) = code(&unknown);
        assert_eq!(kind, "board_invalid");
        assert!(
            detail
                .unwrap()
                .starts_with("blocks[0]: unknown field `titel`"),
            "{unknown}"
        );
        let empty = apply_new("board-1".into(), owner(), checkout(), " ", &[], 1).unwrap_err();
        assert_eq!(code(&empty), ("board_invalid", Some("title: is empty")));
    }

    #[test]
    fn the_limits_are_refused_as_too_large() {
        let many = vec![json!({"type": "divider"}); BLOCK_LIMIT + 1];
        let refusal = apply_new("board-1".into(), owner(), checkout(), "T", &many, 1).unwrap_err();
        assert_eq!(
            code(&refusal),
            (
                "board_too_large",
                Some("blocks: 61 blocks, a board holds at most 60")
            )
        );
        let big = "x".repeat(SECTION_TEXT_CHARS);
        let heavy = vec![json!({"type": "section", "title": "T", "text": big}); 17];
        let refusal = apply_new("board-1".into(), owner(), checkout(), "T", &heavy, 1).unwrap_err();
        let (kind, detail) = code(&refusal);
        assert_eq!(kind, "board_too_large");
        assert!(detail.unwrap().starts_with("board: "), "{refusal}");
        let edits = vec![json!({"op": "remove", "id": "b1"}); EDIT_LIMIT + 1];
        let refusal = apply_edits(&board(json!([])), &edits, 2).unwrap_err();
        assert_eq!(
            code(&refusal),
            (
                "board_too_large",
                Some("edits: 21 edits, a command applies at most 20")
            )
        );
    }

    #[test]
    fn edits_keep_every_other_block_id_and_never_reuse_one() {
        let start = board(json!([
            {"type": "section", "title": "One", "text": "first"},
            {"type": "section", "title": "Two", "text": "second"},
        ]));
        let next = apply_edits(
            &start,
            &[
                json!({"op": "remove", "id": "b1"}),
                json!({"op": "insert", "block": {"type": "divider"}, "after": "b2"}),
                json!({"op": "update", "id": "b2", "changes": {"text": "second, again"}}),
                json!({"op": "move", "id": "b3"}),
            ],
            20,
        )
        .unwrap()
        .unwrap();
        assert_eq!(next.revision, 2);
        assert_eq!(
            next.blocks
                .iter()
                .map(|block| (block.id.as_str(), block.changed_at))
                .collect::<Vec<_>>(),
            [("b2", 2), ("b3", 2)]
        );
        // A removed block's id is never given again.
        let again = apply_edits(
            &next,
            &[json!({"op": "insert", "block": {"type": "divider"}})],
            30,
        )
        .unwrap()
        .unwrap();
        assert_eq!(again.blocks.last().unwrap().id, "b4");
        // The same text again changes nothing and moves no revision.
        assert_eq!(
            apply_edits(
                &again,
                &[json!({"op": "update", "id": "b2", "changes": {"text": "second, again"}})],
                40
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn a_failing_operation_leaves_the_board_as_it_was() {
        let start = board(json!([{"type": "section", "title": "One", "text": "first"}]));
        let refusal = apply_edits(
            &start,
            &[
                json!({"op": "remove", "id": "b1"}),
                json!({"op": "update", "id": "b9", "changes": {"text": "x"}}),
            ],
            2,
        )
        .unwrap_err();
        assert_eq!(
            code(&refusal),
            (
                "board_invalid",
                Some("edits[1].id: no block b9 on this board")
            )
        );
        let refusal = apply_edits(
            &start,
            &[json!({"op": "update", "id": "b1", "changes": {"type": "callout"}})],
            2,
        )
        .unwrap_err();
        assert_eq!(code(&refusal).0, "board_invalid");
        assert!(
            code(&refusal)
                .1
                .unwrap()
                .starts_with("edits[0].changes.type")
        );
    }

    #[test]
    fn set_keeps_the_ids_it_names_and_gives_the_rest_new_ones() {
        let start = board(json!([
            {"type": "section", "title": "One", "text": "first"},
            {"type": "section", "title": "Two", "text": "second"},
        ]));
        let next = apply_set(
            &start,
            Some("Login flow, again"),
            &[
                json!({"id": "b2", "type": "section", "title": "Two", "text": "second"}),
                json!({"type": "divider"}),
            ],
            5,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            next.blocks
                .iter()
                .map(|block| (block.id.as_str(), block.changed_at))
                .collect::<Vec<_>>(),
            [("b2", 1), ("b3", 2)]
        );
        assert_eq!(
            (next.revision, next.title.as_str()),
            (2, "Login flow, again")
        );
        let refusal =
            apply_set(&start, None, &[json!({"id": "b7", "type": "divider"})], 5).unwrap_err();
        assert_eq!(
            code(&refusal),
            (
                "board_invalid",
                Some("blocks[0].id: no block b7 on this board")
            )
        );
    }

    #[test]
    fn a_board_that_cannot_change_says_why() {
        let start = board(json!([]));
        assert_eq!(
            code(&check_changeable(&start, Some(4)).unwrap_err()),
            ("board_moved", Some("the board is at revision 1, not 4"))
        );
        assert!(check_changeable(&start, Some(1)).is_ok());
        let gone = deleted(&start, Remover::Agent, 9);
        assert_eq!(check_changeable(&gone, None).unwrap_err(), "board_deleted");
        assert!(gone.blocks.is_empty());
    }

    #[test]
    fn a_quote_of_a_credential_or_outside_path_is_refused() {
        for path in [
            ".env",
            "config/.env.local",
            "my-secrets/a.txt",
            ".git/config",
            "keys/server.pem",
            "/etc/passwd",
            "../outside.rs",
            "src/../../x.rs",
            "C:\\x.rs",
        ] {
            assert!(quote_path_refused(path).is_some(), "{path}");
        }
        assert_eq!(quote_path_refused("src/main.rs"), None);
        let refusal = parse_body(
            "blocks[0]",
            &json!({"type": "quote", "path": ".env", "start": 1, "end": 1, "lines": ["A=1"]}),
        )
        .unwrap_err();
        assert_eq!(refusal.split('\u{1f}').next(), Some("quote_refused"));
        let missing = parse_body(
            "blocks[0]",
            &json!({"type": "quote", "path": "src/a.rs", "start": 1, "end": 2}),
        )
        .unwrap_err();
        assert!(code(&missing).1.unwrap().starts_with("blocks[0].lines"));
    }

    #[test]
    fn a_section_exports_its_title_then_its_text() {
        assert_eq!(
            md(json!({"type": "section", "title": "Why", "text": "\nOne.\n\nTwo.\n"})),
            "## Why\n\nOne.\n\nTwo."
        );
    }

    #[test]
    fn a_callout_exports_as_a_github_alert_with_its_title_first() {
        // An empty line of the text is `>` alone, with no trailing space.
        assert_eq!(
            md(
                json!({"type": "callout", "tone": "decision", "title": "Store", "text": "Use a folder.\n\nNot SQLite."})
            ),
            "> [!IMPORTANT]\n> **Store**\n>\n> Use a folder.\n>\n> Not SQLite."
        );
        assert_eq!(
            md(json!({"type": "callout", "tone": "info", "text": "Plain."})),
            "> [!NOTE]\n> Plain."
        );
        assert_eq!(
            md(json!({"type": "callout", "tone": "risk", "text": "Careful."})),
            "> [!WARNING]\n> Careful."
        );
    }

    #[test]
    fn a_table_exports_as_gfm_with_its_title_as_a_paragraph() {
        assert_eq!(
            md(
                json!({"type": "table", "title": "Limits", "columns": ["Limit", "Value"], "rows": [["a|b", "60"]]})
            ),
            "**Limits**\n\n| Limit | Value |\n| --- | --- |\n| a\\|b | 60 |"
        );
        assert_eq!(
            md(json!({"type": "table", "columns": ["Only"]})),
            "| Only |\n| --- |"
        );
        let refusal = parse_body(
            "blocks[0]",
            &json!({"type": "table", "columns": ["a"], "rows": [["two\nlines"]]}),
        )
        .unwrap_err();
        assert!(
            code(&refusal)
                .1
                .unwrap()
                .starts_with("blocks[0].rows[0][0]")
        );
    }

    #[test]
    fn code_exports_in_a_fence_longer_than_any_backticks_inside() {
        assert_eq!(
            md(json!({"type": "code", "language": "rust", "code": "fn main() {}\n"})),
            "```rust\nfn main() {}\n```"
        );
        assert_eq!(
            md(json!({"type": "code", "title": "Example", "code": "```inner```"})),
            "**Example**\n\n````\n```inner```\n````"
        );
    }

    #[test]
    fn a_quote_exports_its_place_its_lines_in_their_language_and_its_note() {
        assert_eq!(
            md(
                json!({"type": "quote", "path": "src/main.rs", "start": 3, "end": 4,
                      "lines": ["let a = 1;", "let b = 2;"], "note": "Both start at zero."})
            ),
            "`src/main.rs:3-4`\n\n```rust\nlet a = 1;\nlet b = 2;\n```\n\nBoth start at zero."
        );
        assert_eq!(
            md(
                json!({"type": "quote", "path": "Makefile", "start": 1, "end": 1, "lines": ["all:"]})
            ),
            "`Makefile:1-1`\n\n```\nall:\n```"
        );
    }

    #[test]
    fn a_diagram_exports_as_a_mermaid_fence() {
        assert_eq!(
            md(json!({"type": "diagram", "title": "Flow", "mermaid": "graph TD\n  A --> B\n"})),
            "**Flow**\n\n```mermaid\ngraph TD\n  A --> B\n```"
        );
        assert_eq!(
            md(json!({"type": "diagram", "mermaid": "graph TD"})),
            "```mermaid\ngraph TD\n```"
        );
    }

    #[test]
    fn a_divider_exports_as_a_rule_and_a_board_joins_its_blocks() {
        assert_eq!(md(json!({"type": "divider"})), "---");
        let board = board(json!([
            {"type": "section", "title": "One", "text": "first"},
            {"type": "divider"},
        ]));
        assert_eq!(markdown(&board), "# Login flow\n\n## One\n\nfirst\n\n---\n");
    }

    #[test]
    fn every_refusal_code_has_its_own_next_action() {
        let fallback = next_action("no_such_code");
        for refusal in DETAILED.iter().chain(&[
            "board_unknown",
            "board_not_yours",
            "board_deleted",
            "board_store_unavailable",
            "board_session_unknown",
            "quote_refused",
            "quote_unreadable",
        ]) {
            assert_ne!(next_action(refusal), fallback, "{refusal}");
        }
    }
}
