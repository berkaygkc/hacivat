//! `--bench typing | typing-long-line | completion`: replays keystrokes at 60 Hz through
//! `Window::dispatch_keystroke`, so they reach the editor's real key handler, which calls
//! `LatencyProbe::mark_input`. The probe's frame timestamp is taken after the frame that shows
//! the edit (or the popup) has been drawn and presented.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use hacivat_ui_core::frame_loop::{bench_dir, stamp_run};
use hacivat_ui_core::frames::{LatencyProbe, RunInfo, Verdict, verdict};
use hacivat_ui_core::gpui_surface::{Context, Keystroke, Modifiers, Window};

use crate::element::{BenchProbe, EditorView, ProbeTarget};

const PERIOD: Duration = Duration::from_micros(16_667);
const WARMUP: Duration = Duration::from_millis(1_000);
const SETTLE: Duration = Duration::from_millis(500);
const TYPING_KEYSTROKES: usize = 2_000;
const COMPLETION_KEYSTROKES: usize = 300;
/// "Line 25,000", 1-based.
const TYPING_LINE: usize = 25_000;
const LONG_LINE_MIN_CHARS: usize = 100_000;
/// Lines kept above the caret when the bench scrolls to it.
const CONTEXT_LINES: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchMode {
    Typing,
    TypingLongLine,
    Completion,
}

impl BenchMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "typing" => Some(Self::Typing),
            "typing-long-line" => Some(Self::TypingLongLine),
            "completion" => Some(Self::Completion),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Typing => "b3-typing",
            Self::TypingLongLine => "b3-typing-long-line",
            Self::Completion => "b3-completion",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Backspace,
    Enter,
}

impl Key {
    fn keystroke(self) -> Keystroke {
        let (key, key_char) = match self {
            Key::Char(' ') => ("space".to_string(), Some(" ".to_string())),
            Key::Char(c) => (c.to_string(), Some(c.to_string())),
            Key::Backspace => ("backspace".to_string(), None),
            Key::Enter => ("enter".to_string(), None),
        };
        Keystroke {
            modifiers: Modifiers::default(),
            key,
            key_char,
        }
    }
}

/// Types a SQL fragment (with Turkish text) and erases it again, until `n` keystrokes.
pub fn typing_script(n: usize) -> Vec<Key> {
    const PHRASE: &str = " AND c.city = 'İzmir' OR o.total > 4200";
    let len = PHRASE.chars().count();
    PHRASE
        .chars()
        .map(Key::Char)
        .chain(std::iter::repeat_n(Key::Backspace, len))
        .cycle()
        .take(n)
        .collect()
}

/// Types completion labels one identifier char at a time (each opens or refilters the popup),
/// with an unmeasured Enter between words, until `identifier_keys` identifier keystrokes.
pub fn completion_script(identifier_keys: usize) -> Vec<Key> {
    const WORDS: &[&str] = &[
        "customer_id",
        "city",
        "cancelled_at",
        "count",
        "current_date",
        "case",
        "order_total",
        "payment_status",
        "client_name",
        "region_code",
        "invoice_created_at",
        "shipment_note",
    ];
    let mut script = Vec::new();
    let mut typed = 0;
    for word in WORDS.iter().cycle() {
        for c in word.chars() {
            if typed == identifier_keys {
                return script;
            }
            script.push(Key::Char(c));
            typed += 1;
        }
        script.push(Key::Enter);
    }
    script
}

/// Latency samples a complete replay of `script` yields: every key for the typing benches, only
/// identifier keys for completion (the Enter between words is not measured).
pub fn expected_samples(mode: BenchMode, script: &[Key]) -> usize {
    match mode {
        BenchMode::Typing | BenchMode::TypingLongLine => script.len(),
        BenchMode::Completion => script.iter().filter(|k| matches!(k, Key::Char(_))).count(),
    }
}

