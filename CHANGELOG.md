# Changelog

Releases are added by the ship-it skill.

## Etch v0.5.0 (2026-10-24)

Etch now updates itself. When it starts, it checks for a newer release, downloads it in the background, and runs it from the next start (on Linux the new version is installed at once; on a Mac and on Windows when the last Etch window is closed), then says "Updated to Etch X.Y.Z" once. It only installs releases signed with Cronkite's key, so a tampered download is refused. To turn it off, uncheck Update Automatically (Help on Linux, the Etch menu on a Mac, Help on Windows), or set `CRONKITE_NO_UPDATE=1` for one run. **Copies installed before this release can't update themselves: run the install command once more** (`curl -fsSL https://etch.cronkite.cc/install.sh | sh`, or `irm https://etch.cronkite.cc/install.ps1 | iex` on Windows); from then on updates arrive by themselves.

- DEC-12: Apps that update themselves (ADR 0012)
- UPD-1: Sign each release's latest.json with Cronkite's release key
- UPD-2: Check, verify, download, and stage updates at startup
- UPD-3: Update Automatically next to About, and the one-time "Updated to Etch" notice
- UPD-4: Apply updates on Linux
- UPD-5: Apply updates on macOS
- UPD-6: Apply updates on Windows

## Frame v0.4.0 (2026-10-24)

Frame now updates itself. When it starts, it checks for a newer release, downloads it in the background, and runs it from the next start (on Linux the new version is installed at once; on a Mac and on Windows when the last Frame window is closed), then says "Updated to Frame X.Y.Z" once. It only installs releases signed with Cronkite's key, so a tampered download is refused. To turn it off, uncheck Update Automatically (the ☰ main menu on Linux, the Frame menu on a Mac, Help on Windows), or set `CRONKITE_NO_UPDATE=1` for one run. **Copies installed before this release can't update themselves: run the install command once more** (`curl -fsSL https://frame.cronkite.cc/install.sh | sh`, or `irm https://frame.cronkite.cc/install.ps1 | iex` on Windows); from then on updates arrive by themselves.

- DEC-12: Apps that update themselves (ADR 0012)
- UPD-1: Sign each release's latest.json with Cronkite's release key
- UPD-2: Check, verify, download, and stage updates at startup
- UPD-3: Update Automatically next to About, and the one-time "Updated to Frame" notice
- UPD-4: Apply updates on Linux
- UPD-5: Apply updates on macOS
- UPD-6: Apply updates on Windows

## Slate v0.6.0 (2026-10-24)

Slate now updates itself. When it starts, it checks for a newer release, downloads it in the background, and runs it from the next start (on Linux the new version is installed at once; on a Mac and on Windows when the last Slate window is closed), then says "Updated to Slate X.Y.Z" once. It only installs releases signed with Cronkite's key, so a tampered download is refused. To turn it off, uncheck Update Automatically (the ☰ main menu on Linux, the Slate menu on a Mac, Help on Windows), or set `CRONKITE_NO_UPDATE=1` for one run. **Copies installed before this release can't update themselves: run the install command once more** (`curl -fsSL https://slate.cronkite.cc/install.sh | sh`, or `irm https://slate.cronkite.cc/install.ps1 | iex` on Windows); from then on updates arrive by themselves.

- DEC-12: Apps that update themselves (ADR 0012)
- UPD-1: Sign each release's latest.json with Cronkite's release key
- UPD-2: Check, verify, download, and stage updates at startup
- UPD-3: Update Automatically next to About, and the one-time "Updated to Slate" notice
- UPD-4: Apply updates on Linux
- UPD-5: Apply updates on macOS
- UPD-6: Apply updates on Windows

## v0.6.0 (2026-10-24)

Brick now updates itself. When it starts, it checks for a newer release, downloads it in the background, and runs it from the next start (on Linux the new version is installed at once; on a Mac and on Windows when the last Brick window is closed), then says "Updated to Brick X.Y.Z" once. It only installs releases signed with Cronkite's key, so a tampered download is refused. To turn it off, uncheck Update Automatically (the ☰ main menu on Linux, the Brick menu on a Mac, Help on Windows), or set `CRONKITE_NO_UPDATE=1` for one run. **Copies installed before this release can't update themselves: run the install command once more** (`curl -fsSL https://brick.cronkite.cc/install.sh | sh`, or `irm https://brick.cronkite.cc/install.ps1 | iex` on Windows); from then on updates arrive by themselves.

