//! Crash recovery journal (APP-7, XLSX-1, STYLE-6). Everything the user changed that a
//! save would write is the workbook's sheets (names, order, where each comes from), each
//! sheet's edit state ([`EditState`]: cell edits, the row order (sorts, inserted and
//! deleted rows), the column order, and cleared blocks), its formats and merged cells,
//! and the style records the workbook gained. The undo history can't stand in for it:
//! its memory budget trims old steps. The app writes it, behind a header naming the file
//! it belongs to ([`JournalHeader`]), and after a crash puts it back as one undoable step
//! ([`Workbook::restore_plan`]).
//!
//! Format, little-endian: magic `BRICKSJ3`; the header; `sheet_count u32`, `active u32`
//! (a position); per sheet its id, name, origin, header flag, state, the user's style
//! layers and the merged ranges; then the style pool's file counts and the records added
//! after them; then an FNV-1a hash of everything before it, so a cut or damaged journal
//! is refused rather than half-applied. Journals of earlier versions are still read:
//! `BRICKSJ2` (no formats, merges, or pool) and v0.1.x's `BRICKSJ1` (one table's state,
//! no kind byte), as one sheet.

use crate::cleared::{Cleared, ColSet};
use crate::colmap::ColMap;
use crate::format::{intern, Runs, UserStyles};
use crate::rowmap::{RowOrder, Run};
use crate::style::{
    Align, Baseline, Border, Color, Edge, FileCounts, Fill, Font, HAlign, LineStyle, Pattern,
    StyleId, StylePool, Tint, Underline, VAlign, Xf,
};
use crate::workbook::{Origin, Sheet, SheetId, Workbook};
use crate::{BookEdit, CellArea, CellRef, ColId, CsvTable, Edit, EditOverlay, RowId};
use csv_engine::{Charset, Encoding, RowIndex};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

/// The magics keep the app's earlier name: journals written before the rename to Brick
/// (PKG-5) are still restored.
const MAGIC: &[u8; 8] = b"BRICKSJ3";
/// The journal of XLSX-1 to STYLE-5 (no formats), still read.
const MAGIC_V2: &[u8; 8] = b"BRICKSJ2";
/// The single-table journal of v0.1.x, still read.
const MAGIC_V1: &[u8; 8] = b"BRICKSJ1";
/// More sheets than any workbook has: a damaged count is refused.
const MAX_SHEETS: u32 = 1 << 16;

/// What the user changed, as a save would write it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditState {
    pub(crate) overlay: EditOverlay,
    pub(crate) rows: Option<RowOrder>,
    pub(crate) cols: Option<ColMap>,
    pub(crate) cleared: Cleared,
    pub(crate) next_inserted: u64,
    pub(crate) next_inserted_col: u32,
}

impl EditState {
    /// Nothing changed.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn heap_bytes(&self) -> usize {
        self.overlay.len() * 48 + self.rows.as_ref().map_or(0, RowOrder::heap_bytes)
    }
}

/// What kind of file the journal's document was opened from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Csv,
    Xlsx,
}

/// Which file a journal belongs to and how it was read, plus what the start window shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalHeader {
    /// `None` for an untitled document.
    pub path: Option<PathBuf>,
    /// CSV or .xlsx; journals of v0.1.x are CSV.
    pub kind: FileKind,
    /// The file as it was when opened: a journal applies only to the same bytes.
    pub file_size: u64,
    pub file_mtime_ns: u64,
    pub delimiter: u8,
    pub has_header: bool,
    pub encoding: Encoding,
    /// When the journal was written, in milliseconds since the Unix epoch.
    pub saved_ms: u64,
    /// Unsaved changes and rows shown when written (for the offer's wording).
    pub changes: u64,
    pub rows: u64,
    /// The process that wrote it.
    pub pid: u32,
}

/// One sheet as journaled: who it is, where its rows come from, and what changed.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetState {
    pub id: SheetId,
    pub name: String,
    pub origin: Origin,
    pub has_header: bool,
    pub state: EditState,
    pub formats: SheetFormats,
}

/// A sheet's formats as journaled (STYLE-6): the user's style layers and the merged
/// ranges. Journals before `BRICKSJ3` have none, and leave the file's merges.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SheetFormats {
    pub(crate) styles: UserStyles,
    pub(crate) merges: Option<Vec<CellArea>>,
}

/// A journaled workbook: its sheets in order, the position of the active one, and the
/// style records added to the file's (`None` before `BRICKSJ3`).
#[derive(Debug, Clone, PartialEq)]
pub struct BookState {
    pub active: u32,
    pub sheets: Vec<SheetState>,
    pub pool: Option<JournalPool>,
}

/// The style records a workbook gained after the file's ([`StylePool::from_file`]), and
/// every number format code it has by id.
#[derive(Debug, Clone, PartialEq)]
pub struct JournalPool {
    base: FileCounts,
    fonts: Vec<Font>,
    fills: Vec<Fill>,
    borders: Vec<Border>,
    xfs: Vec<Xf>,
    num_fmts: Vec<(u32, Box<str>)>,
}

impl JournalPool {
    fn of(pool: &StylePool) -> Self {
        let base = pool.from_file;
        let tail = |n: u32, len: usize| (n as usize).min(len);
        Self {
            base,
            fonts: pool.fonts[tail(base.fonts, pool.fonts.len())..].to_vec(),
            fills: pool.fills[tail(base.fills, pool.fills.len())..].to_vec(),
            borders: pool.borders[tail(base.borders, pool.borders.len())..].to_vec(),
            xfs: pool.xfs[tail(base.xfs, pool.xfs.len())..].to_vec(),
            num_fmts: pool
                .num_fmts
                .iter()
                .map(|(&id, code)| (id, code.clone()))
                .collect(),
        }
    }

