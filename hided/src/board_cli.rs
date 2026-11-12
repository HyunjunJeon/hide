//! `hide board …`: an agent's boards, drawn for the operator and kept by the
//! core for the agent's native session, answered without a shell through
//! the pane capability a plan uses, on this machine or a connected device's.
//!
//! The command reads what the core never reads: the blocks file, and each
//! quoted line, from the file of the checkout the quote names, on the
//! machine that holds it. A quote that leaves the checkout or names a
//! credential is refused here, before anything is sent.

use std::path::Path;

use crate::env::Env;
use hide_core::boards::{
    BoardCommand, QUOTE_FILE_BYTES, QUOTE_LINES, next_action, quote_path_refused,
};
use serde_json::{Value, json};

/// The guide `hide board help` prints.
pub const HELP: &str = include_str!("../assets/board/help.md");

pub const USAGE: &str = "hide board help\nhide board new --title <title> (--file <board.json> | --json <blocks>)\nhide board set <board> (--file <board.json> | --json <blocks>) [--title <title>] [--revision <n>]\nhide board edit <board> (--file <edits.json> | --json <edits>) [--revision <n>]\nhide board show [<board>] [--json]\nhide board list\nhide board delete <board>";

/// The largest blocks or edits file read from disk.
const FILE_LIMIT: u64 = 256 * 1024;
/// The largest command sent: a scoped request is one frame of at most
/// 128 KiB, and the request's own fields take the rest.
const REQUEST_LIMIT: usize = 120 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoardCli {
    Help,
    Command(BoardCommand),
    /// A command whose blocks or edits are still to be read, at run time,
    /// so a file that is not one answers on the command's JSON line.
    Draw {
        verb: Verb,
        source: Source,
        title: Option<String>,
        revision: Option<u64>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Verb {
    New,
    Set(String),
    Edit(String),
}

/// Where the blocks or the edits come from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Source {
    File(String),
    Json(String),
}

/// A flag's value: not empty, not another flag, and one line, except a
/// `--json` document, which may be written on several lines.
fn value<'a>(args: &'a [String], at: usize, flag: &str) -> Result<&'a str, String> {
    let value = args
        .get(at + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("{flag} needs a value"))?;
    let layout = |c: char| flag == "--json" && matches!(c, '\n' | '\r' | '\t');
    if value.starts_with("--")
        || value.trim().is_empty()
        || value.chars().any(|c| c.is_control() && !layout(c))
    {
        return Err(format!("{flag} needs a value"));
    }
    Ok(value)
}

/// A verb's positionals, valued options and switches.
struct Flags {
    positional: Vec<String>,
    values: std::collections::BTreeMap<String, String>,
    switches: Vec<String>,
}

fn flags(args: &[String], valued: &[&str], switches: &[&str]) -> Result<Flags, String> {
    let mut flags = Flags {
        positional: Vec::new(),
        values: Default::default(),
        switches: Vec::new(),
    };
    let mut at = 0;
    while at < args.len() {
        let arg = args[at].as_str();
        if valued.contains(&arg) {
            let text = value(args, at, arg)?;
            if flags
                .values
                .insert(arg.to_owned(), text.to_owned())
                .is_some()
            {
                return Err(format!("{arg} is given twice"));
            }
            at += 2;
        } else if switches.contains(&arg) {
            if flags.switches.iter().any(|given| given == arg) {
                return Err(format!("{arg} is given twice"));
            }
            flags.switches.push(arg.to_owned());
            at += 1;
        } else if arg.starts_with('-') {
            return Err(format!("unknown flag for hide board: {arg}"));
        } else {
            flags.positional.push(arg.to_owned());
            at += 1;
        }
    }
    Ok(flags)
}

/// A board id as an agent types it: a key, which the core answers
/// `board_unknown` for when it keeps no such board.
fn board(positional: &[String]) -> Result<String, String> {
    match positional {
        [board] if !board.is_empty() && board.len() <= 256 => Ok(board.clone()),
        _ => Err(USAGE.to_owned()),
    }
}