/// Process exit code for a finished bench: non-zero when the replay aborted or its JSON could
/// not be written (a FAIL verdict on a complete run still exits 0; the verdict is the result).
pub fn exit_code(run: &RunInfo, dumped: bool) -> i32 {
    i32::from(run.aborted || !dumped)
}

/// Places the caret for `mode`, attaches the probe and starts the keystroke replay.
pub fn start(
    mode: BenchMode,
    view: &mut EditorView,
    window: &mut Window,
    cx: &mut Context<EditorView>,
) -> anyhow::Result<()> {
    let buffer = &view.buffer;
    let (script, target) = match mode {
        BenchMode::Typing | BenchMode::Completion => {
            let line = TYPING_LINE - 1;
            if line >= buffer.line_count() {
                bail!("the file has fewer than {TYPING_LINE} lines");
            }
            let caret = buffer.line_to_char(line) + buffer.line_len_chars(line);
            view.place_caret(caret, CONTEXT_LINES);
            if mode == BenchMode::Typing {
                (typing_script(TYPING_KEYSTROKES), ProbeTarget::EditorFrame)
            } else {
                // Complete on a fresh line below line 25,000.
                view.replace(caret..caret, "\n");
                (
                    completion_script(COMPLETION_KEYSTROKES),
                    ProbeTarget::CompletionPopup,
                )
            }
        }
        BenchMode::TypingLongLine => {
            let (line, len) = (0..buffer.line_count())
                .map(|l| (l, buffer.line_len_chars(l)))
                .max_by_key(|&(_, len)| len)
                .context("empty file")?;
            if len < LONG_LINE_MIN_CHARS {
                bail!(
                    "longest line has {len} chars; generate the fixture with --long-line {LONG_LINE_MIN_CHARS}"
                );
            }
            view.place_caret(buffer.line_to_char(line) + len / 2, CONTEXT_LINES);
            (typing_script(TYPING_KEYSTROKES), ProbeTarget::EditorFrame)
        }
    };

    let expected = expected_samples(mode, &script);
    let probe = Rc::new(RefCell::new(LatencyProbe::new(mode.label())));
    view.bench = Some(BenchProbe::new(probe.clone(), target));
    window.focus(&view.focus_handle, cx);
    window.activate_window();
    cx.activate(true);
    cx.notify();

    cx.spawn_in(window, async move |view, cx| {
        let executor = cx.background_executor().clone();
        executor.timer(WARMUP).await;
        let start = Instant::now();
        let mut inactive_keys = 0;
        let mut aborted = false;
        for (i, key) in script.iter().enumerate() {
            let due = start + PERIOD * i as u32;
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                executor.timer(wait).await;
            }
            let sent = cx.update(|window, cx| {
                window.dispatch_keystroke(key.keystroke(), cx);
                window.is_window_active()
            });
            match sent {
                Ok(true) => {}
                Ok(false) => inactive_keys += 1,
                Err(err) => {
                    eprintln!("bench aborted: {err:#}");
                    aborted = true;
                    break;
                }
            }
        }
        executor.timer(SETTLE).await;
        let finished = cx.update(|window, cx| {
            let shaped_lines = view.read_with(cx, |view, _| view.shaped_lines).unwrap_or(0);
            if inactive_keys > 0 {
                eprintln!(
                    "warning: {inactive_keys} keystrokes went to an inactive window; GPUI throttles inactive windows, so this run is not representative"
                );
            }
            let run = RunInfo {
                expected_samples: expected,
                inactive_keys,
                aborted,
                ..stamp_run(window, cx)
            };
            let code = finish(mode, &probe, &run, shaped_lines);
            if code != 0 {
                std::process::exit(code);
            }
            cx.quit();
        });
        if let Err(err) = finished {
            // The window is gone, so no gradeable dump was written.
            eprintln!("bench could not finish: {err:#}");
            std::process::exit(1);
        }
    })
    .detach();
    Ok(())
}

