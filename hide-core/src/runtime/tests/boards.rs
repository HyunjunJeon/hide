//! A session's boards (`hide board`): the commands from the agent's pane,
//! who may change a board, the limits and refusals with the field they name,
//! the read back from the board folder after the runtime is made again, the
//! boards going with their session, and the two stores boards never touch.

use super::*;
use crate::boards::BoardCommand;
use crate::delivery::Actor;
use crate::runtime::delivery::tests::{authority, fixture};
use serde_json::{Value, json};

fn recipient() -> Actor {
    Actor {
        pane_id: "recipient".into(),
        name: "recipient".into(),
        kind: "codex".into(),
        device_id: crate::node::TEST_NODE.into(),
        session: Some("recipient-session".into()),
    }
}

fn run(runtime: &mut Runtime, actor: &Actor, command: Value) -> Result<(Value, bool), String> {
    let command: BoardCommand = serde_json::from_value(command).unwrap();
    runtime.board_command(
        crate::node::TEST_NODE,
        &actor.pane_id,
        &authority(actor).context,
        command,
    )
}

fn ok(runtime: &mut Runtime, actor: &Actor, command: Value) -> Value {
    run(runtime, actor, command).unwrap().0
}

/// The code a refusal answers and the detail the daemon adds to its next
/// action.
fn code(refusal: &str) -> (&str, Option<&str>) {
    crate::coordination::split_refusal(refusal)
}

/// A board of four kinds of block, as an agent draws it.
fn login_flow() -> Value {
    json!({"verb": "new", "title": "Login flow", "blocks": [
        {"type": "section", "title": "What changes", "text": "The first text."},
        {"type": "table", "columns": ["Step", "Owner"], "rows": [["Sign in", "web"], ["Token", "core"]]},
        {"type": "quote", "path": "src/login.rs", "start": 3, "end": 4, "lines": ["fn a() {}", "fn b() {}"]},
        {"type": "diagram", "mermaid": "graph TD\n  A --> B"},
    ]})
}

fn ids(answer: &Value) -> Vec<String> {
    answer["board"]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|block| block["id"].as_str().unwrap().to_owned())
        .collect()
}

fn boards_dir(root: &std::path::Path) -> std::path::PathBuf {
    hide_kit::layout::boards_dir(&root.join("state"))
}

/// Waits until board `id`'s file holds `revision`.
fn saved(root: &std::path::Path, id: &str, revision: u64) {
    let file = hide_kit::layout::board_file(&boards_dir(root), id);
    wait_for(&format!("{id} saved at revision {revision}"), || {
        std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|board| board["revision"] == revision)
    });
}

#[test]
fn new_set_and_edit_make_a_record_whose_block_ids_stay() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _, _) = fixture(root.path());
    let mut runtime = runtime.lock().unwrap();

    let (drawn, changed) = run(&mut runtime, &sender, login_flow()).unwrap();
    assert!(changed);
    assert_eq!(
        (
            &drawn["board"]["id"],
            &drawn["board"]["revision"],
            &drawn["board"]["state"]
        ),
        (&json!("board-1"), &json!(1), &json!("open"))
    );
    assert_eq!(ids(&drawn), ["b1", "b2", "b3", "b4"]);
    assert_eq!(drawn["board"]["blocks"][2]["type"], "quote");
    assert!(drawn.get("view").is_none(), "no View is opened yet");

    // An edit changes one block in place; the others keep their ids.
    let (edited, changed) = run(
        &mut runtime,
        &sender,
        json!({"verb": "edit", "board": "board-1", "edits": [
            {"op": "update", "id": "b1", "changes": {"text": "Second text"}},
            {"op": "remove", "id": "b4"},
            {"op": "insert", "block": {"type": "callout", "tone": "decision", "text": "Keep the token in the core."}, "after": "b1"},
        ]}),
    )
    .unwrap();
    assert!(changed);
    assert_eq!(edited["board"]["revision"], 2);
    assert_eq!(ids(&edited), ["b1", "b5", "b2", "b3"]);

    // A set that names a block keeps its id; a new block never takes a
    // removed one's.
    let (set, _) = run(
        &mut runtime,
        &sender,
        json!({"verb": "set", "board": "board-1", "title": "Login flow, again", "blocks": [
            {"id": "b2", "type": "table", "columns": ["Step", "Owner"], "rows": [["Sign in", "web"], ["Token", "core"]]},
            {"type": "divider"},
        ]}),
    )
    .unwrap();
    assert_eq!(ids(&set), ["b2", "b6"]);
    assert_eq!(
        (&set["board"]["title"], &set["board"]["revision"]),
        (&json!("Login flow, again"), &json!(3))
    );

    let record = ok(
        &mut runtime,
        &sender,
        json!({"verb": "show", "board": "board-1", "json": true}),
    );
    let blocks = record["board"]["blocks"].as_array().unwrap();
    // The table kept the revision it last changed at.
    assert_eq!(
        (
            &blocks[0]["id"],
            &blocks[0]["changed_at"],
            &blocks[1]["changed_at"]
        ),
        (&json!("b2"), &json!(1), &json!(3))
    );
    assert_eq!(record["board"]["owner"]["session_id"], "sender-session");
    assert_eq!(record["board"]["checkout"]["path"], "/checkouts/fixture");

    // The same edit again changes nothing and moves no revision.
    let (_, changed) = run(
        &mut runtime,
        &sender,
        json!({"verb": "edit", "board": "board-1", "edits": [{"op": "update", "id": "b6", "changes": {}}]}),
    )
    .unwrap();
    assert!(!changed);
}

