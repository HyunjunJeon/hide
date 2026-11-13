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
    runtime
        .board_command(
            crate::node::TEST_NODE,
            &actor.pane_id,
            &authority(actor).context,
            command,
        )
        .map(crate::runtime::boards::BoardTurn::answer)
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
    assert_eq!(drawn["view"]["open"], true, "new opens the board's View");

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

const SENDER_SESSION: &str = "0b5e7c1a-sender";
const RECIPIENT_SESSION: &str = "7d2f4e9b-recipient";

/// The fixture's two agents as the sidebar shows them, each on its native
/// session, which is what a board is keyed by.
fn show_rows(runtime: &mut Runtime) {
    let row = |pane: &str| {
        json!({"pane_id": pane, "workspace_id": "w1", "tab_id": "w1:t1", "terminal_id": format!("t-{pane}"),
               "revision": 1, "focused": false, "agent_status": "idle", "agent": "codex", "name": pane,
               "cwd": "/checkouts/fixture", "state_change_seq": 1})
    };
    let payload: crate::sidebar::SessionSnapshotPayload =
        serde_json::from_value(json!({"agents": [row("sender"), row("recipient")]})).unwrap();
    let mut rows = crate::sidebar::project_agents(payload).agents;
    for row in &mut rows {
        row.session_id = Some(
            if row.pane_id == "sender" {
                SENDER_SESSION
            } else {
                RECIPIENT_SESSION
            }
            .into(),
        );
    }
    runtime.snapshot.navigator.agents = rows;
}

/// `pane`'s registration as the delivery ledger keeps it: the native
/// session, with the actor carrying its digest.
fn registration(
    id: &str,
    pane: &str,
    session: &str,
    parent: Option<&str>,
) -> crate::coordination::AgentRecord {
    crate::coordination::AgentRecord {
        id: id.into(),
        name: Some(format!("{pane}-agent")),
        machine: crate::node::TEST_NODE.into(),
        host_scope: "scope".into(),
        native_machine: "fixture-machine".into(),
        session: session.into(),
        instance: format!("terminal-{pane}"),
        pane: pane.into(),
        parent: parent.map(Into::into),
        origin: None,
        project: None,
        actor: Actor {
            pane_id: pane.into(),
            name: pane.into(),
            kind: "codex".into(),
            device_id: crate::node::TEST_NODE.into(),
            session: crate::engine::tokens::session_digest(session),
        },
        ended: false,
        terminal_id: None,
        forked: false,
    }
}

/// A mission's Conductor reads the board its worker drew in the worker's
/// own worktree; an agent that is neither in that checkout nor an ancestor
/// of the drawing session does not, a child does not read up into its
/// parent's board, and reading is all an ancestor does.
#[test]
fn a_parent_reads_a_board_its_child_drew_in_another_checkout_and_no_one_else_does() {
    let root = tempfile::tempdir().unwrap();
    {
        let (runtime, sender, _, _) = fixture(root.path());
        let mut runtime = runtime.lock().unwrap();
        show_rows(&mut runtime);
        ok(&mut runtime, &sender, login_flow());
        let record = ok(
            &mut runtime,
            &sender,
            json!({"verb": "show", "board": "board-1", "json": true}),
        );
        assert_eq!(record["board"]["owner"]["session_id"], SENDER_SESSION);
        // The parent's own board, board-2.
        ok(&mut runtime, &recipient(), login_flow());
        saved(root.path(), "board-1", 1);
        saved(root.path(), "board-2", 1);
    }
    // Each was drawn in a worktree of its own.
    for (id, worktree) in [
        ("board-1", "/checkouts/worker"),
        ("board-2", "/checkouts/lead"),
    ] {
        let file = hide_kit::layout::board_file(&boards_dir(root.path()), id);
        let mut board: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        board["checkout"]["path"] = json!(worktree);
        std::fs::write(&file, serde_json::to_vec(&board).unwrap()).unwrap();
    }

    let (runtime, sender, _, _) = fixture(root.path());
    let mut runtime = runtime.lock().unwrap();
    show_rows(&mut runtime);
    let parent = recipient();
    let show = json!({"verb": "show", "board": "board-1"});
    assert_eq!(
        run(&mut runtime, &parent, show.clone()).unwrap_err(),
        "board_not_yours"
    );
    let mut ledger = crate::delivery::ledger::Ledger::default();
    ledger.agents.push(registration(
        "agent-1",
        "recipient",
        RECIPIENT_SESSION,
        None,
    ));
    ledger.agents.push(registration(
        "agent-2",
        &sender.pane_id,
        SENDER_SESSION,
        Some("agent-1"),
    ));
    runtime.delivery_ledger = Ok(Arc::new(ledger));
    let shown = ok(&mut runtime, &parent, show);
    assert!(
        shown["markdown"]
            .as_str()
            .unwrap()
            .starts_with("# Login flow")
    );
    assert_eq!(
        run(
            &mut runtime,
            &parent,
            json!({"verb": "edit", "board": "board-1", "edits": [{"op": "remove", "id": "b1"}]}),
        )
        .unwrap_err(),
        "board_not_yours"
    );
    assert_eq!(
        run(
            &mut runtime,
            &sender,
            json!({"verb": "show", "board": "board-2"}),
        )
        .unwrap_err(),
        "board_not_yours",
        "a child reads no board of its parent's"
    );
}