fn source(values: &std::collections::BTreeMap<String, String>) -> Result<Source, String> {
    match (values.get("--file"), values.get("--json")) {
        (Some(path), None) => Ok(Source::File(path.clone())),
        // Text that is not JSON is a usage error; JSON that is not blocks
        // says where at run time.
        (None, Some(text)) => {
            serde_json::from_str::<Value>(text)
                .map_err(|error| format!("--json is not JSON: {error}"))?;
            Ok(Source::Json(text.clone()))
        }
        _ => Err("give the blocks once, with --file or --json".into()),
    }
}

fn revision(values: &std::collections::BTreeMap<String, String>) -> Result<Option<u64>, String> {
    values
        .get("--revision")
        .map(|number| {
            number
                .parse::<u64>()
                .map_err(|_| format!("--revision takes a revision number, not {number}"))
        })
        .transpose()
}

/// Parses the words after `hide board`.
pub fn parse<'a>(args: impl Iterator<Item = &'a String>) -> Result<BoardCli, String> {
    let args: Vec<String> = args.cloned().collect();
    let (verb, rest) = args.split_first().ok_or(USAGE)?;
    let command = match verb.as_str() {
        "help" | "--help" => {
            return if rest.is_empty() {
                Ok(BoardCli::Help)
            } else {
                Err(USAGE.to_owned())
            };
        }
        "new" => {
            let Flags {
                positional, values, ..
            } = flags(rest, &["--title", "--file", "--json"], &[])?;
            if !positional.is_empty() {
                return Err(USAGE.to_owned());
            }
            let title = values
                .get("--title")
                .cloned()
                .ok_or("hide board new needs --title")?;
            return Ok(BoardCli::Draw {
                verb: Verb::New,
                source: source(&values)?,
                title: Some(title),
                revision: None,
            });
        }
        "set" | "edit" => {
            let valued: &[&str] = if verb == "set" {
                &["--file", "--json", "--title", "--revision"]
            } else {
                &["--file", "--json", "--revision"]
            };
            let Flags {
                positional, values, ..
            } = flags(rest, valued, &[])?;
            let id = board(&positional)?;
            return Ok(BoardCli::Draw {
                verb: if verb == "set" {
                    Verb::Set(id)
                } else {
                    Verb::Edit(id)
                },
                source: source(&values)?,
                title: values.get("--title").cloned(),
                revision: revision(&values)?,
            });
        }
        "show" => {
            let Flags {
                positional,
                switches,
                ..
            } = flags(rest, &[], &["--json"])?;
            let board = match positional.as_slice() {
                [] => None,
                some => Some(board(some)?),
            };
            BoardCommand::Show {
                board,
                json: !switches.is_empty(),
            }
        }
        "list" => {
            if !rest.is_empty() {
                return Err(USAGE.to_owned());
            }
            BoardCommand::List {}
        }
        "delete" => {
            let Flags { positional, .. } = flags(rest, &[], &[])?;
            BoardCommand::Delete {
                board: board(&positional)?,
            }
        }
        _ => return Err(USAGE.to_owned()),
    };
    if !command.valid() {
        return Err("a board id is invalid".to_owned());
    }
    Ok(BoardCli::Command(command))
}

fn read_file(path: &str) -> Result<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|error| format!("{path}: {error}"))?;
    let mut text = String::new();
    file.take(FILE_LIMIT + 1)
        .read_to_string(&mut text)
        .map_err(|error| format!("{path}: {error}"))?;
    if text.len() as u64 > FILE_LIMIT {
        return Err(format!("{path} is larger than {FILE_LIMIT} bytes"));
    }
    Ok(text)
}

