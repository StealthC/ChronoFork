//! Long-run admission for the bounded worker pool (ADR-0016 §1, F4/K10).
//!
//! The transport serves every request with a **single synchronous pool of `N` workers**.
//! A long operation cannot simply be allowed to occupy a worker: with `N` long runs in
//! flight, `cancel`/`progress`/`list`/`capabilities` would have no thread left to run on,
//! and a lock-free cancel token alone does not create one.
//!
//! This gate is the reservation. It admits at most `N - 1` long runs globally (leaving one
//! worker for the control path) and at most one per principal, by default. Admission is
//! decided **before** the long work starts, so a request that would exceed a bound is
//! answered with a typed `limit_exceeded` instead of occupying a worker while it waits.
//!
//! What counts as a long run is a documented, closed predicate:
//! `is_long_operation` names the operations whose service call can run for the whole
//! host deadline. It is deliberately a property of the operation, not of its arguments,
//! so the bound cannot be dodged by a small-looking request.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// The operations whose one call can occupy a worker for the whole host deadline.
///
/// `session.run` is the only one in this build: it executes up to
/// `max_steps_per_request` steps in a single synchronous call.
#[must_use]
pub(crate) fn is_long_operation(op: &str) -> bool {
    op == "session.run"
}

/// The configured long-run bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AdmissionLimits {
    /// Long runs allowed at once across every principal.
    pub(crate) global: usize,
    /// Long runs allowed at once for one principal.
    pub(crate) per_principal: usize,
}

/// Counts of the long runs currently occupying worker capacity.
#[derive(Debug, Default)]
struct Counters {
    global: usize,
    per_principal: BTreeMap<String, usize>,
}

/// The shared admission gate.
#[derive(Debug)]
pub(crate) struct AdmissionGate {
    counters: Mutex<Counters>,
    limits: AdmissionLimits,
}

impl AdmissionGate {
    /// Creates one gate for the configured bounds.
    pub(crate) fn new(limits: AdmissionLimits) -> Self {
        Self {
            counters: Mutex::new(Counters::default()),
            limits,
        }
    }

    /// Admits one long run, or returns `None` when a bound is already reached.
    ///
    /// The check and the increment are one critical section, so two simultaneous
    /// attempts cannot both take the last slot. The guard owns a clone of the gate, so it
    /// can outlive the borrow of the state it was created from without a lifetime tie.
    pub(crate) fn acquire(gate: &Arc<Self>, principal: &str) -> Option<AdmissionGuard> {
        let mut counters = gate.counters.lock().ok()?;
        if counters.global >= gate.limits.global {
            return None;
        }
        let held = counters.per_principal.get(principal).copied().unwrap_or(0);
        if held >= gate.limits.per_principal {
            return None;
        }
        counters.global += 1;
        counters
            .per_principal
            .insert(principal.to_owned(), held + 1);
        Some(AdmissionGuard {
            gate: Arc::clone(gate),
            principal: principal.to_owned(),
        })
    }

    /// Releases one admission.
    fn release(&self, principal: &str) {
        if let Ok(mut counters) = self.counters.lock() {
            counters.global = counters.global.saturating_sub(1);
            if let Some(held) = counters.per_principal.get_mut(principal) {
                *held = held.saturating_sub(1);
                if *held == 0 {
                    counters.per_principal.remove(principal);
                }
            }
        }
    }
}

/// Releases one long-run admission when the operation ends, including on an early return.
pub(crate) struct AdmissionGuard {
    gate: Arc<AdmissionGate>,
    principal: String,
}

impl Drop for AdmissionGuard {
    fn drop(&mut self) {
        self.gate.release(&self.principal);
    }
}

impl std::fmt::Debug for AdmissionGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdmissionGuard")
            .field("principal", &self.principal)
            .finish()
    }
}
