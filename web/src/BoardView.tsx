// A board View's body (board design, section 3, How it is drawn): the head,
// then each block from the Markdown the core exported for it, in the frame its
// kind asks for. It reads no store and sends no event; the View that shows it
// hands in the board, its state, the opener's mark, the head's controls and
// Close view, so the gallery draws exactly what the app draws.

import { useEffect, useRef, useState, type ReactNode } from "react";
import { AreaEmpty } from "./AreaEmpty";
import { changedBlocks, calloutBody, quoteBody, quoteCaption, type BoardBlock, type BoardCalloutTone, type BoardView as Board, type DrawnBoard } from "./boards";
import { Badge } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import { Separator } from "./components/ui/separator";
import { Hint } from "./components/ui/tooltip";
import { useElapsed } from "./components/elapsed";
import { FileMark } from "./components/file-mark";
import { StateBox, type StateBoxTone } from "./components/state-box";
import { fileIcon } from "./fileIcons";
import { useInterfaceTranslation } from "./i18n/translator";
import { cn } from "./lib/utils";
import { renderMarkdown } from "./mobile/markdown";

/** How long the blocks an edit changed stay marked. */
export const BOARD_CHANGE_MARK_MS = 2_400;

/** A board display's state, from the core's `display_state`: `waiting` until the store has the board, `unavailable` once it was deleted. */
export type BoardDisplayState = "open" | "waiting" | "unavailable";

// A decision is the box a reader must not miss: edged and tinted in the
// working blue; a risk in the warning colour; a note stays quiet.
const BOX: Record<BoardCalloutTone, StateBoxTone> = { info: "quiet", decision: "working", risk: "attention" };
const BOX_FILL: Record<BoardCalloutTone, string> = { info: "", decision: "bg-agent-working/10", risk: "bg-warning/10" };
const BADGE: Record<BoardCalloutTone, string> = { info: "", decision: "bg-agent-working/15 text-agent-working", risk: "bg-warning/15 text-warning" };

const NONE: ReadonlySet<string> = new Set();

export function BoardView({
  board,
  state,
  label,
  opener,
  controls,
  onCloseView,
  markMs = BOARD_CHANGE_MARK_MS,
}: {
  /** The board as `rest.boards` carries it; null until the store has it. */
  board: Board | null;
  state: BoardDisplayState;
  /** The display's label, the title it was opened with, for the head before the board is in. */
  label: string;
  /** The opener's mark (`OpenerMark`), at the head's end as every View's. */
  opener?: ReactNode;
  /** The head's controls (`파일로 남기기`, `지우기`), before the opener's mark. */
  controls?: ReactNode;
  onCloseView: () => void;
  markMs?: number;
}) {
  const { t } = useInterfaceTranslation();
  const marked = useChangeMark(state === "open" ? board : null, markMs);
  const shown = state === "open" && board ? board : null;
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col bg-background" data-board-view={board?.id ?? ""} data-board-state={shown ? "open" : state}>
      <BoardHead title={shown?.title ?? board?.title ?? label} updatedAt={shown?.updated_at_unix_ms ?? null} controls={shown ? controls : null} opener={opener} />
      {shown ? (
        <div className="min-h-0 flex-1 overflow-y-auto" data-board-body="true">
          <article aria-label={shown.title} className="flex min-w-0 flex-col gap-lg px-lg py-lg text-body text-foreground">
            {shown.blocks.map((block) => <Block key={block.id} block={block} changed={marked.has(block.id)} />)}
          </article>
        </div>
      ) : state === "unavailable" ? (
        <AreaEmpty state="board-unavailable" text={t("boardView.deleted")}>
          <Button variant="secondary" data-keyboard-stays="true" onPointerDown={(event) => { event.preventDefault(); event.stopPropagation(); }} onClick={onCloseView} data-board-close="true">
            {t("documents.view.closeView")}
          </Button>
        </AreaEmpty>
      ) : (
        <AreaEmpty state="board-waiting" text={t("boardView.waiting")} />
      )}
    </div>
  );
}

/** The blocks marked and when their moment ends. */
type Mark = { ids: ReadonlySet<string>; until: number };

/**
 * The blocks changed since the revision last drawn, for `markMs`; a later edit
 * within it adds its own and starts the moment again. The moment's end is the
 * mark's own, so nothing but the mark's timer, or another board, takes it
 * down: an edit that changes no block, a wait for the board, or a new
 * `markMs` leaves the running moment to end on time.
 */