/// Writes `boards` into the state folder's board folder, one file each, as
/// a build that keeps boards writes them.
fn stored(root: &std::path::Path, boards: impl Iterator<Item = Value>) {
    let dir = boards_dir(root);
    std::fs::create_dir_all(&dir).unwrap();
    for board in boards {
        let id = board["id"].as_str().unwrap().to_owned();
        std::fs::write(
            hide_kit::layout::board_file(&dir, &id),
            serde_json::to_vec(&board).unwrap(),
        )
        .unwrap();
    }
}

/// Board `n` of `session`, open unless `state` says otherwise, last changed
/// at `updated`.
fn stored_board(n: usize, session: &str, state: Value, updated: u64) -> Value {
    let mut board = json!({
        "id": format!("board-{n}"),
        "owner": {"device_id": crate::node::TEST_NODE, "agent": "codex", "session_id": session},
        "checkout": {"device_id": crate::node::TEST_NODE, "path": "/checkouts/fixture"},
        "title": format!("Board {n}"), "blocks": [], "next_block": 1, "revision": 1,
        "created_at_unix_ms": updated, "updated_at_unix_ms": updated,
    });
    board
        .as_object_mut()
        .unwrap()
        .extend(state.as_object().unwrap().clone());
    board
}

/// Past `BOOK_LIMIT` a new board lets go, one at a time: a deleted stub
/// first, then a kept board, then the open board updated longest ago whose
/// session runs in no pane; a board of a session on screen stays, though it
/// is the oldest of all.
#[test]
fn a_full_book_lets_go_a_stub_then_a_kept_board_then_the_oldest_board_of_no_pane() {
    let root = tempfile::tempdir().unwrap();
    let open = json!({"state": "open"});
    let special = [
        stored_board(1, SENDER_SESSION, open.clone(), 1),
        stored_board(2, "gone", open.clone(), 5),
        stored_board(
            3,
            "gone",
            json!({"state": "kept", "kept_path": "docs/boards/b.md", "kept_at_unix_ms": 50}),
            50,
        ),
        stored_board(
            4,
            "gone",
            json!({"state": "deleted", "deleted_by": "agent", "deleted_at_unix_ms": 100}),
            100,
        ),
    ];
    let rest = (5..=crate::boards::BOOK_LIMIT)
        .map(|n| stored_board(n, &format!("gone-{n}"), open.clone(), 1000 + n as u64));
    stored(root.path(), special.into_iter().chain(rest));
    let (runtime, sender, _, _) = fixture(root.path());
    let mut runtime = runtime.lock().unwrap();
    show_rows(&mut runtime);
    // Board 2's View is open: the operator may be looking at it, so the
    // next oldest board of no pane goes in its place.
    runtime
        .change_view_layout(&key(), |layout, stamp| {
            let display = layout.new_board_display("board-2", "Board 2");
            let area = layout.active_area().id.clone();
            layout.insert(&area, display, stamp)?;
            Ok(((), true))
        })
        .unwrap();
    let shown = |runtime: &mut Runtime, n: u32| {
        run(
            runtime,
            &sender,
            json!({"verb": "show", "board": format!("board-{n}")}),
        )
        .err()
    };
    let draw = json!({"verb": "new", "title": "T", "blocks": []});
    for (drawn, gone) in [(257, 4), (258, 3), (259, 5)] {
        assert_eq!(
            ok(&mut runtime, &sender, draw.clone())["board"]["id"],
            format!("board-{drawn}")
        );
        assert_eq!(
            shown(&mut runtime, gone),
            Some("board_unknown".to_owned()),
            "board-{gone} went for board-{drawn}"
        );
    }
    assert_eq!(
        shown(&mut runtime, 1),
        None,
        "the session on screen keeps its board"
    );
    assert_eq!(shown(&mut runtime, 2), None, "a board on screen stays");
    assert_eq!(shown(&mut runtime, 6), None);
    let file = hide_kit::layout::board_file(&boards_dir(root.path()), "board-5");
    wait_for("board-5's file deleted", || !file.exists());
}

