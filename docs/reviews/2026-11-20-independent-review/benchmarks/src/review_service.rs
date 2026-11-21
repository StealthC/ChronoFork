//! REVIEW-ONLY: service-surface cost of the same work the direct loop measures.
use std::sync::Arc;
use std::time::{Duration, Instant};

use chronofork_backend_api::BackendFactory;
use chronofork_megadrive::{MegadriveFactory, SchedulerConfig, interrupt_rom};
use chronofork_service::json::to_canonical_bytes;
use chronofork_service::{AppService, JsonValue, Principal, Scope, ServiceConfig, ServiceLimits};

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
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let factory: Arc<dyn BackendFactory> = Arc::new(MegadriveFactory::with_scheduler_config(
        interrupt_rom(3),
        SchedulerConfig::ntsc(),
    )?);
    let service = AppService::new(ServiceConfig {
        factory,
        storage_root: None,
        artifacts_root: None,
        limits: ServiceLimits::default(),
    })?;
    let admin = Principal::new("review", [Scope::Admin]);
    let limits = *service.limits();
    let request = |op: &str, input: JsonValue| -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let document = JsonValue::object([("op", JsonValue::string(op)), ("input", input)]);
        Ok(to_canonical_bytes(&document, &limits)?)
    };

    let created = service.dispatch(&admin, &request("session.create", JsonValue::object(std::iter::empty::<(String, JsonValue)>()))?)?;
    println!("session_create_envelope_bytes={}", created.len());
    for _ in 0..10 {
        service.dispatch(
            &admin,
            &request(
                "session.run",
                JsonValue::object([
                    ("session", JsonValue::Int(1)),
                    ("steps", JsonValue::Int(100)),
                ]),
            )?,
        )?;
    }

    let mut overhead_samples = Vec::new();
    let capabilities = request("capabilities", JsonValue::object(std::iter::empty::<(String, JsonValue)>()))?;
    for _ in 0..2_000 {
        let started = Instant::now();
        std::hint::black_box(service.dispatch(&admin, &capabilities)?);
        overhead_samples.push(started.elapsed());
    }
    print_seconds("service_dispatch_capabilities", &overhead_samples);

    let mut run_samples = Vec::new();
    for _ in 0..5 {
        let body = request(
            "session.run",
            JsonValue::object([
                ("session", JsonValue::Int(1)),
                ("steps", JsonValue::Int(1_000)),
            ]),
        )?;
        let started = Instant::now();
        std::hint::black_box(service.dispatch(&admin, &body)?);
        run_samples.push(started.elapsed());
    }
    print_seconds("service_run_1000", &run_samples);
    let total: f64 = run_samples.iter().copied().sum::<Duration>().as_secs_f64();
    println!("service_run_steps_per_second={:.0}", 5_000.0 / total);

    let mut inspect_samples = Vec::new();
    let inspect = request(
        "session.inspect",
        JsonValue::object([("session", JsonValue::Int(1))]),
    )?;
    for _ in 0..100 {
        let started = Instant::now();
        std::hint::black_box(service.dispatch(&admin, &inspect)?);
        inspect_samples.push(started.elapsed());
    }
    print_seconds("service_inspect", &inspect_samples);

    let mut snapshot_samples = Vec::new();
    for _ in 0..20 {
        let body = request(
            "session.snapshot",
            JsonValue::object([("session", JsonValue::Int(1))]),
        )?;
        let started = Instant::now();
        std::hint::black_box(service.dispatch(&admin, &body)?);
        snapshot_samples.push(started.elapsed());
    }
    print_seconds("service_snapshot", &snapshot_samples);

    let mut fork_samples = Vec::new();
    for _ in 0..10 {
        let body = request(
            "session.fork",
            JsonValue::object([("session", JsonValue::Int(1))]),
        )?;
        let started = Instant::now();
        std::hint::black_box(service.dispatch(&admin, &body)?);
        fork_samples.push(started.elapsed());
    }
    print_seconds("service_fork", &fork_samples);
    Ok(())
}