    /// Cell formats in all, the file's and the journal's.
    fn xf_count(&self) -> u64 {
        u64::from(self.base.xfs) + self.xfs.len() as u64
    }

    /// The journal's records refer only to records it or the file has.
    fn check(&self) -> Result<(), JournalError> {
        let fits = |i: u32, base: u32, len: usize| u64::from(i) < u64::from(base) + len as u64;
        let ok = self.xfs.iter().all(|x| {
            fits(x.font, self.base.fonts, self.fonts.len())
                && fits(x.fill, self.base.fills, self.fills.len())
                && fits(x.border, self.base.borders, self.borders.len())
        });
        if ok {
            Ok(())
        } else {
            Err(JournalError::Corrupt(
                "a style names a record the journal lacks",
            ))
        }
    }

    /// Add the journal's records to `pool` (equal ones are reused); the id each of the
    /// journal's cell formats has there, by journal id.
    fn merge_into(&self, pool: &mut StylePool) -> impl Fn(StyleId) -> StyleId {
        let base = self.base;
        let fonts: Vec<u32> = self
            .fonts
            .iter()
            .map(|f| intern(&mut pool.fonts, f.clone()))
            .collect();
        let fills: Vec<u32> = self
            .fills
            .iter()
            .map(|f| intern(&mut pool.fills, f.clone()))
            .collect();
        let borders: Vec<u32> = self
            .borders
            .iter()
            .map(|b| intern(&mut pool.borders, b.clone()))
            .collect();
        let num_fmts: HashMap<u32, u32> = self
            .num_fmts
            .iter()
            .map(|(id, code)| {
                let now = match pool.num_fmts.get(id) {
                    Some(c) if c == code => *id,
                    _ => pool.num_fmt_id(code),
                };
                (*id, now)
            })
            .collect();
        let at = |base: u32, map: &[u32], i: u32| {
            if i < base {
                i
            } else {
                map[(i - base) as usize]
            }
        };
        let xfs: Vec<StyleId> = self
            .xfs
            .iter()
            .map(|x| {
                let mut x = x.clone();
                x.font = at(base.fonts, &fonts, x.font);
                x.fill = at(base.fills, &fills, x.fill);
                x.border = at(base.borders, &borders, x.border);
                x.num_fmt = num_fmts.get(&x.num_fmt).copied().unwrap_or(x.num_fmt);
                pool.intern(x)
            })
            .collect();
        move |id: StyleId| {
            if id.0 < base.xfs {
                id
            } else {
                xfs[(id.0 - base.xfs) as usize]
            }
        }
    }
}

#[derive(Debug)]
pub enum JournalError {
    Io(io::Error),
    /// Not a journal, from a newer version, cut short, or damaged.
    Corrupt(&'static str),
    /// The state names rows or columns the file doesn't have.
    Mismatch(&'static str),
}

impl std::fmt::Display for JournalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Corrupt(why) => write!(f, "the recovery journal is damaged ({why})"),
            Self::Mismatch(why) => write!(f, "the edits don't fit the file ({why})"),
        }
    }
}

impl From<io::Error> for JournalError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            Self::Corrupt("cut short")
        } else {
            Self::Io(e)
        }
    }
}

/// FNV-1a over everything written or read, so the end of a journal can be checked.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

struct Writer<W> {
    inner: W,
    hash: Fnv,
}

impl<W: Write> Writer<W> {
    fn bytes(&mut self, b: &[u8]) -> io::Result<()> {
        self.hash.feed(b);
        self.inner.write_all(b)
    }
    fn u8(&mut self, v: u8) -> io::Result<()> {
        self.bytes(&[v])
    }
    fn u32(&mut self, v: u32) -> io::Result<()> {
        self.bytes(&v.to_le_bytes())
    }
    fn u64(&mut self, v: u64) -> io::Result<()> {
        self.bytes(&v.to_le_bytes())
    }
    fn blob(&mut self, b: &[u8]) -> io::Result<()> {
        self.u64(b.len() as u64)?;
        self.bytes(b)
    }
}

struct Reader<R> {
    inner: R,
    hash: Fnv,
}

impl<R: Read> Reader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            hash: Fnv::new(),
        }
    }
    /// `n` bytes, the buffer growing as they arrive: a damaged length that passed `count`
    /// (up to a terabyte) ends as "cut short" at the journal's end instead of allocating it.
    fn bytes(&mut self, n: usize) -> Result<Vec<u8>, JournalError> {
        let mut b = Vec::with_capacity(n.min(1 << 20));
        (&mut self.inner).take(n as u64).read_to_end(&mut b)?;
        if b.len() != n {
            return Err(JournalError::Corrupt("cut short"));
        }
        self.hash.feed(&b);
        Ok(b)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], JournalError> {
        let mut b = [0; N];
        self.inner.read_exact(&mut b)?;
        self.hash.feed(&b);
        Ok(b)
    }
    fn u8(&mut self) -> Result<u8, JournalError> {
        Ok(self.array::<1>()?[0])
    }
    fn u32(&mut self) -> Result<u32, JournalError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, JournalError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    /// A count of items at least `min_bytes` each: refused if bigger than any real
    /// journal, so a damaged count can't ask for terabytes.
    fn count(&mut self, min_bytes: u64) -> Result<usize, JournalError> {
        let n = self.u64()?;
        if n.saturating_mul(min_bytes.max(1)) > 1 << 40 {
            return Err(JournalError::Corrupt("impossible count"));
        }
        Ok(n as usize)
    }
    fn blob(&mut self) -> Result<Vec<u8>, JournalError> {
        let n = self.count(1)?;
        self.bytes(n)
    }
}