/// The list `text` holds: a JSON array, or an object with the list under
/// `key` (`blocks` or `edits`).
fn list(text: &str, key: &str) -> Result<Vec<Value>, String> {
    match serde_json::from_str::<Value>(text).map_err(|error| format!("not JSON: {error}"))? {
        Value::Array(items) => Ok(items),
        Value::Object(mut fields) if fields.len() == 1 => match fields.remove(key) {
            Some(Value::Array(items)) => Ok(items),
            _ => Err(format!("{key}: a JSON array")),
        },
        _ => Err(format!("a JSON array of {key}, or {{\"{key}\": [...]}}")),
    }
}

/// A refusal this command answers itself: its reason and next action, with
/// the detail that names what to fix.
#[derive(Debug)]
struct Refused {
    code: &'static str,
    detail: String,
}

fn refused(code: &'static str, detail: impl Into<String>) -> Refused {
    Refused {
        code,
        detail: detail.into(),
    }
}

/// The quote blocks of `command` whose lines are still to be read, by
/// where they are in it.
fn quotes(command: &mut BoardCommand) -> Vec<&mut Value> {
    let blocks: Vec<&mut Value> = match command {
        BoardCommand::New { blocks, .. } | BoardCommand::Set { blocks, .. } => {
            blocks.iter_mut().collect()
        }
        BoardCommand::Edit { edits, .. } => edits
            .iter_mut()
            .filter_map(|edit| edit.get_mut("block"))
            .collect(),
        _ => Vec::new(),
    };
    blocks
        .into_iter()
        .filter(|block| block["type"] == "quote")
        .collect()
}

/// Refuses a quote whose path leaves the checkout or names a credential,
/// whether or not its lines are given, before anything is sent.
fn check_quote_paths(command: &mut BoardCommand) -> Result<(), Refused> {
    for quote in quotes(command) {
        if let Some(path) = quote["path"].as_str()
            && let Some(why) = quote_path_refused(path)
        {
            return Err(refused("quote_refused", format!("{path}: {why}")));
        }
    }
    Ok(())
}

/// Fills each quote that has no `lines` with lines `start` to `end` of its
/// file below `root`, the caller's checkout as this machine spells it. A
/// quote whose fields are not a range is sent as it is, and the core names
/// the field to fix.
fn fill_quotes(command: &mut BoardCommand, root: &Path) -> Result<(), Refused> {
    for quote in quotes(command) {
        if quote.get("lines").is_some() {
            continue;
        }
        let (Some(path), Some(start), Some(end)) = (
            quote["path"].as_str(),
            quote["start"].as_u64(),
            quote["end"].as_u64(),
        ) else {
            continue;
        };
        if start == 0 || end < start || end - start >= QUOTE_LINES as u64 {
            continue;
        }
        let lines = read_lines(root, path, start, end)?;
        quote["lines"] = json!(lines);
    }
    Ok(())
}

/// Lines `start..=end` of the file `path` names below `root`: refused when
/// the path, or where a link in it leads, is outside the checkout or a
/// credential; unreadable when the file or the range is not there.
fn read_lines(root: &Path, path: &str, start: u64, end: u64) -> Result<Vec<String>, Refused> {
    if let Some(why) = quote_path_refused(path) {
        return Err(refused("quote_refused", format!("{path}: {why}")));
    }
    let unreadable = |why: String| refused("quote_unreadable", format!("{path}: {why}"));
    let root = hide_platform::fs::identity::canonical(root)
        .map_err(|error| unreadable(format!("the checkout cannot be read: {error}")))?;
    let joined = path
        .split('/')
        .fold(root.clone(), |joined, name| joined.join(name));
    let target = hide_platform::fs::identity::canonical(&joined)
        .map_err(|error| unreadable(error.to_string()))?;
    // A link may lead out of the checkout, or to a credential inside it.
    let inside = hide_platform::path::relative(&root, &target).map_err(|_| {
        refused(
            "quote_refused",
            format!("{path}: leads outside the checkout"),
        )
    })?;
    if let Some(why) = quote_path_refused(inside.as_str()) {
        return Err(refused(
            "quote_refused",
            format!("{path}: leads to a file that {why}"),
        ));
    }
    let file = std::fs::File::open(&target).map_err(|error| unreadable(error.to_string()))?;
    if !file
        .metadata()
        .map_err(|error| unreadable(error.to_string()))?
        .is_file()
    {
        return Err(unreadable("is not a file".into()));
    }
    // Read to the cap and one byte past it, never the whole of a larger file.
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut std::io::Read::take(file, QUOTE_FILE_BYTES + 1),
        &mut bytes,
    )
    .map_err(|error| unreadable(error.to_string()))?;
    if bytes.len() as u64 > QUOTE_FILE_BYTES {
        return Err(unreadable(format!(
            "is larger than {QUOTE_FILE_BYTES} bytes"
        )));
    }
    let text = String::from_utf8(bytes).map_err(|_| unreadable("is not UTF-8 text".into()))?;
    let lines: Vec<&str> = text.lines().collect();
    if end as usize > lines.len() {
        return Err(unreadable(format!(
            "has {} lines, so lines {start} to {end} are not all there",
            lines.len()
        )));
    }
    Ok(lines[start as usize - 1..end as usize]
        .iter()
        .map(|line| (*line).to_owned())
        .collect())
}

