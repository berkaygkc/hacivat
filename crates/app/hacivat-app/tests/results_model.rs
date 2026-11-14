//! M3a Task 26: the grid adapter over a result store (columns, the display cache that survives
//! version bumps, placeholders, change notifications), the console's result tabs (reuse,
//! titles, widths, the status-bar context) and raw-TSV copy (exact text, raw versus display,
//! the guards). Stores are `ResultStore::from_rows` fixtures or a live store with its writer;
//! no core runtime is needed except for the status-bar context test.

// A copy region is a list of row ranges, so `vec![0..3]` means a list holding one range.
#![allow(clippy::single_range_in_vec_init)]

mod support;

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::FutureExt as _;
use gpui::{AppContext as _, ClipboardItem, TestAppContext};
use hacivat_app::results::adapter::{GridAdapter, ResultDisplay};
use hacivat_app::results::cache::{DISPLAY_CACHE_CAPACITY, DisplayCache, DisplayKey};
use hacivat_app::results::context::result_context_of;
use hacivat_app::results::copy::{self, CopyOutcome, MAX_COPY_BYTES, TooLarge};
use hacivat_app::results::tabs::MemoryBanner;
use hacivat_app::results::{ActiveResult, CompletenessView, ConsoleResults};
use hacivat_exec::{ResultTab, SessionLog, TabKey};
use hacivat_l10n::Catalogs;
use hacivat_proto::{AccessLabel, DbmsKind, EnvTag};
use hacivat_results::{
    CloseReason, Completeness, CursorInfo, FetchControl, FetchPolicy, MemoryBudget, PauseReason,
    ResultId, ResultSource, ResultStore, ResultWriter, StoreSource,
};
use hacivat_session::session::{RunId, UnitId};
use hacivat_ui::toast::ToastQueue;
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_grid::model::{CopyRefusal, CopyRegion, Dir};
use hacivat_ui_grid::testing::grid_config;
use hacivat_ui_grid::{
    Align, CellContent, CellKind, ColId, ColumnKind, GridCellFlags, GridCommand, GridDataSource,
    GridView, ModelRow,
};
use hacivat_values::valfmt::DisplayZone;
use hacivat_values::{
    Chunk, ColumnMeta, DecimalVal, ErrorVal, LenUnit, LogicalType, NativeType, ResultSchema,
    TruncatedVal, TsVal, Value,
};

// ---------------------------------------------------------------------------------------------
// Fixtures

fn pg(name: &str, logical: LogicalType, native: &str) -> ColumnMeta {
    ColumnMeta {
        name: name.into(),
        logical,
        native: NativeType::Pg {
            oid: 0,
            typmod: -1,
            name: native.into(),
        },
        origin: None,
    }
}

fn int8() -> LogicalType {
    LogicalType::Int {
        bits: 64,
        signed: true,
    }
}

fn text() -> LogicalType {
    LogicalType::Text {
        max_chars: None,
        fixed: false,
    }
}

fn schema(columns: Vec<ColumnMeta>) -> Arc<ResultSchema> {
    Arc::new(ResultSchema {
        columns: columns.into(),
    })
}

fn t(s: &str) -> Value {
    Value::Text(s.into())
}

/// What the session would hear from a result source.
#[derive(Default)]
struct Control {
    fetches: Mutex<Vec<u64>>,
    fetch_alls: Mutex<u32>,
}

impl FetchControl for Control {
    fn fetch_more(&self, _result: ResultId, rows: u64) {
        self.fetches.lock().unwrap().push(rows);
    }
    fn close(&self, _result: ResultId, _reason: CloseReason) {}
    fn fetch_all(&self, _result: ResultId) {
        *self.fetch_alls.lock().unwrap() += 1;
    }
}

fn source_of(store: Arc<ResultStore>) -> Arc<dyn ResultSource> {
    StoreSource::new(store, Arc::new(Control::default()), FetchPolicy::default())
}

fn en() -> ResultDisplay {
    ResultDisplay::new("en-US", DisplayZone::utc(), DbmsKind::Postgres)
}

fn tr_istanbul() -> ResultDisplay {
    ResultDisplay::new(
        "tr-TR",
        DisplayZone::load(Some("Europe/Istanbul")),
        DbmsKind::Postgres,
    )
}

/// A finished result of `rows` as an adapter.
fn adapter_over(schema: Arc<ResultSchema>, rows: Vec<Vec<Value>>) -> Rc<GridAdapter> {
    let store = ResultStore::from_rows(schema, rows, Completeness::Complete);
    GridAdapter::new(source_of(store), en())
}

fn three_columns() -> Arc<ResultSchema> {
    schema(vec![
        pg("id", int8(), "int8"),
        pg("note", text(), "text"),
        pg("label", text(), "text"),
    ])
}

/// 3 × 3 with a NULL, a tab, quotes, the text `NULLTEXT` and a cell of one space (P18).
fn fixture_3x3() -> Rc<GridAdapter> {
    adapter_over(
        three_columns(),
        vec![
            vec![Value::Int(1), Value::Null, t("a\tb")],
            vec![Value::Int(2), t("say \"hi\""), t("x")],
            vec![Value::Int(3), t("NULLTEXT"), t(" ")],
        ],
    )
}

fn one_cell(value: &str) -> Rc<GridAdapter> {
    adapter_over(schema(vec![pg("v", text(), "text")]), vec![vec![t(value)]])
}

/// Every loaded row and every column.
fn whole(adapter: &GridAdapter) -> CopyRegion {
    CopyRegion {
        rows: vec![0..adapter.snapshot().row_count()],
        cols: adapter.columns().iter().map(|c| c.id).collect(),
    }
}

fn tsv(adapter: &GridAdapter) -> String {
    copy::tsv(adapter, &whole(adapter), false, MAX_COPY_BYTES).expect("within the limits")
}

fn names(adapter: &GridAdapter) -> Vec<String> {
    adapter
        .columns()
        .iter()
        .map(|c| c.name.to_string())
        .collect()
}

/// One frame over `adapter`: the cells of `rows` × every column.
fn paint_rows(
    cx: &mut TestAppContext,
    adapter: &Rc<GridAdapter>,
    rows: Range<u64>,
) -> Vec<Vec<CellContent>> {
    let adapter = adapter.clone();
    cx.update(|cx| {
        let frame = adapter.begin_frame(cx);
        let cols = adapter.columns();
        rows.map(|r| cols.iter().map(|c| frame.cell(ModelRow(r), c.id)).collect())
            .collect()
    })
}

