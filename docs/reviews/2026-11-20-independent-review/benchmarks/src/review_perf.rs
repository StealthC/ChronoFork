//! REVIEW-ONLY instrumentation harness (not part of the repository).
//!
//! Built only in the isolated throwaway copy of baseline `b8d63a1` that carries
//! the `review_*` instrumentation methods documented in
//! `.research/review-2026-11-20/benchmarks/README.md`.
//!
//! Modes:
//!   * `full` (default): every phase, canonical state hash enabled.
//!   * `steps`: step throughput only (works with `REVIEW_HASH=0` too).
//!
//! Every phase prints raw per-trial samples as `key=value` lines; medians are
//! recomputed by the caller so nothing is hidden.

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chronofork_backend_api::Backend;
use chronofork_core::InputFrame;
use chronofork_megadrive::{MegadriveFactory, SchedulerConfig, interrupt_rom};
use chronofork_sessions::SessionManager;

const STEP_TRIALS: usize = 5;
const STEPS_PER_TRIAL: u64 = 10_000;
const WARMUP_STEPS: u64 = 1_000;
const HASH_TRIALS: usize = 200;
const BODY_TRIALS: usize = 200;
const SNAPSHOT_TRIALS: usize = 50;
const RESTORE_TRIALS: usize = 50;
const INSPECT_TRIALS: usize = 50;
const CLONE_VDP_TRIALS: usize = 2_000;
const CLONE_CART_TRIALS: usize = 200;
const CAPTURE_FRAME_TRIALS: usize = 20;
const CREATE_MACHINE_TRIALS: usize = 20;
const SESSION_TRIALS: usize = 10;
const SESSION_INSPECT_TRIALS: usize = 200;
const FRAME_RUN_STEPS: u64 = 400_000;

fn summarize(samples: &[Duration]) -> (Duration, Duration, Duration) {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let min = sorted.first().copied().unwrap_or_default();
    let median = sorted[sorted.len() / 2];
    let max = sorted.last().copied().unwrap_or_default();
    (min, median, max)
}

fn print_seconds(prefix: &str, samples: &[Duration]) {
    let (min, median, max) = summarize(samples);
    println!("{prefix}_min_seconds={:.9}", min.as_secs_f64());
    println!("{prefix}_median_seconds={:.9}", median.as_secs_f64());
    println!("{prefix}_max_seconds={:.9}", max.as_secs_f64());
    println!(
        "{prefix}_per_second={:.3}",
        samples.len() as f64 / samples.iter().copied().sum::<Duration>().as_secs_f64()
    );
}

fn print_trials(prefix: &str, samples: &[Duration]) {
    for (index, sample) in samples.iter().enumerate() {
        println!("{prefix}_trial_{}_seconds={:.9}", index + 1, sample.as_secs_f64());
    }
}

fn minflt() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    // Field 10 (1-based) is minflt; the command name may contain spaces, so split
    // after the closing parenthesis.
    let tail = stat.rsplit_once(')').map_or("", |(_, tail)| tail);
    let fields: Vec<&str> = tail.split_whitespace().collect();
    fields.get(7).and_then(|value| value.parse().ok()).unwrap_or(0)
}