/// A book whose every board belongs to a session on screen refuses a new
/// board rather than let one go.
#[test]
fn a_full_book_of_sessions_on_screen_refuses_a_new_board() {
    let root = tempfile::tempdir().unwrap();
    stored(
        root.path(),
        (1..=crate::boards::BOOK_LIMIT)
            .map(|n| stored_board(n, RECIPIENT_SESSION, json!({"state": "open"}), n as u64)),
    );
    let (runtime, sender, _, _) = fixture(root.path());
    let mut runtime = runtime.lock().unwrap();
    show_rows(&mut runtime);
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
            Some("Hide keeps 256 boards and each belongs to an agent on screen")
        )
    );
    assert!(hide_kit::layout::board_file(&boards_dir(root.path()), "board-1").exists());
}

/// Deleting an archived session from the archive takes the boards it drew,
/// as pruning it does.
#[test]
fn deleting_an_archived_session_drops_its_boards() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _, _) = fixture(root.path());
    let mut runtime = runtime.lock().unwrap();
    ok(&mut runtime, &recipient(), login_flow());
    ok(&mut runtime, &sender, login_flow());
    crate::runtime::delivery::tests::archive_holding(
        &mut runtime,
        &recipient(),
        "/checkouts/fixture",
    );
    for record in runtime
        .snapshot
        .ui_state
        .session_archive
        .records
        .values_mut()
    {
        record.deleting = true;
    }
    let saved = runtime.snapshot.ui_state.session_archive.clone();
    assert!(runtime.ingest_archive_saved(&saved, true));
    let show = |n: u32| json!({"verb": "show", "board": format!("board-{n}")});
    assert_eq!(
        run(&mut runtime, &sender, show(1)).unwrap_err(),
        "board_unknown"
    );
    assert!(run(&mut runtime, &sender, show(2)).is_ok());
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

// ---- The board View (design section 3) ----

use super::unseen_views::{fixture_in, key, keyboard, row, unseen};
use crate::agent_state::RowMark;
use crate::view_layout::DisplayKind;

/// The fixture of `unseen_views` (two working agents in the front checkout,
/// the Files lane open on its documents, the window holding the keyboard),
/// with the operator in `sender`'s pane: an open of `recipient`'s lands
/// behind them.
fn on_screen(root: &std::path::Path, child: bool) -> Arc<Mutex<Runtime>> {
    let runtime = fixture_in(root, child);
    keyboard(&mut runtime.lock().unwrap(), Some("sender"));
    runtime
}

/// The board displays of the fixture's checkout, by View id and board.
fn board_views(runtime: &Runtime) -> Vec<(String, String)> {
    runtime
        .view_layout_of(&key())
        .map(|layout| {
            layout
                .displays()
                .filter(|display| display.kind == DisplayKind::Board)
                .map(|display| (display.id.clone(), display.board.clone().unwrap()))
                .collect()
        })
        .unwrap_or_default()
}

/// The View id a board command answered with.
fn view_of(answer: &Value) -> String {
    answer["view"]["id"].as_str().unwrap().to_owned()
}

fn edit_first_text(runtime: &mut Runtime, actor: &Actor, text: &str) -> Value {
    ok(
        runtime,
        actor,
        json!({"verb": "edit", "board": "board-1", "edits": [
            {"op": "update", "id": "b1", "changes": {"text": text}},
        ]}),
    )
}

