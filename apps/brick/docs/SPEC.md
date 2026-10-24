# Rust Spreadsheet: Backlog Execution Plan

Oct 5, 2026 · @Avniel Dravid

## How this plan works

V0.1 ships at the end of M3; the curl-installer release (per user, on Arch, Ubuntu 24.04, and Fedora) follows at the end of M4. Each milestone closes with a gate tied to the benchmark suite, and nothing moves forward until the gate passes.

**Sequencing rules**

- Retire the two largest risks first: the storage model at 1 GB and grid smoothness. Every later story depends on both.
- Land the benchmark harness in M0 and run it on every merge, so a regression shows up the day it happens.
- Put the data layer behind a trait. The grid asks for cells by logical row and column and never touches the file.
- Design undo with the first mutation in M2. Retrofitting it later costs more than building it.
- Untouched rows save byte-identical to the source. This one rule protects `00123`, odd quoting and every value type inference might mangle.

**Conventions**

- Story IDs: `M2-04` (milestone, sequence). Spikes: `SP-1`.
- Sizes: **S** = a day or less, **M** = two to four days, **L** = one to two weeks. Split any L before starting it.
- Crate tags match the PRD workspace: `csv-engine`, `data-model`, `grid`, `commands`, `app`, plus `bench` for the harness.
- Every story carries acceptance criteria. A story is done when its criteria pass in CI on the test corpus.
- Shortcuts are written once with the primary modifier (`apps/brick/src/keys.rs`, MAC-3): on macOS, Command replaces Ctrl in every shortcut; Redo is ⇧⌘Z (no Ctrl+Y), Replace ⌥⌘F, and Control stays for ⌃Space and ⇧⌃Space (selections), ⌃⇧digit number formats, and ⌃Page Down/Up (sheets), as in Excel for Mac.
- On macOS (MAC-4) the menus are in the menu bar: the app menu (About Brick, Update Automatically, Services, Hide, Quit ⌘Q), then File (New, Open, Open Recent, Close Window ⌘W, Save, Save As), Edit (Undo, Redo, Cut, Copy, Paste, Find, Replace, Go To), Sheet (New, Next, and Previous Sheet; insert and delete rows and columns), Format (bold, italic, underline, strikethrough, Number Format, Clear Formatting, merging), Data (sorting), and Window (Minimize ⌘M, Zoom, the windows), each item with its key. Windows keep macOS's title bar; the header bar's file buttons are the File menu's there, and the delimiter and Header row are a toolbar row over the format bar. Linux keeps the header bar.
- On macOS (MAC-5), Finder's Open and Open With, a file dropped on Brick's Dock icon, and `open -a Brick` open each file in a window of its own in the running Brick (Launch Services keeps one Brick, and GTK hands it the files through GApplication's `open`, as the command line's file goes too); a file already open brings its window forward, and without a file Brick shows its start window. On Linux each `brick FILE` is its own process, as before. The apps draw with cairo on macOS unless `GSK_RENDERER` names another renderer: GTK's GL renderer drew black windows on GitHub's macOS runners and is untried on a real Mac.
- On macOS (MAC-7), Brick is `Brick.app` for Apple Silicon and macOS 14 or later, carrying GTK and everything it reads (no Homebrew), signed ad hoc. The curl installer (`curl -fsSL https://brick.cronkite.cc/install.sh | sh`) puts it in `~/Applications` and registers it with Launch Services, so CSV, TSV, and Excel files list it under Open With; it doesn't change the Mac's default apps (Finder's Get Info, Open with, Change All does), and `--uninstall` removes it and its registration. The download page also offers a DMG, with Gatekeeper's Open Anyway steps.
- On Windows (WIN-1), the CSV Brick shows stays mapped as on Linux, but Windows won't let another program truncate it or write a new file over it while Brick has it open (that program gets an error); writing over bytes in place still works, and a save by rename, Brick's own included, replaces it. So the "file changed on disk" case of a file shrinking under Brick doesn't happen there. Saving is atomic as on Linux, without the folder sync Windows doesn't offer, and the recent files list keeps paths as Windows shows them (`C:\…`, never `\\?\C:\…`).
- On Windows (WIN-2), the recent files list is in `%APPDATA%\Cronkite\Brick\recent`, where it roams with the user's profile, and crash recovery journals and scratch copies (decoded non-UTF-8 files, imported XLSX sheets) in `%LOCALAPPDATA%\Cronkite\Brick` (`recovery\`, `copy-*`); Linux keeps `~/.local/state/brick` and `~/.cache/brick`, macOS `~/Library/Application Support/dev.cronkite.Brick` and `~/Library/Caches/dev.cronkite.Brick`. Open Recent and the start window show each file's folder in full there (`C:\Users\me\Documents`), not with `~`. Crash recovery works as on Linux: a Brick that's killed leaves its journal, which the next Brick offers back, and another running Brick's journals are left alone.
- On Windows (WIN-4), a file on the command line, from Explorer's Open with, or by a registered type (`"brick.exe" "%1"`) opens in a Brick process of its own, as on Linux, and `--bench-*` work as there; a file dragged from Explorer onto a window opens. Text draws through fontconfig with the fonts Brick carries over Windows' own; GTK chooses the renderer (cairo without a GPU).
- On Windows (WIN-3), Brick is a GUI program: no console window opens with it, and `brick --version` (or a usage error) still prints in the `cmd`, PowerShell, or Windows Terminal it was started from (after the next prompt, as the shell doesn't wait for a GUI program), or into a file or pipe it's redirected to. Windows have Windows' own title bar (minimize, maximize, close, snapping); under it a menu bar row holds the macOS menus (File, Edit, Sheet, Format, Data) with Help (Update Automatically, About Brick) after them, and File ends with Exit, which closes every window and asks about unsaved edits. Shortcuts stay Control, as on Linux, and each menu item shows its key. The delimiter and Header row are the toolbar row over the format bar, as on macOS. Brick follows Windows' app mode (Settings → Personalization → Colors), light or dark, the title bar too, and changes with it while running.
- On Windows (WIN-5), Brick installs for the user alone, without admin rights, from its Setup.exe or `irm https://brick.cronkite.cc/install.ps1 | iex`, into `%LOCALAPPDATA%\Programs\Brick` with GTK inside and a Start menu entry. CSV, TSV, and Excel workbooks (`.csv .tsv .xlsx .xlsm`) list it under Open with and in Settings → Default apps; Windows keeps the user's defaults. Settings → Apps uninstalls it; the recent list and recovery journals in `%APPDATA%\Cronkite\Brick` and `%LOCALAPPDATA%\Cronkite\Brick` stay (uninstalling by hand asks whether to remove them too).
- On Windows (WIN-6), brick.cronkite.cc offers a Windows visitor the Setup.exe on its button, the PowerShell command first under Install, and, for a Setup.exe downloaded in a browser, SmartScreen's way past "Windows protected your PC" (More info → Run anyway); Linux and Mac visitors keep theirs first, and every platform's download stays listed.
- Updates (UPD-3): Brick updates itself (ADR 0012): an install from the installer checks its release folder once each start, off the UI thread, downloads a newer release in the background when its Ed25519 signature and sha256 verify, and runs it from the next start (UPD-2 to UPD-6). "Update Automatically" turns it off (UPD-3): a check item in the ☰ main menu at the header bar's end (with About Brick) on Linux, after About Brick in the Brick menu on macOS, and in Help on Windows; on by default, kept in the settings folder's `update` file. `CRONKITE_NO_UPDATE=1` turns it off for one run; off, Brick makes no network request. The first start of a version an update applied says "Updated to Brick X.Y.Z" once, in a banner over the top of its first window that never takes the keys and goes when dismissed or after 20 seconds.

The roadmap below shows each milestone and the gate that closes it.

## Decision spikes

Six decisions shape everything downstream; SP-1 and SP-2 must close inside M0. Each spike is time-boxed and ends in a short written decision record in the repo.

| ID | Decision | Options | Current lean | Exit criteria | Size |
| --- | --- | --- | --- | --- | --- |
| SP-1 | GUI toolkit | egui/eframe, iced, Slint, GTK4 (gtk-rs), custom wgpu grid | Build the same grid spike in two finalists, likely GTK4 and egui. GTK4 wins on native menus, portals and themes; egui wins on iteration speed. | Smooth scroll over 10M virtual rows, native Wayland, working text input and IME, native file dialog, an accessibility path | L |
| SP-2 | Storage model | Full parse into Arrow/Polars; mmap plus row offset index; chunked parse with cache | mmap plus a `u64` offset per row, cells parsed on demand, columnar caches built lazily per column for sort and filter | 1 GB opens with first rows under 1 s; offset index for 10M rows fits in about 80 MB | L |
| SP-3 | Parallel indexing with quoted newlines | Single-threaded `csv-core` pass; speculative chunked parse with validation | Speculative chunks, verified at boundaries, single-thread fallback when a chunk guesses wrong | Correct row count on the adversarial corpus; faster than single-thread on 1 GB | M |
| SP-4 | Encoding scope for V0.1 | UTF-8 only; UTF-8 plus BOM variants; add Windows-1252 and UTF-16 via `encoding_rs` | UTF-8 and BOM detection on the mmap path; transcode other encodings to a temp file on open | Round-trip save preserves the original encoding and BOM | M |
| SP-5 | Row order and filter representation | Physical reorder; logical row map plus visible index | Stable `RowId`s, a logical row map that sort rewrites, and a filter that yields a visible subset of that map | Save under an active filter writes every row in logical order | M |
| SP-6 | Undo memory budget | Full snapshots; inverse commands; compact diffs with a byte cap | Inverse commands; sort stores the prior row map; bulk replace stores compact diffs; history capped by bytes | Undo of sort and replace-all on 1 GB stays within the memory target | S |

## Milestone 0: Technical prototype

M0 proves the thesis on ugly UI: a 1 GB CSV opens, scrolls smoothly, takes an edit and saves safely. It also closes SP-1 through SP-3.

| ID | Story | Crate | Size | Acceptance criteria |
| --- | --- | --- | --- | --- |
| M0-01 | Workspace scaffold with CI running fmt, clippy and tests | all | S | Crates match the PRD layout; CI blocks merges on failure |
| M0-02 | Test corpus generator | bench | M | Seeded output at 1 MB, 100 MB and 1 GB; narrow and 500-column variants; quoted newlines, embedded delimiters, BOM, ragged rows, leading zeros |
| M0-03 | Benchmark harness | bench | M | Records open time, time to first rows, index time, scroll frame times and peak memory; results stored per commit |
| M0-04 | Row offset indexer over mmap | csv-engine | L | Correct row count on every corpus file; parallel path per SP-3 |
| M0-05 | Cell fetch API | data-model | M | `cell(row, col)` returns the raw field; small LRU of parsed rows; no allocation per visible cell on a cache hit |
| M0-06 | Virtualized grid spike in each toolkit finalist | grid | L | Renders only visible cells plus a buffer; scroll and jump to row 10,000,000 stay smooth |
| M0-07 | Minimal edit overlay | data-model | S | Edited cells read from the overlay; source file untouched |
| M0-08 | Safe save | csv-engine | M | Writes to a temp file in the same directory, fsyncs, renames over the original; untouched rows byte-identical; killing the process mid-save leaves the original intact |

**Gate:** SP-1 and SP-2 decided and recorded. The 1 GB corpus file opens, scrolls and round-trips an edit within the proposed targets in the benchmark section.
