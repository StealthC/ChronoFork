# Milestone 8: Multi-backend expansion

**Status:** active
**Started:** 2026-11-17

## Goal

Integrate a second emulated system behind the existing backend and capability contracts,
proving that the session, storage, TAS, trace, debug, service and transport layers are
genuinely backend-neutral. The milestone must report what each backend really supports —
capability differences are surfaced, never normalized away — and must not change the core
semantics of sessions, storage or transport. Scope and exit intent come from
[Stage 8](../roadmap/roadmap.md#stage-8--multi-backend-expansion); the whole-project plan
and phase outline come from the
[roadmap design](../superpowers/specs/2026-11-17-roadmap-design.md). The second-system
choice is deliberately not made here: phase 8.1 produces that decision as an accepted ADR
with alternatives.

## Definition of Done

- [ ] A second system is integrated without changing session, storage or transport core semantics.
- [ ] The capability matrix distinguishes what each backend really supports instead of normalizing differences away.
- [ ] Cross-backend differential tests run and are recorded as evidence.
- [ ] FR-API-004 (external SDK contract and stability policy) is implemented or explicitly deferred with a recorded decision.
- [ ] The full quality gate (`./scripts/check.sh`) passes and the canonical docs/ADRs are updated.
- [ ] Limitations and unverified claims are stated honestly; no new third-party dependency is added without an ADR.

## Phases

1. Phase 8.1 — Second-system decision and capability contract [pending]
2. Phase 8.2 — Second backend core behind the backend contract [pending]
3. Phase 8.3 — Backend-neutral shared layers and capability matrix [pending]
4. Phase 8.4 — Cross-backend differential tests [pending]
5. Phase 8.5 — External SDK contract and stability policy [pending]

## Audit History

| Date | Verdict | Gaps |
|---|---|---|
| — | not audited | — |
