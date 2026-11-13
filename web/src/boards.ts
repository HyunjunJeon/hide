// A board as it arrives on the wire, `rest.boards.items[]` (board design,
// section 3, The wire), and the few decisions the board View makes from it.
// The core makes each block's `markdown` with the one export the kept file
// and Hide AI read too (section 4); the web adds only a block's frame, so
// nothing here builds Markdown, and nothing reads a title out of it.

/** The board's record state; a kept board is read-only, a deleted one is a stub. */
export type BoardState = "open" | "kept" | "deleted";

export type BoardBlockKind = "section" | "callout" | "table" | "code" | "quote" | "diagram" | "divider";

/** A callout's tone: `decision` is the one a person must not miss. */
export type BoardCalloutTone = "info" | "decision" | "risk";

/** The lines of a checkout file a `quote` block shows, both ends included. */
export type BoardQuoteRef = { path: string; start: number; end: number };

export type BoardBlock = {
  /** `b<n>`, never reused within its board. */
  id: string;
  kind: BoardBlockKind;
  /** Present on a callout only. */
  tone?: BoardCalloutTone;
  /** Present on a quote only. */
  quote?: BoardQuoteRef;
  /** The board revision that last changed this block. */
  changed_at: number;
  /** The block's body as section 4's export writes it. */
  markdown: string;
};

/** Where the operator's keep of a board stands (`board_keep_state`). */
export type BoardKeepState = "running" | "failed";

/** Why a keep failed (`board_keep_failure`). */
export type BoardKeepFailure = "keep_path_refused" | "keep_exists" | "keep_busy" | "keep_unknown" | "keep_failed";

/** The operator's keep of a board, while one runs or after one failed. */
export type BoardKeep = {
  /** The path below the board's checkout it is kept at. */
  path: string;
  state: BoardKeepState;
  code?: BoardKeepFailure;
  /** The host's or the rule's words for the failure. */
  reason?: string;
};

export type BoardView = {
  /** `board-<n>`. */
  id: string;
  title: string;
  state: BoardState;
  /** Moves by one with each accepted command that changed the board. */
  revision: number;
  updated_at_unix_ms: number;
  blocks: BoardBlock[];
  /** The operator's keep of it, while one runs or after one failed. */
  keep?: BoardKeep;
};

/** `rest.boards`: the boards a display of the front Workspace shows. */
export type BoardsRest = { items: BoardView[] };

/** The board a display names, as `rest.boards` carries it; null until the store has it. */
export function boardOf(boards: BoardsRest | undefined, id: string | undefined): BoardView | null {
  if (!id) return null;
  return boards?.items.find((board) => board.id === id) ?? null;
}

/**
 * A board View's state from its display's (`display_state`: `open`, or
 * `unavailable` once the board was deleted) and the store: open only once
 * the store has the board too, so a first frame never draws an empty board.
 */
export function boardDisplayState(state: "open" | "opening" | "waiting" | "unavailable", board: BoardView | null): "open" | "waiting" | "unavailable" {
  if (state === "unavailable") return "unavailable";
  return state === "open" && board ? "open" : "waiting";
}

/**
 * Where `파일로 남기기` offers to keep a board (board design, section 4):
 * `docs/boards/<YYYY-MM-DD>-<slug>.md`, the date the operator's own, the slug
 * the title's ASCII words in lower case joined by `-`, or the board's id when
 * the title has none. The core checks whatever path the operator sends.
 */
export function defaultKeepPath(board: { id: string; title: string }, now: Date): string {
  const two = (value: number) => String(value).padStart(2, "0");
  const date = `${now.getFullYear()}-${two(now.getMonth() + 1)}-${two(now.getDate())}`;
  const words = board.title.toLowerCase().match(/[a-z0-9]+/g) ?? [];
  const slug = words.join("-").slice(0, 80).replace(/-+$/, "") || board.id;
  return `docs/boards/${date}-${slug}.md`;
}

/** The board and revision a View last drew: the web's own memory, never the core's (section 3, The edit). */
export type DrawnBoard = { id: string; revision: number };

/**
 * The blocks an edit changed since the View last drew this board, to be
 * marked for a moment. A first draw, a different board in the same View, or a
 * revision that did not move marks nothing: only an edit the person could
 * have missed while looking is news.
 */
export function changedBlocks(board: BoardView, drawn: DrawnBoard | null): string[] {
  if (!drawn || drawn.id !== board.id || board.revision <= drawn.revision) return [];
  return board.blocks.filter((block) => block.changed_at > drawn.revision).map((block) => block.id);
}

/** The GitHub alert line a callout's export opens with (`> [!NOTE]`, `> [!IMPORTANT]`, `> [!WARNING]`). */
const ALERT = /^>[ \t]?\[![A-Za-z]+\][ \t]*$/;

/**
 * A callout's content without its alert line and quote marks, so its box
 * draws it as text rather than as a quotation; the tone comes from the wire.
 * Markdown not in that shape is drawn as it came.
 */
export function calloutBody(markdown: string): string {
  const lines = markdown.replace(/\n+$/, "").split("\n");
  if (!ALERT.test(lines[0] ?? "")) return markdown;
  const rest = lines.slice(1);
  if (!rest.every((line) => line.startsWith(">"))) return markdown;
  return rest.map((line) => line.replace(/^>[ \t]?/, "")).join("\n").replace(/^\n+/, "");
}

/** How a quote's caption names its lines: `path:start–end`, or `path:start` for one line. */
export function quoteCaption(quote: BoardQuoteRef): string {
  return quote.start === quote.end ? `${quote.path}:${quote.start}` : `${quote.path}:${quote.start}–${quote.end}`;
}

/**
 * A quote's Markdown without the `` `path:start-end` `` line the export opens
 * it with, because its caption already says that; a first line naming other
 * lines is kept, so nothing the export wrote is lost.
 */
export function quoteBody(markdown: string, quote: BoardQuoteRef | undefined): string {
  if (!quote) return markdown;
  const newline = markdown.indexOf("\n");
  const first = newline === -1 ? markdown : markdown.slice(0, newline);
  if (first.trim() !== `\`${quote.path}:${quote.start}-${quote.end}\``) return markdown;
  return newline === -1 ? "" : markdown.slice(newline + 1).replace(/^\n+/, "");
}
