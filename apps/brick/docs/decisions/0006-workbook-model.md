# 0006: Workbook model

- Story: DEC-6
- Status: accepted
- Date: 2026-10-10

## Question

How does Bricks hold an XLSX workbook's sheets, and what does a cell reference like `A1` address, so that every existing table feature keeps working per sheet?

## Options

| Option | Measured result | Cost / risk |
| --- | --- | --- |
| A. Sheets as indexed CSV copies: each XLSX sheet is streamed into a CSV copy in the cache directory and mapped like any CSV | Not measured; calamine 0.36's `XlsxCellReader` streams cells in row-major order, and decoded copies already work this way for non-UTF-8 CSVs | Import is a full pass before a sheet shows; formatting is not modelled |
| B. A second store behind a new trait inside `CsvTable` | Not measured | Every read path (sort, filter, search, copy, save, journal) grows a second implementation |
| C. IronCalc's in-memory model | Not measured; IronCalc 0.8.3 holds every cell in memory | Breaks invariant 2 (never materialize the whole file) for large sheets; a second edit and undo model next to ADR 0003's overlay |

## Spike

None; decided with the user while planning M5 from API research (calamine 0.36.1, rust_xlsxwriter 0.99.1, IronCalc 0.8.3 on docs.rs).

## Decision

**Option A.**

1. **Sheets are `CsvTable`s.** An XLSX sheet is imported by streaming its cells (`XlsxCellReader::next_cell_with_formula`) into a CSV copy in the cache directory (the mechanism of `csv_engine::open_text_as` for decoded copies), mapped and indexed like any CSV. Overlay edits, row map, sort, filter, find, copy, paste, undo, and save work unchanged per sheet. Imported copies are comma-separated, `"`-quoted, LF, no header.
2. **References address file rows.** `A1` is the first line of the sheet (the header row when "First Row Is Header" is on), as in Excel and Calc. The row gutter shows file row numbers, and draws `1` in the top-left corner when the header row is on, so typed references match what the user sees. Toggling the header row never changes what a formula means. Columns are view positions (`A` is the first column shown).
3. **One workbook per window** (ADR 0008): a `Workbook` holds the sheets, the active sheet, defined names, and the calculation cache.

## Consequences

- Every table feature works per sheet for free, including atomic save and crash recovery of edits.
- Import is a full pass over the sheet before it shows.
- XLSX formatting (styles, merges, charts, comments) is not modelled; XLSX-2 warns about what a save drops. Superseded by ADR 0009 (styles are modelled and shown) and ADR 0010 (save keeps everything else).
- Raw values remain text: typed XLSX values are written as their canonical text, and export writes numbers, booleans, and dates back only when their text is canonical, so a round trip never changes a cell's text.
- **Revisit if:** per-cell styles become a goal, or import time of 1M-row sheets misses its target.