fn numbered_rows(rows: Range<i64>) -> Vec<Vec<Value>> {
    rows.map(|i| vec![Value::Int(i), t(&format!("row {i}")), t("x")])
        .collect()
}

/// A live result that is still streaming: its store and its writer (kept by the test).
fn streaming(rows: Range<i64>) -> (Arc<ResultStore>, ResultWriter) {
    let schema = three_columns();
    let (store, mut writer) = ResultStore::new(
        ResultId(26),
        schema.clone(),
        MemoryBudget::new(u64::MAX, u64::MAX),
        1,
    );
    writer.append(Chunk::from_values(&schema, numbered_rows(rows)));
    (store, writer)
}

fn append_chunk(writer: &mut ResultWriter, rows: Range<i64>) {
    writer.append(Chunk::from_values(&three_columns(), numbered_rows(rows)));
}

fn install_l10n(cx: &mut TestAppContext) {
    cx.update(|cx| {
        hacivat_app::l10n_global::install_l10n(
            cx,
            Arc::new(support::l10n_en()),
            Arc::new(Catalogs::embedded()),
        );
    });
}

/// Without the isolation marks around placeables.
fn plain(text: &str) -> String {
    text.replace(['\u{2068}', '\u{2069}'], "")
}

fn result_tab(unit_ordinal: u32, source: Arc<dyn ResultSource>) -> ResultTab {
    ResultTab {
        key: TabKey {
            console: 1,
            unit_ordinal,
            result_ordinal: 0,
        },
        // The run service's own title; the tab strip shows the console's `Result N`.
        title: "".into(),
        unit: UnitId {
            run: RunId(1),
            ordinal: unit_ordinal,
        },
        source,
    }
}

fn finished(schema: Arc<ResultSchema>, rows: Vec<Vec<Value>>) -> Arc<dyn ResultSource> {
    source_of(ResultStore::from_rows(schema, rows, Completeness::Complete))
}

// ---------------------------------------------------------------------------------------------
// Copy: the exact TSV

#[test]
fn copy_region_with_null_tab_and_quote_is_exact_tsv() {
    assert_eq!(
        tsv(&fixture_3x3()),
        "1\t\t\"a\tb\"\n2\t\"say \"\"hi\"\"\"\tx\n3\tNULLTEXT\t\" \"\n"
    );
}

#[test]
fn copy_keeps_control_characters_verbatim() {
    assert!(tsv(&one_cell("x\u{202E}y")).contains("x\u{202E}y"));
    // A line break or a carriage return inside a value is kept too, inside quotes.
    assert_eq!(tsv(&one_cell("a\nb")), "\"a\nb\"\n");
    assert_eq!(tsv(&one_cell("a\rb")), "\"a\rb\"\n");
    // Outer white space is quoted, inner is not; the empty string is an empty cell.
    assert_eq!(tsv(&one_cell(" a")), "\" a\"\n");
    assert_eq!(tsv(&one_cell("a b")), "a b\n");
    assert_eq!(tsv(&one_cell("")), "\n");
}

#[test]
fn copy_with_headers_puts_the_column_names_first() {
    let adapter = adapter_over(
        schema(vec![pg("id", int8(), "int8"), pg("a\tb", text(), "text")]),
        vec![vec![Value::Int(7), t("x")]],
    );
    let text = copy::tsv(&adapter, &whole(&adapter), true, MAX_COPY_BYTES).unwrap();
    assert_eq!(text, "id\t\"a\tb\"\n7\tx\n");
}

#[test]
fn copy_takes_the_region_only_and_skips_what_the_result_lacks() {
    let adapter = fixture_3x3();
    // Rows 1..3 of columns 2 and 0, in that order; a row range past the result and a column
    // the schema does not have are left out.
    let region = CopyRegion {
        rows: vec![1..3, 3..9],
        cols: vec![ColId(2), ColId(0), ColId(9)],
    };
    assert_eq!(
        copy::tsv(&adapter, &region, false, MAX_COPY_BYTES).unwrap(),
        "x\t2\n\" \"\t3\n"
    );
}

#[test]
fn a_text_past_the_byte_limit_is_refused() {
    let adapter = fixture_3x3();
    assert_eq!(
        copy::tsv(&adapter, &whole(&adapter), false, 8),
        Err(TooLarge)
    );
    let exact = tsv(&adapter).len();
    assert!(copy::tsv(&adapter, &whole(&adapter), false, exact).is_ok());
    assert_eq!(
        copy::tsv(&adapter, &whole(&adapter), false, exact - 1),
        Err(TooLarge)
    );
}

// ---------------------------------------------------------------------------------------------
// Raw and display never mix

/// 2026-11-11 07:00:00.25 UTC.
const INSTANT_MICROS: i64 = 1_794_380_400_250_000;

fn mixed() -> (Arc<ResultSchema>, Vec<Vec<Value>>) {
    let schema = schema(vec![
        pg("id", int8(), "int8"),
        pg(
            "amount",
            LogicalType::Decimal {
                precision: Some(14),
                scale: Some(2),
            },
            "numeric",
        ),
        pg(
            "at",
            LogicalType::Timestamp {
                precision: 6,
                tz: true,
            },
            "timestamptz",
        ),
        pg("raw", LogicalType::Bytes { max: None }, "bytea"),
        pg("ok", LogicalType::Bool, "bool"),
        pg("big", int8(), "int8"),
    ]);
    let bytes: Vec<u8> = (0u8..16).collect();
    let rows = vec![
        vec![
            Value::Int(1_234_567),
            Value::Decimal(DecimalVal::parse("1234567.50").unwrap()),
            Value::TimestampTz {
                utc: TsVal::from_unix_micros(INSTANT_MICROS),
                offset: None,
            },
            Value::Bytes(bytes.into()),
            Value::Bool(true),
            Value::Int(7_654_321),
        ],
        vec![
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
        ],
    ];
    (schema, rows)
}

