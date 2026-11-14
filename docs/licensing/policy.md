# Licensing Policy

## Position

ChronoFork's preferred license is **MIT**, because it is permissive, compatible
with the MIT-licensed Exodus references, and simple for both open-source and
commercial adopters. MIT is the current project license for original code, but it
is **provisional** until the licensing assessment is reviewed and the project owner
confirms it.

No upstream code has been incorporated into the bootstrap. If that changes, the
decision is re-evaluated before merge.

## Rules

1. **Verify the actual license of every reused component.** A repository-level
   LICENSE is not proof that every directory is under the same terms.
2. **Preserve copyright notices and license texts** in source and binary
   distributions.
3. **Record provenance** for every incorporated file: origin, URL, revision,
   authors, license, destination, modifications.
4. **Do not incorporate unclear-provenance code.**
5. **Do not assume translation to Rust removes copyright obligations.** A port or
   rewrite of protected expression can still be a derivative work.
6. **Do not assume process isolation resolves license incompatibility.** Isolation
   is an engineering boundary, not a license.
7. **Do not use attribution as a substitute for satisfying a license.**
8. **Do not silently change the project license.** A change requires an ADR and
   project-owner approval.
9. **Prefer permissive components.** Prefer independent implementation when reuse
   would impose unacceptable obligations.
10. **Stop and escalate** when a component requires a major licensing decision.

## Compatibility summary

| Source license | Reuse in an MIT project | Notes |
|---|---|---|
| MIT | Generally compatible | Preserve notice; include license text |
| BSD-2/3-Clause | Generally compatible | Preserve notice; do not endorse |
| Apache-2.0 | Generally compatible | Preserve NOTICE and license; patent terms |
| ISC/Zlib | Generally compatible | Preserve notice |
| MPL-2.0 | File-level copyleft | Modified MPL files remain MPL; isolate and disclose |
| LGPL-2.1/3.0 | Conditional | Dynamic linking/relinking obligations; legal review required |
| GPL-2.0/3.0 | Incompatible with MIT distribution as a combined work | Do not incorporate without changing the project license and legal review |
| Unknown/unlicensed | Not reusable | Treat as all rights reserved |

This table is engineering guidance, not legal advice. Edge cases require review.

## BizHawk-specific finding

BizHawk's root `LICENSE` states that the repository contains original MIT work,
embedded submodules with their own licenses, submodules without clearly provided
licenses, and compiled binaries without clear origin. It explicitly says the
repository as-is "should be considered an illegal combination of several
incompatible GPL licenses." Therefore:

- BizHawk is a **research reference only**.
- No BizHawk code, binaries, submodules or fixtures may be copied into ChronoFork.
- BizHawk architectural ideas may be studied and independently implemented, but
  do not copy expression, comments, identifiers or test data.

## Exodus-specific finding

Both Exodus and exodus-for-ais declare MIT (copyright (c) 2024 Roger Sanders) in
their root `License.txt`. That is promising, but before any code reuse:

- Audit the specific files and any bundled third-party code.
- Preserve the MIT notice and license text.
- Record the exact revision and modifications.
- Confirm the headless fork's added files are covered by the same terms.

Research-only use requires no code incorporation and no redistribution.

## ROMs, firmware and fixtures

Never commit:

- commercial ROMs or game images;
- BIOS/firmware without redistribution permission;
- commercial game assets;
- proprietary test data;
- unlicensed binary fixtures;
- downloaded gameplay video/audio.

Prefer synthetic programs generated from source in tests. The bootstrap's test
program is independently authored in `chronofork-test-backend`.

## Required files

- `LICENSE` — project license.
- `THIRD_PARTY_NOTICES.md` — dependencies and incorporated components.
- `docs/licensing/provenance.md` — exact origin/revision/status.
- `docs/licensing/policy.md` — this policy.

## Approval gates

User approval is required before:

- changing the project license;
- incorporating GPL/LGPL/MPL or unclear-provenance code;
- adding a dependency with a non-permissive license;
- distributing binaries containing third-party code;
- publishing a release or remote repository.
