// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BOARD_CHANGE_MARK_MS, BoardView, type BoardDisplayState } from "./BoardView";
import type { BoardView as Board } from "./boards";
import { TooltipProvider } from "./components/ui/tooltip";
import { clientI18n } from "./i18n/translator";
import { referenceBoard } from "./gallery/boardSceneData";

// The shell imports xterm, whose canvas API is absent from jsdom.
vi.hoisted(() => { HTMLCanvasElement.prototype.getContext = () => null; });

// Mermaid needs a page's colours and layout, which jsdom has not: the library
// stands in with its two answers, a picture at its natural width, or the parse
// failure the fixture's unreadable diagram gets.
vi.mock("mermaid", () => ({
  default: {
    initialize: () => undefined,
    render: (_id: string, source: string) => source.trimEnd().endsWith("B[")
      ? Promise.reject(new Error("Parse error on line 3:\n--> B["))
      : Promise.resolve({ svg: '<svg style="max-width: 40em"></svg>' }),
  },
}));

const NOW = 1_794_000_000_000;
const savedLanguage = clientI18n.language;
let container: HTMLDivElement;
let root: Root;

beforeEach(async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  await clientI18n.changeLanguage("en");
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  act(() => root.unmount());
  vi.useRealTimers();
  vi.unstubAllGlobals();
  await clientI18n.changeLanguage(savedLanguage);
  document.body.innerHTML = "";
});

/** Draws and lets the diagrams' answers land, as a frame of the page would. */
async function draw(board: Board | null, state: BoardDisplayState = "open", onCloseView = () => undefined) {
  await act(async () => root.render(<TooltipProvider><BoardView board={board} state={state} label="Login flow" onCloseView={onCloseView} /></TooltipProvider>));
}

const blockIds = () => [...container.querySelectorAll<HTMLElement>("[data-board-block]")].map((block) => block.dataset.boardBlock);
const marked = () => [...container.querySelectorAll<HTMLElement>("[data-board-changed]")].map((block) => block.dataset.boardBlock);

describe("BoardView", () => {
  it("names every block, of every kind, by its id", async () => {
    const board = referenceBoard(NOW);
    await draw(board);
    expect(blockIds()).toEqual(board.blocks.map((block) => block.id));
    const kinds = [...container.querySelectorAll<HTMLElement>("[data-board-block]")].map((block) => block.dataset.boardKind);
    expect(new Set(kinds)).toEqual(new Set(["section", "callout", "table", "code", "quote", "diagram", "divider"]));
  });

  it("marks for a moment only the blocks an edit changed after the revision last drawn", async () => {
    await draw(referenceBoard(NOW, 7));
    expect(marked()).toEqual([]);
    // The fixture's edit to revision 8 changes the decision and the table.
    await draw(referenceBoard(NOW, 8));
    expect(marked()).toEqual(["b3", "b5"]);
    expect(container.querySelector('[data-board-block="b3"]')?.textContent).toContain("Just changed");
    // The same revision drawn again from a new snapshot is no news and keeps the moment.
    await draw(referenceBoard(NOW, 8));
    act(() => vi.advanceTimersByTime(BOARD_CHANGE_MARK_MS - 1));
    expect(marked()).toEqual(["b3", "b5"]);
    act(() => vi.advanceTimersByTime(1));
    expect(marked()).toEqual([]);
  });

  it("ends the moment on time when a later edit within it changes no block", async () => {
    await draw(referenceBoard(NOW, 7));
    await draw(referenceBoard(NOW, 8));
    act(() => vi.advanceTimersByTime(BOARD_CHANGE_MARK_MS / 2));
    // A `remove` is an edit that moves the revision and changes no block left.
    const removed = referenceBoard(NOW, 9);
    await draw({ ...removed, blocks: removed.blocks.filter((block) => block.id !== "b11") });
    expect(marked()).toEqual(["b3", "b5"]);
    act(() => vi.advanceTimersByTime(BOARD_CHANGE_MARK_MS / 2));
    expect(marked()).toEqual([]);
  });

  it("ends the moment on time across a wait for the board", async () => {
    await draw(referenceBoard(NOW, 7));
    await draw(referenceBoard(NOW, 8));
    await draw(null, "waiting");
    await draw(referenceBoard(NOW, 8));
    expect(marked()).toEqual(["b3", "b5"]);
    act(() => vi.advanceTimersByTime(BOARD_CHANGE_MARK_MS));
    expect(marked()).toEqual([]);
  });

  it("marks nothing when the View draws another board", async () => {
    await draw(referenceBoard(NOW, 7));
    await draw({ ...referenceBoard(NOW, 8), id: "board-4" });
    expect(marked()).toEqual([]);
  });

  it("draws a callout in its tone's box without the alert's marks, and a quote's place once", async () => {
    await draw(referenceBoard(NOW));
    const decision = container.querySelector<HTMLElement>('[data-board-block="b3"] [data-board-callout]');
    expect(decision?.dataset.boardCallout).toBe("decision");
    expect(decision?.textContent).toContain("Decision");
    expect(decision?.textContent).toContain("갱신은 세션 관리자 한 곳에서만 해요");
    expect(decision?.textContent).not.toContain("[!IMPORTANT]");
    expect(decision?.querySelector("blockquote")).toBeNull();
    const quote = container.querySelector<HTMLElement>('[data-board-block="b7"]');
    expect(quote?.querySelector("[data-board-quote]")?.getAttribute("data-board-quote")).toBe("web/src/auth/sessionManager.ts:41-47");
    expect(quote?.textContent).not.toContain("sessionManager.ts:41-47`");
    expect(quote?.textContent?.split("sessionManager.ts").length).toBe(2);
  });

  it("draws a readable diagram as its picture and an unreadable one as its source under why", async () => {
    await draw(referenceBoard(NOW));
    // The whole picture shows at a glance: it shrinks with its column and is never wider (operator's decision).
    const picture = container.querySelector<SVGSVGElement>('[data-board-block="b8"] [data-markdown-diagram="mermaid"] svg');
    expect(picture?.style.maxWidth).toBe("40em");
    expect(picture?.style.minWidth).toBe("");
    const unreadable = container.querySelector('[data-board-block="b10"]');
    expect(unreadable?.querySelector("[data-markdown-diagram]")).toBeNull();
    expect(unreadable?.textContent).toContain("Diagram not drawn: Parse error on line 3");
    expect(unreadable?.querySelector("pre")?.textContent).toContain("flowchart LR");
  });

  it("draws a table as a table, with the escaped pipe as text", async () => {
    await draw(referenceBoard(NOW));
    const table = container.querySelector('[data-board-block="b5"] table');
    expect(table?.querySelectorAll("tr")).toHaveLength(4);
    expect(table?.textContent).toContain("5분마다 | 앱이 깨어날 때");
  });

  it("waits with no blocks, and offers Close view once the board was deleted", async () => {
    await draw(null, "waiting");
    expect(blockIds()).toEqual([]);
    expect(container.querySelector('[data-area-empty="board-waiting"]')?.textContent).toBe("Waiting for the board…");
    const close = vi.fn();
    await draw(referenceBoard(NOW), "unavailable", close);
    expect(blockIds()).toEqual([]);
    expect(container.querySelector('[data-area-empty="board-unavailable"]')?.textContent).toContain("This board was deleted.");
    act(() => container.querySelector<HTMLButtonElement>("[data-board-close]")!.click());
    expect(close).toHaveBeenCalledTimes(1);
  });
});