fn charset_code(c: Charset) -> u8 {
    match c {
        Charset::Utf8 => 0,
        Charset::Utf16Le => 1,
        Charset::Utf16Be => 2,
        Charset::Windows1252 => 3,
    }
}

fn charset_of(code: u8) -> Result<Charset, JournalError> {
    Ok(match code {
        0 => Charset::Utf8,
        1 => Charset::Utf16Le,
        2 => Charset::Utf16Be,
        3 => Charset::Windows1252,
        _ => return Err(JournalError::Corrupt("unknown encoding")),
    })
}

fn write_header<W: Write>(w: &mut Writer<W>, h: &JournalHeader) -> io::Result<()> {
    w.bytes(MAGIC)?;
    let path = match &h.path {
        Some(p) => suite_file::path_bytes(p).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "the file's name isn't Unicode")
        })?,
        None => &[],
    };
    w.blob(path)?;
    w.u8(u8::from(h.path.is_some()))?;
    w.u8(match h.kind {
        FileKind::Csv => 0,
        FileKind::Xlsx => 1,
    })?;
    w.u64(h.file_size)?;
    w.u64(h.file_mtime_ns)?;
    w.u8(h.delimiter)?;
    w.u8(u8::from(h.has_header))?;
    w.u8(charset_code(h.encoding.charset))?;
    w.u8(u8::from(h.encoding.bom))?;
    w.u64(h.saved_ms)?;
    w.u64(h.changes)?;
    w.u64(h.rows)?;
    w.u32(h.pid)
}

/// The header, and the journal's version (1 to 3).
fn read_header_from<R: Read>(r: &mut Reader<R>) -> Result<(JournalHeader, u8), JournalError> {
    let magic = r.array::<8>()?;
    let version = match &magic {
        m if m == MAGIC => 3,
        m if m == MAGIC_V2 => 2,
        m if m == MAGIC_V1 => 1,
        _ => return Err(JournalError::Corrupt("not a journal")),
    };
    let v1 = version == 1;
    let path = r.blob()?;
    let titled = r.u8()? == 1;
    let kind = if v1 {
        FileKind::Csv
    } else {
        match r.u8()? {
            0 => FileKind::Csv,
            1 => FileKind::Xlsx,
            _ => return Err(JournalError::Corrupt("unknown file kind")),
        }
    };
    let path = match titled {
        true => Some(
            suite_file::path_from_bytes(&path)
                .ok_or(JournalError::Corrupt("the file's name isn't Unicode"))?,
        ),
        false => None,
    };
    let header = JournalHeader {
        path,
        kind,
        file_size: r.u64()?,
        file_mtime_ns: r.u64()?,
        delimiter: r.u8()?,
        has_header: r.u8()? == 1,
        encoding: Encoding {
            charset: charset_of(r.u8()?)?,
            bom: r.u8()? == 1,
        },
        saved_ms: r.u64()?,
        changes: r.u64()?,
        rows: r.u64()?,
        pid: r.u32()?,
    };
    Ok((header, version))
}

/// Read only the header (any version): enough to list journals without reading their
/// state.
pub fn read_journal_header(r: impl Read) -> Result<JournalHeader, JournalError> {
    read_header_from(&mut Reader::new(r)).map(|(h, _)| h)
}

fn write_colset<W: Write>(w: &mut Writer<W>, c: &ColSet) -> io::Result<()> {
    let (ids, rest) = c.parts();
    w.u64(ids.len() as u64)?;
    for id in ids {
        w.u32(id.0)?;
    }
    w.u8(u8::from(rest.is_some()))?;
    w.u32(rest.unwrap_or(0))
}

fn read_colset<R: Read>(r: &mut Reader<R>) -> Result<ColSet, JournalError> {
    let n = r.count(4)?;
    let ids = (0..n)
        .map(|_| r.u32().map(ColId))
        .collect::<Result<_, _>>()?;
    let has_rest = r.u8()? == 1;
    let rest = r.u32()?;
    Ok(ColSet::new(ids, has_rest.then_some(rest)))
}

fn write_state<W: Write>(w: &mut Writer<W>, t: &CsvTable) -> io::Result<()> {
    // Cell edits, sorted so the same state always writes the same bytes.
    let mut rows: Vec<_> = t.overlay.iter_rows().collect();
    rows.sort_unstable_by_key(|(id, _)| **id);
    w.u64(rows.len() as u64)?;
    for (id, cells) in rows {
        w.u64(id.0)?;
        w.u64(cells.len() as u64)?;
        for (col, value) in cells {
            w.u32(col.0)?;
            w.blob(value.as_bytes())?;
        }
    }
    match &t.rows {
        None => w.u8(0)?,
        Some(RowOrder::Runs(m)) => {
            w.u8(1)?;
            let mut runs = Vec::new();
            m.for_each(|r| runs.push(*r));
            w.u64(runs.len() as u64)?;
            for r in runs {
                w.u64(r.first.0)?;
                w.u64(r.len)?;
            }
        }
        Some(RowOrder::Sorted(ids)) => {
            w.u8(2)?;
            w.u64(ids.len() as u64)?;
            // One write for the whole order: it can be 19M rows.
            let bytes: Vec<u8> = ids.iter().flat_map(|v| v.to_le_bytes()).collect();
            w.bytes(&bytes)?;
        }
    }
    match &t.cols {
        None => w.u8(0)?,
        Some(m) => {
            w.u8(1)?;
            let (explicit, tail) = m.parts();
            w.u32(tail)?;
            w.u64(explicit.len() as u64)?;
            for id in explicit {
                w.u32(id.0)?;
            }
        }
    }
    let blocks: Vec<_> = t.cleared.blocks().collect();
    w.u64(blocks.len() as u64)?;
    for (ranges, cols) in blocks {
        w.u64(ranges.len() as u64)?;
        for r in ranges {
            w.u64(r.start)?;
            w.u64(r.end)?;
        }
        write_colset(w, cols)?;
    }
    w.u64(t.next_inserted)?;
    w.u32(t.next_inserted_col)
}