#[test]
fn show_prints_the_board_as_its_markdown() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _, _) = fixture(root.path());
    let mut runtime = runtime.lock().unwrap();
    ok(&mut runtime, &sender, login_flow());
    let shown = ok(
        &mut runtime,
        &sender,
        json!({"verb": "show", "board": "board-1"}),
    );
    let markdown = shown["markdown"].as_str().unwrap();
    assert!(
        markdown
            .starts_with("# Login flow\n\n## What changes\n\nThe first text.\n\n| Step | Owner |")
    );
    assert!(markdown.contains("`src/login.rs:3-4`\n\n```rust\nfn a() {}\nfn b() {}\n```"));
    assert!(markdown.ends_with("```mermaid\ngraph TD\n  A --> B\n```\n"));
    // Without an id, every board of the caller's session.
    let mine = ok(&mut runtime, &sender, json!({"verb": "show"}));
    assert_eq!(mine["boards"][0]["markdown"], shown["markdown"]);
}

#[test]
fn a_bad_block_or_a_passed_limit_is_refused_with_the_field_named() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _, _) = fixture(root.path());
    let mut runtime = runtime.lock().unwrap();

    let refusal = run(
        &mut runtime,
        &sender,
        json!({"verb": "new", "title": "T", "blocks": [
            {"type": "divider"},
            {"type": "table", "columns": ["a", "b"], "rows": [["1", "2", "3"]]},
        ]}),
    )
    .unwrap_err();
    assert_eq!(
        code(&refusal),
        (
            "board_invalid",
            Some("blocks[1].rows[0]: 3 cells, the table has 2 columns")
        )
    );
    let refusal = run(
        &mut runtime,
        &sender,
        json!({"verb": "new", "title": "T", "blocks": [{"type": "chart"}]}),
    )
    .unwrap_err();
    assert_eq!(code(&refusal).0, "board_invalid");
    assert!(
        code(&refusal)
            .1
            .unwrap()
            .starts_with("blocks[0]: unknown variant `chart`")
    );

    let many = vec![json!({"type": "divider"}); crate::boards::BLOCK_LIMIT + 1];
    let refusal = run(
        &mut runtime,
        &sender,
        json!({"verb": "new", "title": "T", "blocks": many}),
    )
    .unwrap_err();
    assert_eq!(
        code(&refusal),
        (
            "board_too_large",
            Some("blocks: 61 blocks, a board holds at most 60")
        )
    );

    // A quote of a credential file is refused even with its lines given.
    let refusal = run(
        &mut runtime,
        &sender,
        json!({"verb": "new", "title": "T", "blocks": [
            {"type": "quote", "path": ".env", "start": 1, "end": 1, "lines": ["TOKEN=x"]},
        ]}),
    )
    .unwrap_err();
    assert_eq!(code(&refusal).0, "quote_refused");

    // Nothing refused was kept: the first board drawn is board-1.
    for _ in 0..crate::boards::OPEN_PER_SESSION {
        ok(
            &mut runtime,
            &sender,
            json!({"verb": "new", "title": "T", "blocks": []}),
        );
    }
    let list = ok(&mut runtime, &sender, json!({"verb": "list"}));
    assert_eq!(list["boards"][0]["id"], "board-1");
    let refusal = run(
        &mut runtime,
        &sender,
        json!({"verb": "new", "title": "T", "blocks": []}),
    )
    .unwrap_err();
    assert_eq!(
        code(&refusal),
        (
            "board_limit",
            Some("this session has 16 open boards, at most 16")
        )
    );
}