fn rss_kib() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest
                .split_whitespace()
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
        }
    }
    0
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "full".to_owned());
    let hash_on = std::env::var_os("REVIEW_HASH").as_deref() != Some(std::ffi::OsStr::new("0"));
    let config = SchedulerConfig::ntsc();

    println!("review-perf v1");
    println!("mode={mode}");
    println!("program=interrupt_rom(3)");
    println!("video=ntsc");
    println!("hash_enabled={}", u8::from(hash_on));
    println!("debug_assertions={}", u8::from(cfg!(debug_assertions)));
    println!("rss_start_kib={}", rss_kib());

    // ---- Step throughput -------------------------------------------------
    let factory = MegadriveFactory::with_scheduler_config(interrupt_rom(3), config)?;
    let mut machine = factory.create_machine()?;
    println!("machine_footprint_bytes={}", machine.review_footprint_bytes());
    println!("vdp_size_bytes={}", machine.review_clone_vdp());
    println!("cartridge_size_bytes={}", machine.review_clone_cartridge());
    println!("state_body_bytes_len={}", machine.review_state_body().len());
    for _ in 0..WARMUP_STEPS {
        black_box(machine.step(InputFrame::NEUTRAL)?);
    }
    let minflt_before_steps = minflt();
    let mut step_samples = Vec::with_capacity(STEP_TRIALS);
    for _ in 0..STEP_TRIALS {
        let started = Instant::now();
        for _ in 0..STEPS_PER_TRIAL {
            black_box(machine.step(InputFrame::NEUTRAL)?);
        }
        step_samples.push(started.elapsed());
    }
    let minflt_after_steps = minflt();
    println!("minflt_delta_steps={}", minflt_after_steps.saturating_sub(minflt_before_steps));
    print_trials("step", &step_samples);
    print_seconds("step", &step_samples);
    let steps_total = STEPS_PER_TRIAL * u64::try_from(STEP_TRIALS)?;
    let steps_seconds: f64 = step_samples.iter().copied().sum::<Duration>().as_secs_f64();
    println!("instructions_per_second={:.0}", steps_total as f64 / steps_seconds);
    println!("final_tick={}", machine.inspect()?.tick.raw());

    // ---- Frame-boundary run (needs a machine that has not been warmed) ---
    if mode == "frames" {
        let factory = MegadriveFactory::with_scheduler_config(interrupt_rom(3), config)?;
        let mut frame_machine = factory.create_machine()?;
        let mut frame_steps = 0_u64;
        let mut frame_times = Vec::new();
        let mut other_times = Vec::new();
        for _ in 0..FRAME_RUN_STEPS {
            let started = Instant::now();
            black_box(frame_machine.step(InputFrame::NEUTRAL)?);
            let elapsed = started.elapsed();
            let produced = frame_machine
                .media_mut()
                .and_then(|media| media.take_video_frame())
                .is_some();
            if produced {
                frame_steps += 1;
                frame_times.push(elapsed);
            } else {
                other_times.push(elapsed);
            }
        }
        println!("frame_run_steps={FRAME_RUN_STEPS}");
        println!("frame_run_frames={frame_steps}");
        println!("frame_run_final_tick={}", frame_machine.inspect()?.tick.raw());
        print_seconds("frame_step", &frame_times);
        print_seconds("nonframe_step", &other_times);
    }

    if mode == "steps" {
        println!("rss_end_kib={}", rss_kib());
        return Ok(());
    }

    // ---- Per-phase costs on a fixed machine ------------------------------
    let factory = MegadriveFactory::with_scheduler_config(interrupt_rom(3), config)?;
    let mut machine = factory.create_machine()?;
    for _ in 0..WARMUP_STEPS {
        black_box(machine.step(InputFrame::NEUTRAL)?);
    }

    let mut create_samples = Vec::with_capacity(CREATE_MACHINE_TRIALS);
    for _ in 0..CREATE_MACHINE_TRIALS {
        let started = Instant::now();
        let created = black_box(factory.create_machine()?);
        create_samples.push(started.elapsed());
        drop(created);
    }
    print_seconds("create_machine", &create_samples);

    let minflt_before_hash = minflt();
    let mut hash_samples = Vec::with_capacity(HASH_TRIALS);
    if hash_on {
        for _ in 0..HASH_TRIALS {
            let started = Instant::now();
            black_box(machine.review_state_hash());
            hash_samples.push(started.elapsed());
        }
        let minflt_after_hash = minflt();
        println!("minflt_delta_hash_only={}", minflt_after_hash.saturating_sub(minflt_before_hash));
        print_seconds("hash_only", &hash_samples);
    }

    let mut body_samples = Vec::with_capacity(BODY_TRIALS);
    for _ in 0..BODY_TRIALS {
        let started = Instant::now();
        black_box(machine.review_state_body());
        body_samples.push(started.elapsed());
    }
    print_seconds("body_only", &body_samples);

    let mut clone_vdp_samples = Vec::with_capacity(CLONE_VDP_TRIALS);
    for _ in 0..CLONE_VDP_TRIALS {
        let started = Instant::now();
        black_box(machine.review_clone_vdp());
        clone_vdp_samples.push(started.elapsed());
    }
    print_seconds("clone_vdp", &clone_vdp_samples);

    let mut clone_cart_samples = Vec::with_capacity(CLONE_CART_TRIALS);
    for _ in 0..CLONE_CART_TRIALS {
        let started = Instant::now();
        black_box(machine.review_clone_cartridge());
        clone_cart_samples.push(started.elapsed());
    }
    print_seconds("clone_cartridge", &clone_cart_samples);

    let mut capture_samples = Vec::with_capacity(CAPTURE_FRAME_TRIALS);
    for _ in 0..CAPTURE_FRAME_TRIALS {
        let started = Instant::now();
        black_box(machine.review_capture_frame()?);
        capture_samples.push(started.elapsed());
    }
    print_seconds("capture_frame", &capture_samples);

    let snapshot = machine.snapshot()?;
    println!("backend_payload_bytes={}", snapshot.payload().len());
    println!("backend_state_hash={}", snapshot.state_hash().to_hex());

    let mut snapshot_samples = Vec::with_capacity(SNAPSHOT_TRIALS);
    for _ in 0..SNAPSHOT_TRIALS {
        let started = Instant::now();
        black_box(machine.snapshot()?);
        snapshot_samples.push(started.elapsed());
    }
    print_seconds("backend_snapshot", &snapshot_samples);

    let mut restore_samples = Vec::with_capacity(RESTORE_TRIALS);
    for _ in 0..RESTORE_TRIALS {
        let started = Instant::now();
        machine.restore(&snapshot)?;
        restore_samples.push(started.elapsed());
    }
    print_seconds("backend_restore", &restore_samples);

    // Trace-on step cost: same plain step loop with the bounded instruction trace on.
    let mut trace_machine = factory.create_machine()?;
    for _ in 0..WARMUP_STEPS {
        black_box(trace_machine.step(InputFrame::NEUTRAL)?);
    }
    let mut trace_samples = Vec::with_capacity(STEP_TRIALS);
    for _ in 0..STEP_TRIALS {
        let started = Instant::now();
        for _ in 0..STEPS_PER_TRIAL {
            black_box(trace_machine.step(InputFrame::NEUTRAL)?);
        }
        trace_samples.push(started.elapsed());
    }
    print_seconds("step_trace_disabled_warm", &trace_samples);
    trace_machine.enable_trace(65_536)?;
    let mut trace_on_samples = Vec::with_capacity(STEP_TRIALS);
    for _ in 0..STEP_TRIALS {
        let started = Instant::now();
        for _ in 0..STEPS_PER_TRIAL {
            black_box(trace_machine.step(InputFrame::NEUTRAL)?);
        }
        trace_on_samples.push(started.elapsed());
    }
    print_seconds("step_trace_65536", &trace_on_samples);
    let started = Instant::now();
    let records = trace_machine.trace().map_or(0, |trace| trace.len());
    println!("trace_records_retained={records}");
    println!("trace_read_seconds={:.9}", started.elapsed().as_secs_f64());
    trace_machine.disable_trace();

    let mut inspect_samples = Vec::with_capacity(INSPECT_TRIALS);
    for _ in 0..INSPECT_TRIALS {
        let started = Instant::now();
        black_box(machine.inspect()?);
        inspect_samples.push(started.elapsed());
    }
    print_seconds("backend_inspect", &inspect_samples);

    // ---- Session layer: checkpoint, fork, inspect ------------------------
    let factory = MegadriveFactory::with_scheduler_config(interrupt_rom(3), config)?;
    let rss_before = rss_kib();
    let mut manager = SessionManager::new(Arc::new(factory));
    let mut create_session_samples = Vec::with_capacity(SESSION_TRIALS);
    let mut sessions = Vec::new();
    for _ in 0..SESSION_TRIALS {
        let started = Instant::now();
        let id = manager.create_session()?;
        create_session_samples.push(started.elapsed());
        sessions.push(id);
    }
    print_seconds("session_create", &create_session_samples);
    let rss_after_sessions = rss_kib();
    let session = sessions[0];
    for _ in 0..WARMUP_STEPS {
        manager.step(session, InputFrame::NEUTRAL)?;
    }

    let mut session_step_samples = Vec::with_capacity(SESSION_TRIALS);
    for _ in 0..SESSION_TRIALS {
        let started = Instant::now();
        for _ in 0..100 {
            black_box(manager.step(session, InputFrame::NEUTRAL)?);
        }
        session_step_samples.push(started.elapsed());
    }
    print_seconds("session_step_100", &session_step_samples);

    let mut session_snapshot_samples = Vec::with_capacity(SESSION_TRIALS);
    let mut snapshot_ids = Vec::new();
    for _ in 0..SESSION_TRIALS {
        let started = Instant::now();
        let id = manager.snapshot(session)?;
        session_snapshot_samples.push(started.elapsed());
        snapshot_ids.push(id);
    }
    print_seconds("session_snapshot", &session_snapshot_samples);
    let rss_after_snapshots = rss_kib();
    let checkpoint = snapshot_ids[0];
    let bytes = manager.snapshot_bytes(session, checkpoint)?;
    println!("serialized_checkpoint_bytes={}", bytes.len());

    let mut serialize_samples = Vec::with_capacity(SESSION_TRIALS);
    for _ in 0..SESSION_TRIALS {
        let started = Instant::now();
        black_box(manager.snapshot_bytes(session, checkpoint)?);
        serialize_samples.push(started.elapsed());
    }
    print_seconds("session_snapshot_bytes", &serialize_samples);

    let mut session_restore_samples = Vec::with_capacity(SESSION_TRIALS);
    for _ in 0..SESSION_TRIALS {
        let started = Instant::now();
        manager.restore(session, checkpoint)?;
        session_restore_samples.push(started.elapsed());
    }
    print_seconds("session_restore", &session_restore_samples);

    let mut fork_samples = Vec::with_capacity(SESSION_TRIALS);
    let mut forks = Vec::new();
    for _ in 0..SESSION_TRIALS {
        let started = Instant::now();
        let id = manager.fork_current(session)?;
        fork_samples.push(started.elapsed());
        forks.push(id);
    }
    print_seconds("session_fork_current", &fork_samples);
    let rss_after_forks = rss_kib();

    let mut session_inspect_samples = Vec::with_capacity(SESSION_INSPECT_TRIALS);
    for _ in 0..SESSION_INSPECT_TRIALS {
        let started = Instant::now();
        black_box(manager.inspect(session)?);
        session_inspect_samples.push(started.elapsed());
    }
    print_seconds("session_inspect", &session_inspect_samples);

    println!("rss_before_sessions_kib={rss_before}");
    println!("rss_after_{SESSION_TRIALS}_sessions_kib={rss_after_sessions}");
    println!("rss_after_{SESSION_TRIALS}_checkpoints_kib={rss_after_snapshots}");
    println!("rss_after_{SESSION_TRIALS}_forks_kib={rss_after_forks}");
    println!("rss_end_kib={}", rss_kib());
    println!(
        "rss_delta_per_session_kib={:.1}",
        (rss_after_sessions.saturating_sub(rss_before)) as f64 / SESSION_TRIALS as f64
    );
    println!(
        "rss_delta_per_checkpoint_kib={:.1}",
        (rss_after_snapshots.saturating_sub(rss_after_sessions)) as f64 / SESSION_TRIALS as f64
    );
    println!(
        "rss_delta_per_fork_kib={:.1}",
        (rss_after_forks.saturating_sub(rss_after_snapshots)) as f64 / SESSION_TRIALS as f64
    );
    Ok(())
}