#[gpui::test]
fn the_grid_shows_display_text_and_copy_takes_raw_text(cx: &mut TestAppContext) {
    let (schema, rows) = mixed();
    let store = ResultStore::from_rows(schema, rows, Completeness::Complete);
    let adapter = GridAdapter::new(source_of(store), tr_istanbul());
    let cells = paint_rows(cx, &adapter, 0..2);
    let shown: Vec<&str> = cells[0].iter().map(|c| c.text.as_ref()).collect();
    // The ID column is never grouped; the other numbers follow the format locale.
    assert_eq!(shown[0], "1234567");
    assert_eq!(shown[1], "1.234.567,50");
    assert_eq!(shown[5], "7.654.321");
    // A 16-byte value shows as a UUID (`Detect`).
    assert_eq!(shown[3], "00010203-0405-0607-0809-0a0b0c0d0e0f");
    assert_eq!(shown[4], "true");

    let copied = tsv(&adapter);
    let line: Vec<&str> = copied.lines().next().unwrap().split('\t').collect();
    assert_eq!(line[0], "1234567");
    assert_eq!(line[1], "1234567.50", "copy is locale-free");
    assert_eq!(line[5], "7654321");
    // The copied instant is the one the cell shows, in the console's display zone.
    assert_eq!(line[2], shown[2]);
    assert!(line[2].ends_with("+03"), "{}", line[2]);
    // Bytes copy as hex whatever the cell shows.
    assert_eq!(line[3], "\\x000102030405060708090a0b0c0d0e0f");
    assert_eq!(line[4], "true");
    // NULL: the marker in the grid, an empty cell in the copy.
    assert!(cells[1].iter().all(|c| c.kind == CellKind::Null));
    assert!(cells[1].iter().all(|c| c.text.as_ref() == "NULL"));
    assert_eq!(copied.lines().nth(1), Some("\t\t\t\t\t"));
}

#[test]
fn lossy_text_copies_as_escapes_that_a_literal_can_spell_too() {
    // The documented limit of the raw channel: not byte-exact, and ambiguous with a literal.
    let lossy = adapter_over(
        schema(vec![pg("v", text(), "text")]),
        vec![vec![Value::Lossy(b"ab\xffcd".as_slice().into())]],
    );
    let literal = one_cell("ab\\xffcd");
    assert_eq!(tsv(&lossy), "ab\\xffcd\n");
    assert_eq!(tsv(&lossy), tsv(&literal));
}

#[gpui::test]
fn truncated_and_undecodable_cells(cx: &mut TestAppContext) {
    install_l10n(cx);
    let truncated = Value::Truncated(Box::new(TruncatedVal {
        prefix: t("the kept prefix"),
        total_len: Some(5_452_595),
        unit: LenUnit::Bytes,
    }));
    let error = Value::Error(Arc::new(ErrorVal {
        code: "decode".into(),
        message: "invalid byte sequence".into(),
    }));
    let adapter = adapter_over(
        schema(vec![pg("a", text(), "text"), pg("b", text(), "text")]),
        vec![vec![truncated, error]],
    );
    let cells = paint_rows(cx, &adapter, 0..1);
    let (cut, bad) = (&cells[0][0], &cells[0][1]);
    assert_eq!(cut.text.as_ref(), "the kept prefix");
    assert!(cut.flags.contains(GridCellFlags::TRUNCATED));
    // The chip is one Fluent message with the unit inside (5,452,595 bytes are 5.2 MB).
    assert_eq!(plain(cut.chip.as_ref().expect("a chip")), "… 5.2 MB");
    // A decode error shows chrome text; the message stays out of the cell.
    assert_eq!(bad.kind, CellKind::Error);
    assert_eq!(bad.text.as_ref(), "Can't decode");
    assert_eq!(bad.align, Align::Start);
    // Copy: the kept prefix without a marker, and the error's message.
    assert_eq!(tsv(&adapter), "the kept prefix\tinvalid byte sequence\n");
}

#[gpui::test]
fn control_characters_are_flagged_for_the_grid(cx: &mut TestAppContext) {
    let adapter = adapter_over(
        schema(vec![pg("v", text(), "text")]),
        vec![vec![t("one\ntwo")], vec![t("a\tb")], vec![t("plain")]],
    );
    let cells = paint_rows(cx, &adapter, 0..3);
    assert!(
        cells[0][0]
            .flags
            .contains(GridCellFlags::MULTILINE | GridCellFlags::HAS_CONTROLS)
    );
    assert_eq!(cells[1][0].flags, GridCellFlags::HAS_CONTROLS);
    assert_eq!(cells[2][0].flags, GridCellFlags::empty());
    // The text itself is untouched: the grid draws the chips.
    assert_eq!(cells[0][0].text.as_ref(), "one\ntwo");
}

// ---------------------------------------------------------------------------------------------
// Columns

#[test]
fn duplicate_column_names_keep_their_order() {
    let dupes = adapter_over(
        schema(vec![
            pg("id", int8(), "int8"),
            pg("id", text(), "text"),
            pg("name", text(), "text"),
        ]),
        vec![vec![Value::Int(1), t("second"), t("n")]],
    );
    assert_eq!(names(&dupes), ["id", "id", "name"]);
    let ids: Vec<u32> = dupes.columns().iter().map(|c| c.id.0).collect();
    assert_eq!(ids, [0, 1, 2]);
    assert_eq!(tsv(&dupes), "1\tsecond\tn\n");
}

#[test]
fn columns_carry_the_native_type_kind_alignment_and_default_width() {
    let (schema, rows) = mixed();
    let adapter = adapter_over(schema, rows);
    let cols = adapter.columns();
    let labels: Vec<&str> = cols.iter().map(|c| c.type_label.as_ref()).collect();
    assert_eq!(
        labels,
        [
            "int8",
            "numeric(14,2)",
            "timestamptz",
            "bytea",
            "bool",
            "int8"
        ]
    );
    let kinds: Vec<ColumnKind> = cols.iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [
            ColumnKind::Number,
            ColumnKind::Number,
            ColumnKind::Temporal,
            ColumnKind::Binary,
            ColumnKind::Bool,
            ColumnKind::Number
        ]
    );
    let aligns: Vec<Align> = cols.iter().map(|c| c.align).collect();
    assert_eq!(
        aligns,
        [
            Align::End,
            Align::End,
            Align::Start,
            Align::Start,
            Align::Center,
            Align::End
        ]
    );
    let widths: Vec<f32> = cols.iter().map(|c| c.default_width).collect();
    assert_eq!(widths, [96.0, 120.0, 200.0, 200.0, 64.0, 96.0]);
}