#[test]
fn only_the_session_that_drew_a_board_changes_it() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _, _) = fixture(root.path());
    let mut runtime = runtime.lock().unwrap();
    ok(&mut runtime, &sender, login_flow());
    let other = recipient();

    for command in [
        json!({"verb": "edit", "board": "board-1", "edits": [{"op": "remove", "id": "b1"}]}),
        json!({"verb": "set", "board": "board-1", "blocks": []}),
        json!({"verb": "delete", "board": "board-1"}),
    ] {
        assert_eq!(
            run(&mut runtime, &other, command.clone()).unwrap_err(),
            "board_not_yours",
            "{command}"
        );
    }
    // An agent in the board's checkout reads it, and lists only its own.
    let shown = ok(
        &mut runtime,
        &other,
        json!({"verb": "show", "board": "board-1"}),
    );
    assert_eq!(shown["board"]["revision"], 1);
    assert_eq!(
        ok(&mut runtime, &other, json!({"verb": "list"}))["boards"],
        json!([])
    );
    assert_eq!(
        run(
            &mut runtime,
            &other,
            json!({"verb": "show", "board": "board-9"})
        )
        .unwrap_err(),
        "board_unknown"
    );
}

#[test]
fn an_edit_at_a_revision_the_board_moved_past_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _, _) = fixture(root.path());
    let mut runtime = runtime.lock().unwrap();
    ok(&mut runtime, &sender, login_flow());
    let edit = |revision: u64| {
        json!({"verb": "edit", "board": "board-1", "revision": revision,
               "edits": [{"op": "update", "id": "b1", "changes": {"text": format!("at {revision}")}}]})
    };
    assert_eq!(ok(&mut runtime, &sender, edit(1))["board"]["revision"], 2);
    let refusal = run(&mut runtime, &sender, edit(1)).unwrap_err();
    assert_eq!(
        code(&refusal),
        ("board_moved", Some("the board is at revision 2, not 1"))
    );
    let refusal = run(
        &mut runtime,
        &sender,
        json!({"verb": "set", "board": "board-1", "revision": 1, "blocks": []}),
    )
    .unwrap_err();
    assert_eq!(code(&refusal).0, "board_moved");
}

#[test]
fn a_deleted_board_answers_board_deleted_to_every_later_command() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _, _) = fixture(root.path());
    let mut runtime = runtime.lock().unwrap();
    ok(&mut runtime, &sender, login_flow());
    let (deleted, changed) = run(
        &mut runtime,
        &sender,
        json!({"verb": "delete", "board": "board-1"}),
    )
    .unwrap();
    assert!(changed);
    assert_eq!(deleted["board"]["state"], "deleted");
    assert_eq!(deleted["board"]["blocks"], json!([]));
    for command in [
        json!({"verb": "edit", "board": "board-1", "edits": [{"op": "remove", "id": "b1"}]}),
        json!({"verb": "set", "board": "board-1", "blocks": []}),
        json!({"verb": "show", "board": "board-1"}),
        json!({"verb": "delete", "board": "board-1"}),
    ] {
        assert_eq!(
            run(&mut runtime, &sender, command.clone()).unwrap_err(),
            "board_deleted",
            "{command}"
        );
    }
    let list = ok(&mut runtime, &sender, json!({"verb": "list"}));
    assert_eq!(
        (&list["boards"][0]["id"], &list["boards"][0]["state"]),
        (&json!("board-1"), &json!("deleted"))
    );
    // The next board takes the next id, never the deleted one's.
    assert_eq!(
        ok(&mut runtime, &sender, login_flow())["board"]["id"],
        "board-2"
    );
}