/// Prints the stats, writes the JSON dump (with `run`'s metadata) and returns the exit code.
fn finish(
    mode: BenchMode,
    probe: &Rc<RefCell<LatencyProbe>>,
    run: &RunInfo,
    shaped_lines: u64,
) -> i32 {
    let probe = probe.borrow();
    let stats = probe.stats();
    let os = std::env::consts::OS;
    let verdict = match verdict(mode.label(), os, &stats, run) {
        Verdict::Pass => "PASS",
        Verdict::Fail => "FAIL",
        Verdict::NoThreshold => "n/a",
    };
    println!(
        "{}: samples={}/{} p50={:.2}ms p95={:.2}ms max={:.2}ms verdict={verdict} inactive_keys={} aborted={} lines_shaped={shaped_lines} build={}",
        mode.label(),
        stats.samples,
        run.expected_samples,
        stats.p50_ms,
        stats.p95_ms,
        stats.max_ms,
        run.inactive_keys,
        run.aborted,
        run.build,
    );
    let dumped = match probe.dump_json(&bench_dir(), os, run) {
        Ok(path) => {
            println!("wrote {}", path.display());
            true
        }
        Err(err) => {
            eprintln!("could not write bench json: {err}");
            false
        }
    };
    exit_code(run, dumped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::complete;

    #[test]
    fn typing_script_has_2000_keystrokes_and_stays_balanced() {
        let s = typing_script(TYPING_KEYSTROKES);
        assert_eq!(s.len(), 2_000);
        let net: isize = s
            .iter()
            .map(|k| if *k == Key::Backspace { -1 } else { 1 })
            .sum();
        assert!((0..=40).contains(&net), "line grows by {net}");
    }

    #[test]
    fn every_completion_keystroke_keeps_the_popup_non_empty() {
        let s = completion_script(COMPLETION_KEYSTROKES);
        let identifier = s.iter().filter(|k| matches!(k, Key::Char(_))).count();
        assert_eq!(identifier, 300);
        let mut prefix = String::new();
        for key in s {
            match key {
                Key::Char(c) => {
                    prefix.push(c);
                    assert!(!complete(&prefix).is_empty(), "{prefix}");
                }
                _ => prefix.clear(),
            }
        }
    }

    #[test]
    fn bench_labels_match_the_report_thresholds() {
        for mode in ["typing", "typing-long-line", "completion"] {
            let label = BenchMode::parse(mode).unwrap().label();
            let stats = |p95| hacivat_ui_core::frames::LatencyStats {
                samples: 1,
                p50_ms: 1.0,
                p95_ms: p95,
                max_ms: p95,
            };
            let run = RunInfo {
                expected_samples: 1,
                ..RunInfo::unstamped()
            };
            assert_eq!(verdict(label, "macos", &stats(1e3), &run), Verdict::Fail);
            assert_eq!(verdict(label, "macos", &stats(1.0), &run), Verdict::Pass);
        }
    }

    #[test]
    fn expected_samples_count_only_measured_keys() {
        let typing = typing_script(TYPING_KEYSTROKES);
        assert_eq!(expected_samples(BenchMode::Typing, &typing), 2_000);
        assert_eq!(expected_samples(BenchMode::TypingLongLine, &typing), 2_000);
        // Enter between completion words is not measured.
        let completion = completion_script(COMPLETION_KEYSTROKES);
        assert!(completion.len() > 300);
        assert_eq!(expected_samples(BenchMode::Completion, &completion), 300);
    }

    #[test]
    fn aborted_or_unwritten_runs_exit_non_zero() {
        let done = RunInfo::unstamped();
        assert_eq!(exit_code(&done, true), 0);
        assert_eq!(exit_code(&done, false), 1);
        let aborted = RunInfo {
            aborted: true,
            ..RunInfo::unstamped()
        };
        assert_eq!(exit_code(&aborted, true), 1);
    }
}
