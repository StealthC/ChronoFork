# Cross-Crate Tests

Crate-local unit and integration tests live next to their crates:

- `crates/chronofork-core/src/*` — core primitive unit tests.
- `crates/chronofork-sessions/tests/acceptance.rs` — session/fork/snapshot acceptance.
- `crates/chronofork-test-backend/src/lib.rs` — synthetic backend unit tests.
- `crates/chronofork-cli/tests/demo.rs` — CLI integration tests.

This root directory is reserved for future cross-crate fixtures, trace corpora and
end-to-end harnesses that do not belong to a single crate. Do not place commercial
ROMs, firmware or generated media here; see the fixture policy in
[`docs/quality/testing-strategy.md`](../docs/quality/testing-strategy.md).
