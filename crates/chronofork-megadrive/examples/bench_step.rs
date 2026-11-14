//! Dependency-free, reproducible step and snapshot benchmark.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p chronofork-megadrive --example bench_step --release
//! ```
//!
//! The workload is fixed in source: the independently authored
//! `interrupt_rom(3)` program runs on the minimal 68000 + VDP + interrupt
//! scheduler slice at a four-instruction line quantum, so step throughput
//! includes VDP port access and interrupt scheduling. Snapshot cost is measured
//! through the session layer, which owns the serialized checkpoint envelope.
//!
//! Every measurement uses [`std::time::Instant`] and a median (min/max reported
//! alongside it). Results are printed as `key=value` lines for machine parsing;
//! only the timing values vary between runs of the same build.

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chronofork_backend_api::Backend;
use chronofork_core::InputFrame;
use chronofork_megadrive::{MegadriveFactory, SchedulerConfig, VideoStandard, interrupt_rom};
use chronofork_sessions::{SessionManager, Snapshot};

/// Scheduler quantum used by the interrupt program.
const INSTRUCTIONS_PER_LINE: u32 = 4;
/// Number of timed step trials.
const STEP_TRIALS: usize = 5;
/// Steps executed per timed trial.
///
/// Each step returns a canonical state hash over the full machine state (64 KiB
/// work RAM plus the 64 KiB VDP VRAM), so this is deliberately smaller than a
/// pure CPU microbenchmark budget; the reported throughput is the real full-step
/// cost.
const STEPS_PER_TRIAL: u64 = 10_000;
/// Warmup steps before timing starts.
const WARMUP_STEPS: u64 = 1_000;
/// Steps executed before the checkpoint is captured.
const SESSION_STEPS: u64 = 1_000;
/// Timed serialization trials.
const SERIALIZATION_TRIALS: usize = 25;
/// Timed restore trials.
const RESTORE_TRIALS: usize = 25;

/// Sorts `samples` in place and returns (min, median, max).
fn summarize(samples: &mut [Duration]) -> (Duration, Duration, Duration) {
    samples.sort_unstable();
    let min = samples.first().copied().unwrap_or_default();
    let median = samples[samples.len() / 2];
    let max = samples.last().copied().unwrap_or_default();
    (min, median, max)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = SchedulerConfig::new(VideoStandard::Ntsc, INSTRUCTIONS_PER_LINE)?;

    // Step throughput: one fixed program, repeated timed trials.
    let factory = MegadriveFactory::with_scheduler_config(interrupt_rom(3), config)?;
    let mut machine = factory.create_machine()?;
    for _ in 0..WARMUP_STEPS {
        black_box(machine.step(InputFrame::NEUTRAL)?);
    }
    let mut step_samples = Vec::with_capacity(STEP_TRIALS);
    for _ in 0..STEP_TRIALS {
        let started = Instant::now();
        for _ in 0..STEPS_PER_TRIAL {
            black_box(machine.step(InputFrame::NEUTRAL)?);
        }
        step_samples.push(started.elapsed());
    }
    let total_step_time: Duration = step_samples.iter().copied().sum();
    let instruction_count = STEPS_PER_TRIAL * u64::try_from(STEP_TRIALS)?;
    let instructions_per_second = instruction_count as f64 / total_step_time.as_secs_f64();
    let (step_min, step_median, step_max) = summarize(&mut step_samples);

    // Snapshot size + serialization/restore cost through the session layer.
    let factory = MegadriveFactory::with_scheduler_config(interrupt_rom(3), config)?;
    let mut manager = SessionManager::new(Arc::new(factory));
    let session = manager.create_session()?;
    for _ in 0..SESSION_STEPS {
        manager.step(session, InputFrame::NEUTRAL)?;
    }
    let snapshot = manager.snapshot(session)?;
    let serialized = manager.snapshot_bytes(session, snapshot)?;
    let final_tick = manager.inspect(session)?.machine.tick.raw();
    let final_state_hash = manager.inspect(session)?.machine.state_hash;

    let mut serialization_samples = Vec::with_capacity(SERIALIZATION_TRIALS);
    for _ in 0..SERIALIZATION_TRIALS {
        let started = Instant::now();
        black_box(manager.snapshot_bytes(session, snapshot)?);
        serialization_samples.push(started.elapsed());
    }
    let (serialize_min, serialize_median, serialize_max) = summarize(&mut serialization_samples);

    let mut restore_samples = Vec::with_capacity(RESTORE_TRIALS);
    for _ in 0..RESTORE_TRIALS {
        let started = Instant::now();
        manager.restore(session, snapshot)?;
        restore_samples.push(started.elapsed());
    }
    let (restore_min, restore_median, restore_max) = summarize(&mut restore_samples);

    let payload_bytes = Snapshot::from_bytes(&serialized)?.backend().payload().len();
    let restored_state_hash_matches =
        manager.inspect(session)?.machine.state_hash == final_state_hash;

    println!("chronofork-bench-step v1");
    println!("program=interrupt_rom(3)");
    println!("video=ntsc");
    println!("instructions_per_line={INSTRUCTIONS_PER_LINE}");
    println!("debug_assertions={}", u8::from(cfg!(debug_assertions)));
    println!("warmup_steps={WARMUP_STEPS}");
    println!("step_trials={STEP_TRIALS}");
    println!("steps_per_trial={STEPS_PER_TRIAL}");
    println!("instruction_count={instruction_count}");
    println!("total_step_seconds={:.9}", total_step_time.as_secs_f64());
    println!("step_min_seconds={:.9}", step_min.as_secs_f64());
    println!("step_median_seconds={:.9}", step_median.as_secs_f64());
    println!("step_max_seconds={:.9}", step_max.as_secs_f64());
    println!("instructions_per_second={instructions_per_second:.0}");
    println!("session_steps={SESSION_STEPS}");
    println!("final_tick={final_tick}");
    println!("final_state_hash={final_state_hash}");
    println!("serialized_checkpoint_bytes={}", serialized.len());
    println!("backend_payload_bytes={payload_bytes}");
    println!("serialization_trials={SERIALIZATION_TRIALS}");
    println!(
        "serialization_min_seconds={:.9}",
        serialize_min.as_secs_f64()
    );
    println!(
        "serialization_median_seconds={:.9}",
        serialize_median.as_secs_f64()
    );
    println!(
        "serialization_max_seconds={:.9}",
        serialize_max.as_secs_f64()
    );
    println!("restore_trials={RESTORE_TRIALS}");
    println!("restore_min_seconds={:.9}", restore_min.as_secs_f64());
    println!("restore_median_seconds={:.9}", restore_median.as_secs_f64());
    println!("restore_max_seconds={:.9}", restore_max.as_secs_f64());
    println!(
        "restore_state_hash_matches={}",
        u8::from(restored_state_hash_matches)
    );
    Ok(())
}
