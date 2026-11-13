import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { agentPane, type HideAnswer } from "./agent-pane";
import { startFixture, type EngineServer } from "./engine-fixture";
import { startHided, type Daemon } from "./hided-fixture";
import { enterWorkspace, screenshot } from "./wire";

// A session's boards (`hide board`): the core keeps the board an agent draws
// for its native session, the agent changes it block by block, and only that
// session changes it. Each command runs as the agent from inside its own pane,
// against an isolated daemon, and reads its answer from the line it prints.
// The board opens as a View that waits in Needs You, and what the agent edits
// after the operator looked reaches the screen in place (board design,
// section 3).

test.describe.configure({ timeout: 120_000 }); // wait-ok: two agent stand-ins start and report their sessions (agentPane allows each 20 s) before the journey runs

type Answer = {
  ok: boolean;
  reason?: string;
  next_action?: string;
  result?: Record<string, unknown> & {
    board?: { id: string; title: string; revision: number; state: string; blocks: { id: string; type: string }[] };
    view?: { id?: string; open: boolean; unseen?: boolean };
    markdown?: string;
    boards?: { id: string; title: string; state: string; revision: number }[];
  };
};

/** The checkout file a quote reads, a credential file no board may quote, and a board's blocks in a file. */
function writeBoardFiles(engine: EngineServer): string {
  const checkout = path.join(engine.root, "fixture");
  fs.mkdirSync(path.join(checkout, "src"), { recursive: true });
  fs.writeFileSync(path.join(checkout, "src", "login.rs"), ["fn sign_in() {", "    let token = read_token();", "    keep(token);", "}"].join("\n") + "\n");
  fs.writeFileSync(path.join(checkout, ".env"), "TOKEN=never-on-a-board\n");
  const boardFile = path.join(engine.root, "board.json");
  fs.writeFileSync(boardFile, JSON.stringify([
    { type: "section", title: "What changes", text: "The first text." },
    { type: "table", columns: ["Step", "Owner"], rows: [["Sign in", "web"], ["Token", "core"]] },
    { type: "quote", path: "src/login.rs", start: 2, end: 3, note: "Where the token is kept." },
    { type: "diagram", mermaid: "graph TD\n  A[Sign in] --> B[Token]" },
  ]));
  return boardFile;
}

/** The JSON line a `hide board` command printed, with its exit status. */
function answer(ran: HideAnswer): Answer & { status: number } {
  const line = ran.out.trim().split("\n").at(-1) ?? "";
  return { ...(JSON.parse(line) as Answer), status: ran.status };
}

test("an agent draws, edits, reads and deletes its board, and another agent's edit is refused", async () => {
  const engine = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const boardFile = writeBoardFiles(engine);
    daemon = await startHided(engine, "board");
    const [pane, other] = engine.panes;
    const agent = await agentPane(engine, daemon.stateDir, pane!);
    const board = async (args: string[]) => answer(await agent(["board", ...args]));

    // Drawn from a file: the core numbers the blocks and the command filled the quote from the checkout.
    const drawn = await board(["new", "--title", "Login flow", "--file", boardFile]);
    expect(drawn.ok, JSON.stringify(drawn)).toBe(true);
    expect(drawn.result!.board).toMatchObject({ id: "board-1", title: "Login flow", revision: 1, state: "open" });
    expect(drawn.result!.board!.blocks).toEqual([
      { id: "b1", type: "section" },
      { id: "b2", type: "table" },
      { id: "b3", type: "quote" },
      { id: "b4", type: "diagram" },
    ]);

    // One block changes in place; the others keep their ids.
    const edited = await board(["edit", "board-1", "--json", JSON.stringify([{ op: "update", id: "b1", changes: { text: "Second text" } }])]);
    expect(edited.ok, JSON.stringify(edited)).toBe(true);
    expect(edited.result!.board!.revision).toBe(2);
    expect(edited.result!.board!.blocks.map((block) => block.id)).toEqual(["b1", "b2", "b3", "b4"]);

    // Show prints the board as the Markdown the screen, a kept file and Hide AI share.
    const shown = await board(["show", "board-1"]);
    expect(shown.ok, JSON.stringify(shown)).toBe(true);
    const markdown = shown.result!.markdown!;
    expect(markdown).toContain("# Login flow");
    expect(markdown).toContain("## What changes\n\nSecond text");
    expect(markdown).toContain("`src/login.rs:2-3`\n\n```rust\n    let token = read_token();\n    keep(token);\n```\n\nWhere the token is kept.");
    expect(markdown).toContain("```mermaid\ngraph TD");

    const listed = await board(["list"]);
    expect(listed.ok, JSON.stringify(listed)).toBe(true);
    expect(listed.result!.boards).toEqual([expect.objectContaining({ id: "board-1", title: "Login flow", state: "open", revision: 2 })]);

    // A quote of a credential file is refused before anything is sent.
    const secret = await board(["new", "--title", "Secret", "--json", JSON.stringify([{ type: "quote", path: ".env", start: 1, end: 1 }])]);
    expect(secret).toMatchObject({ ok: false, reason: "quote_refused" });
    expect(secret.status).not.toBe(0);
    // The command refused it by its path (the core's refusal would name `blocks[0].path`).
    expect(secret.next_action).toContain("(.env: names a credential file");
    // A bad block is refused with the field to fix named.
    const bad = await board(["new", "--title", "Bad", "--json", JSON.stringify([{ type: "table", columns: ["a", "b"], rows: [["1", "2", "3"]] }])]);
    expect(bad).toMatchObject({ ok: false, reason: "board_invalid" });
    expect(bad.next_action).toContain("blocks[0].rows[0]: 3 cells, the table has 2 columns");

    // Another agent's session reads a board of its checkout but cannot change it.
    const second = await agentPane(engine, daemon.stateDir, other!);
    const theirs = answer(await second(["board", "edit", "board-1", "--json", JSON.stringify([{ op: "remove", id: "b1" }])]));
    expect(theirs).toMatchObject({ ok: false, reason: "board_not_yours" });
    expect(answer(await second(["board", "list"])).result!.boards).toEqual([]);

    // The drawing agent deletes it, and every later command says so.
    const deleted = await board(["delete", "board-1"]);
    expect(deleted.ok, JSON.stringify(deleted)).toBe(true);
    expect(deleted.result!.board).toMatchObject({ id: "board-1", state: "deleted", blocks: [] });
    expect(await board(["edit", "board-1", "--json", JSON.stringify([{ op: "remove", id: "b1" }])])).toMatchObject({ ok: false, reason: "board_deleted" });
    expect(await board(["show", "board-1"])).toMatchObject({ ok: false, reason: "board_deleted" });
  } finally {
    daemon?.stop();
    engine.stop();
  }
});