- DEC-12: Apps that update themselves (ADR 0012)
- UPD-1: Sign each release's latest.json with Cronkite's release key
- UPD-2: Check, verify, download, and stage updates at startup
- UPD-3: Update Automatically next to About, and the one-time "Updated to Brick" notice
- UPD-4: Apply updates on Linux
- UPD-5: Apply updates on macOS
- UPD-6: Apply updates on Windows
- APP-7: A damaged crash-recovery journal is refused instead of stopping Brick

## Etch v0.4.0 (2026-10-17)

Etch runs on Windows 10 and 11 (64-bit) as a preview: install it for your user, without admin rights, with `irm https://etch.cronkite.cc/install.ps1 | iex` in PowerShell or the Setup.exe. It has Windows' own title bar with a menu bar under it, follows Windows' light and dark mode, opens images from Explorer's Open with and by drag and drop, keeps its settings in `%APPDATA%` and its recovery in `%LOCALAPPDATA%`, and uninstalls from Settings → Apps. The Setup.exe isn't signed yet, so a browser download meets SmartScreen once (More info → Run anyway). On every platform, a file dragged onto a window opens even when the drag's last move crosses into another part of it.

- WIN-1: Build and test every crate on Windows; atomic saves and recent lists work there
- WIN-2: Keep each app's data in %APPDATA% and %LOCALAPPDATA%; crash recovery on Windows
- WIN-3: No console window, Windows' own title bar, a menu bar row, and Windows' dark mode
- WIN-4: Open files from Explorer and the command line, and by drag and drop from Explorer
- WIN-5: Per-user Setup.exe and PowerShell installer, uninstalled from Settings → Apps
- WIN-6: Offer Windows on the download page

## Frame v0.3.0 (2026-10-17)

