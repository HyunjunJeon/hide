// The board View over invented boards (board design, section 3, Gallery):
// every block kind and a diagram Mermaid cannot read, at any theme, text size
// and lane width, and the View's states. `state=changed` draws the board once
// and then the edit that changed two blocks, holding their mark so a capture
// can see it. The keep and delete states (node `keep`) draw the head's
// controls with the popover or the confirmation open, a keep running, one
// that failed, a kept board, and the session record's board tiles.

import { useEffect, useLayoutEffect, useMemo, useState } from "react";
import { createActions } from "../actions";
import { BoardControls, BoardKeepNotice, BoardView } from "../BoardView";
import { defaultKeepPath, type BoardView as Board } from "../boards";
import { RecordGallery } from "../RecordGallery";
import type { GalleryTile } from "../galleryTiles";
import { TooltipProvider } from "../components/ui/tooltip";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { requireInterfaceLanguage } from "../i18n/locale";
import { FONT_SIZE_BASE, FONT_SIZE_MAX } from "../settings";
import { OpenerMark } from "../ViewComments";
import { referenceBoard } from "./boardSceneData";

/** The longest delay a timer takes; a longer one fires at once. */
const HELD_MARK_MS = 2 ** 31 - 1;

type SceneState = "open" | "changed" | "waiting" | "unavailable" | "keep" | "delete" | "keeping" | "keep-failed" | "kept" | "record";

const STATES: readonly SceneState[] = ["open", "changed", "waiting", "unavailable", "keep", "delete", "keeping", "keep-failed", "kept", "record"];

export function boardSceneParams(params: URLSearchParams) {
  const state = params.get("state");
  return {
    theme: params.get("theme") === "light" ? ("light" as const) : ("dark" as const),
    language: requireInterfaceLanguage(params.get("lang") ?? "ko"),
    // `scale=largest` is the largest interface text size Settings offers.
    scale: params.get("scale") === "largest" ? FONT_SIZE_MAX / FONT_SIZE_BASE : Number(params.get("scale") ?? 1),
    width: Number(params.get("width") ?? 760),
    state: STATES.find((known) => known === state) ?? "open",
  };
}

export function BoardScene({ theme, language, scale, width, state }: ReturnType<typeof boardSceneParams>) {
  useFollowInterfaceLanguage(language);
  const now = useMemo(() => Date.now(), []);
  const [revision, setRevision] = useState(7);
  // The edit lands after the first draw, as it would while the person looks.
  useEffect(() => {
    if (state !== "changed") return;
    const frame = window.requestAnimationFrame(() => setRevision(8));
    return () => window.cancelAnimationFrame(frame);
  }, [state]);
  const reference = useMemo(() => referenceBoard(now, revision), [now, revision]);
  const board: Board = useMemo(() => {
    if (state === "keeping") return { ...reference, keep: { path: KEPT_AT, state: "running" } };
    if (state === "keep-failed") return { ...reference, keep: { path: KEPT_AT, state: "failed", code: "keep_exists", reason: `${KEPT_AT} already holds something else` } };
    if (state === "kept") return { ...reference, state: "kept" };
    return reference;
  }, [reference, state]);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
  }, [theme, scale]);
  const opener = <OpenerMark opener={{ name: "로그인 정리", provider: "claude", device_id: "local", pane_id: "w1:p2", resumable: true }} actions={actions} />;
  const viewState = state === "changed" ? "open" : state === "waiting" || state === "unavailable" ? state : "open";
  if (state === "record") {
    return (
      <TooltipProvider>
        <div style={{ width }} className="flex min-h-screen flex-col gap-sm border-x border-border bg-background p-md text-foreground" data-gallery-scene="board" data-board-scene-state={state}>
          <RecordGallery tiles={recordTiles(now)} omitted={0} actions={actions} />
        </div>
      </TooltipProvider>
    );
  }
  return (
    <TooltipProvider>
      <div style={{ width }} className="flex min-h-screen flex-col border-x border-border bg-background text-foreground" data-gallery-scene="board" data-board-scene-state={state}>
        <BoardView
          board={state === "waiting" ? null : board}
          state={viewState}
          label={board.title}
          opener={opener}
          controls={<BoardControls board={board} defaultPath={defaultKeepPath(board, new Date(now))} onKeep={() => undefined} onDelete={() => undefined} opened={state === "keep" || state === "delete" ? state : undefined} />}
          notice={board.keep?.state === "failed" && board.keep.code ? <BoardKeepNotice code={board.keep.code} path={board.keep.path} onRetry={() => undefined} /> : null}
          onCloseView={() => undefined}
          markMs={state === "changed" ? HELD_MARK_MS : undefined}
        />
      </div>
    </TooltipProvider>
  );
}

const KEPT_AT = "docs/boards/2026-11-13-login-token-refresh.md";

/** A session record's 결과물 with boards: a page, an open board on screen or not, a kept one and a long Korean title. */
function recordTiles(now: number): GalleryTile[] {
  const session = { device_id: "local", agent: "claude", session_id: "gallery" };
  const board = (id: string, title: string, kept?: string): GalleryTile => ({
    kind: "board",
    board: { id, title, state: kept ? "kept" : "open", device_id: "local", checkout_path: "/work/app", updated_at_unix_ms: now, session, ...(kept ? { kept_path: kept } : {}) },
  });
  return [
    { kind: "view", view: { device_id: "local", checkout_path: "/work/app", display_id: "v1", kind: "file", label: "release-notes.md", path: "/work/app/docs/release-notes.md", lines: ["# 1.4 릴리스 노트"], session } },
    board("board-3", "Login token refresh"),
    board("board-2", "세션 만료 뒤 다시 로그인하는 흐름과 토큰을 새로 받는 순서를 한 장에 정리한 보드"),
    board("board-1", "Cache layout", "docs/boards/2026-11-12-cache-layout.md"),
    { kind: "artifact", name: "Benchmark notes" },
  ];
}