#[test]
fn an_agents_board_opened_behind_the_operator_waits_in_needs_you_named_by_its_title() {
    let root = tempfile::tempdir().unwrap();
    let runtime = on_screen(root.path(), false);
    let mut guard = runtime.lock().unwrap();

    let drawn = ok(&mut guard, &recipient(), login_flow());
    assert_eq!(
        (&drawn["view"]["open"], &drawn["view"]["unseen"]),
        (&json!(true), &json!(true))
    );
    let view = view_of(&drawn);
    assert_eq!(board_views(&guard), [(view.clone(), "board-1".to_owned())]);
    assert!(unseen(&guard, &view));
    let agent = row(&guard, "recipient");
    assert_eq!(
        (agent.group.as_str(), agent.mark),
        ("needs_you", RowMark::Look)
    );
    let look = agent.look.as_ref().unwrap();
    assert_eq!(
        (look.view_id.as_str(), look.label.as_str()),
        (view.as_str(), "Login flow")
    );
    // `hide view list` names it by kind and board id.
    let listed = guard
        .workspace_control_query(
            crate::node::TEST_NODE,
            "recipient",
            crate::workspace_control::Query::ViewList,
        )
        .unwrap();
    let listed = serde_json::to_value(listed).unwrap();
    let board = listed["views"]
        .as_array()
        .unwrap()
        .iter()
        .find(|listed| listed["view_id"] == view.as_str())
        .unwrap();
    assert_eq!(
        (&board["kind"], &board["target"], &board["unseen"]),
        (&json!("board"), &json!("board-1"), &json!(true))
    );

    // A report letter names it by id and title, for an ancestor to read.
    let mut report = String::new();
    guard.list_opened_views("recipient", &mut report);
    assert!(
        report.contains(&format!(
            "- {view} board board-1 · Login flow (not seen by the operator)"
        )),
        "{report}"
    );

    // A new title is the row's at once.
    ok(
        &mut guard,
        &recipient(),
        json!({"verb": "set", "board": "board-1", "title": "Login flow, again", "blocks": [
            {"type": "divider"},
        ]}),
    );
    assert_eq!(
        row(&guard, "recipient").look.as_ref().unwrap().label,
        "Login flow, again"
    );

    // `--no-open` draws the board alone.
    let quiet = ok(
        &mut guard,
        &recipient(),
        json!({"verb": "new", "title": "Notes", "no_open": true, "blocks": [{"type": "divider"}]}),
    );
    assert!(quiet.get("view").is_none());
    assert_eq!(board_views(&guard).len(), 1);
}

#[test]
fn a_delegated_childs_board_turns_only_its_ancestors_unread() {
    let root = tempfile::tempdir().unwrap();
    let runtime = on_screen(root.path(), true);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, None);
    assert!(!row(&guard, "sender").unread);

    let view = view_of(&ok(&mut guard, &recipient(), login_flow()));
    assert!(unseen(&guard, &view));
    let child = row(&guard, "recipient");
    assert!(child.delegated);
    assert_eq!(
        child.group, "working",
        "a delegated child is no Needs You row"
    );
    assert!(child.look.is_none());
    let parent = row(&guard, "sender");
    assert!(parent.unread);
    assert_ne!(parent.group, "needs_you");
}