fn read_state<R: Read>(r: &mut Reader<R>) -> Result<EditState, JournalError> {
    let mut s = EditState::default();
    for _ in 0..r.count(16)? {
        let row = RowId(r.u64()?);
        for _ in 0..r.count(12)? {
            let col = ColId(r.u32()?);
            let value = String::from_utf8(r.blob()?)
                .map_err(|_| JournalError::Corrupt("a value isn't UTF-8"))?;
            s.overlay.set(CellRef { row, col }, value);
        }
    }
    s.rows = match r.u8()? {
        0 => None,
        1 => {
            let runs = (0..r.count(16)?)
                .map(|_| {
                    Ok(Run {
                        first: RowId(r.u64()?),
                        len: r.u64()?,
                    })
                })
                .collect::<Result<Vec<_>, JournalError>>()?;
            Some(RowOrder::from_runs(&runs))
        }
        2 => {
            let n = r.count(4)?;
            let bytes = r.bytes(n * 4)?;
            let ids = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|&c| u32::from_le_bytes(c))
                .collect();
            Some(RowOrder::Sorted(Arc::new(ids)))
        }
        _ => return Err(JournalError::Corrupt("unknown row order")),
    };
    s.cols = match r.u8()? {
        0 => None,
        1 => {
            let tail = r.u32()?;
            let explicit = (0..r.count(4)?)
                .map(|_| r.u32().map(ColId))
                .collect::<Result<_, _>>()?;
            Some(ColMap::from_parts(explicit, tail))
        }
        _ => return Err(JournalError::Corrupt("unknown column order")),
    };
    for _ in 0..r.count(24)? {
        let ranges: Vec<Range<u64>> = (0..r.count(16)?)
            .map(|_| Ok(r.u64()?..r.u64()?))
            .collect::<Result<_, JournalError>>()?;
        let cols = read_colset(r)?;
        s.cleared.push(ranges, cols);
    }
    s.next_inserted = r.u64()?;
    s.next_inserted_col = r.u32()?;
    Ok(s)
}

fn write_runs<W: Write>(w: &mut Writer<W>, runs: &Runs) -> io::Result<()> {
    w.u64(runs.len() as u64)?;
    for (&s, &(e, st)) in runs {
        w.u64(s)?;
        w.u64(e)?;
        w.u32(st.0)?;
    }
    Ok(())
}

fn read_runs<R: Read>(r: &mut Reader<R>) -> Result<Runs, JournalError> {
    let mut runs = Runs::new();
    let mut last = 0;
    for i in 0..r.count(20)? {
        let (s, e, st) = (r.u64()?, r.u64()?, StyleId(r.u32()?));
        if s >= e || (i > 0 && s < last) {
            return Err(JournalError::Corrupt("style runs out of order"));
        }
        last = e;
        runs.insert(s, (e, st));
    }
    Ok(runs)
}

/// The user's style layers and the merged ranges, sorted so the same state always writes
/// the same bytes.
fn write_formats<W: Write>(w: &mut Writer<W>, t: &CsvTable) -> io::Result<()> {
    let u = &t.user_styles;
    let mut cells: Vec<_> = u.cells.iter().collect();
    cells.sort_unstable_by_key(|(c, _)| **c);
    w.u64(cells.len() as u64)?;
    for (c, runs) in cells {
        w.u32(c.0)?;
        write_runs(w, runs)?;
    }
    write_runs(w, &u.rows)?;
    let mut cols: Vec<_> = u.cols.iter().collect();
    cols.sort_unstable_by_key(|(c, _)| **c);
    w.u64(cols.len() as u64)?;
    for (c, s) in cols {
        w.u32(c.0)?;
        w.u32(s.0)?;
    }
    w.u64(t.merges().len() as u64)?;
    for m in t.merges() {
        w.u64(m.first_row)?;
        w.u32(m.first_col)?;
        w.u64(m.last_row)?;
        w.u32(m.last_col)?;
    }
    Ok(())
}

fn read_formats<R: Read>(r: &mut Reader<R>) -> Result<SheetFormats, JournalError> {
    let mut styles = UserStyles::default();
    for _ in 0..r.count(12)? {
        let col = ColId(r.u32()?);
        let runs = read_runs(r)?;
        if !runs.is_empty() && styles.cells.insert(col, runs).is_some() {
            return Err(JournalError::Corrupt("a column's styles twice"));
        }
    }
    styles.rows = read_runs(r)?;
    for _ in 0..r.count(8)? {
        let col = ColId(r.u32()?);
        styles.cols.insert(col, StyleId(r.u32()?));
    }
    let merges = (0..r.count(24)?)
        .map(|_| {
            Ok(CellArea {
                first_row: r.u64()?,
                first_col: r.u32()?,
                last_row: r.u64()?,
                last_col: r.u32()?,
            })
        })
        .collect::<Result<Vec<_>, JournalError>>()?;
    Ok(SheetFormats {
        styles,
        merges: Some(merges),
    })
}

