// Invented boards for the board View's gallery scene and its tests. Each
// block's `markdown` is written here the way section 4 of the board design
// (The export) says the core's `boards::markdown` writes it, because the View
// draws that output unchanged; this is a fixture's copy of the table, not a
// second exporter the app uses.

import type { BoardBlock, BoardCalloutTone, BoardView } from "../boards";

type Body =
  | { kind: "section"; title: string; text: string }
  | { kind: "callout"; tone: BoardCalloutTone; title?: string; text: string }
  | { kind: "table"; title?: string; columns: string[]; rows: string[][] }
  | { kind: "code"; title?: string; language?: string; code: string }
  | { kind: "quote"; path: string; start: number; end: number; lines: string[]; note?: string }
  | { kind: "diagram"; title?: string; mermaid: string }
  | { kind: "divider" };

const ALERT: Record<BoardCalloutTone, string> = { info: "NOTE", decision: "IMPORTANT", risk: "WARNING" };

/** The language a quoted file's extension names, for its fence. */
const LANGUAGE: Record<string, string> = { rs: "rust", ts: "typescript", tsx: "tsx", js: "javascript", py: "python", json: "json", md: "markdown", toml: "toml", sh: "sh" };

/** A fence longer than any run of backticks inside the block. */
function fenced(code: string, language = ""): string {
  const longest = Math.max(0, ...(code.match(/`+/g) ?? []).map((run) => run.length));
  const fence = "`".repeat(Math.max(3, longest + 1));
  return `${fence}${language}\n${code}\n${fence}`;
}

const titled = (title: string | undefined, rest: string) => (title ? `**${title}**\n\n${rest}` : rest);
const cell = (text: string) => text.replaceAll("|", "\\|");

/** One block's Markdown, as section 4's table writes it. */
export function exportBlock(body: Body): string {
  switch (body.kind) {
    case "section":
      return `## ${body.title}\n\n${body.text}`;
    case "callout": {
      const lines = [`[!${ALERT[body.tone]}]`, ...(body.title ? [`**${body.title}**`, ""] : []), ...body.text.split("\n")];
      return lines.map((line) => (line === "" ? ">" : `> ${line}`)).join("\n");
    }
    case "table": {
      const row = (cells: string[]) => `| ${cells.map(cell).join(" | ")} |`;
      return titled(body.title, [row(body.columns), row(body.columns.map(() => "---")), ...body.rows.map(row)].join("\n"));
    }
    case "code":
      return titled(body.title, fenced(body.code, body.language));
    case "quote": {
      const extension = body.path.split(".").at(-1) ?? "";
      const parts = [`\`${body.path}:${body.start}-${body.end}\``, fenced(body.lines.join("\n"), LANGUAGE[extension] ?? "")];
      return (body.note ? [...parts, body.note] : parts).join("\n\n");
    }
    case "diagram":
      return titled(body.title, fenced(body.mermaid, "mermaid"));
    case "divider":
      return "---";
  }
}

/** A block on the wire: its id, the revision that last changed it, and its export. */
export function boardBlock(id: string, changedAt: number, body: Body): BoardBlock {
  return {
    id,
    kind: body.kind,
    ...(body.kind === "callout" ? { tone: body.tone } : {}),
    ...(body.kind === "quote" ? { quote: { path: body.path, start: body.start, end: body.end } } : {}),
    changed_at: changedAt,
    markdown: exportBlock(body),
  };
}

/** A board showing every block kind, and a diagram Mermaid cannot read. */
export function referenceBoard(now: number, revision = 7): BoardView {
  const edited = revision > 7;
  return {
    id: "board-3",
    title: "로그인 흐름 정리: 토큰 갱신을 한 곳으로 모으기",
    state: "open",
    revision,
    updated_at_unix_ms: now - 4 * 60_000,
    blocks: [
      boardBlock("b1", 1, {
        kind: "section",
        title: "지금 무엇이 문제인가",
        text: "토큰 갱신이 **세 군데**에서 따로 일어나요. 화면이 둘 이상 열려 있으면 같은 순간에 두 번 갱신하고, 늦게 끝난 쪽이 먼저 받은 토큰을 덮어써요.\n\n- 로그인 화면의 `refresh()`\n- API 클라이언트의 재시도\n- 백그라운드 동기화",
      }),
      boardBlock("b2", 2, { kind: "callout", tone: "info", text: "이 보드는 `auth/` 아래 코드만 다뤄요. 결제 쪽 토큰은 따로 정리할게요." }),
      boardBlock("b3", edited ? 8 : 3, {
        kind: "callout",
        tone: "decision",
        title: "갱신은 세션 관리자 한 곳에서만 해요",
        text: edited
          ? "다른 두 곳은 갱신을 요청만 하고, 진행 중인 갱신이 있으면 그 결과를 기다려요. 실패하면 한 번만 다시 시도해요."
          : "다른 두 곳은 갱신을 요청만 하고, 진행 중인 갱신이 있으면 그 결과를 기다려요.",
      }),
      boardBlock("b4", 3, { kind: "callout", tone: "risk", title: "배포 순서", text: "새 클라이언트가 먼저 나가면 옛 서버가 `409`를 돌려줘요. 서버를 먼저 배포해야 해요." }),
      boardBlock("b5", edited ? 8 : 4, {
        kind: "table",
        title: "갱신이 일어나는 곳",
        columns: ["위치", "언제", "바꾼 뒤"],
        rows: [
          ["`LoginScreen.refresh`", "화면이 열릴 때", "관리자에게 요청"],
          ["`ApiClient.retry`", "401을 받았을 때", "관리자에게 요청"],
          ["`SyncWorker`", "5분마다 | 앱이 깨어날 때", edited ? "그대로 두되 요청만" : "관리자에게 요청"],
        ],
      }),
      boardBlock("b6", 5, {
        kind: "code",
        title: "요청하는 쪽의 모양",
        language: "typescript",
        code: "export async function freshToken(manager: SessionManager): Promise<Token> {\n  // 진행 중인 갱신이 있으면 그 약속을 같이 기다려요.\n  return manager.inflight ?? manager.refresh();\n}",
      }),
      boardBlock("b7", 5, {
        kind: "quote",
        path: "web/src/auth/sessionManager.ts",
        start: 41,
        end: 47,
        lines: ["  async refresh(): Promise<Token> {", "    if (this.inflight) return this.inflight;", "    this.inflight = this.api.refresh(this.token)", "      .finally(() => {", "        this.inflight = null;", "      });", "    return this.inflight;"],
        note: "`inflight`를 비우는 일은 `finally`가 맡아서, 실패한 갱신도 다음 요청을 막지 않아요.",
      }),
      boardBlock("b8", 6, {
        kind: "diagram",
        title: "바꾼 뒤의 흐름",
        mermaid: "flowchart LR\n  L[로그인 화면] -->|요청| M[세션 관리자]\n  A[API 클라이언트] -->|요청| M\n  S[동기화] -->|요청| M\n  M -->|한 번만| T[토큰 서버]",
      }),
      boardBlock("b9", 6, { kind: "divider" }),
      boardBlock("b10", 6, { kind: "diagram", title: "읽지 못하는 그림", mermaid: "flowchart LR\n  A --> \n  --> B[" }),
      boardBlock("b11", 6, {
        kind: "section",
        title: "다음에 할 일",
        text: "1. 서버에 동시 갱신 검사를 넣어요.\n2. 클라이언트의 세 곳을 관리자 요청으로 바꿔요.\n3. 화면 둘을 열고 갱신이 한 번만 일어나는지 확인해요.",
      }),
    ],
  };
}