const front = (page: Page) => page.locator('[data-view-tab-bar] [role="tab"][aria-selected="true"]');

test("a board the operator looked at shows the agent's later edit in place", async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 1000 });
  const engine = await startFixture({ agents: false });
  let daemon: Daemon | null = null;
  try {
    const boardFile = writeBoardFiles(engine);
    daemon = await startHided(engine, "board-view");
    const [pane, other] = engine.panes;
    const agent = await agentPane(engine, daemon.stateDir, pane!);
    const board = async (args: string[]) => answer(await agent(["board", ...args]));
    await page.goto(`${daemon.origin}/?probe=1#token=${daemon.token}`);
    await enterWorkspace(page, "fixture");
    const row = page.locator(`nav[data-sidebar] [data-pane="${pane}"]`).first();
    const mark = row.locator("[data-agent-status-mark]");

    // The operator is in the other pane, so the agent's board lands behind them and waits as To look at.
    await page.locator(`[data-pane-view="${other}"] .xterm-screen`).click();
    await expect(page.locator(`[data-pane-view="${other}"]`)).toHaveAttribute("data-focused", "true");
    const drawn = await board(["new", "--title", "Login flow", "--file", boardFile]);
    expect(drawn.ok, JSON.stringify(drawn)).toBe(true);
    expect(drawn.result!.view).toMatchObject({ open: true, unseen: true });
    const view = drawn.result!.view!.id!;
    await expect(mark).toHaveAttribute("data-agent-status-mark", "look");
    await expect(row).toContainText("Login flow");

    // The operator looks: the row brings the board forward, and it is seen.
    await row.locator(`[data-agent-open="${pane}"]`).click();
    await expect(front(page)).toHaveAttribute("aria-label", /Login flow/);
    await expect(mark).not.toHaveAttribute("data-agent-status-mark", "look");
    const first = page.locator('[data-board-view="board-1"] [data-board-block="b1"]');
    await expect(first).toContainText("The first text.");
    await expect(page.locator('[data-board-view="board-1"] [data-board-block="b3"] [data-board-quote="src/login.rs:2-3"]')).toBeVisible();

    // The agent edits after the look: the block's text changes where it is, with no click, reload or reopen.
    await page.evaluate(() => { (window as unknown as { boardProbe?: boolean }).boardProbe = true; });
    const edited = await board(["edit", "board-1", "--json", JSON.stringify([{ op: "update", id: "b1", changes: { text: "Second text" } }])]);
    expect(edited.ok, JSON.stringify(edited)).toBe(true);
    expect(edited.result!.view).toMatchObject({ id: view, open: true, unseen: false });
    await expect(first).toContainText("Second text");
    await expect(first).not.toContainText("The first text.");
    expect(await page.evaluate(() => (window as unknown as { boardProbe?: boolean }).boardProbe), "the page was not loaded again").toBe(true);
    await expect(front(page)).toHaveAttribute("aria-label", /Login flow/);
    await expect(mark).not.toHaveAttribute("data-agent-status-mark", "look");

    // The View is still listed after the look, and the record says what the screen says.
    const listed = await agent(["view", "list"]);
    expect(listed.status, listed.out).toBe(0);
    expect(listed.out).toContain(`"view_id":"${view}"`);
    expect(listed.out).toMatch(/"kind":"board"[^}]*"target":"board-1"/);
    const shown = await board(["show", "board-1"]);
    expect(shown.result!.markdown).toContain("## What changes\n\nSecond text");
    await screenshot(page, "board-view");
  } finally {
    daemon?.stop();
    engine.stop();
  }
});
