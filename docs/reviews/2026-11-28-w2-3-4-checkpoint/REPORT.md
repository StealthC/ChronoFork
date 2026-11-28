# Checkpoint W2.3/W2.4 — resumo durável

- **Versão:** 1.0 — 2026-11-28 (America/Fortaleza).
- **Estado:** **implementado e publicado para avaliação externa; não mergeado.**
  Branch `agent/w2-3-2026-11-27` sobre a base `cbba190`; o PR do checkpoint é aberto contra
  `main` e **o merge não está autorizado**. A avaliação do dono é o próximo passo.
- **Natureza:** resumo durável para permanecer rastreável depois que o worktree de tarefa for
  removido. Não é uma nova fonte de verdade: escopo/capacidade continuam em
  [`docs/roadmap/milestones.md`](../../roadmap/milestones.md), as decisões nos
  [ADRs](../../adr/README.md) e o estado de execução em
  [`docs/planning/STATE.md`](../../planning/STATE.md) e
  [`docs/planning/ROADMAP.md`](../../planning/ROADMAP.md).
- **Não declara W2 concluído, M8 concluído nem desempenho de jogo.** W2.5–W2.8 não foram
  iniciados e v2 é servido, mas não promovido, até o W3.6.

## 1. Identidade e veredito

| Item | Valor |
|---|---|
| Base / HEAD | `cbba190e201e6e8bae6a6de3239e5069479427cb` (`cbba190`, `main`) |
| Branch do checkpoint | `agent/w2-3-2026-11-27` |
| `sha256(git diff)` revisado | `ac32fb9d726b3f65d488412b27451229c345b1f2d2815080b9f897d0cb401b9b` (64 arquivos rastreados, +1738/−864; 9 arquivos novos bindados por SHA-256) |
| Commit de implementação | `60e5207556d7eaab04fa42db1605eef44e5af74d` |
| Veredito de revisão final | **t25 PASS** (revisão adversarial independente); autoriza apenas a avaliação externa, não merge nem W2.5+/W3.6 |
| Evidência bruta (ignorada) | `.research/consolidation-w2-2026-11-27/{CHECKPOINT-W2-3-4.md,w2-3-4-verify/,w2-3-4-review/,w2-3-4-review-final/,repair-t22/,repair-t24/,t20/}` |

## 2. Escopo entregue

- **W2.3a:** `StepOutcome { tick, halted, verified }` (forma alvo, sem campo de digest; um
  doctest `compile_fail` fixa a remoção) e `VerifiedStepOutcome { tick, halted, state_hash }`
  como tipo distinto — o tipo é a garantia de verificação.
- **W2.3b:** `Backend::run_batch` como único primitivo de lote; gramática tipada de parada
  (`Pc`/`Memory`/`Event`/`Frame`/`Tick`, fase `PreStep`/`PostStep`, `StopSet` ≤4), precedência
  documentada (`cancelled > deadline > halted > stop_condition > completed`; entre condições
  `pc > memory > event > frame > tick`, depois valor, depois posição), `StopBudget`,
  `InputSequence` limitada, `VerificationPolicy`, `Session::run_until`/`run_cycles` e
  `RunRecord` (`k` + quadros explícitos, padding neutro implícito).
- **W2.3c:** `CancelToken` (`Arc<AtomicBool>`, sem lock no hot path) e `RunControl` (prazo
  absoluto de host imutável); host time só nos limites de passo; resultado cancelado/deadline é
  o prefixo determinístico exato.
- **W2.4a:** `Backend::status()` barato (`MachineStatus` com mapas CPU/device separados e
  limitados, sem digest nem cópia de RAM); `MachineInspection` sem `state_hash` e memória só
  sob janela explícita; `session.list` sem `inspect` por sessão; JSON v1 preservado.
- **W2.4b:** wire v2 **no servidor** com `performed`/`cycles`/`stop_reason` do primitivo de
  lote, v1 com a forma exata, versão desconhecida recusada **antes do dispatch** sem execução
  parcial, e MCP em `2025-03-26` (sem bump). `expected_tick` usa `status()` e mantém um único
  digest.

**Fora deste checkpoint:** W2.5 (buffers/`Arc`/`reap`), W2.6 (render opcional), W2.7 (G-PERF),
W2.8 (política de movie) e a promoção cliente/GUI de v2 (W3.6). W2 **não** está completo e v2 é
**servido, não promovido**.