#[test]
fn an_edit_leaves_a_board_the_operator_saw_seen_and_an_open_makes_it_news_again() {
    let root = tempfile::tempdir().unwrap();
    let runtime = on_screen(root.path(), false);
    let mut guard = runtime.lock().unwrap();
    let view = view_of(&ok(&mut guard, &recipient(), login_flow()));
    // The operator looks: the board is in front of the agent's own strip.
    keyboard(&mut guard, Some("recipient"));
    assert!(!unseen(&guard, &view));
    keyboard(&mut guard, Some("sender"));

    let edited = edit_first_text(&mut guard, &recipient(), "Second text");
    assert_eq!(
        (&edited["board"]["revision"], &edited["view"]["unseen"]),
        (&json!(2), &json!(false))
    );
    assert!(!unseen(&guard, &view), "an edit is not news");
    assert!(row(&guard, "recipient").look.is_none());

    let opened = ok(
        &mut guard,
        &recipient(),
        json!({"verb": "open", "board": "board-1"}),
    );
    assert_eq!(view_of(&opened), view, "an open finds the board's one View");
    assert_eq!(opened["view"]["unseen"], true);
    assert_eq!(board_views(&guard).len(), 1);
    assert!(unseen(&guard, &view), "an open is news again");
    assert_eq!(row(&guard, "recipient").mark, RowMark::Look);

    // An agent that only shares the checkout reads it with `show` and cannot
    // show it.
    let stranger = Actor {
        pane_id: "sender".into(),
        name: "sender".into(),
        kind: "codex".into(),
        device_id: crate::node::TEST_NODE.into(),
        session: Some("sender-session".into()),
    };
    assert_eq!(
        code(
            &run(
                &mut guard,
                &stranger,
                json!({"verb": "open", "board": "board-1"})
            )
            .unwrap_err()
        )
        .0,
        "board_not_yours"
    );

    // An open keeps no receipt: any number of them leaves room for the
    // next action.
    for _ in 0..130 {
        ok(
            &mut guard,
            &recipient(),
            json!({"verb": "open", "board": "board-1"}),
        );
    }
}

/// `hide browser open <url>` from `actor`'s pane, as the daemon commits it.
fn open_page(runtime: &mut Runtime, actor: &Actor, url: &str) -> String {
    runtime
        .workspace_control_action(
            crate::node::TEST_NODE,
            &actor.pane_id,
            &authority(actor).context,
            &format!("{}-page-{url}", super::unix_milliseconds()),
            crate::workspace_control::Action::OpenBrowser {
                url: url.to_owned(),
                area_id: None,
                new_target: false,
            },
            Ok(None),
        )
        .unwrap()
        .view_id
}

#[test]
fn the_view_cap_closes_a_board_view_and_the_board_opens_again() {
    let root = tempfile::tempdir().unwrap();
    let runtime = on_screen(root.path(), false);
    let mut guard = runtime.lock().unwrap();
    // The operator works in the agent's pane, so each page comes to the
    // front and the board, looked at longest ago, is behind them.
    keyboard(&mut guard, Some("recipient"));
    ok(&mut guard, &recipient(), login_flow());
    for page in 0..crate::view_layout::MAX_VIEW_DISPLAYS {
        open_page(
            &mut guard,
            &recipient(),
            &format!("https://example.test/{page}"),
        );
    }
    assert!(
        board_views(&guard).is_empty(),
        "the cap closed the board's View"
    );
    let listed = ok(&mut guard, &recipient(), json!({"verb": "list"}));
    assert_eq!(
        (&listed["boards"][0]["state"], &listed["boards"][0]["view"]),
        (&json!("open"), &json!({"open": false}))
    );
    let opened = run(
        &mut guard,
        &recipient(),
        json!({"verb": "open", "board": "board-1"}),
    )
    .unwrap()
    .0;
    assert_eq!(opened["view"]["open"], true);
    assert_eq!(board_views(&guard).len(), 1);
}

#[test]
fn a_board_drawn_into_a_full_workspace_is_kept_and_its_view_says_why_it_is_closed() {
    let root = tempfile::tempdir().unwrap();
    let runtime = on_screen(root.path(), false);
    let mut guard = runtime.lock().unwrap();
    // Files still to be read hold no document the cap could close.
    guard
        .change_view_layout(&key(), |layout, stamp| {
            for n in 0..crate::view_layout::MAX_VIEW_DISPLAYS {
                let display =
                    layout.new_display(&format!("src/{n}.rs"), DisplayKind::File, None, false);
                let area = layout.active_area().id.clone();
                layout.insert(&area, display, stamp)?;
            }
            Ok(((), true))
        })
        .unwrap();

    let drawn = ok(&mut guard, &recipient(), login_flow());
    assert_eq!(
        (
            &drawn["board"]["id"],
            &drawn["view"]["open"],
            &drawn["view"]["reason"]
        ),
        (&json!("board-1"), &json!(false), &json!("view_limit"))
    );
    assert!(board_views(&guard).is_empty());
    assert_eq!(
        code(
            &run(
                &mut guard,
                &recipient(),
                json!({"verb": "open", "board": "board-1"})
            )
            .unwrap_err()
        )
        .0,
        "view_limit"
    );
}

