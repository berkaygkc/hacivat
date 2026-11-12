# M0 controller rulings

These are the decisions the SDD controller made on the PO's behalf during M0, in the order they were made. Each one says what it costs if it was wrong. Source: the M0 SDD ledger (deleted after merge).

- Ruling: parallel implementers despite the skill's "never parallel" rule — PO explicitly asked for it; conflicts are contained with per-agent git worktrees, merged into m0-spike by the controller after each task review — cost if wrong: merge conflicts in shared files (xtask main.rs, ui-core lib.rs), resolved at merge.
- Ruling:s:
- Ruling R1: T1 creates xtask with a `match` dispatch and one module per subcommand; later tasks add one arm + one module — trivial merges — cost if wrong: small manual merge.
- Ruling R2: T1 declares workspace `members = ["crates/ui/*", "xtask", "spikes/*"]` (globs) so spike tasks never edit the root members list — cost if wrong: none.
- Ruling R3: generated `TypeRamp` gets `pub fn all(&self) -> Vec<&TypeStyle>` (part of codegen output) — cost if wrong: test signature tweak.
- Ruling R4: `ImeState::new(text)` puts the caret (empty selection) at end of text — matches the tests' expectations — cost if wrong: test edits only.
- Ruling: wave 2 started before Task 1 review returned — saves a wave of wall-clock; cost if wrong: T1 fixes may need a rebase of W2 branches at merge.
- Ruling: all worktrees share CARGO_TARGET_DIR=<repo>/target so GPUI compiles once; cargo's lock serializes builds — keeps the machine from choking (PO request); cost: agents wait on the lock.
- Ruling: handed to Task 11, whose dispatch already widens [graph] targets; T11 must also extend allow-git — cost if wrong: deny fails on CI until fixed.
- Ruling: name-only bans (cargo-deny cannot scope by source) accepted as a safe over-block — cost if wrong: a future crates.io crate named text/ui would need an explicit exception.
- Ruling: TypeStyle.line_height is logical px (unitless × size) — GPUI line heights are absolute lengths; plan only said f32 — cost if wrong: one conversion change in codegen.
- Ruling: wave 4 (T5,T6,T8,T9) dispatched on m0-spike + T3 branch while T3 is in review — PO wants parallelism; cost if wrong: small rebase if fonts API changes.
- Ruling: models — T5 grid and T6 editor on opus (perf-critical custom GPUI elements, design judgment); T8 harman and T9 probes on sonnet — cost: token spend vs. rework risk.
- Ruling: VF criterion (c) reads as "tabular figures" (the spec's intent, DESIGN.md Tabular Everywhere Rule); tnum stays requested as a no-op; Task 9 asserts equal digit advances; Task 12 addendum records it — cost if wrong: none for rendering (Chromium renders the same digits).
- Ruling: elevated minors #1 (a11y prints ok without a tree) and #3 (swallowed handle.update → silent skip) into fix round 1 — they can produce a false PASS in the gate record; cost: one cheap round.
- Ruling: keep the shared target dir (machine load) but require `cargo clean -p <workspace crates>` + rebuild before final runs, and xtask via `cargo run -p xtask` from the worktree root; authoritative numbers come from Task 12 on merged m0-spike anyway — cost if wrong: an agent-local result built from mixed sources (re-run at gate).
- Ruling: 11px floor applies to nominal type size; synthesized small caps are a PO-approved exception — DESIGN.md label-sc fontFeature must be updated to "synthesized small caps" in Task 12 docs — cost if wrong: one token-doc edit.
- Ruling: VF (e) 1x/1.5x deferred to Task 12 (Windows 150 % host / 1x display) — cost: VF (e) open until then.
- Ruling: B3 is defined as app-side latency (handler entry → frame handed to compositor); verdict stands on that definition; m0.md carries measurement notes + display Hz; Task 12 adds 60/120 Hz runs — cost if wrong: B3 must be re-defined with an external latency rig (typometer-style).
- Ruling: dark gutter FAILs recorded as an open VF(b) item pending an sRGB-native capture (Windows host) in Task 12 — not waived, not hidden — cost if wrong: a real 1-step colour error in the adapter, fixed at T12.
- Ruling: controller changed CI runner macos-14→macos-15 directly (image retired; one-token infra change, actionlint clean) — cost if wrong: another runner label tweak.
- Ruling: Linux .rpm unsupported by cargo-packager 0.11.8 → spec §7.3 allows per-OS scripts; rpm via cargo-generate-rpm later (M9 packaging) — cost: one extra tool.
- Ruling: controller fixed the residual DESIGN.md/design.json "Real Small Caps Rule" prose to match the PO decision (docs-only; tokens --check unaffected) — cost if wrong: wording.

## Deferred minors (triaged by the final review into M1 / addendum)

- Task 1: minor (deferred): deny.toml graph targets only aarch64-apple-darwin — CI on win/linux needs targets added (T11)
- Task 1: minor (deferred): check_deps lacks tests for dev-dep and gpui_platform-only paths; smoke test asserts only is_ok (plan-mandated).
- Task 4: minor (deferred): serde_json listed in both deps and dev-deps; bench-report doesn't escape '|' in label/host; --milestone unsanitised path; duplicate rows on rerun.
- Task 2: minor (deferred): radii test checks DARK only; no-lineHeight default 1.35×size is an implementer assumption; motion fields m_-prefixed.
- Task 3: minor (deferred): no comment that tnum is a no-op; register_fonts errors only eprintln (silent fallback would corrupt VF); FragmentMono-Italic registered but unused; fmt churn in frames.rs/lib.rs.
- Task 9: minor (deferred): turkish probe proxies "dotted" by glyph-id difference; locl result is macOS CoreText only (addendum must say so); win/linux paths not compiled (CI will); clipboard probe overwrites clipboard.
- Task 5: minor (deferred): scroll_px reads DARK spacing directly; roles bake colours (no runtime theme switch); floor viewport_rows leaves <21px strip; shape-cache key omits font features; LRU no hard cap within one frame.
- Task 6: minor (deferred): column_window doesn't cover >512 visible cols; mid-string window highlighting wrong; pending mark dropped at run end; long-line bench types mid-line only.
- Task 8: minor (deferred): hover/text/syn_*/green not sampled (no flat area); scanline expectations hand-derived; run_inner arg order.
- Task 6: minor (deferred): porcelain quoting of non-ASCII paths may false-dirty; bench-report rewrite makes second pre-commit run -dirty; insert_rows keys on any '|' line.
- Task 7: minor (deferred): StringBuf test double rounds mid-surrogate up vs ropey down; reversed range not normalised; on_blur wiring untested; composing-guard could deaden keys if platform ends composition without unmark; Pinyin commit may open completion.
- Task 10: minor (deferred): manifest URL from argv in shipped binary; @SIGNING_IDENTITY@ placeholder left in win/linux configs; DMG named aarch64 though universal; ls *.dmg multiple-match; key path repeated in 3 scripts; key unencrypted (spike).
- Task 10: minor (deferred): REQUIRE_NOTARIZATION compares literal 1 only; probe discards stdout.
- Task 12 IME fix: minor (deferred): blur tests restate implementation (no captured IME trace); bug-2 tests are characterization only.
