// The board View over invented boards (board design, section 3, Gallery):
// every block kind and a diagram Mermaid cannot read, at any theme, text size
// and lane width, and the View's states. `state=changed` draws the board once
// and then the edit that changed two blocks, holding their mark so a capture
// can see it.

import { useEffect, useLayoutEffect, useMemo, useState } from "react";
import { createActions } from "../actions";
import { BoardView } from "../BoardView";
import { TooltipProvider } from "../components/ui/tooltip";
import { useFollowInterfaceLanguage } from "../i18n/translator";
import { requireInterfaceLanguage } from "../i18n/locale";
import { FONT_SIZE_BASE, FONT_SIZE_MAX } from "../settings";
import { OpenerMark } from "../ViewComments";
import { referenceBoard } from "./boardSceneData";

/** The longest delay a timer takes; a longer one fires at once. */
const HELD_MARK_MS = 2 ** 31 - 1;

type SceneState = "open" | "changed" | "waiting" | "unavailable";

export function boardSceneParams(params: URLSearchParams) {
  const state = params.get("state");
  return {
    theme: params.get("theme") === "light" ? ("light" as const) : ("dark" as const),
    language: requireInterfaceLanguage(params.get("lang") ?? "ko"),
    // `scale=largest` is the largest interface text size Settings offers.
    scale: params.get("scale") === "largest" ? FONT_SIZE_MAX / FONT_SIZE_BASE : Number(params.get("scale") ?? 1),
    width: Number(params.get("width") ?? 760),
    state: (state === "changed" || state === "waiting" || state === "unavailable" ? state : "open") as SceneState,
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
  const board = useMemo(() => referenceBoard(now, revision), [now, revision]);
  const actions = useMemo(() => createActions(() => true), []);
  useLayoutEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    document.documentElement.style.setProperty("--interface-scale", String(scale));
  }, [theme, scale]);
  const opener = <OpenerMark opener={{ name: "로그인 정리", provider: "claude", device_id: "local", pane_id: "w1:p2", resumable: true }} actions={actions} />;
  return (
    <TooltipProvider>
      <div style={{ width }} className="flex min-h-screen flex-col border-x border-border bg-background text-foreground" data-gallery-scene="board" data-board-scene-state={state}>
        <BoardView
          board={state === "waiting" ? null : board}
          state={state === "changed" ? "open" : state}
          label={board.title}
          opener={opener}
          onCloseView={() => undefined}
          markMs={state === "changed" ? HELD_MARK_MS : undefined}
        />
      </div>
    </TooltipProvider>
  );
}