/// The caller's checkout as this machine spells it, from the pane's own
/// context (the hook route needs no open shell, as `hide board` does not).
fn checkout_root(env: &Env) -> Result<std::path::PathBuf, String> {
    let answer = crate::workspace_cli::request_hook(
        env,
        hide_node_link::panes::HookCommand::WorkspaceInfo {},
    )?;
    if answer["ok"] != true {
        return Err(answer["reason"]
            .as_str()
            .unwrap_or("workspace_unavailable")
            .to_owned());
    }
    let wire = answer["result"]["context"]["checkout_path"]
        .as_str()
        .ok_or("workspace_unavailable")?;
    hide_platform::path::from_wire(wire).map_err(|_| "workspace_unavailable".to_owned())
}

fn refuse(reason: &str, next: &str) -> Result<(), String> {
    println!("{}", json!({"ok":false,"reason":reason,"next_action":next}));
    Err(reason.to_owned())
}

fn refuse_with(refusal: Refused) -> Result<(), String> {
    refuse(
        refusal.code,
        &format!("{} ({})", next_action(refusal.code), refusal.detail),
    )
}

/// The command a `Draw` makes once its blocks or edits are read.
fn drawn(
    verb: Verb,
    source: Source,
    title: Option<String>,
    revision: Option<u64>,
) -> Result<BoardCommand, String> {
    let text = match source {
        Source::File(path) => read_file(&path)?,
        Source::Json(text) => text,
    };
    Ok(match verb {
        Verb::New => BoardCommand::New {
            title: title.unwrap_or_default(),
            blocks: list(&text, "blocks")?,
        },
        Verb::Set(board) => BoardCommand::Set {
            board,
            blocks: list(&text, "blocks")?,
            title,
            revision,
        },
        Verb::Edit(board) => BoardCommand::Edit {
            board,
            edits: list(&text, "edits")?,
            revision,
        },
    })
}