#[test]
fn a_column_without_a_native_name_is_labelled_by_its_logical_type() {
    let mysql = |name: &str, logical: LogicalType| ColumnMeta {
        name: name.into(),
        logical,
        native: NativeType::MySql {
            column_type: 0,
            flags: 0,
            charset: 0,
            length: 0,
            decimals: 0,
        },
        origin: None,
    };
    let adapter = adapter_over(
        schema(vec![
            mysql(
                "n",
                LogicalType::Int {
                    bits: 32,
                    signed: false,
                },
            ),
            mysql(
                "s",
                LogicalType::Text {
                    max_chars: Some(40),
                    fixed: false,
                },
            ),
        ]),
        vec![],
    );
    let labels: Vec<String> = adapter
        .columns()
        .iter()
        .map(|c| c.type_label.to_string())
        .collect();
    assert_eq!(labels, ["int unsigned", "varchar(40)"]);
}

// ---------------------------------------------------------------------------------------------
// The display cache

#[gpui::test]
fn display_cache_survives_version_bumps(cx: &mut TestAppContext) {
    let (store, mut writer) = streaming(0..60);
    let adapter = GridAdapter::new(source_of(store.clone()), en());
    let first = paint_rows(cx, &adapter, 0..40);
    let n = adapter.format_calls();
    assert_eq!(n, 40 * 3, "every cell was formatted once");
    let version = store.snapshot().version();
    append_chunk(&mut writer, 60..120);
    assert!(store.snapshot().version() > version, "the append published");
    let again = paint_rows(cx, &adapter, 0..40);
    assert_eq!(adapter.format_calls(), n, "an append formats nothing again");
    assert_eq!(first, again);
    // The appended rows are there, and only they are formatted.
    let more = paint_rows(cx, &adapter, 60..62);
    assert_eq!(more[0][1].text.as_ref(), "row 60");
    assert_eq!(adapter.format_calls(), n + 2 * 3);
}

#[gpui::test]
fn new_display_settings_format_again(cx: &mut TestAppContext) {
    let (schema, rows) = mixed();
    let store = ResultStore::from_rows(schema, rows, Completeness::Complete);
    let adapter = GridAdapter::new(source_of(store), en());
    assert_eq!(
        paint_rows(cx, &adapter, 0..1)[0][1].text.as_ref(),
        "1,234,567.50"
    );
    let before = adapter.format_calls();
    cx.update(|cx| adapter.set_display(tr_istanbul(), cx));
    assert_eq!(
        paint_rows(cx, &adapter, 0..1)[0][1].text.as_ref(),
        "1.234.567,50"
    );
    assert!(adapter.format_calls() > before);
}

#[test]
fn the_cache_is_bounded_and_keeps_the_current_frame() {
    assert_eq!(DISPLAY_CACHE_CAPACITY, 65_536);
    let mut cache = DisplayCache::with_capacity(64);
    let key = |row: u64| DisplayKey {
        generation: 1,
        row,
        col: 0,
        settings_version: 0,
    };
    let cell = |row: u64| CellContent {
        text: row.to_string().into(),
        kind: CellKind::Text,
        flags: GridCellFlags::empty(),
        align: Align::Start,
        chip: None,
    };
    for frame in 0..10u64 {
        cache.begin_frame();
        for row in frame * 40..frame * 40 + 40 {
            cache.get_or_format(key(row), || cell(row));
        }
        assert!(cache.len() <= 64, "frame {frame}: {}", cache.len());
    }
    // The last frame's cells are still there: reading them formats nothing.
    let formats = cache.formats();
    for row in 360..400 {
        assert_eq!(
            cache.get_or_format(key(row), || cell(0)).text.as_ref(),
            row.to_string()
        );
    }
    assert_eq!(cache.formats(), formats);
    // A different generation or settings version is a different cell.
    let other = DisplayKey {
        generation: 2,
        ..key(399)
    };
    assert_eq!(cache.get_or_format(other, || cell(7)).text.as_ref(), "7");
}

#[gpui::test]
fn rows_past_the_loaded_ones_and_unknown_columns_are_placeholders(cx: &mut TestAppContext) {
    let adapter = fixture_3x3();
    let frame_cells = {
        let adapter = adapter.clone();
        cx.update(|cx| {
            let frame = adapter.begin_frame(cx);
            (
                frame.cell(ModelRow(3), ColId(0)),
                frame.cell(ModelRow(u64::MAX), ColId(2)),
                frame.cell(ModelRow(0), ColId(3)),
                frame.row_count(),
                frame.generation(),
            )
        })
    };
    for cell in [&frame_cells.0, &frame_cells.1, &frame_cells.2] {
        assert_eq!(cell.kind, CellKind::Placeholder);
    }
    assert_eq!(adapter.format_calls(), 0, "a placeholder reads no value");
    assert_eq!(frame_cells.3.loaded, 3);
    assert!(!frame_cells.3.more);
    assert_eq!(frame_cells.3.exact_total, Some(3));
    assert_eq!(frame_cells.4.0, 1);
}

#[gpui::test]
fn a_streaming_result_has_more_rows_and_no_total(cx: &mut TestAppContext) {
    let (store, mut writer) = streaming(0..5);
    let adapter = GridAdapter::new(source_of(store), en());
    let count = |cx: &mut TestAppContext| {
        let adapter = adapter.clone();
        cx.update(|cx| adapter.begin_frame(cx).row_count())
    };
    let streaming = count(cx);
    assert_eq!((streaming.loaded, streaming.more), (5, true));
    assert_eq!(streaming.exact_total, None);
    writer.set_state(Completeness::Complete);
    let done = count(cx);
    assert_eq!(
        (done.loaded, done.more, done.exact_total),
        (5, false, Some(5))
    );
}