const LINES: [LineStyle; 14] = [
    LineStyle::None,
    LineStyle::Thin,
    LineStyle::Medium,
    LineStyle::Thick,
    LineStyle::Dashed,
    LineStyle::Dotted,
    LineStyle::Double,
    LineStyle::Hair,
    LineStyle::MediumDashed,
    LineStyle::DashDot,
    LineStyle::MediumDashDot,
    LineStyle::DashDotDot,
    LineStyle::MediumDashDotDot,
    LineStyle::SlantDashDot,
];
const HALIGNS: [HAlign; 8] = [
    HAlign::General,
    HAlign::Left,
    HAlign::Center,
    HAlign::Right,
    HAlign::Fill,
    HAlign::Justify,
    HAlign::CenterContinuous,
    HAlign::Distributed,
];
const VALIGNS: [VAlign; 5] = [
    VAlign::Top,
    VAlign::Center,
    VAlign::Bottom,
    VAlign::Justify,
    VAlign::Distributed,
];
const UNDERLINES: [Underline; 5] = [
    Underline::None,
    Underline::Single,
    Underline::Double,
    Underline::SingleAccounting,
    Underline::DoubleAccounting,
];

/// `v`'s position in `all`, every variant's list.
fn code_of<T: PartialEq>(all: &[T], v: &T) -> u8 {
    all.iter()
        .position(|x| x == v)
        .expect("every variant is listed") as u8
}

fn variant<T: Copy>(all: &[T], code: u8) -> Result<T, JournalError> {
    all.get(code as usize)
        .copied()
        .ok_or(JournalError::Corrupt("an unknown style value"))
}

fn write_color<W: Write>(w: &mut Writer<W>, c: Color) -> io::Result<()> {
    match c {
        Color::Auto => w.u8(0),
        Color::Rgb(rgb) => {
            w.u8(1)?;
            w.u32(rgb)
        }
        Color::Theme { index, tint } => {
            w.u8(2)?;
            w.u8(index)?;
            w.u64(tint.value().to_bits())
        }
        Color::Indexed(n) => {
            w.u8(3)?;
            w.u8(n)
        }
    }
}

fn read_color<R: Read>(r: &mut Reader<R>) -> Result<Color, JournalError> {
    Ok(match r.u8()? {
        0 => Color::Auto,
        1 => Color::Rgb(r.u32()?),
        2 => Color::Theme {
            index: r.u8()?,
            tint: Tint::new(f64::from_bits(r.u64()?)),
        },
        3 => Color::Indexed(r.u8()?),
        _ => return Err(JournalError::Corrupt("an unknown colour")),
    })
}

fn write_opt_u8<W: Write>(w: &mut Writer<W>, v: Option<u8>) -> io::Result<()> {
    w.u8(u8::from(v.is_some()))?;
    w.u8(v.unwrap_or(0))
}

fn read_opt_u8<R: Read>(r: &mut Reader<R>) -> Result<Option<u8>, JournalError> {
    let some = r.u8()? == 1;
    let v = r.u8()?;
    Ok(some.then_some(v))
}

fn read_text<R: Read>(r: &mut Reader<R>) -> Result<Box<str>, JournalError> {
    String::from_utf8(r.blob()?)
        .map(String::into_boxed_str)
        .map_err(|_| JournalError::Corrupt("a style's text isn't UTF-8"))
}

fn write_font<W: Write>(w: &mut Writer<W>, f: &Font) -> io::Result<()> {
    w.blob(f.name.as_bytes())?;
    w.u32(f.size)?;
    w.u8(u8::from(f.bold) | u8::from(f.italic) << 1 | u8::from(f.strike) << 2)?;
    w.u8(code_of(&UNDERLINES, &f.underline))?;
    write_color(w, f.color)?;
    w.u8(match f.baseline {
        None => 0,
        Some(Baseline::Superscript) => 1,
        Some(Baseline::Subscript) => 2,
    })?;
    write_opt_u8(w, f.family)?;
    w.u8(u8::from(f.scheme.is_some()))?;
    w.blob(f.scheme.as_deref().unwrap_or("").as_bytes())?;
    write_opt_u8(w, f.charset)
}

fn read_font<R: Read>(r: &mut Reader<R>) -> Result<Font, JournalError> {
    let name = read_text(r)?;
    let size = r.u32()?;
    let flags = r.u8()?;
    let underline = variant(&UNDERLINES, r.u8()?)?;
    let color = read_color(r)?;
    let baseline = match r.u8()? {
        0 => None,
        1 => Some(Baseline::Superscript),
        2 => Some(Baseline::Subscript),
        _ => return Err(JournalError::Corrupt("an unknown baseline")),
    };
    let family = read_opt_u8(r)?;
    let has_scheme = r.u8()? == 1;
    let scheme = read_text(r)?;
    Ok(Font {
        name,
        size,
        bold: flags & 1 != 0,
        italic: flags & 2 != 0,
        underline,
        strike: flags & 4 != 0,
        color,
        baseline,
        family,
        scheme: has_scheme.then_some(scheme),
        charset: read_opt_u8(r)?,
    })
}

fn write_fill<W: Write>(w: &mut Writer<W>, f: &Fill) -> io::Result<()> {
    match f {
        Fill::Pattern { pattern, fg, bg } => {
            w.u8(0)?;
            match pattern {
                Pattern::None => w.u8(0)?,
                Pattern::Solid => w.u8(1)?,
                Pattern::Other(name) => {
                    w.u8(2)?;
                    w.blob(name.as_bytes())?;
                }
            }
            write_color(w, *fg)?;
            write_color(w, *bg)
        }
        Fill::Gradient { first } => {
            w.u8(1)?;
            write_color(w, *first)
        }
    }
}

