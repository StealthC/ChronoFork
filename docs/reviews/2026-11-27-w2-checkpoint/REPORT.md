# Checkpoint W2.1/W2.2 — resumo durável

- **Versão:** 1.0 — 2026-11-27 (America/Fortaleza).
- **Estado:** **integrado**. PR #42 mergeado em `main` no commit de merge
  `cbba190e201e6e8bae6a6de3239e5069479427cb`; CI pós-merge (run `46079267918`) com sucesso.
- **Natureza:** resumo durável de checkpoint para permanecer rastreável depois que o worktree
  de tarefa for removido. Não é uma nova fonte de verdade: escopo/capacidade continuam em
  [`docs/roadmap/milestones.md`](../../roadmap/milestones.md), as decisões nos
  [ADRs](../../adr/README.md) e o estado de execução em
  [`docs/planning/STATE.md`](../../planning/STATE.md) e
  [`docs/planning/ROADMAP.md`](../../planning/ROADMAP.md).
- **Não declara W2 concluído, M8 concluído nem desempenho de jogo.** W2.3 não foi iniciado.

## 1. Aprovação e integração

| Item | Valor |
|---|---|
| PR | [#42](https://github.com/StealthC/ChronoFork/pull/42) — "W2.1/W2.2 checkpoint: loopback bind, Origin/pre-auth, explicit state_hash (for owner evaluation)" |
| Aprovação do dono | merge do PR #42 pelo dono (`mergedBy: StealthC`) em 2026-11-27T22:52:53Z |
| Merge commit | `cbba190e201e6e8bae6a6de3239e5069479427cb` (`cbba190`) |
| CI pós-merge | run `46079267918`, workflow `CI`, `headSha cbba190`, conclusão **success** |
| Commits do checkpoint | `3da8ba4` segurança; `9e56f65` W2.1/W2.2; `74ba70a` somente `docs/planning/` |
| Diff de produto revisado | `sha256 284c757550b4d1dfb678b4c3af5474f2b599f41a1963450359f066d6f8544735` |
| Evidência bruta (ignorada) | `.research/consolidation-w2-2026-11-27/{CHECKPOINT.md,verify/,review/,bench/,repair-t10/}` |

O relatório completo de 13 seções é `.research/consolidation-w2-2026-11-27/CHECKPOINT.md`
(ignorado); este documento é o resumo versionado, sem logs extensos.

## 2. Escopo entregue

- **W2.1** (design W2-1): `Backend::state_hash()` obrigatório, `VerifiedStepOutcome`,
  `step_verified` com uma única fonte canônica, `StepOutcome` com `state_hash: Option<_>` e
  `verified`.
- **W2.2** (design W2-2): `step` ordinário sem digest (`state_hash = None`, `verified = false`);
  consumidores de digest migrados para `step_verified`/`state_hash()`.
- **Segurança aprovada** (O-1/ADR-0014): recusa de bind não-loopback e recusa de `Origin`
  com prazo de pré-autenticação absoluto.
- **Fora deste checkpoint:** W2.3–W2.8 (ver §7); nenhum `run_batch`, `status()`/wire v2,
  remoção do `Option`, `Arc`/`reap`, render novo ou política nova de movie.

## 3. Contratos modificados

1. `Backend::state_hash(&self) -> Result<StateHash, BackendError>` — obrigatório, sem corpo
   padrão; cada backend delega ao serializador que já existia (nenhum segundo serializador).
2. `VerifiedStepOutcome { tick, halted, state_hash, verified }` — tipo distinto (`Copy`).
3. `Backend::step_verified` — default `step` + exatamente uma chamada a `state_hash()`.
4. `StepOutcome { tick, state_hash: Option<StateHash>, halted, verified }` — `Option` é ponte
   de migração; a remoção é W2-3.
5. `Session::step_verified`/`SessionManager::step_verified` — usados por movie v1/v2, âncoras,
   replay, player seek, edit rebuild, artefatos de experimento e relatórios do debugger.
6. Segurança: recusa não-loopback no parsing do CLI (antes do token, serviço, socket e
   `record=serve`) e no transporte antes de `TcpListener::bind`; todo `Origin` recebe
   `403 forbidden` antes de rota/autenticação/corpo; `pre_auth_timeout` absoluto desde o
   `accept`, incluindo a fila.

Wire/serviço continua **v1**; MCP e formatos de snapshot/movie/TAS não mudaram.

## 4. Migração e censo remedido

- **19** implementações de `Backend` (2 de produção, 17 dublês) e **19** implementações do
  novo `state_hash`; **40** declarações `fn state_hash` no workspace (trait, serializadores
  inerentes, acessores, decodificadores do GUI, dublês).
- **36** ocorrências de `step_verified` (3 definições + 33 usos; 15 em produção).
- Consumidores migrados: movie v1/v2 e âncoras, replay, player seek, edit rebuild,
  `service/experiment.rs`, `debug/debugger.rs`, TAS/hardware e testes de controle.
- Nenhum consumidor restante lê `StepOutcome.state_hash`; os que só usam `tick`/`halted`
  continuam no caminho ordinário. Serializador canônico: um por backend.

## 5. Verificação e revisão

- **t5 (verificação round 1): FAIL** apenas no critério de planejamento (STATE/plano ainda
  diziam "só avisa" e "não implementado"); os demais critérios passaram.
- **t8 (reparo)**: corrigiu F1 (STATE/plano §12.2), F2 (cabeçalhos dos testes W2.1→W2.2) e F3
  (nota de supersessão no registro histórico M6); **t9 (round 2): PASS**.
- **t6 (revisão adversarial round 1): NEEDS_REVISION** com C1 (prazo de pré-auth reiniciado no
  despacho do worker) e C2 (comparações de digest viravam `None == None`).
- **t10 (reparo)**: prazo carimbado no `accept`, fila podada por vencimento e prazo propagado
  sem reset; FIFO/arbitragem com `step_verified`; **t11 (round 2): PASS**, sem achado
  médio/alto remanescente.
- Gates executados com `CARGO_HOME=/tmp/t7-cargo`: `cargo fmt --all -- --check`,
  `cargo check --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` (**142 binários; 1658 passed, 0 failed, 1 ignored** — o `ignore` é o
  `oracle_harness` pré-existente), `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`,
  `./scripts/check.sh` e `git diff --check` — todos exit 0.
- Testes nomeados: `twin_machines_normal_vs_verified`,
  `verified_step_hashes_exactly_once_per_step`, `hash_single_source`,
  `state_hash_is_side_effect_free`, `ordinary_step_does_not_claim_verification`,
  `queued_connections_expire_against_the_accept_deadline`, `serve_bind` (2),
  suíte HTTP 27/27, arbitragem 8/8, FIFO reparado.

## 6. Equivalência e segurança

- **Equivalência:** `final_tick=87052`,
  `final_state_hash=f41e97af1260d2118a40d2f3bb93b7c94d160b8cf1eb02ab9deea3f067379a5f`,
  `serialized_checkpoint_bytes=206437`, `backend_payload_bytes=206261`,
  `restore_state_hash_matches=1` idênticos entre a base `1390841` e a árvore W2.2.
  Demos byte-idênticas (`demo` 986 B `9b4dd564…`; `megadrive demo` 1994 B `e3098538…`;
  `gui run` 2842 B `d3abf57a…`), goldens não re-gravados, payload/estado/config **v9**,
  movies v1/v2/TAS e wire inalterados.
- **Segurança:** `--bind` não-loopback → exit 1, stdout vazio, diagnóstico estável, nada
  escutando; `127.0.0.1:0` ainda chega à checagem de token; `/health` 200, `/v1/tools` 200 com
  token e requisição com `Origin` → **403 `forbidden`**. Sonda de fila (2 workers, 6 heads,
  orçamento 5 s) fechou em **5,09–5,24 s** (uma janela, não ondas 5/10/15 s); regressivo de
  12 conexões/400 ms + 500 ms passa. Adiados: limite por peer e validação `Host` (ADR-0014
  §1.5; `docs/architecture/agent-interfaces.md` §12).

## 7. Benchmark (sem alegação de FPS/G-PERF)

Comando idêntico nas duas árvores:
`CARGO_HOME=/tmp/t7-cargo cargo run -p chronofork-megadrive --example bench_step --release`
(`interrupt_rom(3)`, NTSC, 5 trials × 10 000 passos, host WSL2). Base ~**4 734** passos/s;
W2.2 ~**3 643 025** passos/s → **≈770×** no caminho ordinário (p50 ≈856×; rodadas anteriores
805×–922×). O rótulo `instructions_per_second` conta passos de backend, não instruções de
convidado. **Nenhuma meta/claim de FPS ou jogo**; G-PERF/G1–G6 é W2.7 e exige runner
calibrado.

## 8. Limites e W2 pendente (nada iniciado)

- **W2.3 / W2-3 (ativo):** remover o `Option` de `StepOutcome`, migrar call sites e adicionar o
  primitivo único de lote (`run_batch`/`run_until`/`run_cycles`), cancelamento lock-free por
  instrução e gramática/precedência de `stop_on`.
- **W2.4a / W2-4:** `MachineStatus`/`status()` barato, `inspect()` sem hash, `list` sem
  inspecionar cada sessão e wire/serviço **v2** (mantendo v1 para chamadas v1).
- **W2.5:** buffers persistentes e `Arc` para payload/`initial_state`, `reap()` seguro, sem COW.
- **W2.6 (opcional):** render preservando índice/janela de frame, sem deletar o artefato.
- **W2.7 (G-PERF):** workload fixado, A/B intercalado, p50+p95, minflt, modo de alocador,
  caminho verificado medido; metodologia registrada, sem meta de fps.
- **W2.8:** política de movie/versionamento no envelope e compatibilidade de snapshot.

O `Option` ainda existe; não há `run_batch`/`status`/wire v2/`Arc`/`reap`/render novo. W2.3–W2.8
não foram iniciados neste checkpoint.

## 9. Referências

- [PR #42](https://github.com/StealthC/ChronoFork/pull/42); CI
  [run 46079267918](https://github.com/StealthC/ChronoFork/actions/runs/46079267918).
- [ADR-0014](../../adr/0014-v1-scope-single-user-loopback.md),
  [ADR-0015](../../adr/0015-verification-contract.md),
  [ADR-0016](../../adr/0016-session-service-and-agent-api.md).
- [Plano de consolidação](../../superpowers/plans/2026-11-20-consolidation-and-agent-api.md)
  (§4 W2, §12 ledger) e [design W2](../../superpowers/specs/2026-11-20-w2-verification-migration-design.md).
- Estado: [STATE.md](../../planning/STATE.md), [ROADMAP.md](../../planning/ROADMAP.md);
  capacidade: [milestones.md](../../roadmap/milestones.md).
- Evidência ignorada: `.research/consolidation-w2-2026-11-27/` (`CHECKPOINT.md`, `verify/REPORT.md`,
  `review/REVIEW.md`, `review/REVIEW-ROUND2.md`, `bench/`, `repair-t10/`).