#[gpui::test]
fn ensure_rows_reaches_the_source_which_coalesces(cx: &mut TestAppContext) {
    let now = Instant::now();
    let cursor = CursorInfo {
        opened_at: now,
        last_fetch: now,
        holds_tx: true,
        idle_bound: Duration::from_secs(600),
    };
    let control = Arc::new(Control::default());
    let suspended = ResultStore::from_rows(
        three_columns(),
        numbered_rows(0..500),
        Completeness::Suspended(cursor),
    );
    let source = StoreSource::new(suspended, control.clone(), FetchPolicy::default());
    let adapter = GridAdapter::new(source, en());
    assert!(adapter.caps().has_cursor);
    cx.update(|cx| {
        adapter.ensure_rows(520, cx);
        adapter.ensure_rows(540, cx);
    });
    assert_eq!(
        control.fetches.lock().unwrap().len(),
        1,
        "one ask per version"
    );

    // At the memory limit the source asks for nothing: the banner is the way on.
    let control = Arc::new(Control::default());
    let paused = ResultStore::from_rows_paused(
        three_columns(),
        numbered_rows(0..500),
        cursor,
        PauseReason::ResultCap,
    );
    let source = StoreSource::new(paused, control.clone(), FetchPolicy::default());
    let adapter = GridAdapter::new(source, en());
    cx.update(|cx| adapter.ensure_rows(520, cx));
    assert!(control.fetches.lock().unwrap().is_empty());
    assert_eq!(adapter.snapshot().paused(), Some(PauseReason::ResultCap));
}

// ---------------------------------------------------------------------------------------------
// Change notifications

#[gpui::test]
fn a_change_calls_the_listener_once_until_a_frame_reads_the_adapter(cx: &mut TestAppContext) {
    let (store, mut writer) = streaming(0..10);
    let adapter = GridAdapter::new(source_of(store), en());
    let calls = Rc::new(Cell::new(0u32));
    let seen = calls.clone();
    cx.update(|cx| adapter.on_change(cx, move |_| seen.set(seen.get() + 1)));
    cx.run_until_parked();
    assert_eq!(calls.get(), 0, "nothing changed yet");

    append_chunk(&mut writer, 10..20);
    append_chunk(&mut writer, 20..30);
    cx.run_until_parked();
    assert_eq!(calls.get(), 1);
    // No frame has read the adapter: further changes add no call.
    append_chunk(&mut writer, 30..40);
    cx.run_until_parked();
    assert_eq!(calls.get(), 1);
    // A frame reads it (and sees every row so far); the next change calls again.
    assert_eq!(
        paint_rows(cx, &adapter, 39..40)[0][1].text.as_ref(),
        "row 39"
    );
    writer.set_state(Completeness::Complete);
    cx.run_until_parked();
    assert_eq!(calls.get(), 2);
}

#[gpui::test]
fn a_publish_before_the_listener_is_attached_is_not_missed(cx: &mut TestAppContext) {
    // The adapter subscribes before its first snapshot; a result that publishes its last
    // state between that and `on_change` would otherwise never wake anyone.
    let (store, mut writer) = streaming(0..10);
    let adapter = GridAdapter::new(source_of(store), en());
    append_chunk(&mut writer, 10..20);
    writer.set_state(Completeness::Complete);
    let calls = Rc::new(Cell::new(0u32));
    let seen = calls.clone();
    cx.update(|cx| adapter.on_change(cx, move |_| seen.set(seen.get() + 1)));
    cx.run_until_parked();
    assert_eq!(calls.get(), 1);
}

// ---------------------------------------------------------------------------------------------
// Re-runs and the grid

fn open_grid<'a>(
    cx: &'a mut TestAppContext,
    adapter: &Rc<GridAdapter>,
) -> (gpui::Entity<GridView>, &'a mut gpui::VisualTestContext) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let source: Rc<dyn GridDataSource> = adapter.clone();
    let (view, vcx) = cx.add_window_view(|_, cx| {
        let mut config = grid_config("Result 1");
        config.auto_size = true;
        GridView::new(source, config, cx)
    });
    let target = view.clone();
    vcx.update(|_, cx| {
        adapter.on_change(cx, move |cx| target.update(cx, |_, cx| cx.notify()));
    });
    vcx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(400.0)));
    vcx.run_until_parked();
    (view, vcx)
}

#[gpui::test]
fn generation_change_clears_the_selection(cx: &mut TestAppContext) {
    let first = ResultStore::from_rows(
        three_columns(),
        numbered_rows(0..50),
        Completeness::Complete,
    );
    let adapter = GridAdapter::new(source_of(first), en());
    let (view, vcx) = open_grid(cx, &adapter);
    for cmd in [
        GridCommand::Move {
            dir: Dir::Down,
            extend: false,
        },
        GridCommand::Move {
            dir: Dir::Down,
            extend: true,
        },
    ] {
        vcx.update(|window, cx| view.update(cx, |v, cx| v.dispatch(cmd, window, cx)));
    }
    vcx.run_until_parked();
    let selected = |vcx: &mut gpui::VisualTestContext| {
        view.read_with(vcx, |v, _| {
            v.selection().cell_count(30, v.column_layout().visible())
        })
    };
    assert_eq!(
        selected(vcx),
        2,
        "two cells of the first result are selected"
    );
    let widths = |vcx: &mut gpui::VisualTestContext| {
        view.read_with(vcx, |v, _| {
            (0..3)
                .map(|c| v.column_layout().width_of(ColId(c)).unwrap())
                .collect::<Vec<f32>>()
        })
    };
    vcx.update(|_, cx| view.update(cx, |v, cx| v.set_column_width(ColId(1), 333.0, cx)));
    vcx.run_until_parked();
    let before = widths(vcx);
    assert_eq!(before[1], 333.0);
    let measured = view.read_with(vcx, |v, _| v.frame_log().autosize_measures);
    assert_eq!(
        measured,
        3 * 50,
        "the first result's columns were auto-sized"
    );

    // A re-run into the same tab: both stores are generation 1 of their own, so the adapter's
    // generation is what tells the grid. Nothing but the adapter's listener asks for the frame.
    let second = ResultStore::from_rows(
        three_columns(),
        numbered_rows(100..130),
        Completeness::Complete,
    );
    vcx.update(|_, cx| adapter.retarget(source_of(second), cx));
    vcx.run_until_parked();
    assert_eq!(adapter.generation(), 2);
    // The range over the old rows is gone: what is left is the focus cell, clamped into the
    // new rows, and the view starts at the top.
    assert_eq!(selected(vcx), 1);
    assert_eq!(view.read_with(vcx, |v, _| v.scroll_state().top_row), 0);
    // The names match, so the widths stay and nothing is measured again.
    assert_eq!(widths(vcx), before);
    assert_eq!(
        view.read_with(vcx, |v, _| v.frame_log().autosize_measures),
        measured
    );
    // The new rows are what the adapter formats.
    let shown = {
        let adapter = adapter.clone();
        vcx.update(|_, cx| adapter.begin_frame(cx).cell(ModelRow(0), ColId(0)))
    };
    assert_eq!(shown.text.as_ref(), "100");
}