fn read_fill<R: Read>(r: &mut Reader<R>) -> Result<Fill, JournalError> {
    Ok(match r.u8()? {
        0 => {
            let pattern = match r.u8()? {
                0 => Pattern::None,
                1 => Pattern::Solid,
                2 => Pattern::Other(read_text(r)?),
                _ => return Err(JournalError::Corrupt("an unknown fill pattern")),
            };
            Fill::Pattern {
                pattern,
                fg: read_color(r)?,
                bg: read_color(r)?,
            }
        }
        1 => Fill::Gradient {
            first: read_color(r)?,
        },
        _ => return Err(JournalError::Corrupt("an unknown fill")),
    })
}

fn write_border<W: Write>(w: &mut Writer<W>, b: &Border) -> io::Result<()> {
    for e in [&b.left, &b.right, &b.top, &b.bottom, &b.diagonal] {
        w.u8(code_of(&LINES, &e.style))?;
        write_color(w, e.color)?;
    }
    w.u8(u8::from(b.diagonal_up) | u8::from(b.diagonal_down) << 1)
}

fn read_border<R: Read>(r: &mut Reader<R>) -> Result<Border, JournalError> {
    let mut edge = || -> Result<Edge, JournalError> {
        Ok(Edge {
            style: variant(&LINES, r.u8()?)?,
            color: read_color(r)?,
        })
    };
    let (left, right, top, bottom, diagonal) = (edge()?, edge()?, edge()?, edge()?, edge()?);
    let flags = r.u8()?;
    Ok(Border {
        left,
        right,
        top,
        bottom,
        diagonal,
        diagonal_up: flags & 1 != 0,
        diagonal_down: flags & 2 != 0,
    })
}

fn write_xf<W: Write>(w: &mut Writer<W>, x: &Xf) -> io::Result<()> {
    for v in [x.num_fmt, x.font, x.fill, x.border, x.xf_id] {
        w.u32(v)?;
    }
    let a = &x.align;
    w.u8(code_of(&HALIGNS, &a.horizontal))?;
    w.u8(code_of(&VALIGNS, &a.vertical))?;
    w.u8(u8::from(a.wrap) | u8::from(a.shrink) << 1)?;
    w.u8(a.indent)?;
    w.u8(a.rotation)?;
    w.u8(u8::from(x.locked) | u8::from(x.hidden) << 1 | u8::from(x.quote_prefix) << 2)
}

fn read_xf<R: Read>(r: &mut Reader<R>) -> Result<Xf, JournalError> {
    let (num_fmt, font, fill, border, xf_id) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?);
    let horizontal = variant(&HALIGNS, r.u8()?)?;
    let vertical = variant(&VALIGNS, r.u8()?)?;
    let flags = r.u8()?;
    let (indent, rotation) = (r.u8()?, r.u8()?);
    let marks = r.u8()?;
    Ok(Xf {
        num_fmt,
        font,
        fill,
        border,
        xf_id,
        align: Align {
            horizontal,
            vertical,
            wrap: flags & 1 != 0,
            shrink: flags & 2 != 0,
            indent,
            rotation,
        },
        locked: marks & 1 != 0,
        hidden: marks & 2 != 0,
        quote_prefix: marks & 4 != 0,
    })
}

fn write_list<W: Write, T>(
    w: &mut Writer<W>,
    items: &[T],
    mut write: impl FnMut(&mut Writer<W>, &T) -> io::Result<()>,
) -> io::Result<()> {
    w.u64(items.len() as u64)?;
    items.iter().try_for_each(|item| write(w, item))
}

fn read_list<R: Read, T>(
    r: &mut Reader<R>,
    min_bytes: u64,
    mut read: impl FnMut(&mut Reader<R>) -> Result<T, JournalError>,
) -> Result<Vec<T>, JournalError> {
    (0..r.count(min_bytes)?).map(|_| read(r)).collect()
}

fn write_pool<W: Write>(w: &mut Writer<W>, p: &JournalPool) -> io::Result<()> {
    for n in [p.base.fonts, p.base.fills, p.base.borders, p.base.xfs] {
        w.u32(n)?;
    }
    write_list(w, &p.fonts, write_font)?;
    write_list(w, &p.fills, write_fill)?;
    write_list(w, &p.borders, write_border)?;
    write_list(w, &p.xfs, write_xf)?;
    write_list(w, &p.num_fmts, |w, (id, code)| {
        w.u32(*id)?;
        w.blob(code.as_bytes())
    })
}

fn read_pool<R: Read>(r: &mut Reader<R>) -> Result<JournalPool, JournalError> {
    let base = FileCounts {
        fonts: r.u32()?,
        fills: r.u32()?,
        borders: r.u32()?,
        xfs: r.u32()?,
    };
    let pool = JournalPool {
        base,
        fonts: read_list(r, 16, read_font)?,
        fills: read_list(r, 3, read_fill)?,
        borders: read_list(r, 11, read_border)?,
        xfs: read_list(r, 26, read_xf)?,
        num_fmts: read_list(r, 12, |r| Ok((r.u32()?, read_text(r)?)))?,
    };
    pool.check()?;
    Ok(pool)
}

impl Workbook {
    /// Write the journal of every sheet (on a snapshot, off the UI thread).
    pub fn write_journal(&self, header: &JournalHeader, out: impl Write) -> io::Result<()> {
        let mut w = Writer {
            inner: io::BufWriter::new(out),
            hash: Fnv::new(),
        };
        write_header(&mut w, header)?;
        w.u32(self.sheets().len() as u32)?;
        let active = self
            .index_of(self.active_id())
            .expect("the active sheet exists");
        w.u32(active as u32)?;
        for s in self.sheets() {
            w.u32(s.id.0)?;
            w.blob(s.name.as_bytes())?;
            match s.origin {
                Origin::Csv => w.u8(0)?,
                Origin::Xlsx(k) => {
                    w.u8(1)?;
                    w.u32(k)?;
                }
                Origin::New => w.u8(2)?,
            }
            w.u8(u8::from(s.table.has_header()))?;
            write_state(&mut w, &s.table)?;
            write_formats(&mut w, &s.table)?;
        }
        write_pool(&mut w, &JournalPool::of(self.styles()))?;
        let hash = w.hash.0;
        w.inner.write_all(&hash.to_le_bytes())?;
        w.inner.flush()
    }