#[test]
fn a_comment_on_a_board_view_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let runtime = on_screen(root.path(), false);
    let mut guard = runtime.lock().unwrap();
    let view = view_of(&ok(&mut guard, &recipient(), login_flow()));
    for anchor in [
        json!({"kind": "lines", "start": 1, "end": 1, "lines": ["The first text."]}),
        json!({"kind": "area", "rect": {"x": 1.0, "y": 1.0, "width": 10.0, "height": 10.0}}),
    ] {
        let payload = serde_json::from_value(json!({
            "workspace": {"device_id": crate::node::TEST_NODE, "path": key().1},
            "action": "add", "display_id": view, "anchor": anchor, "note": "Why?",
        }))
        .unwrap();
        guard.apply_view_comment(payload);
        assert_eq!(
            guard
                .snapshot
                .status
                .last_error
                .as_ref()
                .map(|error| error.kind.as_str()),
            Some("view_comment.invalid_anchor")
        );
    }
    assert!(
        guard
            .workspace_views
            .as_ref()
            .unwrap()
            .views
            .get(&key().0, &key().1)
            .is_none_or(|view| view.feedback.comments.is_empty())
    );
}

#[test]
fn a_restart_puts_each_board_view_back_in_its_area_with_its_unseen_record() {
    let root = tempfile::tempdir().unwrap();
    let (seen, waiting, area) = {
        let runtime = on_screen(root.path(), false);
        let mut guard = runtime.lock().unwrap();
        let seen = view_of(&ok(&mut guard, &recipient(), login_flow()));
        let waiting = view_of(&ok(
            &mut guard,
            &recipient(),
            json!({"verb": "new", "title": "Rollout", "blocks": [{"type": "divider"}]}),
        ));
        // The operator looks at the first, in front of its area; the
        // second stays behind it, still to look at.
        keyboard(&mut guard, Some("recipient"));
        keyboard(&mut guard, Some("sender"));
        assert!(!unseen(&guard, &seen) && unseen(&guard, &waiting));
        let area = guard
            .view_layout_of(&key())
            .and_then(|layout| layout.area_of(&waiting))
            .unwrap()
            .id
            .clone();
        guard.snapshot_delta_payload(0);
        (seen, waiting, area)
    };
    let hinted = |id: &str, unseen: bool| {
        let file = hide_kit::layout::board_file(&boards_dir(root.path()), id);
        wait_for(&format!("{id}'s View hint saved"), || {
            std::fs::read(&file)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .is_some_and(|board| {
                    board["view"]["area_id"] == area.as_str()
                        && board["view"].get("unseen").is_some() == unseen
                })
        });
    };
    hinted("board-1", false);
    hinted("board-2", true);
    // No board reaches the View layouts' own file.
    let views = std::fs::read_to_string(root.path().join("views.json")).unwrap();
    assert!(!views.contains("board"), "{views}");

    let runtime = on_screen(root.path(), false);
    let mut guard = runtime.lock().unwrap();
    guard.snapshot_delta_payload(0);
    let layout = guard.view_layout_of(&key()).unwrap();
    let back: Vec<(String, Option<String>, bool)> = layout
        .displays()
        .filter(|display| display.kind == DisplayKind::Board)
        .map(|display| {
            (
                layout.area_of(&display.id).unwrap().id.clone(),
                display.board.clone(),
                display.unseen.is_some(),
            )
        })
        .collect();
    assert_eq!(
        back,
        [
            (area.clone(), Some("board-1".to_owned()), false),
            (area.clone(), Some("board-2".to_owned()), true),
        ]
    );
    let agent = row(&guard, "recipient");
    assert_eq!(
        (agent.group.as_str(), agent.mark),
        ("needs_you", RowMark::Look)
    );
    assert_eq!(agent.look.as_ref().unwrap().label, "Rollout");
    // The View ids are minted again; the board ids are what lasts.
    let _ = (seen, waiting);
}

/// The `boards` rest field the last frame carried, and its revision.
struct RestBoards {
    value: Arc<crate::runtime::boards::BoardsSnapshot>,
    revision: u64,
}