#[gpui::test]
fn appended_rows_reach_the_grid_through_the_adapters_listener(cx: &mut TestAppContext) {
    let (store, mut writer) = streaming(0..5);
    let adapter = GridAdapter::new(source_of(store), en());
    let (view, vcx) = open_grid(cx, &adapter);
    let frames = view.read_with(vcx, |v, _| v.frame_log().frames);
    append_chunk(&mut writer, 5..9);
    vcx.run_until_parked();
    let after = view.read_with(vcx, |v, _| v.frame_log().frames);
    assert!(after > frames, "the append repainted the grid");
    assert_eq!(
        view.read_with(vcx, |v, _| v.geometry().map(|g| g.rows.end)),
        Some(9)
    );
}

// ---------------------------------------------------------------------------------------------
// Result tabs

fn console_results(cx: &mut TestAppContext) -> gpui::Entity<ConsoleResults> {
    cx.new(|_| ConsoleResults::new(SessionLog::new()))
}

#[gpui::test]
fn rerun_reuses_the_tab_and_keeps_widths_when_names_match(cx: &mut TestAppContext) {
    let results = console_results(cx);
    let first = result_tab(0, finished(three_columns(), numbered_rows(0..3)));
    let changed = results.update(cx, |r, cx| r.apply(std::slice::from_ref(&first), cx));
    assert_eq!(changed, [0]);
    // The run service republishes its list: the same result is not a re-run.
    let again = results.update(cx, |r, cx| r.apply(std::slice::from_ref(&first), cx));
    assert!(again.is_empty());
    let adapter = results.read_with(cx, |r, _| r.tabs()[0].adapter().clone());
    assert_eq!(adapter.generation(), 1);
    results.update(cx, |r, _| {
        r.set_width(0, ColId(1), 222.0);
        r.set_width(0, ColId(1), 240.0);
        r.set_width(0, ColId(2), 90.0);
    });

    // The same key with a new result of the same columns: the tab, its adapter and its widths
    // stay; the adapter's generation moves.
    let rerun = result_tab(0, finished(three_columns(), numbered_rows(10..12)));
    let changed = results.update(cx, |r, cx| r.apply(&[rerun], cx));
    assert_eq!(changed, [0]);
    results.read_with(cx, |r, _| {
        assert_eq!(r.tabs().len(), 1);
        assert!(Rc::ptr_eq(r.tabs()[0].adapter(), &adapter));
        assert_eq!(r.tabs()[0].widths(), [(ColId(1), 240.0), (ColId(2), 90.0)]);
        assert_eq!(r.tabs()[0].number(), 1);
    });
    assert_eq!(adapter.generation(), 2);
    assert_eq!(adapter.snapshot().row_count(), 2);

    // Other column names: the widths belong to columns that are gone.
    let other = schema(vec![pg("id", int8(), "int8"), pg("total", int8(), "int8")]);
    let rerun = result_tab(0, finished(other, vec![vec![Value::Int(1), Value::Int(2)]]));
    results.update(cx, |r, cx| r.apply(&[rerun], cx));
    results.read_with(cx, |r, _| {
        assert_eq!(r.tabs().len(), 1);
        assert!(r.tabs()[0].widths().is_empty());
    });
    assert_eq!(adapter.generation(), 3);
    assert_eq!(names(&adapter), ["id", "total"]);
}

fn two_units(cx: &mut TestAppContext) -> gpui::Entity<ConsoleResults> {
    let results = console_results(cx);
    let tabs = [
        result_tab(0, finished(three_columns(), numbered_rows(0..3))),
        result_tab(1, finished(three_columns(), numbered_rows(0..1))),
    ];
    results.update(cx, |r, cx| r.apply(&tabs, cx));
    results
}

fn titles(cx: &mut TestAppContext, results: &gpui::Entity<ConsoleResults>) -> Vec<String> {
    results.read_with(cx, |r, cx| {
        (0..r.tabs().len())
            .map(|ix| plain(&r.title(ix, cx).expect("a tab")))
            .collect()
    })
}

#[gpui::test]
fn titles_are_result_n(cx: &mut TestAppContext) {
    install_l10n(cx);
    let results = two_units(cx);
    assert_eq!(titles(cx, &results), ["Result 1", "Result 2"]);
    // A re-run of the second unit keeps its number; a third unit gets the next one, also
    // after a tab was closed.
    let tabs = [
        result_tab(1, finished(three_columns(), numbered_rows(0..2))),
        result_tab(2, finished(three_columns(), numbered_rows(0..2))),
    ];
    results.update(cx, |r, cx| r.apply(&tabs, cx));
    assert_eq!(titles(cx, &results), ["Result 1", "Result 2", "Result 3"]);
    results.update(cx, |r, cx| r.close_tab(0, cx));
    assert_eq!(titles(cx, &results), ["Result 2", "Result 3"]);
    let tabs = [result_tab(7, finished(three_columns(), vec![]))];
    results.update(cx, |r, cx| r.apply(&tabs, cx));
    assert_eq!(titles(cx, &results), ["Result 2", "Result 3", "Result 4"]);
}

#[gpui::test]
fn output_leads_and_the_first_new_result_becomes_active(cx: &mut TestAppContext) {
    let results = console_results(cx);
    results.read_with(cx, |r, _| {
        assert_eq!(r.active(), ActiveResult::Output);
        assert!(r.tabs().is_empty() && !r.unread_output());
        assert!(r.active_tab().is_none());
    });
    // A line while Output is active is seen at once.
    results.update(cx, |r, cx| r.note_output(cx));
    assert!(!results.read_with(cx, |r, _| r.unread_output()));

    let results = two_units(cx);
    assert_eq!(
        results.read_with(cx, |r, _| r.active()),
        ActiveResult::Tab(0)
    );
    results.update(cx, |r, cx| r.note_output(cx));
    assert!(results.read_with(cx, |r, _| r.unread_output()));
    results.update(cx, |r, cx| r.set_active(ActiveResult::Tab(9), cx));
    assert_eq!(
        results.read_with(cx, |r, _| r.active()),
        ActiveResult::Tab(0),
        "a tab that does not exist is not shown"
    );
    results.update(cx, |r, cx| r.set_active(ActiveResult::Output, cx));
    results.read_with(cx, |r, _| {
        assert_eq!(r.active(), ActiveResult::Output);
        assert!(!r.unread_output(), "showing Output reads its lines");
    });
    // A re-run of the second unit shows that tab.
    let rerun = result_tab(1, finished(three_columns(), vec![]));
    results.update(cx, |r, cx| r.apply(&[rerun], cx));
    assert_eq!(
        results.read_with(cx, |r, _| (
            r.active(),
            r.tab_by_key(TabKey {
                console: 1,
                unit_ordinal: 1,
                result_ordinal: 0
            })
        )),
        (ActiveResult::Tab(1), Some(1))
    );
}

