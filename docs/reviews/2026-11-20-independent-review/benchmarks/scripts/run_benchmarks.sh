#!/bin/sh
# Reproduce the área-03 evidence from a clean checkout of baseline b8d63a1.
#
# It never edits the repository: everything happens in /tmp/perf-iso and in
# /tmp/perf-alloc. See ../README.md for the rationale of each experiment.
set -eu

WORKTREE=${WORKTREE:-/home/stealthc/projects/ChronoFork/.agents/worktrees/review-2026-11-20-architecture}
export CARGO_HOME=/tmp/t7-cargo
OUT=${OUT:-/tmp/perf-runs}
mkdir -p "$OUT"

# 1. Baseline bench_step, unpatched, 3 runs.
cd "$WORKTREE"
CARGO_TARGET_DIR=/tmp/chronofork-review-target \
  cargo build -p chronofork-megadrive --example bench_step --release --locked
for i in 1 2 3; do
  taskset -c 6 /tmp/chronofork-review-target/release/examples/bench_step > "$OUT/baseline-$i.txt"
done

# 2. Isolated, instrumented copy.
rm -rf /tmp/perf-iso
mkdir -p /tmp/perf-iso
git archive HEAD | tar -x -C /tmp/perf-iso
cd /tmp/perf-iso
patch -p3 < "$WORKTREE/.research/review-2026-11-20/benchmarks/patches/review-instrumentation.patch"
cp "$WORKTREE/.research/review-2026-11-20/benchmarks/src/review_perf.rs" \
  crates/chronofork-megadrive/examples/review_perf.rs
mkdir -p crates/chronofork-cli/examples
cp "$WORKTREE/.research/review-2026-11-20/benchmarks/src/review_service.rs" \
  crates/chronofork-cli/examples/review_service.rs
CARGO_TARGET_DIR=/tmp/perf-iso-target \
  cargo build -p chronofork-megadrive --example review_perf --release --locked
CARGO_TARGET_DIR=/tmp/perf-iso-target \
  cargo build -p chronofork-cli --example review_service --release --locked

# 3. Phases with the canonical hash per step (default allocator and tuned).
for i in 1 2 3; do taskset -c 6 /tmp/perf-iso-target/release/examples/review_perf full > "$OUT/iso-hash-full-$i.txt"; done
for i in 1 2 3; do MALLOC_MMAP_THRESHOLD_=1048576 MALLOC_TRIM_THRESHOLD_=1048576 \
  taskset -c 6 /tmp/perf-iso-target/release/examples/review_perf steps > "$OUT/iso-hash-tuned-$i.txt"; done

# 4. Pure execution (hash disabled) and trace overhead.
for i in 1 2 3; do REVIEW_HASH=0 taskset -c 6 /tmp/perf-iso-target/release/examples/review_perf steps > "$OUT/iso-nohash-steps-$i.txt"; done
for i in 1 2 3; do REVIEW_HASH=0 MALLOC_MMAP_THRESHOLD_=1048576 MALLOC_TRIM_THRESHOLD_=1048576 \
  taskset -c 6 /tmp/perf-iso-target/release/examples/review_perf full > "$OUT/iso-nohash-tuned-full-$i.txt"; done

# 5. Frame-boundary run (39 frames in 400k steps).
for i in 1 2; do REVIEW_HASH=0 taskset -c 5 /tmp/perf-iso-target/release/examples/review_perf frames > "$OUT/frames-nohash-$i.txt"; done

# 6. Service surface.
for i in 1 2 3; do taskset -c 6 /tmp/perf-iso-target/release/examples/review_service > "$OUT/service-hash-on-$i.txt"; done

# 7. Allocation probe: scratch crate, outside the workspace because the
#    workspace forbids `unsafe` and a counting allocator needs it.
if [ ! -d /tmp/perf-alloc ]; then
  echo "missing /tmp/perf-alloc; create it from benchmarks/src/review_alloc.rs" >&2
else
  cd /tmp/perf-alloc
  CARGO_TARGET_DIR=/tmp/perf-alloc-target cargo build --release
  taskset -c 6 /tmp/perf-alloc-target/release/review-alloc > "$OUT/alloc-hash-on.txt"
  REVIEW_HASH=0 taskset -c 6 /tmp/perf-alloc-target/release/review-alloc > "$OUT/alloc-hash-off.txt"
fi

# 8. Tables.
cd "$WORKTREE/.research/review-2026-11-20/benchmarks"
python3 scripts/summarize.py "$OUT" > /tmp/results-tables.md
echo "raw logs in $OUT; tables in /tmp/results-tables.md"
