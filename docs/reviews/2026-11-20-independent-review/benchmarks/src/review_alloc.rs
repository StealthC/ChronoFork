use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

struct Counting;

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static DEALLOCS: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        DEALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn snap() -> (u64, u64, u64) {
    (ALLOCS.load(Ordering::Relaxed), BYTES.load(Ordering::Relaxed), DEALLOCS.load(Ordering::Relaxed))
}

use chronofork_backend_api::Backend;
use chronofork_core::InputFrame;
use chronofork_megadrive::{MegadriveFactory, SchedulerConfig, interrupt_rom};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let hash_on = std::env::var_os("REVIEW_HASH").as_deref() != Some(std::ffi::OsStr::new("0"));
    println!("alloc-probe v1 hash_enabled={}", u8::from(hash_on));
    let factory = MegadriveFactory::with_scheduler_config(interrupt_rom(3), SchedulerConfig::ntsc())?;
    let mut m = factory.create_machine()?;
    for _ in 0..100 { m.step(InputFrame::NEUTRAL)?; }
    let a = snap();
    let n = 1000u64;
    for _ in 0..n { m.step(InputFrame::NEUTRAL)?; }
    let b = snap();
    println!("per_step_allocs={:.2}", (b.0 - a.0) as f64 / n as f64);
    println!("per_step_alloc_bytes={:.0}", (b.1 - a.1) as f64 / n as f64);
    println!("per_step_deallocs={:.2}", (b.2 - a.2) as f64 / n as f64);

    let a = snap();
    for _ in 0..100 { std::hint::black_box(m.snapshot()?); }
    let b = snap();
    println!("per_backend_snapshot_allocs={:.2}", (b.0 - a.0) as f64 / 100.0);
    println!("per_backend_snapshot_alloc_bytes={:.0}", (b.1 - a.1) as f64 / 100.0);

    let a = snap();
    for _ in 0..100 { std::hint::black_box(m.inspect()?); }
    let b = snap();
    println!("per_inspect_allocs={:.2}", (b.0 - a.0) as f64 / 100.0);
    println!("per_inspect_alloc_bytes={:.0}", (b.1 - a.1) as f64 / 100.0);

    let a = snap();
    for _ in 0..100 { std::hint::black_box(m.review_state_hash()); }
    let b = snap();
    println!("per_state_hash_allocs={:.2}", (b.0 - a.0) as f64 / 100.0);
    println!("per_state_hash_alloc_bytes={:.0}", (b.1 - a.1) as f64 / 100.0);

    let a = snap();
    for _ in 0..100 { std::hint::black_box(m.review_state_body()); }
    let b = snap();
    println!("per_state_body_allocs={:.2}", (b.0 - a.0) as f64 / 100.0);
    println!("per_state_body_alloc_bytes={:.0}", (b.1 - a.1) as f64 / 100.0);
    Ok(())
}
