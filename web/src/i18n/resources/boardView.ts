import type { Catalog, Catalogs } from "../schema";

// A board View's words (board design, section 3). `board.` is the issue board's.
export const boardViewEnglish = {
  "boardView.tone.info": "Note",
  "boardView.tone.decision": "Decision",
  "boardView.tone.risk": "Risk",
  "boardView.changed": "Just changed",
  "boardView.waiting": "Waiting for the board…",
  "boardView.deleted": "This board was deleted.",
} as const;

const ko = {
  "boardView.tone.info": "참고",
  "boardView.tone.decision": "결정",
  "boardView.tone.risk": "위험",
  "boardView.changed": "방금 바뀜",
  "boardView.waiting": "보드를 기다리는 중…",
  "boardView.deleted": "지워진 보드예요.",
} satisfies Catalog<typeof boardViewEnglish>;

const zhCN = {
  "boardView.tone.info": "备注",
  "boardView.tone.decision": "决定",
  "boardView.tone.risk": "风险",
  "boardView.changed": "刚刚更改",
  "boardView.waiting": "正在等待白板…",
  "boardView.deleted": "此白板已删除。",
} satisfies Catalog<typeof boardViewEnglish>;

const ja = {
  "boardView.tone.info": "メモ",
  "boardView.tone.decision": "決定",
  "boardView.tone.risk": "リスク",
  "boardView.changed": "今変更",
  "boardView.waiting": "ボードを待っています…",
  "boardView.deleted": "このボードは削除されました。",
} satisfies Catalog<typeof boardViewEnglish>;

export const boardViewCatalogs = { en: boardViewEnglish, ko, "zh-CN": zhCN, ja } satisfies Catalogs<typeof boardViewEnglish>;