function useChangeMark(board: Board | null, markMs: number): ReadonlySet<string> {
  const drawn = useRef<DrawnBoard | null>(null);
  const [mark, setMark] = useState<Mark | null>(null);
  const id = board?.id ?? null;
  const revision = board?.revision ?? null;
  useEffect(() => {
    if (!board) return;
    const changed = changedBlocks(board, drawn.current);
    const sameBoard = drawn.current?.id === board.id;
    drawn.current = { id: board.id, revision: board.revision };
    if (!sameBoard) setMark(null);
    else if (changed.length > 0) setMark((before) => ({ ids: new Set([...(before?.ids ?? []), ...changed]), until: Date.now() + markMs }));
    // The board object is new on every snapshot; only its id and revision are news.
  }, [id, revision]);
  useEffect(() => {
    if (!mark) return;
    const timer = window.setTimeout(() => setMark(null), Math.max(0, mark.until - Date.now()));
    return () => window.clearTimeout(timer);
  }, [mark]);
  return mark?.ids ?? NONE;
}

function BoardHead({ title, updatedAt, controls, opener }: { title: string; updatedAt: number | null; controls: ReactNode; opener: ReactNode }) {
  const updated = useElapsed(updatedAt);
  return (
    <div className="flex h-(--size-tab-strip) shrink-0 items-center gap-sm border-b border-border px-md text-caption text-subtle-foreground" data-board-head="true">
      <Hint label={title} reveals>
        <h2 className="min-w-0 flex-1 truncate text-body font-semibold text-foreground" data-board-title="true">{title}</h2>
      </Hint>
      {updated ? <span className="shrink-0 text-muted-foreground" data-board-updated="true">{updated}</span> : null}
      {controls}
      {opener}
    </div>
  );
}

// The export draws a heading as a bold line the size of the text; on a board a
// section's `##` heads what follows, and a table's header row reads as one.
const PROSE = "flex min-w-0 flex-col gap-sm [&_[data-markdown-heading='1']]:text-headline [&_[data-markdown-heading='2']]:text-title [&_[data-markdown-heading='3']]:text-subhead [&_th]:bg-secondary [&_table]:text-body";

function Block({ block, changed }: { block: BoardBlock; changed: boolean }) {
  const { t } = useInterfaceTranslation();
  return (
    <div
      className={cn(
        "-mx-sm rounded-md px-sm py-xs ring-inset transition-colors duration-[calc(var(--motion-update-pulse)*1s)] ease-out motion-reduce:transition-none",
        changed && "bg-foreground/5 ring-1 ring-foreground/30",
      )}
      data-board-block={block.id}
      data-board-kind={block.kind}
      data-board-changed={changed ? "true" : undefined}
    >
      {/* The mark is the tint and its edge; the words are for a screen reader. */}
      {changed ? <span className="sr-only">{t("boardView.changed")}</span> : null}
      <BlockBody block={block} />
    </div>
  );
}

function BlockBody({ block }: { block: BoardBlock }) {
  const { t } = useInterfaceTranslation();
  switch (block.kind) {
    case "divider":
      return <Separator />;
    case "callout": {
      const tone = block.tone ?? "info";
      return (
        <StateBox tone={BOX[tone]} className={BOX_FILL[tone]} data-board-callout={tone}>
          <Badge className={BADGE[tone]}>{t(`boardView.tone.${tone}`)}</Badge>
          <div className={PROSE}>{renderMarkdown(calloutBody(block.markdown))}</div>
        </StateBox>
      );
    }
    case "quote":
      return (
        <figure className="m-none flex min-w-0 flex-col gap-xs">
          {block.quote ? (
            <figcaption className="flex min-w-0 items-center gap-xs text-caption text-subtle-foreground" data-board-quote={`${block.quote.path}:${block.quote.start}-${block.quote.end}`}>
              <FileMark icon={fileIcon(block.quote.path)} className="size-(--size-icon-sm)" />
              <span className="min-w-0 truncate text-left font-mono [direction:rtl]"><bdi dir="ltr">{quoteCaption(block.quote)}</bdi></span>
            </figcaption>
          ) : null}
          <div className={PROSE}>{renderMarkdown(quoteBody(block.markdown, block.quote))}</div>
        </figure>
      );
    default:
      return <div className={PROSE}>{renderMarkdown(block.markdown)}</div>;
  }
}
