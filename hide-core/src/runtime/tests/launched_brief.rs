//! A spawn whose brief rides its agent's start (`AgentApi::LaunchPrompt`):
//! the agent takes the brief as it starts and works on it at once, while the
//! engine reads its start as launching until that turn ends. The spawn
//! registers the child as soon as the agent is seen taking its brief, not
//! after a wait for input readiness, and the clock the brief's record is owed
//! by starts there.
//!
//! The spawn runs on this machine's fake engine
//! (`beside_spawn::local_spawner`), which reads the started agent in the
//! order the test sets.

use super::beside_spawn::{Ask, BESIDE, child_row, local_spawner};
use crate::delivery::typed::SendRoute;
use serde_json::{Value, json};

/// The started agent as the engine reads it while its start has not
/// settled (`launch_pending`, `Readiness::Launching`), in `status`.
fn launching(status: &str) -> Value {
    let mut row = child_row();
    row["launch_pending"] = json!(true);
    row["agent_status"] = json!(status);
    row
}

/// The agent shows idle as it starts, then works on its brief, and its
/// start never settles: the child is registered once it is seen working,
/// with no further reading, and the brief's record clock starts at that
/// reading rather than at the start.
#[test]
fn a_launched_brief_registers_its_child_once_its_agent_is_seen_working_on_it() {
    let spawner = local_spawner();
    spawner.reads([launching("idle"), launching("working")]);
    let child = spawner
        .spawn(
            "launched",
            Ask {
                parent: Some("here"),
                beside: false,
                brief: Some("fix it"),
                ..BESIDE
            },
        )
        .expect("the child is registered");
    assert_eq!(child["pane"], "w2:p1");
    assert_eq!(child["parent"], "agent-1");

    let ledger = spawner.ledger();
    let spawn = &ledger.spawns[0];
    assert!(spawn.completed);
    assert_eq!(spawn.child.as_deref(), child["id"].as_str());
    let input = spawn.brief_input.as_ref().expect("the brief's input");
    assert_eq!(input.route, SendRoute::Launch, "the brief rode the start");
    assert!(
        input.typed_at_unix_ms > input.first_typed_at_unix_ms,
        "the clock starts where the agent was seen taking its brief: {input:?}"
    );

    // The registration is published with the child's tokens, after the
    // two readings that saw the agent start and then work: the spawn did
    // not read on while the engine still said launching.
    let methods = spawner.methods();
    let published = methods
        .iter()
        .position(|method| *method == "report_pane_tokens")
        .expect("the child's tokens were published");
    let readings = methods[..published]
        .iter()
        .filter(|method| **method == "delivery_agent")
        .count();
    assert_eq!(readings, 2, "{methods:?}");
}