#[test]
fn a_board_is_read_back_from_its_folder_after_the_runtime_is_made_again() {
    let root = tempfile::tempdir().unwrap();
    {
        let (runtime, sender, _, _) = fixture(root.path());
        let mut runtime = runtime.lock().unwrap();
        ok(&mut runtime, &sender, login_flow());
        ok(
            &mut runtime,
            &sender,
            json!({"verb": "edit", "board": "board-1", "edits": [
                {"op": "update", "id": "b1", "changes": {"text": "Second text"}},
            ]}),
        );
        ok(&mut runtime, &sender, login_flow());
        ok(
            &mut runtime,
            &sender,
            json!({"verb": "delete", "board": "board-2"}),
        );
        saved(root.path(), "board-1", 2);
        saved(root.path(), "board-2", 2);
    }
    let (runtime, sender, _, _) = fixture(root.path());
    let mut runtime = runtime.lock().unwrap();
    let shown = ok(
        &mut runtime,
        &sender,
        json!({"verb": "show", "board": "board-1"}),
    );
    assert_eq!(shown["board"]["revision"], 2);
    assert!(shown["markdown"].as_str().unwrap().contains("Second text"));
    assert_eq!(ids(&shown), ["b1", "b2", "b3", "b4"]);
    assert_eq!(
        run(
            &mut runtime,
            &sender,
            json!({"verb": "show", "board": "board-2"})
        )
        .unwrap_err(),
        "board_deleted"
    );
    // The edit goes on from the read revision, and a new board takes the
    // next id.
    let edited = ok(
        &mut runtime,
        &sender,
        json!({"verb": "edit", "board": "board-1", "revision": 2, "edits": [
            {"op": "insert", "block": {"type": "divider"}},
        ]}),
    );
    assert_eq!(ids(&edited).last().unwrap(), "b5");
    assert_eq!(
        ok(&mut runtime, &sender, login_flow())["board"]["id"],
        "board-3"
    );
}

#[test]
fn deleting_a_sessions_records_drops_its_boards_and_their_files() {
    let root = tempfile::tempdir().unwrap();
    {
        let (runtime, sender, _, _) = fixture(root.path());
        let mut runtime = runtime.lock().unwrap();
        ok(&mut runtime, &sender, login_flow());
        ok(&mut runtime, &recipient(), login_flow());
        saved(root.path(), "board-2", 1);
        runtime.ingest_link_pruned(vec![crate::links::ledger::SessionIdentity {
            device_id: crate::node::TEST_NODE.into(),
            agent: "codex".into(),
            session_id: "sender-session".into(),
        }]);
        assert_eq!(
            run(
                &mut runtime,
                &sender,
                json!({"verb": "show", "board": "board-1"})
            )
            .unwrap_err(),
            "board_unknown"
        );
        let file = hide_kit::layout::board_file(&boards_dir(root.path()), "board-1");
        wait_for("board-1's file deleted", || !file.exists());
    }
    let (runtime, _, _, _) = fixture(root.path());
    let mut runtime = runtime.lock().unwrap();
    let other = ok(&mut runtime, &recipient(), json!({"verb": "list"}));
    assert_eq!(other["boards"][0]["id"], "board-2");
    assert_eq!(other["boards"].as_array().unwrap().len(), 1);
}

#[test]
fn a_pane_with_no_agent_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _, _) = fixture(root.path());
    let mut runtime = runtime.lock().unwrap();
    let tab = &mut runtime.snapshot.navigator.workspaces[0].checkouts[0].tabs[0];
    let mut shell = tab.panes[0].clone();
    shell.id = "shell".into();
    tab.panes.push(shell);
    let plain = Actor {
        pane_id: "shell".into(),
        ..sender
    };
    assert_eq!(
        run(&mut runtime, &plain, json!({"verb": "list"})).unwrap_err(),
        "agent_pane_required"
    );
}

/// Boards keep a folder of their own, so a build that knows no board, or
/// upstream's next schema of either store, still reads both (design
/// section 1, Where it is stored).
#[test]
fn a_saved_board_leaves_the_link_record_at_16_and_the_view_layouts_at_5() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let (runtime, sender, _, _) = fixture(root.path());
    runtime.lock().unwrap().install_worker_context(
        Arc::downgrade(&runtime),
        crate::handle::ChangeNotifier::noop(),
    );
    let _links =
        super::super::links::spawn_worker(&runtime, crate::handle::ChangeNotifier::noop()).unwrap();
    let links = hide_kit::layout::links_store(&state);
    let version = || -> i64 {
        rusqlite::Connection::open_with_flags(&links, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .and_then(|store| store.pragma_query_value(None, "user_version", |row| row.get(0)))
            .unwrap_or(0)
    };
    wait_for("the link record made at its schema", || version() > 0);
    {
        let mut runtime = runtime.lock().unwrap();
        ok(&mut runtime, &sender, login_flow());
        runtime.worker_context = None;
        runtime.persist_workspace_views();
    }
    saved(root.path(), "board-1", 1);
    let views = root.path().join("views.json");
    assert!(views.is_file(), "the View layouts were saved");
    assert_eq!(version(), 16);
    let saved: Value = serde_json::from_slice(&std::fs::read(&views).unwrap()).unwrap();
    assert_eq!(saved["schema_version"], 5);
    assert!(
        !std::fs::read_to_string(&views).unwrap().contains("board"),
        "no board reaches the View layouts"
    );
}
