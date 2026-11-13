# hide board
Draw a board for the operator: a title and blocks (an explanation, a callout for a decision or a risk, a table, code you write, code quoted from a file of your checkout, one Mermaid diagram, a divider). Hide keeps it for your session, so it outlives a restart; you change it block by block, and only your session changes it. It opens as a board View in the operator's View area and waits in their Needs You as To look at until they look; what you change afterwards reaches their screen in place, even after they looked.

```
hide board new --title <title> (--file <board.json> | --json <blocks>) [--no-open]
hide board open <board>
hide board set <board> (--file <board.json> | --json <blocks>) [--title <title>] [--revision <n>]
hide board edit <board> (--file <edits.json> | --json <edits>) [--revision <n>]
hide board show [<board>] [--json]        hide board list        hide board delete <board>        hide board help
```

Write the blocks to a file and pass `--file`: a board passed with `--json` can outgrow a command line, and PowerShell changes its quotes. The file holds the blocks as a JSON array (or `{"blocks": [...]}`):

```json
[{"type":"section","title":"What changes","text":"The token moves to the core. *Markdown* works here."},
 {"type":"callout","tone":"decision","title":"Store","text":"One file per board."},
 {"type":"table","title":"Limits","columns":["Limit","Value"],"rows":[["Blocks","60"],["Size","64 KiB"]]},
 {"type":"code","language":"rust","code":"fn main() {}"},
 {"type":"quote","path":"src/login.rs","start":10,"end":24,"note":"Where the token is read."},
 {"type":"diagram","mermaid":"graph TD\n  A[Sign in] --> B[Token]"},
 {"type":"divider"}]
```

- Blocks and their fields: `section` {`title`, `text`}; `callout` {`tone`: `info`|`decision`|`risk`, `title`?, `text`}; `table` {`title`?, `columns` (1 to 8), `rows` (0 to 40, each as wide as `columns`, one line per cell)}; `code` {`title`?, `language`?, `code`}; `quote` {`path`, `start`, `end`, `note`?}; `diagram` {`title`?, `mermaid`}; `divider` {}. A field Hide does not know is refused, not dropped.
- A quote names a file of your checkout by its path relative to the checkout's root and lines `start` to `end` (from 1, at most 80 lines). `hide board` reads those lines itself, on the machine that holds the file, and the board keeps them as they were when you drew it. A path outside the checkout, with `..`, in `.git`, or naming a credential file (`.env`, keys, secrets) is refused as `quote_refused` before anything is sent.
- Limits: a title is one line of at most 120 characters; a section's text 4000 characters, a callout's 1000; code 8000 characters, 120 lines of at most 240; a diagram 8000 characters; a board 60 blocks and 64 KiB; your session 16 open boards. A refusal names the field to fix, such as `blocks[2].rows[3]: 9 cells, the table has 8 columns`.
- `new` answers the board's id (`board-<n>`), its revision and each block's id (`b1`, `b2`, ...). A block keeps its id for good, and a removed block's id is never given again.
- `new` opens the board's View as `hide file open` opens a file, unless `--no-open`: its answer's `view` has the View's id and `unseen` while the operator has still to look. A View that cannot open now (`view_limit`) says why there, and the board is kept. `open <board>` brings the View forward, or opens it again once it was closed; like any open of yours, it makes a board the operator already saw To look at again. A board a delegated child drew turns only you unread: to show it to the operator, open it from your own pane with `hide board open <id>` (its id is in the child's report), and it waits in their Needs You as yours to look at; only the drawing session and its ancestors open a board. An `edit` or a `set` never does: their answers carry the View's `unseen`, so you know whether the operator has looked.
- `edit` applies a list of operations, all of them or none, as one revision; the file holds them as a JSON array (or `{"edits": [...]}`):

```json
[{"op":"update","id":"b1","changes":{"text":"The token moves to the core first."}},
 {"op":"insert","block":{"type":"divider"},"after":"b3"},
 {"op":"replace","id":"b4","block":{"type":"code","code":"fn main() { run() }"}},
 {"op":"move","id":"b2","after":"b5"},
 {"op":"remove","id":"b6"}]
```

- `update` changes some fields of one block and keeps its type (`null` takes an optional field away); `replace` puts a whole new block in its place under the same id; `insert` and `move` put the block after `after`, or at the end without it. A quote whose `path`, `start` or `end` changes is replaced, so its lines are read again. At most 20 operations per edit.
- `set` replaces the title (with `--title`) and every block at once; a block that carries `"id": "b3"` keeps that id. Give `--revision <n>` to `set` or `edit` to have it refused as `board_moved` when the board changed since the revision you read.
- `show <board>` prints the board as Markdown (`--json` prints its record); it answers for your own boards, a board drawn in your checkout, and a board a session you started drew. `show` alone prints all of yours; `list` prints each of yours with its state, revision and View.
- `delete <board>` deletes one of your own boards for good: its content goes, its Views close, and every later command on it answers `board_deleted`, as it does when the operator deleted it. A board whose session records are deleted goes with them. `delete` also removes the record of a board the operator kept; its file stays.
- The operator can keep a board as a Markdown file of the checkout they look at it in (for a board your parent showed them, your parent's checkout): it is then read-only, `show` still prints it, and a change answers `board_kept` naming the file, and its checkout when it is not yours, which you edit as a file. While they keep it, a change answers `board_keeping`; run it again in a moment.
- Every answer is one JSON line, `{"ok":true,"result":...}` or `{"ok":false,"reason":...,"next_action":...}`.
