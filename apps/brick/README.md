# Brick

A fast, lightweight spreadsheet for Linux, written in Rust. V0.1 goal: open, browse, edit, sort, filter, search, and save ~1 GB CSV files faster than LibreOffice Calc.

## Install

For your user, without sudo (x86_64 Linux with GTK 4.14 or newer: Arch, Ubuntu 24.04, Fedora, and newer). Download page: <https://brick.cronkite.cc>.

```sh
curl -fsSL https://brick.cronkite.cc/install.sh | sh                    # install or update
curl -fsSL https://brick.cronkite.cc/install.sh | sh -s -- --uninstall  # remove
```

It puts `brick` in `~/.local/bin`, adds Brick to the app menu, and makes it the default for CSV and TSV files. The download is checked against the release's sha256 first. Releases before 0.1.1 were called Spreadsheet (`spreadsheet`); installing replaces that one, keeping its recent files.

## Start here

1. Install Rust stable (`rustup`), Python 3, GTK 4.14+ dev files (`gtk4` on Arch, `libgtk-4-dev` on Debian/Ubuntu), Docker (release builds and checks), and Node (for `npx wrangler` when shipping).
2. `cargo test --workspace` should pass on a fresh clone.
3. Paste the product spec into `apps/brick/docs/SPEC.md`.
4. `python3 scripts/gen_corpus.py` builds test files in `corpus/` (about 5 GB at full size; use `--max-mb 100` to start small).
5. Open the repo in Claude Code and say **"next story"**.

## Run it

```sh
cargo run --release -p brick -- path/to/file.csv
cargo run --release -p brick                      # start window with an Open button
```

Ctrl+O (or the folder button in the header bar) opens the desktop's file dialog through xdg-desktop-portal. Each file opens in its own window; opening a file that is already open brings its window forward. Ctrl+N (or New) starts an untitled table. Ctrl+Shift+S (or the Save As button) saves to another file and can change the delimiter (comma, tab, semicolon, pipe) and the encoding (UTF-8 with or without a BOM, UTF-16 LE/BE, Windows-1252); a character the encoding can't hold stops the save with a message, and the window then shows the new file. Ctrl+S on an untitled table asks where to save it.

The grid always shows one empty row below the data (once the file is indexed) and one empty column to its right: type there to add a row or a column. End and Ctrl+End still stop at the last cell with data.

Move the cell cursor like LibreOffice Calc: arrows, Tab/Shift+Tab, Enter/Shift+Enter, Home/End, Ctrl+Home/End, Page Up/Down, and Shift with any of them to select (checklist: `apps/brick/docs/navigation-checklist.md`). Select with the mouse too: click a cell, Shift+click or drag for a range (dragging past an edge scrolls), click or drag row numbers and column letters for whole rows and columns, and the corner for everything. Shift+Space selects rows, Ctrl+Space columns, Shift+Ctrl+Space everything. The mouse wheel and scrollbars scroll. Ctrl+S saves over the file atomically: untouched rows are written byte for byte, and a crash mid-save leaves the original intact. The status bar shows the row count (growing, with a percentage, while the file indexes), unsaved edits and save progress, and the delimiter and encoding; the title marks unsaved edits with •. The delimiter (comma, tab, semicolon, pipe) is detected on open; the dropdown in the header bar overrides it without a restart (save edits first). Encodings: UTF-8 (with or without BOM), UTF-16 LE/BE, and Windows-1252 ("Latin-1") are detected on open and kept on save, BOM included.

Edit in the cell, as in Calc: start typing to replace a cell's content, or press F2 (or double-click) to change it. Enter commits and moves down, Shift+Enter up, Tab and Shift+Tab across (Enter after a run of Tabs returns to the column you started in); Esc cancels. After typing, arrow keys also commit and move; after F2 they move the text cursor. Clicking elsewhere keeps what you typed, and Ctrl+S saves it.

Delete empties the selected cells: a range, whole rows (every column, also those of wider rows further down), or whole columns (every row, once the file is indexed). It is one step for Ctrl+Z however many cells it covers, and a whole column of a 1 GB file clears instantly; the save writes those fields empty and every other field as it was.