    /// The steps that turn this freshly opened workbook into the journaled one: sheets the
    /// journal lacks removed, sheets added in the crashed session put back (empty, then
    /// restored), names and order as journaled, and every sheet's edits as
    /// [`Edit::Restore`], its formats as [`Edit::Styles`], and its merged cells as
    /// [`Edit::Merges`]. Apply them in order (one undo step). Each state is checked first
    /// ([`CsvTable::check_edit_state`]) against the sheet with the same origin, so nothing is
    /// half-applied; the journal's style records are then added to the pool (it only
    /// grows, so that needs no undo). Header-row flags aren't part of the plan: the caller
    /// sets them from `state`, and activates the sheet at `state.active`.
    pub fn restore_plan(&mut self, state: BookState) -> Result<Vec<BookEdit>, JournalError> {
        let untitled = CsvTable::untitled();
        // The fresh sheet each journal sheet becomes (`None`: added in the session).
        let mut matched: Vec<Option<SheetId>> = Vec::with_capacity(state.sheets.len());
        let mut used = vec![false; self.sheets().len()];
        let xfs = state.pool.as_ref().map_or(0, JournalPool::xf_count);
        for s in &state.sheets {
            if s.origin == Origin::New {
                untitled.check_edit_state(&s.state)?;
                check_formats(&s.formats, xfs)?;
                matched.push(None);
                continue;
            }
            let Some(i) =
                (0..used.len()).find(|&i| !used[i] && self.sheets()[i].origin == s.origin)
            else {
                return Err(JournalError::Mismatch(
                    "a sheet in the journal isn't in the file",
                ));
            };
            used[i] = true;
            self.sheets()[i].table.check_edit_state(&s.state)?;
            check_formats(&s.formats, xfs)?;
            matched.push(Some(self.sheets()[i].id));
        }
        if let Some(p) = &state.pool {
            if self.styles().from_file != p.base {
                return Err(JournalError::Mismatch("the file's styles changed"));
            }
        }
        let restyle: Box<dyn Fn(StyleId) -> StyleId> = match &state.pool {
            Some(p) => Box::new(p.merge_into(self.styles_mut())),
            None => Box::new(|id| id),
        };
        let mut plan = Vec::new();
        // The sheets, in order, as the plan so far leaves them.
        let mut order: Vec<SheetId> = self.sheets().iter().map(|s| s.id).collect();
        // Added sheets first, at the end, so removing never empties the workbook.
        let mut next = self.sheets().iter().map(|s| s.id.0 + 1).max().unwrap_or(0);
        let mut ids = Vec::with_capacity(state.sheets.len());
        for (s, m) in state.sheets.iter().zip(&matched) {
            let id = m.unwrap_or_else(|| {
                let id = SheetId(next);
                next += 1;
                plan.push(BookEdit::Insert {
                    at: order.len(),
                    sheet: Box::new(Sheet {
                        id,
                        name: s.name.clone(),
                        table: CsvTable::untitled(),
                        origin: Origin::New,
                        cached: Default::default(),
                    }),
                });
                order.push(id);
                id
            });
            ids.push(id);
        }
        for at in (0..used.len()).rev().filter(|&i| !used[i]) {
            plan.push(BookEdit::Remove { at });
            order.remove(at);
        }
        for (to, id) in ids.iter().enumerate() {
            let from = order
                .iter()
                .position(|o| o == id)
                .expect("every journal sheet is in the book");
            if from != to {
                plan.push(BookEdit::Move { from, to });
                let id = order.remove(from);
                order.insert(to, id);
            }
        }
        let mut restores = Vec::new();
        for ((s, m), &id) in state.sheets.into_iter().zip(&matched).zip(&ids) {
            if m.is_some() && self.sheet(id).is_some_and(|f| f.name != s.name) {
                plan.push(BookEdit::Rename { id, name: s.name });
            }
            if !s.state.is_empty() {
                restores.push((id, Edit::Restore(Box::new(s.state))));
            }
            let styles = s.formats.styles.as_edit(&restyle);
            if !styles.is_empty() {
                restores.push((id, Edit::Styles(styles)));
            }
            if let Some(list) = s.formats.merges {
                let now = match m {
                    Some(f) => self.sheet(*f).map_or(&[][..], |f| f.table.merges()),
                    None => &[],
                };
                if list != now {
                    restores.push((id, Edit::Merges(Arc::new(list))));
                }
            }
        }
        if !restores.is_empty() {
            plan.push(BookEdit::Sheets(restores));
        }
        Ok(plan)
    }
}

