#!/usr/bin/env bash
# M0 gate runs on macOS (Task 12). Authoritative B1/B3 numbers, then the a11y probe.
# Usage: scripts/m0-gate-macos.sh [runs-per-bench]   (default 3)
# Before running: make the display you want to measure the MAIN display
# (System Settings > Displays), plug in power, close other apps.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
export CARGO_TARGET_DIR="$PWD/target"
RUNS="${1:-3}"

if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "Working tree has uncommitted changes; commit first so rows are not stamped -dirty." >&2
  exit 1
fi

echo "==> Building release binaries (once)"
cargo build --release --locked -p spike-grid -p spike-editor -p spike-probes
cargo run -q --locked -p xtask -- gen-sql --lines 50000 --long-line 100000 --out target/fixtures/b3-50k.sql

marker="$(mktemp)"
trap 'rm -f "$marker"' EXIT

pause() {
  printf '\n%s\n' "$1"
  if [ -t 0 ]; then
    read -r -p "Hazır olunca Enter'a bas (pencere açılınca klavye/fareye dokunma)... " _
  else
    for s in 5 4 3 2 1; do printf '%s ' "$s"; sleep 1; done
    printf '\n(klavye/fareye dokunma)\n'
  fi
}

caffeinate -dims -w $$ &

for i in $(seq 1 "$RUNS"); do
  pause "B1 fling, koşu $i/$RUNS (~20 sn)"
  target/release/spike-grid --bench fling
done

for mode in typing typing-long-line completion; do
  for i in $(seq 1 "$RUNS"); do
    pause "B3 $mode, koşu $i/$RUNS"
    target/release/spike-editor --file target/fixtures/b3-50k.sql --bench "$mode"
  done
done

echo "==> Recording rows"
files=()
while IFS= read -r f; do files+=("$f"); done < <(find target/bench -name '*.json' -newer "$marker" | sort)
if [ "${#files[@]}" -eq 0 ]; then echo "No bench files produced." >&2; exit 1; fi
cargo run -q --locked -p xtask -- bench-report "${files[@]}" --milestone m0

if [ -t 0 ]; then
  pause "Ekran okuyucu: VoiceOver'ı aç (Cmd+F5), sonra devam et"
  target/release/spike-probes a11y || true
  echo "VoiceOver'ı kapatabilirsin (Cmd+F5)."
else
  echo "Ekran okuyucu testi ayrıca: VoiceOver'ı aç (Cmd+F5) ve 'target/release/spike-probes a11y' çalıştır."
fi

echo
echo "Bitti. Sonuçlar docs/benchmarks/m0.md içinde; commit etmeden önce controller'a haber ver."