impl std::ops::Deref for RestBoards {
    type Target = crate::runtime::boards::BoardsSnapshot;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

fn rest_boards(runtime: &mut Runtime) -> RestBoards {
    let payload = runtime.snapshot_delta_payload(0);
    let boards = &payload.current_rest.boards;
    RestBoards {
        value: Arc::clone(&boards.value),
        revision: boards.revision,
    }
}

#[test]
fn the_front_workspaces_boards_ride_the_wire_and_move_only_with_an_accepted_change() {
    let root = tempfile::tempdir().unwrap();
    let runtime = on_screen(root.path(), false);
    let mut guard = runtime.lock().unwrap();
    // A board with no View is not on the wire.
    ok(
        &mut guard,
        &recipient(),
        json!({"verb": "new", "title": "Notes", "no_open": true, "blocks": [{"type": "divider"}]}),
    );
    assert!(rest_boards(&mut guard).items.is_empty());

    ok(&mut guard, &recipient(), login_flow());
    let first = rest_boards(&mut guard);
    let board = &first.items[0];
    assert_eq!(
        (board.id.as_str(), board.revision, board.state),
        ("board-2", 1, "open")
    );
    let kinds: Vec<&str> = board.blocks.iter().map(|block| block.kind).collect();
    assert_eq!(kinds, ["section", "table", "quote", "diagram"]);
    assert_eq!(
        board.blocks[0].markdown,
        "## What changes\n\nThe first text."
    );
    assert_eq!(
        board.blocks[2]
            .quote
            .as_ref()
            .map(|quote| (quote.path.as_str(), quote.start, quote.end)),
        Some(("src/login.rs", 3, 4))
    );

    // A frame with nothing new copies no board.
    let again = rest_boards(&mut guard);
    assert!(Arc::ptr_eq(&first.value, &again.value));
    assert_eq!(first.revision, again.revision);

    // A refused or unchanged command moves nothing.
    assert!(
        run(
            &mut guard,
            &recipient(),
            json!({"verb": "edit", "board": "board-2", "edits": [{"op": "remove", "id": "b99"}]}),
        )
        .is_err()
    );
    let (_, changed) = run(
        &mut guard,
        &recipient(),
        json!({"verb": "edit", "board": "board-2", "edits": [
            {"op": "update", "id": "b1", "changes": {"text": "The first text."}},
        ]}),
    )
    .unwrap();
    assert!(!changed);
    assert!(Arc::ptr_eq(&first.value, &rest_boards(&mut guard).value));

    // An accepted edit is on the next frame, block by block.
    let (_, changed) = run(
        &mut guard,
        &recipient(),
        json!({"verb": "edit", "board": "board-2", "edits": [
            {"op": "update", "id": "b1", "changes": {"text": "Second text"}},
        ]}),
    )
    .unwrap();
    assert!(changed);
    let edited = rest_boards(&mut guard);
    assert!(edited.revision > first.revision);
    let board = &edited.items[0];
    assert_eq!(
        (
            board.revision,
            board.blocks[0].changed_at,
            board.blocks[1].changed_at
        ),
        (2, 2, 1)
    );
    assert_eq!(board.blocks[0].markdown, "## What changes\n\nSecond text");
}

#[test]
fn a_board_the_agent_deleted_leaves_its_view_unavailable() {
    let root = tempfile::tempdir().unwrap();
    let runtime = on_screen(root.path(), false);
    let mut guard = runtime.lock().unwrap();
    let view = view_of(&ok(&mut guard, &recipient(), login_flow()));
    ok(
        &mut guard,
        &recipient(),
        json!({"verb": "delete", "board": "board-1"}),
    );
    // The front strip is the agent's, which shows the View it opened.
    keyboard(&mut guard, Some("recipient"));
    guard.snapshot_delta_payload(0);
    let layout = &guard.snapshot.workspace_view.as_ref().unwrap().layout;
    let display = super::view_areas::areas(layout)
        .into_iter()
        .flat_map(|(_, _, displays)| displays)
        .find(|display| display.id == view)
        .unwrap();
    assert_eq!(
        (display.state, display.reason.as_deref(), display.kind),
        (
            crate::model::ViewDisplayState::Unavailable,
            Some(crate::boards::DELETED_REASON),
            DisplayKind::Board
        )
    );
    assert_eq!(display.board.as_deref(), Some("board-1"));
    assert_eq!(
        code(
            &run(
                &mut guard,
                &recipient(),
                json!({"verb": "open", "board": "board-1"})
            )
            .unwrap_err()
        )
        .0,
        "board_deleted"
    );
}

#[test]
fn a_mission_keeps_nothing_of_a_board_view() {
    let mut layout = crate::view_layout::Layout::default();
    let display = layout.new_board_display("board-1", "Login flow");
    assert!(crate::missions::KeptView::of(&display, "Login flow".into()).is_none());
}

#[test]
fn the_operators_close_takes_a_board_view_and_leaves_its_board_for_open() {
    let root = tempfile::tempdir().unwrap();
    let runtime = on_screen(root.path(), false);
    let mut guard = runtime.lock().unwrap();
    let view = view_of(&ok(&mut guard, &recipient(), login_flow()));
    assert!(super::view_areas::act(
        &mut guard,
        json!({"action": "close", "display_id": view}),
    ));
    assert!(board_views(&guard).is_empty());
    assert!(
        row(&guard, "recipient").look.is_none(),
        "a closed View is nothing to look at"
    );
    let listed = ok(&mut guard, &recipient(), json!({"verb": "list"}));
    assert_eq!(
        (&listed["boards"][0]["state"], &listed["boards"][0]["view"]),
        (&json!("open"), &json!({"open": false}))
    );
    let opened = ok(
        &mut guard,
        &recipient(),
        json!({"verb": "open", "board": "board-1"}),
    );
    assert_eq!(
        (&opened["view"]["open"], &opened["view"]["unseen"]),
        (&json!(true), &json!(true))
    );
    assert_eq!(board_views(&guard).len(), 1);
}

#[test]
fn a_parent_opens_its_childs_board_as_its_own_and_only_the_parents_row_waits() {
    let root = tempfile::tempdir().unwrap();
    let runtime = on_screen(root.path(), true);
    let mut guard = runtime.lock().unwrap();
    keyboard(&mut guard, None);
    // The ledger's registrations make `sender` the parent of `recipient`,
    // by the native sessions a board is keyed by.
    let mut ledger = crate::delivery::ledger::Ledger::default();
    ledger
        .agents
        .push(registration("agent-1", "sender", "native-sender", None));
    ledger.agents.push(registration(
        "agent-2",
        "recipient",
        "native-recipient",
        Some("agent-1"),
    ));
    guard.delivery_ledger = Ok(Arc::new(ledger));
    ok(
        &mut guard,
        &recipient(),
        json!({"verb": "new", "title": "Login flow", "no_open": true, "blocks": [{"type": "divider"}]}),
    );
    let parent = Actor {
        pane_id: "sender".into(),
        name: "sender".into(),
        kind: "codex".into(),
        device_id: crate::node::TEST_NODE.into(),
        session: Some("sender-session".into()),
    };

    let opened = ok(
        &mut guard,
        &parent,
        json!({"verb": "open", "board": "board-1"}),
    );
    assert_eq!(opened["view"]["unseen"], true);
    let view = view_of(&opened);
    let display = guard
        .view_layout_of(&key())
        .and_then(|layout| layout.display(&view))
        .unwrap();
    assert_eq!(
        display
            .opener
            .as_ref()
            .map(|opener| opener.terminal_id.as_str()),
        Some("term-sender"),
        "the parent is its opener"
    );
    let lead = row(&guard, "sender");
    assert_eq!(
        (lead.group.as_str(), lead.mark),
        ("needs_you", RowMark::Look)
    );
    assert_eq!(
        (
            lead.look.as_ref().unwrap().view_id.as_str(),
            lead.look.as_ref().unwrap().label.as_str()
        ),
        (view.as_str(), "Login flow")
    );
    let child = row(&guard, "recipient");
    assert_ne!(child.group, "needs_you");
    assert!(child.look.is_none());
    // The child still changes its board, and the parent's View is the one
    // its answer names.
    let edited = ok(
        &mut guard,
        &recipient(),
        json!({"verb": "edit", "board": "board-1", "edits": [{"op": "insert", "block": {"type": "divider"}}]}),
    );
    assert_eq!(edited["view"]["id"], view.as_str());
}