impl CsvTable {
    /// Whether `state` fits this table: rows and columns it names exist (each row once),
    /// and inserted ones are below its counters. Needs the whole file indexed.
    pub fn check_edit_state(&self, s: &EditState) -> Result<(), JournalError> {
        use JournalError::Mismatch;
        if !self.index.is_complete() {
            return Err(Mismatch("the file isn't indexed yet"));
        }
        let file_rows = self.index.row_count();
        let row_ok = |id: RowId| {
            if id.is_inserted() {
                id.0 & !(1 << 63) < s.next_inserted
            } else {
                id.0 < file_rows
            }
        };
        let col_ok = |id: ColId| !id.is_inserted() || id.0 & !(1 << 31) < s.next_inserted_col;
        for (id, cells) in s.overlay.iter_rows() {
            if !row_ok(*id) || !cells.keys().all(|&c| col_ok(c)) {
                return Err(Mismatch(
                    "an edit names a row or column the file doesn't have",
                ));
            }
        }
        if let Some(order) = &s.rows {
            let mut seen = vec![0u64; (file_rows as usize).div_ceil(64)];
            let mut bad = false;
            order.for_each(|run| {
                for i in 0..run.len {
                    let id = RowId(run.first.0 + i);
                    if !row_ok(id) {
                        bad = true;
                    } else if !id.is_inserted() {
                        let (w, b) = ((id.0 / 64) as usize, id.0 % 64);
                        bad |= seen[w] >> b & 1 == 1;
                        seen[w] |= 1 << b;
                    }
                }
            });
            if bad {
                return Err(Mismatch(
                    "the row order names a row twice or one the file lacks",
                ));
            }
        }
        if let Some(m) = &s.cols {
            if !m.parts().0.iter().all(|&c| col_ok(c)) {
                return Err(Mismatch(
                    "the column order names a column that doesn't exist",
                ));
            }
        }
        if s.cleared
            .blocks()
            .any(|(ranges, _)| ranges.iter().any(|r| r.end > file_rows))
        {
            return Err(Mismatch("a cleared block reaches past the last row"));
        }
        Ok(())
    }

    pub(crate) fn take_edit_state(&mut self) -> EditState {
        EditState {
            overlay: std::mem::take(&mut self.overlay),
            rows: self.rows.take(),
            cols: self.cols.take(),
            cleared: std::mem::take(&mut self.cleared),
            next_inserted: self.next_inserted,
            next_inserted_col: self.next_inserted_col,
        }
    }

    pub(crate) fn put_edit_state(&mut self, s: EditState) {
        self.overlay = s.overlay;
        self.rows = s.rows;
        self.cols = s.cols;
        self.cleared = s.cleared;
        // Counters only grow, so an undone restore can't hand out an id twice.
        self.next_inserted = self.next_inserted.max(s.next_inserted);
        self.next_inserted_col = self.next_inserted_col.max(s.next_inserted_col);
    }
}

/// Whether journaled formats name only cell formats the journal's pool has (`xfs` in
/// all). Rows and columns aren't checked: a style on a row that isn't there shows
/// nowhere.
fn check_formats(f: &SheetFormats, xfs: u64) -> Result<(), JournalError> {
    let u = &f.styles;
    let runs = u.cells.values().chain(std::iter::once(&u.rows));
    let styles = runs
        .flat_map(|r| r.values().map(|(_, s)| *s))
        .chain(u.cols.values().copied());
    for s in styles {
        if u64::from(s.0) >= xfs {
            return Err(JournalError::Corrupt("a format the journal lacks"));
        }
    }
    Ok(())
}

/// Read a whole journal (any version), after checking the hash: its header and the
/// workbook state. A v0.1.x journal reads as one sheet, named after its file.
pub fn read_book_journal(r: impl Read) -> Result<(JournalHeader, BookState), JournalError> {
    let mut r = Reader::new(io::BufReader::new(r));
    let (header, version) = read_header_from(&mut r)?;
    let book = if version == 1 {
        let name = header
            .path
            .as_deref()
            .and_then(|p| p.file_stem())
            .map_or_else(|| "Sheet1".to_owned(), |s| s.to_string_lossy().into_owned());
        BookState {
            active: 0,
            sheets: vec![SheetState {
                id: SheetId(0),
                name,
                origin: if header.path.is_some() {
                    Origin::Csv
                } else {
                    Origin::New
                },
                has_header: header.has_header,
                state: read_state(&mut r)?,
                formats: SheetFormats::default(),
            }],
            pool: None,
        }
    } else {
        read_sheets(&mut r, version)?
    };
    let want = r.hash.0;
    let mut got = [0; 8];
    r.inner.read_exact(&mut got)?;
    if u64::from_le_bytes(got) != want {
        return Err(JournalError::Corrupt("checksum mismatch"));
    }
    if r.inner.read(&mut [0])? != 0 {
        return Err(JournalError::Corrupt("trailing bytes"));
    }
    Ok((header, book))
}

fn read_sheets<R: Read>(r: &mut Reader<R>, version: u8) -> Result<BookState, JournalError> {
    let count = r.u32()?;
    if count == 0 || count > MAX_SHEETS {
        return Err(JournalError::Corrupt("impossible sheet count"));
    }
    let active = r.u32()?;
    if active >= count {
        return Err(JournalError::Corrupt("the active sheet isn't there"));
    }
    let mut sheets: Vec<SheetState> = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let id = SheetId(r.u32()?);
        if sheets.iter().any(|s| s.id == id) {
            return Err(JournalError::Corrupt("two sheets share an id"));
        }
        let name = String::from_utf8(r.blob()?)
            .map_err(|_| JournalError::Corrupt("a sheet name isn't UTF-8"))?;
        let origin = match r.u8()? {
            0 => Origin::Csv,
            1 => Origin::Xlsx(r.u32()?),
            2 => Origin::New,
            _ => return Err(JournalError::Corrupt("unknown sheet origin")),
        };
        let has_header = r.u8()? == 1;
        let state = read_state(r)?;
        let formats = if version >= 3 {
            read_formats(r)?
        } else {
            SheetFormats::default()
        };
        sheets.push(SheetState {
            id,
            name,
            origin,
            has_header,
            state,
            formats,
        });
    }
    let pool = if version >= 3 {
        Some(read_pool(r)?)
    } else {
        None
    };
    Ok(BookState {
        active,
        sheets,
        pool,
    })
}
