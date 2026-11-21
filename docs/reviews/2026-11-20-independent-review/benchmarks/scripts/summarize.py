#!/usr/bin/env python3
"""Summarize the raw `key=value` benchmark logs into markdown tables.

Usage: python3 summarize.py RAW_DIR > results-tables.md

Every table prints each run's value plus the median across runs, because the
shared 8-vCPU WSL host makes single runs unreliable (see README.md).
"""
import glob
import os
import statistics
import sys

RAW = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "..", "raw")

GROUPS = [
    ("A. Baseline `bench_step` (worktree, release, unpatched)", "baseline-*.txt", [
        ("step_median_seconds", "step p50 (s / 10k steps)"),
        ("step_min_seconds", "step min (s / 10k steps)"),
        ("step_max_seconds", "step max (s / 10k steps)"),
        ("instructions_per_second", "instructions/s"),
        ("serialization_median_seconds", "checkpoint serialize p50 (s)"),
        ("restore_median_seconds", "restore p50 (s)"),
        ("serialized_checkpoint_bytes", "checkpoint bytes"),
    ]),
    ("B. Instrumented, canonical hash per step, default glibc allocator", "iso-hash-full-[123].txt", [
        ("step_median_seconds", "step p50 (s / 10k steps)"),
        ("instructions_per_second", "instructions/s"),
        ("step_per_second", "trials/s (1 / total trial time)"),
        ("minflt_delta_steps", "minor faults / 50k steps"),
        ("hash_only_median_seconds", "state hash only p50 (s)"),
        ("body_only_median_seconds", "state body only p50 (s)"),
        ("backend_snapshot_median_seconds", "backend snapshot p50 (s)"),
        ("backend_restore_median_seconds", "backend restore p50 (s)"),
        ("backend_inspect_median_seconds", "backend inspect p50 (s)"),
        ("session_snapshot_median_seconds", "session checkpoint p50 (s)"),
        ("session_snapshot_bytes_median_seconds", "checkpoint serialize p50 (s)"),
        ("session_restore_median_seconds", "session restore p50 (s)"),
        ("session_fork_current_median_seconds", "fork_current p50 (s)"),
        ("session_create_median_seconds", "session create p50 (s)"),
        ("session_inspect_median_seconds", "session inspect p50 (s)"),
    ]),
    ("C. Instrumented, canonical hash per step, glibc mmap/trim thresholds raised", "iso-hash-tuned-[123].txt iso-tuned-full-1.txt iso-tuned-full-2.txt iso-tuned-trace-1.txt", [
        ("step_median_seconds", "step p50 (s / 10k steps)"),
        ("instructions_per_second", "instructions/s"),
        ("minflt_delta_steps", "minor faults / 50k steps"),
        ("hash_only_median_seconds", "state hash only p50 (s)"),
        ("body_only_median_seconds", "state body only p50 (s)"),
        ("backend_snapshot_median_seconds", "backend snapshot p50 (s)"),
        ("backend_restore_median_seconds", "backend restore p50 (s)"),
        ("backend_inspect_median_seconds", "backend inspect p50 (s)"),
        ("session_snapshot_median_seconds", "session checkpoint p50 (s)"),
        ("session_fork_current_median_seconds", "fork_current p50 (s)"),
    ]),
    ("D. Pure execution (`REVIEW_HASH=0`), default allocator", "iso-nohash-steps-[123].txt", [
        ("step_median_seconds", "step p50 (s / 10k steps)"),
        ("instructions_per_second", "instructions/s"),
        ("minflt_delta_steps", "minor faults / 50k steps"),
    ]),
    ("E. Pure execution (`REVIEW_HASH=0`), glibc thresholds raised", "iso-nohash-tuned-full-[123].txt", [
        ("step_median_seconds", "step p50 (s / 10k steps)"),
        ("instructions_per_second", "instructions/s"),
        ("step_trace_disabled_warm_median_seconds", "step p50, trace off (s / 10k)"),
        ("step_trace_65536_median_seconds", "step p50, trace on 65k (s / 10k)"),
        ("session_step_100_median_seconds", "session step 100 p50 (s / 100 steps)"),
        ("backend_snapshot_median_seconds", "backend snapshot p50 (s)"),
        ("backend_inspect_median_seconds", "backend inspect p50 (s)"),
        ("session_snapshot_median_seconds", "session checkpoint p50 (s)"),
        ("session_fork_current_median_seconds", "fork_current p50 (s)"),
    ]),
    ("F. Frame-boundary run (`REVIEW_HASH=0`, 400k steps)", "frames-nohash-[12].txt", [
        ("frame_run_frames", "frames captured"),
        ("frame_step_median_seconds", "step that captures a frame p50 (s)"),
        ("nonframe_step_median_seconds", "other step p50 (s)"),
    ]),
    ("G. Service surface (hash per step, default allocator)", "service-hash-on-[123].txt", [
        ("service_dispatch_capabilities_median_seconds", "dispatch overhead p50 (s)"),
        ("service_run_1000_median_seconds", "session.run 1000 p50 (s)"),
        ("service_run_steps_per_second", "steps/s"),
        ("service_inspect_median_seconds", "session.inspect p50 (s)"),
        ("service_snapshot_median_seconds", "session.snapshot p50 (s)"),
        ("service_fork_median_seconds", "session.fork p50 (s)"),
    ]),
]


def parse(path):
    values = {}
    with open(path, encoding="utf-8", errors="replace") as handle:
        for line in handle:
            if "=" not in line:
                continue
            key, _, value = line.strip().partition("=")
            try:
                values[key] = float(value)
            except ValueError:
                continue
    return values


def runs(patterns):
    paths = []
    for pattern in patterns.split():
        paths.extend(glob.glob(os.path.join(RAW, pattern)))
    return sorted(set(paths))


print("<!-- Generated by summarize.py from the raw logs in raw/. -->")
for title, pattern, keys in GROUPS:
    paths = runs(pattern)
    if not paths:
        continue
    print(f"\n### {title}\n")
    print(f"Runs: {', '.join(os.path.basename(path) for path in paths)}\n")
    header = "| Métrica | " + " | ".join(os.path.basename(p).replace(".txt", "") for p in paths) + " | **mediana** |"
    print(header)
    print("|---" * (len(paths) + 2) + "|")
    parsed = [parse(path) for path in paths]
    for key, label in keys:
        row = []
        for values in parsed:
            value = values.get(key)
            if value is None:
                row.append("—")
            elif value == int(value) and abs(value) >= 100:
                row.append(f"{int(value):,}".replace(",", " "))
            else:
                row.append(f"{value:.9g}")
        available = [values[key] for values in parsed if key in values]
        median = f"{statistics.median(available):.9g}" if available else "—"
        print(f"| {label} | " + " | ".join(row) + f" | **{median}** |")