#[gpui::test]
fn the_memory_banner_offers_a_raise_until_the_global_budget_is_what_stops_it(
    cx: &mut TestAppContext,
) {
    let now = Instant::now();
    let cursor = CursorInfo {
        opened_at: now,
        last_fetch: now,
        holds_tx: false,
        idle_bound: Duration::from_secs(600),
    };
    let control = Arc::new(Control::default());
    let paused = |reason| -> (Arc<ResultStore>, Arc<dyn ResultSource>) {
        let store =
            ResultStore::from_rows_paused(three_columns(), numbered_rows(0..20), cursor, reason);
        let source = StoreSource::new(store.clone(), control.clone(), FetchPolicy::default());
        (store, source)
    };
    let (capped, capped_source) = paused(PauseReason::ResultCap);
    let (_global, global_source) = paused(PauseReason::GlobalBudget);
    let results = console_results(cx);
    let tabs = [
        result_tab(0, capped_source),
        result_tab(1, global_source),
        result_tab(2, finished(three_columns(), numbered_rows(0..3))),
    ];
    results.update(cx, |r, cx| r.apply(&tabs, cx));
    let banner = |cx: &mut TestAppContext, ix: usize| {
        results.read_with(cx, |r, _| r.tabs()[ix].memory_banner())
    };
    let offered = |reason| {
        Some(MemoryBanner {
            reason,
            can_raise: true,
        })
    };
    assert_eq!(banner(cx, 0), offered(PauseReason::ResultCap));
    assert_eq!(banner(cx, 1), offered(PauseReason::GlobalBudget));
    assert_eq!(banner(cx, 2), None, "a complete result has no banner");

    // "Fetch more anyway": the store's cap is raised and the source fetches on.
    results.update(cx, |r, cx| r.raise_cap(0, cx));
    assert!(capped.cap_raised());
    assert_eq!(*control.fetch_alls.lock().unwrap(), 1);
    // The fixture stays paused at the result cap; that pause can still be raised past.
    assert_eq!(banner(cx, 0), offered(PauseReason::ResultCap));
    // After a raise, a pause for the global budget has nothing left to raise.
    results.update(cx, |r, cx| r.raise_cap(1, cx));
    assert_eq!(
        banner(cx, 1),
        Some(MemoryBanner {
            reason: PauseReason::GlobalBudget,
            can_raise: false
        })
    );
    // A re-run into the tab is a new result: its banner offers the raise again.
    let (_again, again_source) = paused(PauseReason::GlobalBudget);
    results.update(cx, |r, cx| r.apply(&[result_tab(1, again_source)], cx));
    assert_eq!(banner(cx, 1), offered(PauseReason::GlobalBudget));
}

#[gpui::test]
fn the_status_bar_context_follows_the_active_result_tab(cx: &mut TestAppContext) {
    use support::*;
    let vcx = shell_with_connections(cx, "en");
    let profile = test_profile("orders-prod", Some(EnvTag::Production), "127.0.0.1");
    let id = add_profile(vcx, profile.clone());
    let (session, _fake) = test_session(vcx, profile);
    let console = register_test_console(vcx, id, session);
    let results = vcx.cx.read(|cx| {
        hacivat_app::console::Consoles::get(cx, console)
            .and_then(|s| s.results.clone())
            .expect("the console has results")
    });
    let context = |vcx: &gpui::VisualTestContext| vcx.cx.read(|cx| result_context_of(console, cx));
    assert_eq!(context(vcx), None, "Output is active");

    let (store, mut writer) = streaming(0..40);
    let key = TabKey {
        console: 1,
        unit_ordinal: 0,
        result_ordinal: 0,
    };
    vcx.cx.update(|cx| {
        results.update(cx, |r, cx| {
            r.apply(&[result_tab(0, source_of(store))], cx);
            r.set_timing(key, Some(12), None, cx);
        })
    });
    let streaming = context(vcx).expect("a result tab is active");
    assert_eq!(streaming.profile, id);
    assert_eq!(streaming.env, Some(EnvTag::Production));
    // Not connected through the model: a production profile's result reads read-only.
    assert_eq!(streaming.access, AccessLabel::ReadOnlyGuarded);
    assert_eq!((streaming.loaded, streaming.exact), (40, None));
    assert_eq!(streaming.completeness, CompletenessView::Streaming);
    assert_eq!((streaming.exec_ms, streaming.fetch_ms), (Some(12), None));

    append_chunk(&mut writer, 40..50);
    writer.set_state(Completeness::Complete);
    vcx.cx.update(|cx| {
        results.update(cx, |r, cx| r.set_timing(key, Some(12), Some(30), cx));
    });
    let done = context(vcx).expect("still active");
    assert_eq!((done.loaded, done.exact), (50, Some(50)));
    assert_eq!(done.completeness, CompletenessView::Complete);
    assert_eq!(done.fetch_ms, Some(30));

    vcx.cx
        .update(|cx| results.update(cx, |r, cx| r.set_active(ActiveResult::Output, cx)));
    assert_eq!(context(vcx), None);
    stop_core_runtime_test(cx);
}

// ---------------------------------------------------------------------------------------------
// Copy: the clipboard, the toasts and the guards

fn toasts(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| ToastQueue::visible_ids(cx).len())
}

#[gpui::test]
fn copy_puts_one_string_on_the_clipboard_and_says_so(cx: &mut TestAppContext) {
    let adapter = fixture_3x3();
    let region = whole(&adapter);
    let task = cx.update(|cx| copy::copy_region(&adapter, region, false, cx));
    let outcome = task.now_or_never().expect("a small copy ends at once");
    assert_eq!(outcome, CopyOutcome::Copied { rows: 3, cols: 3 });
    let item = cx.read_from_clipboard().expect("something was copied");
    assert_eq!(item.entries().len(), 1, "one clipboard item, one entry");
    assert_eq!(item.text().as_deref(), Some(tsv(&adapter).as_str()));
    assert_eq!(toasts(cx), 1);
}