## 3. Ciclo de revisão (honesto)

| Etapa | Veredito | Conteúdo |
|---|---|---|
| t18 | PASS com achados | verificação independente; F1 de documentação e notas menores |
| t19 | **NEEDS_REVISION** | C1 tick zero inventado no default; C2 cap de registro; C3 `kind`/`value` contraditórios; C4 `expected_tick` com dois digests; C5 teste cross-thread sem sincronização |
| t22 | reparo | C1–C5 corrigidos com testes |
| t23 | **NEEDS_REVISION** | C1b: motivo de controle vencedor podia apagar `halted`/tick autoritativos do passo |
| t24 | reparo | C1b corrigido, prioridade de motivos inalterada |
| t25 | **PASS (final)** | C1–C5/C1b resolvidos; probe independente 8/8; gates completos verdes; uma advisory low (margem de 200 ms do teste de deadline) |

`t19` e `t23` **não** passaram e não são citados como PASS.

## 4. Evidência executada nesta publicação

- Gates (todos exit 0): `fmt --check`, `check --workspace`, `clippy --workspace --all-targets
  -- -D warnings`, `cargo test --workspace` = **148 alvos, 1682 passed, 0 failed, 1 ignored**
  (o `ignore` é o `oracle_harness` pré-existente), `./scripts/check.sh` (inclui rustdoc
  `-D warnings`; `==> all quality gates passed`), `git diff --check`.
- Suítes nomeadas verdes, incluindo `w2_control` 10 (C1/C1b/C5 incluídos), `sessions batch` 4,
  `w2_wire_v2` 3, `transport http` 27 (segurança), `cli serve_bind` 2.
- Probe independente do t25: **8/8**, incluindo o contraexemplo halt+cancel síncrono
  (`Cancelled`, `halted=true`, performed=1, cycles=10, tick=110).
- Equivalência: `final_tick=87052`, `final_state_hash=f41e97af…`,
  `serialized_checkpoint_bytes=206437`, `backend_payload_bytes=206261`,
  `restore_state_hash_matches=1` idênticos à base; demos byte-idênticos
  (`demo` 986 B `9b4dd564…`, `megadrive demo` 1994 B `e3098538…`, `gui run` 2842 B
  `d3abf57a…`); payload/estado/config **v9**, movies v1/v2 e TAS inalterados, MCP `2025-03-26`.
- Benchmark (release `bench_step`, `interrupt_rom(3)`, 5×10 000 passos, `taskset -c 2`):
  base ≈4,7 k passos/s (p50 ≈2,38 s/10 mil) vs checkpoint ≈3,65 M passos/s (p50 ≈0,0027 s;
  p95 ≈0,0028 s), mesmos tick/hash/checkpoint bytes. **Sem FPS/G-PERF/gameplay claim**; a base
  não imprime p95 (UNAVAILABLE) e a medição é de um host WSL2.

## 5. Caveats e limitações registradas

- **`AppService::run` em lotes de um passo:** composição com `Session::run_until` para publicar
  exatamente o prefixo aplicado quando um passo falha; é uma limitação de throughput
  documentada, não uma alegação de desempenho.
- **C1b preservado:** após cada passo o laço restaura tick/halt autoritativos antes de escolher
  o motivo de controle; a prioridade `cancelled > deadline > halted` não muda.
- **Advisory low do t25:** a margem de 200 ms do teste de deadline não é garantia universal de
  scheduler; o probe síncrono de cancelamento cobre a mesma lógica sem essa suposição.
- **Cobertura de políticas esparsas:** `EveryN`/`AtTick`/`OnDemand` foram provadas por probes
  do revisor, não por testes permanentes; adicionar regressões antes de futuros consumidores.
- **`durable`** ainda calcula um digest redundante após `SessionInspection` (follow-up menor).
- **GUI:** o decoder estrito aceita a folha v2 `cycles` (necessário porque os canais GUI já
  pedem v2); isso não implementa o probe/promoção W3.6.
- Não verificado: CI remoto/outros OS, gameplay comercial, G-PERF calibrado.

## 6. Pendências

W2.5 (buffers/`Arc`/`reap`), W2.6 (render opcional), W2.7 (G-PERF) e W2.8 (política de movie)
só após a avaliação externa deste checkpoint. W3.6 trata da promoção cliente/GUI de v2. Nada
disso foi iniciado e nenhum merge é reivindicado ou autorizado por este resumo.