Ctrl+C (or Ctrl+Insert) copies the selection; Ctrl+X (or Shift+Delete) copies it and then empties the cells, as Calc does (Ctrl+Z puts them back). The clipboard gets tab-separated text, for text editors, and an HTML table, which LibreOffice Calc and Excel paste straight into cells. Values a spreadsheet would change, such as `00123`, IDs longer than 15 digits, and text starting with `=`, are marked as text so they arrive as they are. The copy is made in the background, with progress in the status bar, so copying a whole column of a 1 GB file (about 4 s) doesn't stop the window. Copies of more than about a million cells are plain text only.

Ctrl+V (or Shift+Insert) pastes tab-separated text (what LibreOffice, Excel, and this app copy) with its first cell at the cursor, as raw text, and selects what it pasted. A paste that runs past the last row adds rows (once the file is fully indexed); rows that are too short get longer. One Ctrl+Z takes the whole paste back. 10,000 rows copied from LibreOffice paste in under 20 ms; a paste is limited to about 2 million cells.

Insert and delete rows with Ctrl++ (as many empty rows as the selection spans, above it) and Ctrl+- (the selected rows); with whole columns selected (click the column letters), the same keys insert columns to the left and delete them. Right-click a cell or header for Insert Rows Above / Below, Delete Rows, Insert Columns Left / Right, and Delete Columns. Nothing is rewritten until you save: untouched rows are then still copied byte for byte, unless columns changed, in which case every row is re-assembled from its raw fields (3.5 s for 1 GB). Rows can be inserted and deleted once the file is fully indexed (under a second for 1 GB).

Ctrl+Z undoes the last edit and moves the cursor to the cell it changed; Ctrl+Shift+Z or Ctrl+Y redoes it. History keeps the last 1,000 steps within 50 MB, whatever the file size, dropping the oldest first. Undo waits while a save runs, and the history starts over after a save.

On macOS, Command replaces Ctrl in every shortcut (⌘S, ⌘Z, ⌘C, ⌘Home, …); Redo is ⇧⌘Z, Replace ⌥⌘F, and as in Excel for Mac a few stay on Control: ⌃Space and ⇧⌃Space select columns and everything, ⌃⇧1, ⌃⇧3, ⌃⇧4, ⌃⇧5 and ⌃⇧~ set number formats, and ⌃Page Down/Up switch sheets.

Resize a column or row by dragging its border in the column letters or row numbers; selected whole columns resize together. Double-click a column border to fit the text on screen, or a row border to restore its height. Sizes stay with their rows and columns through inserts, deletes, undo, and saving, and last while the window is open (CSV has nowhere to store them).

A header row is detected on open: when the first row names the columns, it becomes the column titles (next to the letters) and isn't counted as data. **Data → First Row Is Header** flips it (on Linux, Data is in the main menu, ☰); your choice holds when the delimiter changes or the file is saved. The file itself never changes: a save writes the first row as it is.

## Layout

Brick is one app of a suite (Slate, Frame); the suite's layout is in the root `README.md`.

```
apps/brick/                     the desktop app (GTK4, ADR 0001)
apps/brick/crates/csv-engine    parsing, dialects, encodings, row index
apps/brick/crates/data-model    cells, edit overlay, types, sort/filter views
apps/brick/crates/formula       formula language, functions, number formats
apps/brick/crates/xlsx          XLSX import and patching save
apps/brick/crates/grid          viewport, selection, navigation (no toolkit)
apps/brick/crates/commands      undo/redo
apps/brick/crates/file-format   atomic save
apps/brick/crates/jobs          background jobs
apps/brick/docs/                spec, benchmarks, architecture decisions
crates/suite-*                   infrastructure shared with Slate and Frame
BACKLOG.md                       the work queue, read by scripts/backlog.py
sites/brick/                    download page and installer deployed to Cloudflare Pages
scripts/                         backlog helper, corpus generator, release pipeline
.claude/skills/                  next-story and ship-it skills for Claude Code
```

## Claude Code skills

- **next-story**: picks the next unblocked story from `BACKLOG.md`, writes a failing test from its acceptance criterion, implements it, verifies fmt/clippy/tests/benchmarks, updates the backlog, and commits locally. Stops at milestone gates and decision reviews for your approval.
- **ship-it**: preflight, version bump, release build, upload to Cloudflare R2, deploy the download page to Cloudflare Pages, tag. Asks before anything leaves your machine. Needs `.env.ship` (see `.env.ship.example`).

## License

MIT OR Apache-2.0 (open question in the plan; change before the first release if needed).