/// Runs a parsed `hide board` command and prints its answer as one JSON
/// line: `{"ok":true,"result":…}`, or the refusal's reason and next action.
pub fn run(env: &Env, command: BoardCli) -> Result<(), String> {
    let mut command = match command {
        BoardCli::Help => {
            print!("{HELP}");
            return Ok(());
        }
        BoardCli::Command(command) => command,
        BoardCli::Draw {
            verb,
            source,
            title,
            revision,
        } => match drawn(verb, source, title, revision) {
            Ok(command) => command,
            Err(message) => {
                return refuse_with(refused("board_invalid", message));
            }
        },
    };
    if let Err(refusal) = check_quote_paths(&mut command) {
        return refuse_with(refusal);
    }
    let unread = quotes(&mut command)
        .iter()
        .any(|quote| quote.get("lines").is_none());
    if unread {
        let root = match checkout_root(env) {
            Ok(root) => root,
            Err(reason) => return refuse(&reason, crate::cli::request_next_action(&reason)),
        };
        if let Err(refusal) = fill_quotes(&mut command, &root) {
            return refuse_with(refusal);
        }
    }
    let size = serde_json::to_vec(&command).map_or(usize::MAX, |bytes| bytes.len());
    if size > REQUEST_LIMIT {
        return refuse_with(refused(
            "board_too_large",
            format!("the command is {size} bytes, at most {REQUEST_LIMIT}"),
        ));
    }
    let answer = (|| {
        let mut credential = crate::workspace_cli::Credential::acquire(env)?;
        crate::workspace_cli::request_board(&mut credential, &command)
    })();
    match answer {
        Ok(answer) if answer["ok"] == true => {
            println!("{}", json!({"ok":true,"result":answer["result"]}));
            Ok(())
        }
        Ok(answer) => {
            let reason = answer["reason"]
                .as_str()
                .unwrap_or("board_store_unavailable")
                .to_owned();
            let next = answer["next_action"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| next_action(&reason).to_owned());
            println!("{}", json!({"ok":false,"reason":reason,"next_action":next}));
            Err(reason)
        }
        Err(reason) => {
            let next = if reason == "request_timeout" {
                "The command may have been applied; run hide board list before running it again"
            } else if crate::cli::bootstrap_refused(&reason) {
                crate::cli::request_next_action(&reason)
            } else {
                next_action(&reason)
            };
            println!("{}", json!({"ok":false,"reason":reason,"next_action":next}));
            Err(reason)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &[&str]) -> Vec<String> {
        line.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn new_needs_a_title_and_the_blocks_once() {
        assert_eq!(
            parse(words(&["new", "--title", "Login", "--file", "b.json"]).iter()).unwrap(),
            BoardCli::Draw {
                verb: Verb::New,
                source: Source::File("b.json".into()),
                title: Some("Login".into()),
                revision: None,
            }
        );
        assert!(parse(words(&["new", "--file", "b.json"]).iter()).is_err());
        assert!(parse(words(&["new", "--title", "T"]).iter()).is_err());
        assert!(
            parse(words(&["new", "--title", "T", "--file", "b", "--json", "[]"]).iter()).is_err()
        );
        assert!(parse(words(&["new", "--title", "T", "--json", "[oops"]).iter()).is_err());
        // A JSON document may be written on several lines; a title may not.
        assert!(
            parse(
                words(&[
                    "new",
                    "--title",
                    "T",
                    "--json",
                    "[\n  {\"type\": \"divider\"}\n]"
                ])
                .iter()
            )
            .is_ok()
        );
        assert!(parse(words(&["new", "--title", "T\nU", "--json", "[]"]).iter()).is_err());
    }

    #[test]
    fn edit_and_set_name_one_board_and_may_name_a_revision() {
        assert_eq!(
            parse(words(&["edit", "board-3", "--json", "[]", "--revision", "4"]).iter()).unwrap(),
            BoardCli::Draw {
                verb: Verb::Edit("board-3".into()),
                source: Source::Json("[]".into()),
                title: None,
                revision: Some(4),
            }
        );
        assert!(parse(words(&["edit", "board-3", "--json", "[]", "--title", "T"]).iter()).is_err());
        assert!(parse(words(&["set", "--json", "[]"]).iter()).is_err());
        assert!(
            parse(words(&["set", "board-1", "--json", "[]", "--revision", "x"]).iter()).is_err()
        );
    }

    #[test]
    fn show_list_and_delete_read_their_words() {
        assert_eq!(
            parse(words(&["show", "board-2", "--json"]).iter()).unwrap(),
            BoardCli::Command(BoardCommand::Show {
                board: Some("board-2".into()),
                json: true,
            })
        );
        assert_eq!(
            parse(words(&["show"]).iter()).unwrap(),
            BoardCli::Command(BoardCommand::Show {
                board: None,
                json: false,
            })
        );
        assert_eq!(
            parse(words(&["list"]).iter()).unwrap(),
            BoardCli::Command(BoardCommand::List {})
        );
        assert!(parse(words(&["delete"]).iter()).is_err());
        assert!(parse(words(&["list", "x"]).iter()).is_err());
    }

    #[test]
    fn a_file_holds_the_list_bare_or_under_its_name() {
        assert_eq!(list("[1]", "blocks").unwrap(), vec![json!(1)]);
        assert_eq!(list(r#"{"edits":[2]}"#, "edits").unwrap(), vec![json!(2)]);
        assert!(list(r#"{"nodes":[]}"#, "blocks").is_err());
    }

    fn quote(path: &str, start: u64, end: u64) -> BoardCommand {
        BoardCommand::New {
            title: "T".into(),
            blocks: vec![json!({"type": "quote", "path": path, "start": start, "end": end})],
        }
    }

    fn lines_of(command: &BoardCommand) -> Value {
        match command {
            BoardCommand::New { blocks, .. } => blocks[0]["lines"].clone(),
            _ => Value::Null,
        }
    }

    #[test]
    fn a_quote_is_read_from_the_checkout_and_kept_inside_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("checkout");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/login.rs"), "one\r\ntwo\nthree\n").unwrap();
        std::fs::write(root.join(".env"), "TOKEN=secret\n").unwrap();
        std::fs::write(dir.path().join("outside.rs"), "far\n").unwrap();

        let mut read = quote("src/login.rs", 2, 3);
        fill_quotes(&mut read, &root).unwrap();
        assert_eq!(lines_of(&read), json!(["two", "three"]));

        let mut env = quote(".env", 1, 1);
        assert_eq!(
            check_quote_paths(&mut env).unwrap_err().code,
            "quote_refused"
        );
        // Refused by its name before anything is read: a credential file that
        // is not there is refused, not unreadable.
        let mut absent = quote("keys/deploy.pem", 1, 1);
        let refusal = fill_quotes(&mut absent, &root).unwrap_err();
        assert_eq!(
            (refusal.code, refusal.detail.as_str()),
            (
                "quote_refused",
                "keys/deploy.pem: names a credential file, a secrets folder or .git"
            )
        );
        assert_eq!(
            fill_quotes(&mut env, &root).unwrap_err().code,
            "quote_refused"
        );
        let mut up = quote("../outside.rs", 1, 1);
        assert_eq!(
            fill_quotes(&mut up, &root).unwrap_err().code,
            "quote_refused"
        );
        let mut past = quote("src/login.rs", 3, 5);
        assert_eq!(
            fill_quotes(&mut past, &root).unwrap_err().code,
            "quote_unreadable"
        );
        let mut missing = quote("src/none.rs", 1, 1);
        assert_eq!(
            fill_quotes(&mut missing, &root).unwrap_err().code,
            "quote_unreadable"
        );
    }

    /// A link at `at` to the file `target`; false where the account may not
    /// make one (Windows without Developer Mode or the privilege).
    fn link(target: &Path, at: &Path) -> bool {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, at).expect("the link is made");
            true
        }
        // Windows refuses a link to an account without the privilege
        // (error 1314), which a runner may or may not have.
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(target, at).is_ok()
        }
    }

    #[test]
    fn a_link_that_leads_out_or_to_a_credential_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("checkout");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(dir.path().join("outside.rs"), "far\n").unwrap();
        std::fs::write(root.join(".env"), "TOKEN=secret\n").unwrap();
        if !link(&dir.path().join("outside.rs"), &root.join("out.rs"))
            || !link(&root.join(".env"), &root.join("config.txt"))
        {
            eprintln!("skipped: this account may not make links");
            return;
        }
        let mut out = quote("out.rs", 1, 1);
        assert_eq!(
            fill_quotes(&mut out, &root).unwrap_err().code,
            "quote_refused"
        );
        let mut secret = quote("config.txt", 1, 1);
        assert_eq!(
            fill_quotes(&mut secret, &root).unwrap_err().code,
            "quote_refused"
        );
    }
}