#[gpui::test]
fn a_large_copy_is_built_in_the_background(cx: &mut TestAppContext) {
    // 5,001 × 3 cells: above the 10,000 the UI thread builds itself.
    let adapter = adapter_over(three_columns(), numbered_rows(0..5_001));
    let region = whole(&adapter);
    assert!(region.cell_count() > copy::BACKGROUND_COPY_CELLS);
    let mut task = cx.update(|cx| copy::copy_region(&adapter, region, true, cx));
    assert!(
        (&mut task).now_or_never().is_none(),
        "the text is not built on the UI thread"
    );
    assert!(cx.read_from_clipboard().is_none());
    assert_eq!(toasts(cx), 1, "the Copying toast");
    cx.run_until_parked();
    let outcome = task.now_or_never().expect("the copy ended");
    assert_eq!(
        outcome,
        CopyOutcome::Copied {
            rows: 5_001,
            cols: 3
        }
    );
    let text = cx.read_from_clipboard().and_then(|i| i.text()).unwrap();
    assert_eq!(text.lines().count(), 5_002, "the header and every row");
    assert!(text.starts_with("id\tnote\tlabel\n0\trow 0\tx\n"));
    assert!(text.ends_with("5000\trow 5000\tx\n"));
    assert_eq!(
        toasts(cx),
        1,
        "the Copying toast gave way to the Copied one"
    );
}

#[gpui::test]
fn copy_over_a_million_cells_is_refused(cx: &mut TestAppContext) {
    let adapter = fixture_3x3();
    cx.write_to_clipboard(ClipboardItem::new_string("before".into()));
    // The guard counts the region; it reads no cell.
    let region = CopyRegion {
        rows: vec![0..333_334],
        cols: vec![ColId(0), ColId(1), ColId(2)],
    };
    let cells = region.cell_count();
    assert_eq!(cells, 1_000_002);
    let task = cx.update(|cx| copy::copy_region(&adapter, region, false, cx));
    assert_eq!(
        task.now_or_never(),
        Some(CopyOutcome::TooManyCells { cells })
    );
    assert_eq!(
        cx.read_from_clipboard().and_then(|i| i.text()).as_deref(),
        Some("before"),
        "a refused copy leaves the clipboard alone"
    );
    assert_eq!(toasts(cx), 1);
    // Exactly a million cells is allowed (here over a result that has only 3 rows of them).
    let region = CopyRegion {
        rows: vec![0..500_000],
        cols: vec![ColId(0), ColId(1)],
    };
    assert_eq!(region.cell_count(), copy::MAX_COPY_CELLS);
    let task = cx.update(|cx| copy::copy_region(&adapter, region, false, cx));
    cx.run_until_parked();
    assert!(matches!(
        task.now_or_never(),
        Some(CopyOutcome::Copied { .. })
    ));
}

#[gpui::test]
fn copy_toasts_are_the_mocks_messages(cx: &mut TestAppContext) {
    use support::*;
    let vcx = shell(cx, "en");
    let adapter = fixture_3x3();
    let region = whole(&adapter);
    vcx.cx
        .update(|cx| copy::copy_region(&adapter, region, false, cx))
        .detach();
    vcx.run_until_parked();
    assert_eq!(
        toast_texts(vcx),
        ["Copied 3 × 3 cells", "Tab-separated, raw values"]
    );
    assert_eq!(toast_ids(vcx), ["hv-copy-done", "hv-copy-done-line"]);

    vcx.cx.update(|cx| {
        copy::copy_refused(CopyRefusal::TooMany { cells: 8_000_000 }, cx);
    });
    vcx.run_until_parked();
    let texts = toast_texts(vcx);
    assert!(texts.contains(&"Too much to copy".to_string()), "{texts:?}");
    assert!(
        texts.contains(
            &"The selection has 8,000,000 cells. Copy takes at most 1,000,000 cells or 100 MB; \
              select fewer rows or columns."
                .to_string()
        ),
        "{texts:?}"
    );
    vcx.cx
        .update(|cx| copy::copy_refused(CopyRefusal::MultipleSelection, cx));
    vcx.run_until_parked();
    assert!(
        toast_ids(vcx).contains(&"hv-copy-refused-multiple".to_string()),
        "{:?}",
        toast_ids(vcx)
    );
}

// ---------------------------------------------------------------------------------------------
// Perf (the gate's `--test results_model -- --ignored` line, release)

#[gpui::test]
#[ignore = "perf"]
fn cell_through_the_adapter_hit_under_200_ns(cx: &mut TestAppContext) {
    if !hacivat_testkit::perf_enabled() {
        return;
    }
    let (schema, row) = mixed();
    let rows: Vec<Vec<Value>> = (0..50).map(|_| row[0].clone()).collect();
    let store = ResultStore::from_rows(schema, rows, Completeness::Complete);
    let adapter = GridAdapter::new(source_of(store), tr_istanbul());
    let cols = adapter.columns();
    // A viewport of 50 rows × 6 columns, read as the grid reads it: one frame, every cell.
    paint_rows(cx, &adapter, 0..50);
    let formats = adapter.format_calls();
    let mut per_cell: Vec<f64> = Vec::with_capacity(2_000);
    for _ in 0..2_000 {
        let adapter = adapter.clone();
        let cols = cols.clone();
        per_cell.push(cx.update(|cx| {
            let frame = adapter.begin_frame(cx);
            let started = Instant::now();
            for row in 0..50 {
                for col in cols.iter() {
                    std::hint::black_box(frame.cell(ModelRow(row), col.id));
                }
            }
            started.elapsed().as_nanos() as f64 / (50.0 * cols.len() as f64)
        }));
    }
    assert_eq!(
        adapter.format_calls(),
        formats,
        "every read was a cache hit"
    );
    per_cell.sort_by(f64::total_cmp);
    let p50 = per_cell[per_cell.len() / 2];
    let p99 = per_cell[per_cell.len() * 99 / 100];
    println!("adapter cell hit: p50 {p50:.1} ns, p99 {p99:.1} ns");
    assert!(p50 < 200.0, "p50 {p50:.1} ns per cell");
}
