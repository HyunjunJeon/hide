import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { boardDisplayState, boardOf, calloutBody, changedBlocks, defaultKeepPath, quoteBody, quoteCaption, type BoardBlockKind, type BoardCalloutTone, type BoardKeepFailure, type BoardKeepState, type BoardState, type BoardView } from "./boards";
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
    const keeps: Record<BoardKeepState, true> = { running: true, failed: true };
    const failures: Record<BoardKeepFailure, true> = { keep_path_refused: true, keep_exists: true, keep_busy: true, keep_unknown: true, keep_failed: true };
    expect(Object.keys(keeps)).toEqual(contract.board_keep_state);
    expect(Object.keys(failures)).toEqual(contract.board_keep_failure);
  });

  it("counts two Views the same content only when they show one board", () => {
    const display = (id: string, board?: string): ViewDisplaySnapshot => ({ id, tab_id: null, path: "", label: "Login flow", kind: board ? "board" : "browser", committed: null, preview: false, state: "open", reason: null, board });
    expect(showsSameDocument(display("d1", "board-1"), display("d2", "board-1"))).toBe(true);
    expect(showsSameDocument(display("d1", "board-1"), display("d2", "board-2"))).toBe(false);
    expect(showsSameDocument(display("d1", "board-1"), display("d2"))).toBe(false);
  });
});

describe("where 파일로 남기기 offers to keep a board (board design, section 4)", () => {
  const on = new Date(2026, 10, 3, 23, 59);
  const keep = (title: string) => defaultKeepPath({ id: "board-4", title }, on);
  const name = (path: string) => path.slice("docs/boards/".length);
  it("is docs/boards/, the operator's date, then the title's words in lower case", () => {
    expect(keep("Login flow: v2 (draft)")).toBe("docs/boards/2026-11-03-login-flow-v2-draft.md");
  });
  it("keeps a Korean title's own letters, and Korean mixed with English", () => {
    expect(keep("로그인 흐름 정리: 토큰 갱신")).toBe("docs/boards/2026-11-03-로그인-흐름-정리-토큰-갱신.md");
    expect(keep("API 설계 v2")).toBe("docs/boards/2026-11-03-api-설계-v2.md");
  });
  it("keeps Japanese and Chinese as written, composed (NFC)", () => {
    expect(keep("ログイン設計")).toBe("docs/boards/2026-11-03-ログイン設計.md");
    expect(keep("登录 流程")).toBe("docs/boards/2026-11-03-登录-流程.md");
    const decomposed = "로그인".normalize("NFD");
    expect(keep(decomposed)).toBe("docs/boards/2026-11-03-로그인.md");
  });
  it("falls back to the board's id when no letter or digit is left", () => {
    expect(keep("🚀✨")).toBe("docs/boards/2026-11-03-board-4.md");
    expect(keep("...")).toBe("docs/boards/2026-11-03-board-4.md");
  });
  it("leaves nothing a Windows, macOS or Linux file name refuses", () => {
    const path = keep('a<b>c:d"e/f\\g|h?i*j\u0001k. ');
    expect(path).toBe("docs/boards/2026-11-03-a-b-c-d-e-f-g-h-i-j-k.md");
    expect(name(path)).not.toMatch(/[<>:"/\\|?*]|[. ]\.md$/);
    expect([...name(path)].some((letter) => letter.charCodeAt(0) < 0x20)).toBe(false);
  });
  it("cuts a long title between two characters, within 255 bytes for the name and 300 characters for the path", () => {
    for (const title of ["갱".repeat(200), "word ".repeat(80), "가a".repeat(150)]) {
      const path = keep(title);
      expect(new TextEncoder().encode(name(path)).length).toBeLessThanOrEqual(255);
      expect([...path].length).toBeLessThanOrEqual(300);
      expect(path).toMatch(/^docs\/boards\/2026-11-03-[^-].*[^-]\.md$/);
      expect(path.includes("\ufffd")).toBe(false);
    }
    // 241 bytes are left for the slug: 80 Hangul syllables of three bytes.
    expect(name(keep("갱".repeat(200)))).toBe(`2026-11-03-${"갱".repeat(80)}.md`);
  });
});
