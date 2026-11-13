import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { boardDisplayState, boardOf, calloutBody, changedBlocks, quoteBody, quoteCaption, type BoardBlockKind, type BoardCalloutTone, type BoardState, type BoardView } from "./boards";
import type { ViewDisplaySnapshot } from "./snapshot";
import { showsSameDocument } from "./viewLayout";
import { boardBlock, exportBlock } from "./gallery/boardSceneData";

function board(revision: number, changed: Record<string, number>, id = "board-1"): BoardView {
  return {
    id,
    title: "Login flow",
    state: "open",
    revision,
    updated_at_unix_ms: 0,
    blocks: Object.entries(changed).map(([block, at]) => boardBlock(block, at, { kind: "section", title: block, text: "text" })),
  };
}

describe("which blocks an edit changed (board design, section 3, The edit)", () => {
  it("marks only the blocks changed after the revision last drawn", () => {
    // b1 changed at the revision drawn, the first value that is not news; b2 at the next one.
    expect(changedBlocks(board(6, { b1: 4, b2: 5, b3: 6 }), { id: "board-1", revision: 4 })).toEqual(["b2", "b3"]);
  });

  it("marks nothing on a first draw, for another board, or when the revision did not move", () => {
    const edited = board(6, { b1: 6 });
    expect(changedBlocks(edited, null)).toEqual([]);
    expect(changedBlocks(edited, { id: "board-2", revision: 5 })).toEqual([]);
    expect(changedBlocks(edited, { id: "board-1", revision: 6 })).toEqual([]);
  });
});

describe("a callout's content inside its box", () => {
  it("leaves the alert line and quote marks to the frame, keeping a title and paragraphs", () => {
    const markdown = exportBlock({ kind: "callout", tone: "decision", title: "One place", text: "First line\n\nSecond `code`" });
    expect(markdown).toBe("> [!IMPORTANT]\n> **One place**\n>\n> First line\n>\n> Second `code`");
    expect(calloutBody(markdown)).toBe("**One place**\n\nFirst line\n\nSecond `code`");
    expect(calloutBody(exportBlock({ kind: "callout", tone: "risk", text: "Deploy the server first." }))).toBe("Deploy the server first.");
  });

  it("draws Markdown that is not an alert as it came", () => {
    expect(calloutBody("Plain words")).toBe("Plain words");
    expect(calloutBody("> [!NOTE]\nnot quoted")).toBe("> [!NOTE]\nnot quoted");
  });
});

describe("a quote under its caption", () => {
  const quote = { path: "web/src/auth/session.ts", start: 41, end: 47 };
  const markdown = exportBlock({ kind: "quote", ...quote, lines: ["a()", "b()"], note: "Why it matters." });

  it("drops the export's location line, which the caption says", () => {
    expect(markdown).toBe("`web/src/auth/session.ts:41-47`\n\n```typescript\na()\nb()\n```\n\nWhy it matters.");
    expect(quoteBody(markdown, quote)).toBe("```typescript\na()\nb()\n```\n\nWhy it matters.");
    expect(quoteCaption(quote)).toBe("web/src/auth/session.ts:41–47");
    expect(quoteCaption({ ...quote, end: 41 })).toBe("web/src/auth/session.ts:41");
  });

  it("keeps a first line that names other lines", () => {
    expect(quoteBody(markdown, { ...quote, end: 48 })).toBe(markdown);
    expect(quoteBody(markdown, undefined)).toBe(markdown);
  });
});

describe("a board display fed from rest.boards (board design, section 3, The wire)", () => {
  it("finds its board by id, and none before the store has it", () => {
    const shown = { items: [board(2, { b1: 2 }), board(1, {}, "board-2")] };
    expect(boardOf(shown, "board-2")?.id).toBe("board-2");
    expect(boardOf(shown, "board-3")).toBeNull();
    expect(boardOf(undefined, "board-1")).toBeNull();
    expect(boardOf(shown, undefined)).toBeNull();
  });

  it("is open only once both the core and the store have the board", () => {
    const shown = board(1, {});
    expect(boardDisplayState("open", shown)).toBe("open");
    expect(boardDisplayState("open", null)).toBe("waiting");
    expect(boardDisplayState("waiting", shown)).toBe("waiting");
    expect(boardDisplayState("unavailable", shown)).toBe("unavailable");
  });

  it("decodes every block kind, tone and state the core sends", () => {
    const contract = JSON.parse(fs.readFileSync(path.resolve(__dirname, "../../contracts/snapshot-wire-enums.json"), "utf8"));
    const kinds: Record<BoardBlockKind, true> = { section: true, callout: true, table: true, code: true, quote: true, diagram: true, divider: true };
    const tones: Record<BoardCalloutTone, true> = { info: true, decision: true, risk: true };
    const states: Record<BoardState, true> = { open: true, kept: true, deleted: true };
    expect(Object.keys(kinds)).toEqual(contract.board_block_kind);
    expect(Object.keys(tones)).toEqual(contract.board_callout_tone);
    expect(Object.keys(states)).toEqual(contract.board_state);
  });

  it("counts two Views the same content only when they show one board", () => {
    const display = (id: string, board?: string): ViewDisplaySnapshot => ({ id, tab_id: null, path: "", label: "Login flow", kind: board ? "board" : "browser", committed: null, preview: false, state: "open", reason: null, board });
    expect(showsSameDocument(display("d1", "board-1"), display("d2", "board-1"))).toBe(true);
    expect(showsSameDocument(display("d1", "board-1"), display("d2", "board-2"))).toBe(false);
    expect(showsSameDocument(display("d1", "board-1"), display("d2"))).toBe(false);
  });
});