Frame runs on Windows 10 and 11 (64-bit) as a preview: install it for your user, without admin rights, with `irm https://frame.cronkite.cc/install.ps1 | iex` in PowerShell or the Setup.exe. It has Windows' own title bar with a menu bar and toolbar row, follows Windows' light and dark mode, opens decks from Explorer's Open with and by drag and drop, draws PowerPoint's fonts (Windows' own, or the bundled metric-compatible ones), and uninstalls from Settings → Apps. The Setup.exe isn't signed yet, so a browser download meets SmartScreen once (More info → Run anyway).

- WIN-1: Build and test every crate on Windows; atomic saves work there
- WIN-2: Keep each app's data in %APPDATA% and %LOCALAPPDATA%
- WIN-3: No console window, Windows' own title bar, a menu bar row, and Windows' dark mode
- WIN-4: Fonts on Windows; open files from Explorer and the command line, and by drag and drop
- WIN-5: Per-user Setup.exe and PowerShell installer, uninstalled from Settings → Apps
- WIN-6: Offer Windows on the download page

## Slate v0.5.0 (2026-10-17)

Slate runs on Windows 10 and 11 (64-bit) as a preview: install it for your user, without admin rights, with `irm https://slate.cronkite.cc/install.ps1 | iex` in PowerShell or the Setup.exe. It has Windows' own title bar with a menu bar under it, follows Windows' light and dark mode, opens documents from Explorer's Open with and by drag and drop, draws Word documents in Windows' Office fonts or the bundled metric-compatible ones, keeps its settings in `%APPDATA%` and its recovery in `%LOCALAPPDATA%`, and uninstalls from Settings → Apps. The Setup.exe isn't signed yet, so a browser download meets SmartScreen once (More info → Run anyway).

- WIN-1: Build and test every crate on Windows; atomic saves and recent lists work there
- WIN-2: Keep each app's data in %APPDATA% and %LOCALAPPDATA%; crash recovery and package copies on Windows
- WIN-3: No console window, Windows' own title bar, a menu bar row, and Windows' dark mode
- WIN-4: Fonts on Windows; open files from Explorer and the command line, and by drag and drop
- WIN-5: Per-user Setup.exe and PowerShell installer, uninstalled from Settings → Apps
- WIN-6: Offer Windows on the download page

## v0.5.0 (2026-10-17)

Brick runs on Windows 10 and 11 (64-bit) as a preview: install it for your user, without admin rights, with `irm https://brick.cronkite.cc/install.ps1 | iex` in PowerShell or the Setup.exe. It has Windows' own title bar with a menu bar and toolbar row, follows Windows' light and dark mode, opens spreadsheets from Explorer's Open with and by drag and drop, keeps its settings and recent files in `%APPDATA%` and its recovery in `%LOCALAPPDATA%`, and uninstalls from Settings → Apps. While Brick has a CSV open on Windows, other programs can't overwrite or truncate that file. The Setup.exe isn't signed yet, so a browser download meets SmartScreen once (More info → Run anyway).

- WIN-1: Build and test every crate on Windows; atomic saves, recent lists, and the mapped CSV work there
- WIN-2: Keep each app's data in %APPDATA% and %LOCALAPPDATA%; crash recovery on Windows
- WIN-3: No console window, Windows' own title bar, a menu bar row, and Windows' dark mode
- WIN-4: Open files from Explorer and the command line, and by drag and drop from Explorer
- WIN-5: Per-user Setup.exe and PowerShell installer, uninstalled from Settings → Apps
- WIN-6: Offer Windows on the download page

## Etch v0.3.0 (2026-10-16)

Etch runs on macOS (Apple Silicon, macOS 14 or later) as a preview: install it with the same curl command or a DMG. It uses Command shortcuts and the macOS menu bar with native title bars, follows light and dark mode, opens images from Finder, and keeps its settings in `~/Library`. The download isn't signed by Apple yet, so a DMG from the browser needs Open Anyway once (the download page says how). On Linux, files now reach Etch through GTK's open handling, and every release carries the licence notices of the Rust crates and libraries inside it.

- MAC-1: Add a macOS CI job and make every crate and test portable to macOS
- MAC-2: Keep each app's data in ~/Library on macOS
- MAC-3: Use Command shortcuts on macOS
- MAC-4: Put the menus in the macOS menu bar, with native title bars
- MAC-5: Open files from Finder and the command line through GApplication
- MAC-6: Follow macOS light and dark mode
- MAC-7: Ship a self-contained macOS app, DMG, and curl install
- PKG-6: Include the licence notices of the Rust crates in every release

## Frame v0.2.0 (2026-10-16)

Frame runs on macOS (Apple Silicon, macOS 14 or later) as a preview: install it with the same curl command or a DMG. It uses Command shortcuts and the macOS menu bar with a toolbar row and native title bars, follows light and dark mode, opens decks from Finder, and draws PowerPoint's fonts with the bundled metric-compatible ones. The download isn't signed by Apple yet, so a DMG from the browser needs Open Anyway once. Every release now carries the licence notices of the Rust crates and libraries inside it.

- MAC-1: Add a macOS CI job and make every crate and test portable to macOS
- MAC-2: Keep each app's data in ~/Library on macOS
- MAC-3: Use Command shortcuts on macOS
- MAC-4: Put the menus in the macOS menu bar, with native title bars
- MAC-5: Open files from Finder and the command line through GApplication
- MAC-6: Bundle the metric-compatible fonts and follow macOS light and dark mode
- MAC-7: Ship a self-contained macOS app, DMG, and curl install
- PKG-6: Include the licence notices of the Rust crates in every release

## Slate v0.4.0 (2026-10-16)

Slate runs on macOS (Apple Silicon, macOS 14 or later) as a preview: install it with the same curl command or a DMG. It uses Command shortcuts and Mac text navigation, the macOS menu bar with native title bars, follows light and dark mode, opens documents from Finder, keeps its data and crash recovery in `~/Library`, and draws Word documents in the bundled metric-compatible fonts with the same line breaks as on Linux. The download isn't signed by Apple yet, so a DMG from the browser needs Open Anyway once. Every release now carries the licence notices of the Rust crates and libraries inside it.

- MAC-1: Add a macOS CI job and make every crate and test portable to macOS
- MAC-2: Keep each app's data in ~/Library on macOS; a second Slate no longer removes the first one's working copy
- MAC-3: Use Command shortcuts and Mac caret keys on macOS
- MAC-4: Put the menus in the macOS menu bar, with native title bars
- MAC-5: Open files from Finder and the command line through GApplication
- MAC-6: Bundle the metric-compatible fonts and follow macOS light and dark mode
- MAC-7: Ship a self-contained macOS app, DMG, and curl install
- PKG-6: Include the licence notices of the Rust crates in every release

## v0.4.0 (2026-10-16)

Brick runs on macOS (Apple Silicon, macOS 14 or later) as a preview: install it with the same curl command or a DMG. It uses Command shortcuts and the macOS menu bar with a toolbar row and native title bars, follows light and dark mode, opens spreadsheets from Finder, and keeps its recent files and recovery in `~/Library`. The download isn't signed by Apple yet, so a DMG from the browser needs Open Anyway once. Every release now carries the licence notices of the Rust crates and libraries inside it.

- MAC-1: Add a macOS CI job and make every crate and test portable to macOS
- MAC-2: Keep each app's data in ~/Library on macOS
- MAC-3: Use Command shortcuts on macOS
- MAC-4: Put the menus in the macOS menu bar, with native title bars
- MAC-5: Open files from Finder and the command line through GApplication
- MAC-6: Bundle the metric-compatible fonts and follow macOS light and dark mode
- MAC-7: Ship a self-contained macOS app, DMG, and curl install
- PKG-6: Include the licence notices of the Rust crates in every release

## Etch v0.2.1 (2026-10-15)

The canvas draws cleanly on HiDPI screens: at a display scale of 2 (or 1.5), faint pale lines no longer cross the image every 256 pixels, and the checkerboard behind transparent pixels has crisp squares right up to the image's edge, with any GTK renderer.

- ETCH-26: Draw the canvas in device pixels so tiles meet without seams

## Etch v0.2.0 (2026-10-15)

Etch edits real images now, milestones ET1 to ET3: the V0.1 the spec defines. Open PNG, JPEG, and BMP files (Etch becomes their default app when installed), or drop them on a window, and save them back atomically; closing an edited image asks first. Draw with the brush (hard or soft), fill with a tolerance, pick colours from the image, draw lines, rectangles, and ellipses (outlined, filled, or both, previewed as you drag), and type text in any font through your input method. Select and move pixels, cut, copy, and paste with other apps, crop, resize the image or the canvas, rotate, and flip. Every step undoes. Colours come from a hex field and GTK's colour chooser too, every tool has its Paint key, Open Recent lists the last ten images, and after a crash Etch offers back your unsaved images without touching their files.

- ETCH-6: Open PNG, JPEG, and BMP off the UI thread; Etch the default app for them after install
- ETCH-7: Save, Save As, and Export PNG, JPEG, and BMP atomically; mark unsaved changes and ask before closing
- ETCH-8: Add the brush with a size and a hard or soft edge
- ETCH-9: Add the fill bucket: contiguous region, exact colour
- ETCH-10: Add the colour picker, alpha included
- ETCH-11: Add line, rectangle, and ellipse with previews: outline, filled, or both
- ETCH-12: Add the text tool: font, size, colour, bold, italic, typed through the input method
- ETCH-13: Add New Image's background choice and remember its size and background across runs
- ETCH-14: Add the fill's tolerance
- ETCH-15: Add the rectangular selection: select, move pixels, nudge, Select All, Deselect, Delete
- ETCH-16: Add cut, copy, and paste through the system clipboard
- ETCH-17: Add Crop to Selection
- ETCH-18: Add Resize Image: width, height, keep aspect, nearest or smooth
- ETCH-19: Add Resize Canvas with a nine-way anchor
- ETCH-20: Add Rotate 90° either way and 180° and Flip horizontally and vertically
- ETCH-21: Open files dropped on a window
- ETCH-22: Add File → Open Recent
- ETCH-23: Add the hex field and GTK's colour chooser
- ETCH-24: Bind every tool to its key
- ETCH-25: Offer back unsaved images after a crash

## Etch v0.1.0 (2026-10-14)

Etch's first release, milestone ET0: an early preview of a simple, native image editor for Linux, closer to Paint than to Photoshop. Draw on a blank canvas of any size up to 16384 pixels a side with a pencil and an eraser, in any of Paint's 20 colours and any size from 1 to 100 pixels; the eraser erases to transparency. Zoom from 10% to 3200% around the pointer, with pixels kept square from 100%, and pan with the wheel, the middle button, or Space. Every stroke undoes and redoes. A stroke on an 8K image draws in about a millisecond a frame. Opening and saving PNG, JPEG, and BMP come next.

- ETCH-1: Add the image model: RGBA8 image, pencil strokes, tiled undo history
- ETCH-2: Show the canvas: blank image, zoom, pan, pencil, undo, New Image
- ETCH-3: Erase to transparency over a checkerboard; add the tool column
- ETCH-4: Pick foreground and background colours from the palette; size the pencil and eraser
- ETCH-5: Install Etch per user with a desktop entry, icon, and download page

## Slate v0.3.0 (2026-10-14)

Word documents keep the layout their authors made. A Word document opens at its page's text width (View → Text Width → Page, the new default), so lines wrap and tab-aligned dates and places land where they do in Word and Google Docs. Tabs go to the document's own tab stops, with leaders. Fonts a .docx carries are used to draw it, and a missing font falls back to serif or sans as the document describes it. A paragraph's own spacing, indents, borders, and shading are drawn and saved, and list bullets and numbers are the ones the document defines, at its indents.

- SLATE-23: Open Word documents at their page's text width
- SLATE-24: Lay out tabs at the document's tab stops
- SLATE-25: Draw Word documents in their embedded fonts
- SLATE-26: Draw and save a paragraph's own spacing and indents
- SLATE-27: Draw and save paragraph borders and shading
- SLATE-28: Draw Word list markers from their numbering levels

## Slate v0.2.0 (2026-10-14)

Word documents look like themselves. Slate draws each one in its own fonts and paragraph styles (custom styles included), with their spacing, indents, and line spacing. A metric-compatible font stands in when one isn't installed: Carlito for Calibri, Caladea for Cambria, Liberation for Arial, Times New Roman, and Courier New. A format bar over the page sets the paragraph style, font, size, bold, italic, underline, strikethrough, colour, and alignment, and saving a .docx keeps them. After a crash, Slate offers back the unsaved edits of every open document.

- SLATE-19: Read each document's style sheet and fonts from the .docx
- SLATE-20: Draw each document's own fonts, paragraph spacing, indents, and line spacing
- SLATE-21: Add a format bar for style, font, size, looks, colour, and alignment
- SLATE-22: Save the format bar's fonts, styles, and looks in .docx
- SLATE-17: Journal unsaved edits and offer them back after a crash

## Slate v0.1.2 (2026-10-13)

Find and replace: Ctrl+F opens a find bar that highlights every match in the document, table cells included, and counts them; Enter and Shift+Enter step through them, and Match Case makes case matter. Ctrl+H adds Replace and Replace All, and one undo takes back a Replace All.

- SLATE-16: Find and replace with a find bar, highlights, and one-step Replace All

## Slate v0.1.1 (2026-10-13)

Start a new document: Ctrl+N (File → New) or New Document on the start window opens an Untitled Word document with the usual styles; the first save asks where, suggesting Untitled.docx in your Documents folder.

- SLATE-18: Open a new blank document with Ctrl+N or New Document

## v0.3.0 (2026-10-12)

Bricks is now Brick. The command is `brick`, the app is installed from brick.cronkite.cc, and installing it replaces Bricks (or the older Spreadsheet) with your recent files and unsaved-edit recovery kept. The old install command still works. Empty cells now fill the window past your data, as in Excel and Calc.

- PKG-5: Rename the app to Brick: `brick` command, app ID, window titles, state folder, download site
- GRID-6: Fill the window with empty cells past the data and grow as you go

## Frame v0.1.0 (2026-10-12)

Frame's first release, milestones FR0 and FR1: a lightweight presentation app that opens, shows, edits, and saves PowerPoint decks (.pptx). Edit on the slide or in an outline, format with one inspector instead of a ribbon, and play the deck full screen. Saving patches the original file, so everything you didn't edit stays as it was. The spec's four-slide reference deck takes 15 keys and clicks; PowerPoint's shortest documented way takes 21.

- FRAME-1: Presentation model and PPTX package: slide order, size, shapes, placeholder inheritance
- FRAME-2: PPTX text: runs, fonts, sizes, colours, bullets, levels, alignment, inherited styles
- FRAME-3: PPTX pictures, fills, outlines, backgrounds, groups, and tables
- FRAME-4: Viewer window: slide list and canvas
- FRAME-5: Full-screen slideshow
- FRAME-6: Edit slide text on the slide with undo
- FRAME-7: Add, delete, duplicate, and reorder slides; layouts as thumbnails
- FRAME-8: Save .pptx by patching the original package
- FRAME-10: Select, move, resize, and nudge shapes with snapping and smart guides
- FRAME-11: Inspector sidebar for the selection, no formatting dialogs
- FRAME-12: Insert text boxes, shapes, pictures, and tables; text shrinks to fit
- FRAME-13: Outline view of titles and bullets
- FRAME-14: File → New from the built-in blank deck
- FRAME-15: Build the reference deck in fewer keys and clicks than PowerPoint
- FRAME-9: Desktop entry, icon, and installer

## Slate v0.1.0 (2026-10-12)

Slate's first release, milestones SL0 and SL1: a lightweight, pageless word processor that opens, edits, and saves Word documents (.docx), Markdown, and plain text. Saving a .docx patches the original file, so everything you didn't edit stays as it was; an edited Markdown file changes only the blocks you edited.

- SLATE-1: Document model and plain-text reader
- SLATE-2: Read Markdown into the document model
- SLATE-3: Read DOCX styles, runs, lists, tables, links, and pictures
- SLATE-4: Viewer window: open, drop, show formatted
- SLATE-10: Pageless column with a text width setting
- SLATE-5: Choose the editing engine: Slate's own Pango flow (ADR 0007)
- SLATE-11: Show documents in the Pango flow
- SLATE-6: Caret, typing, and formatting as undoable commands
- SLATE-12: Edit inside table cells
- SLATE-13: Input method composition
- SLATE-14: Selection and clipboard across paragraphs and tables
- SLATE-15: Screen reader access to the text, caret, looks, and edits
- SLATE-7: Save .txt and .md atomically, rewriting only edited Markdown blocks
- SLATE-8: Save .docx by patching the original package
- SLATE-9: Desktop entry, icon, and installer

## v0.2.1 (2026-10-11)

Fixes how real workbooks draw, and lists .xlsx and .xlsm in the desktop entry.

- STYLE-8: Real-workbook drawing fixes: centre across selection, grid lines off, tight rows
- XLSX-1: List .xlsx and .xlsm in the desktop entry (Open With)

## v0.2.0 (2026-10-11)

Milestone M5: XLSX workbooks, formulas, multiple sheets, and cell styles. Bricks opens .xlsx files with every sheet, shows their fonts, colours, fills, borders, alignment, number formats, and merged cells, lets you edit values, formulas, sheets, and formats, and saves by patching the original file so charts, comments, conditional formats, and macros (in .xlsm) survive.

- DEC-6: Workbook model: XLSX sheets as indexed CSV copies; references address file rows
- DEC-7: Formula engine: lazy evaluation over raw cells with a revision cache
- DEC-8: Multiple sheets: one workbook per window, one undo history
- FML-1: Formula language: parse, render, A1 references, sheet names, errors
- FML-2: Evaluator: values, coercion, operators, cycles, deep chains
- FML-3: Workbook and calculation in data-model: formula cells, cached results, computed values for jobs
- FML-4: Grid shows formula results; edits and undo go through the workbook
- XLSX-1: Open and save .xlsx: every sheet as a tab, typed values, formulas with results, names, crash recovery
- SHEET-1: Add, rename, delete, and move sheets; each step undoable
- SHEET-2: References across sheets; renaming a sheet updates them; deleting one gives #REF!
- XLSX-2: Warn about workbook content Bricks drops; ask before saving over it
- FML-5: Functions: math, statistics, logic, information
- FML-6: Functions: text, including TEXT format codes
- FML-7: Functions: lookup and reference
- FML-8: Functions: dates and times
- FML-9: Formula bar and name box
- FML-10: Formulas follow inserted, deleted, and sorted rows and columns
- FML-11: Sort, filter, find, type inference, and copy use formula results
- FML-12: Copy and paste formulas inside Bricks with relative references shifted
- BENCH-2: XLSX and formula rows in the benchmark table
- DEC-9: Styles model: workbook style pool, per-column style runs, Excel layering
- DEC-10: XLSX save patches the original package
- STYLE-1: Read styles, merges, column widths, and row heights from .xlsx
- STYLE-2: Excel number formats for display and TEXT
- STYLE-3: Grid shows fonts, colours, fills, borders, alignment, wrap, number formats, widths, and heights
- STYLE-4: Merged cells: drawing, navigation, merge and unmerge
- STYLE-5: Save .xlsx by patching the original package
- STYLE-6: Edit formatting: format bar, menu, and shortcuts, each undoable
- STYLE-7: Copy and paste carry formats inside Bricks
- BENCH-3: Styled XLSX rows in the benchmark table

## v0.1.0 (2026-10-10)

First release: the large-CSV editor (milestones S0 to M3) plus the M4 desktop work and the curl installer.

- DEC-1: Choose UI toolkit (GTK4 vs Slint vs egui vs Iced)
- DEC-2: Choose source storage (mmap + sparse index vs chunked arena vs columnar)
- DEC-3: Choose edit model (overlay + row map vs piece table)
- DEC-4: Choose sort/filter representation
- DEC-5: Choose concurrency model (Rayon + channels + cancel tokens vs async)
- INFRA-1: CI: fmt, clippy -D warnings, test on every PR
- INFRA-2: Test corpus generator
- ENG-1: Stream-index a CSV into a row offset index
- ENG-2: Fetch any row range by logical index
- ENG-3: RFC 4180 parsing incl. quoted fields with embedded newlines
- GRID-1: Virtualized grid renders visible cells plus a buffer
- GRID-2: Scrollbar drag jumps to arbitrary positions
- EDIT-1: Single-cell edit stored in overlay
- SAVE-1: Atomic save: temp file, verify, rename
- BENCH-1: Benchmark harness over the corpus
- ENG-4: Auto-detect delimiter: comma, tab, semicolon, pipe
- ENG-5: Manual delimiter override, re-index in background
- ENG-6: Encoding detection: UTF-8, UTF-8 BOM, UTF-16, Latin-1
- ENG-7: Header row detection with toggle
- GRID-3: Keyboard navigation: arrows, Tab, Enter, Shift variants, Ctrl+Home/End, PgUp/PgDn
- GRID-4: Cell, range, row, and column selection
- GRID-5: Resize columns and rows; double-click autofits column
- APP-1: Open via native file dialog (xdg-desktop-portal)
- APP-2: Status bar: row count, encoding, delimiter, save state
- APP-3: Show first rows before indexing finishes
- CMD-1: Command pattern for all mutations
- CMD-2: Undo/redo stores operations, not snapshots
- EDIT-2: In-cell editor; F2 or typing starts edit
- EDIT-3: Insert and delete rows
- EDIT-4: Insert and delete columns
- EDIT-5: Delete key clears selected cells
- CLIP-1: Copy/cut range as TSV plus text/html
- CLIP-2: Paste TSV into a range, expanding as needed
- SAVE-2: Save, Save As, New
- SAVE-3: Background save with progress and cancel
- SAVE-4: Preserve quoting style and line endings of untouched rows
- APP-9: Ask before closing with unsaved edits
- ENG-8: Background job framework: progress and cancel
- TYPE-1: Infer column types by sampling
- TYPE-2: Show inferred type in header; allow override
- SORT-1: Type-aware sort, ascending and descending
- SORT-2: Sort moves whole rows and honors the header
- FILT-1: Per-column filter from header dropdown
- FILT-2: Combine filters across columns (AND)
- FILT-3: Saving with an active filter keeps hidden rows
- FILT-4: Edits and paste respect the filtered view
- SRCH-1: Find bar (Ctrl+F): whole file or current column, case toggle
- SRCH-2: Next/previous result; hit count streams in
- SRCH-3: Find and replace; replace all
- APP-4: Recent files menu
- APP-5: Drag and drop to open
- APP-6: Follow system dark/light theme; high-DPI
- APP-7: Crash recovery via periodic overlay journal
- APP-8: Clear errors for malformed files
- PKG-3: Portable release build: link against Ubuntu 24.04's glibc (build in a container)
- PKG-1: curl installer (per user, no sudo): `install.sh` on the download site; .desktop file, icon, text/csv association; `--uninstall`

## v0.1.1 (2026-10-10)

The app is now called Bricks: run `bricks`. Installing it replaces v0.1.0 (Spreadsheet) and keeps its recent files.

- PKG-4: Rename the app to Bricks: `bricks` command, window titles, app menu, state folder
- PKG-1: Download page and installer point at bricks.cronkite.cc and dl.cronkite.cc
