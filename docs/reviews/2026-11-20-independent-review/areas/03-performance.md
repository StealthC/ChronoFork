# Área 03 — Desempenho, hashing e modelo de execução

**Revisor:** perf · **Data:** 2026-11-20 (America/Fortaleza) · **Baseline:**
`b8d63a17349e672ba6945eaba3379cec38cf9d1b` (`origin/main`) · **Worktree:**
`review-2026-11-20-architecture` · **Evidência bruta e scripts:**
[`../benchmarks/`](../benchmarks/README.md) · **Tabelas completas:**
[`../benchmarks/results-tables.md`](../benchmarks/results-tables.md)

Classificação usada em cada afirmação: **[fato observado]** (executado, log em
`benchmarks/raw/`), **[hipótese]** (mecanismo provável, não medido),
**[inferência]** (decorre de fatos medidos), **[opinião]**. Confiança
(alta/média/baixa) e prioridade (P0–P3) por achado.

---

## 1. Sumário executivo

O step do backend Mega Drive custa **209,8 µs** (mediana de 3 execuções;
5.185–5.269 steps/s), dos quais **99,9% são verificação** — serialização do
estado canônico completo (197.048 B) + SHA-256 — e **0,08% é emulação**
(0,17 µs). A execução pura medida é **5,55 M steps/s**; com o alocador glibc
estabilizado (proxy de buffer persistente) o mesmo step cai para **93,2 µs**
(10.730 steps/s), provando que **metade do custo atual é churn de alocador**
(2.568.723 minor faults / 50.000 steps = **51,4 page faults por step**).

Consequências, medidas e não especulativas:

| Cenário | Medido hoje | Com hash fora do step + buffer reaproveitado |
|---|---|---|
| 1 frame NTSC (10.256 steps) | **0,95–2,15 s** → 0,47–1,05 fps | **3,1 ms** → 319–329 fps |
| Margem para 60 fps (16,67 ms) | **57–129× acima** do orçamento | ~5,3× de folga |
| `session.run` de 1.000 steps | 209,5 ms | ~0,3 ms |
| Checkpoint (`session.snapshot`) | 183 µs / 192 KiB | ~90–180 µs (CoW/dedup) |
| Fork (`fork_current`) | 666 µs / 822 KiB | ~300 µs / ~400 KiB (dedup) |

Nenhuma meta interativa de 60/50 fps é atingível com hash canônico por
instrução; **todas são folgadas** se a verificação sair do caminho de execução
e for feita sob demanda (por frame, por checkpoint ou em janela de replay).
Os dois gargalos acionáveis agora são **P0**: (F-01) hash completo por step no
contrato `Backend::step`; (F-02) 4 alocações/460 KB transitórios por step, sem
reuso, que dobram o custo via page faults. Ambos já têm ganho medido, não
estimado.

---

## 2. O que o contrato faz hoje (fatos observados, com evidência)

1. `Backend::step(&mut self, InputFrame) -> StepOutcome` devolve `state_hash`
   **em toda** chamada ([`crates/chronofork-backend-api/src/traits.rs:14-33`](../../../../crates/chronofork-backend-api/src/traits.rs)).
2. O step Mega Drive fecha com `state_hash()` ([`backend.rs:632-685`](../../../../crates/chronofork-megadrive/src/backend.rs)),
   que monta uma `StateView` e chama `state_hash_for`. `state_hash` ([`backend.rs:488-513`](../../../../crates/chronofork-megadrive/src/backend.rs))
   serializa: 8 D + 8 A + PC + SR + halted + tick + USP/SSP + cpu_cycles +
   stopped + **64 KiB RAM** + **snapshot completo do VDP (VRAM/CRAM/VSRAM/regs/FIFO/DMA/timing)** +
   7 bancos do mapper + SRAM control + **64 KiB SRAM** + TMSS + scheduler.
3. `state_body` aloca um `Vec` novo por chamada ([`backend.rs:1201-1253`](../../../../crates/chronofork-megadrive/src/backend.rs));
   `hash_state_body` aloca **outro** buffer `STATE_MAGIC + body` e hasheia
   ([`backend.rs:1255-1263`](../../../../crates/chronofork-megadrive/src/backend.rs));
   `Vdp::snapshot_bytes` aloca **outro** `Vec` de `SNAPSHOT_LEN`
   ([`vdp/mod.rs:1081-1128`](../../../../crates/chronofork-megadrive/src/vdp/mod.rs)).
   **Medido:** 4 alocações / 459.970 B por step; 3 alocações / 262.911 B só para
   o corpo; 7 / 657.059 B para `snapshot()`. Com `REVIEW_HASH=0`: **0 alocações
   por step**.
4. `snapshot()` refaz corpo + hash + envelope de payload
   ([`backend.rs:765-779`](../../../../crates/chronofork-megadrive/src/backend.rs));
   `inspect()` **também** calcula o hash completo e copia 64 KiB de RAM
   ([`backend.rs:883-977`](../../../../crates/chronofork-megadrive/src/backend.rs), cópia de RAM em :975):
   **60 alocações / 528.688 B** e 96 µs por inspeção, contra 3,85 µs/56
   alocações quando o hash é desligado (95,9% é verificação).
5. `Session::snapshot` ([`session.rs:151-158`](../../../../crates/chronofork-sessions/src/session.rs))
   materializa e **retém** um payload novo de 197 KB por checkpoint (183 µs,
   +90 µs sobre o `backend.snapshot()` de 94 µs, medido inclusive com hash
   desligado — o custo extra é page fault da retenção, não hash).
   `fork_current` = `snapshot` + `fork_from_snapshot`, que cria backend, restaura,
   tira **outro** snapshot para `initial_state` e ainda clona o snapshot adotado
   ([`manager.rs:184-227`](../../../../crates/chronofork-sessions/src/manager.rs)):
   3 cópias do mesmo estado, **822 KiB e 666 µs por fork** (RSS medido).
6. `AppService::run` ([`service.rs:461-491`](../../../../crates/chronofork-service/src/service.rs))
   descarta o `state_hash` de cada `StepOutcome` (só usa `halted`) e faz **um
   `inspect()`** no fim — mas paga o hash em todos os steps. Segura o
   `Mutex<SessionManager>` global durante todo o loop ([`service.rs:695`](../../../../crates/chronofork-service/src/service.rs)),
   com `max_steps_per_request = 100_000` ([`limits.rs:37`](../../../../crates/chronofork-service/src/limits.rs)):
   a 5.269 steps/s isso são **~19 s de lock global** por requisição máxima.
7. `capture_frame` roda **em todo início de frame**, mesmo sem consumidor de
   mídia ([`backend.rs:723-731`](../../../../crates/chronofork-megadrive/src/backend.rs)):
   clona o VDP e renderiza plano/janela/sprite. Medido: **1,267 ms por frame**
   (clone do VDP: 1,23 µs = 0,1%). Em headless sem consumidor, 39 renders em
   400.000 steps = 49,4 ms de 118,6 ms totais (execução 69,2 ms): **42% do
   trabalho pós-hash é renderização descartada**.
8. `create_machine` clona o `Cartridge` inteiro por máquina
   ([`backend.rs:302-303`](../../../../crates/chronofork-megadrive/src/backend.rs)).
   Medido com `interrupt_rom`: ROM = 1 KiB, clone 1,03 µs, footprint da máquina
   132.424 B. Para ROM de 4 MiB o efeito é aritmético (≈4 MiB de RSS por sessão
   e um `memcpy` por máquina) — **[hipótese]** não medida (o slice não carrega
   ROM comercial).
9. Debug/tracing desligados: o trace não foi acionado e o observador está
   ausente; o step puro mede 173–180 ns. **Trace ligado (capacidade 65.536) mede
   186,5 ns/step: +7,6%** (13,4 ns/step; ~0,13 ms/frame a 60 fps). O custo dos
   hooks de debug com tudo desligado não foi isolado por ablação; ele está
   contido nos 173 ns e é irrelevante enquanto o hash dominar.

---

## 3. Decomposição medida do step (mediana de 3 execuções)

Workload `interrupt_rom(3)`, NTSC. "Churn" = overhead de alocador/páginas,
obtido comparando a mesma medição com `MALLOC_MMAP_THRESHOLD_`/
`MALLOC_TRIM_THRESHOLD_` elevados (mesmo código, mesmo hash).

| Componente | Tempo/step | % do step padrão | % do step sem churn |
|---|---|---|---|
| Emulação (CPU + VDP + scheduler + hooks) | **0,173 µs** | 0,08% | 0,19% |
| Serialização do corpo canônico (197.048 B) | **4,02 µs** | 1,9% | 4,3% |
| SHA-256 sobre `STATE_MAGIC + body` (~2,3 GB/s efetivos) | **86,9 µs** | 41,4% | 93,4% |
| **Verificação total (corpo + hash)** | **91,0 µs** | **43,4%** | **97,7%** |
| Churn de alocador/páginas (4 allocs/460 KB por step) | **116,6 µs** | **55,6%** | 0% (eliminado) |
| Residual (medição) | ~2 µs | 0,9% | 2,1% |
| **Step completo** | **209,8 µs** (padrão) / **93,2 µs** (sem churn) | 100% | 100% |

Verificação ÷ emulação = **~502× por instrução**. Confirmação independente:
2.568.723 minor faults em 50.000 steps = **51,4 faults/step** (~205 KB de
páginas novas por step) com o alocador padrão; **82 faults em 50.000 steps**
com os thresholds elevados, e o tempo cai de 209,8 µs para 93,2 µs sem nenhuma
mudança de código semântico.

---

## 4. Três execuções e medianas (headline)

Completo em [`../benchmarks/results-tables.md`](../benchmarks/results-tables.md).
Runs: release, `taskset -c 6`, host compartilhado (load 2,9–4,0).

| Métrica | run 1 | run 2 | run 3 | **mediana** |
|---|---|---|---|---|
| `bench_step` intacto: step p50 (s/10k) | 2,1739 | 2,1385 | 2,1406 | **2,1406** |
| `bench_step` intacto: instructions/s | 5.166 | 5.248 | 5.185 | **5.185** |
| `bench_step` intacto: min do p50 (s/10k) | 1,0492 | 1,0264 | 1,0391 | **1,0391** |
| hash por step, alocador padrão: instructions/s | 5.228 | 5.269 | 5.389 | **5.269** |
| hash por step, alocador padrão: minor faults/50k | 2.568.723 | 2.568.723 | 2.568.723 | **2.568.723** |
| hash por step, thresholds elevados: instructions/s | 10.720 | 10.794 | 10.730 | **10.730** |
| execução pura, alocador padrão: instructions/s | 2.813.962 | 2.304.390 | 2.840.598 | **2.813.962** |
| execução pura, thresholds elevados: instructions/s | 5.553.202 | 5.417.811 | 5.623.358 | **5.553.202** |
| trace 65.536 na execução pura: ns/step | 186,5 | 186,6 | 182,6 | **186,5** |
| frame: step que captura frame p50 (s) | 1,2725 | 1,2610 | — | **1,2668** |
| frame: step comum p50 (ns) | 92 | 102 | — | **97** |
| serviço `session.run` 1.000 steps p50 (s) | 0,20881 | 0,20946 | 0,21090 | **0,20946** |

Nota de método: o `bench_step` intacto mostra **bimodalidade** (min ~1,03 s vs
p50 ~2,14 s por 10k steps) porque o alocador alterna entre `mmap`/`brk` novo e
reuso de heap. Reportar só a mediana de 5 trials esconde isso; a distribuição
completa está em `benchmarks/raw/`.

Custos de operações (p50, 3 execuções; hash por step quando aplicável):

| Operação | Tempo | Espaço |
|---|---|---|
| `backend.snapshot()` | 92,4–93,9 µs (88,2 µs com `REVIEW_HASH=0`; o hash interno do snapshot continua ativo) | payload 197.058 B |
| `backend.restore()` | 98,0–104,4 µs | — |
| `backend.inspect()` | 95,9 µs (3,85 µs sem hash) | 60 allocs/529 KB |
| `session.snapshot()` | 183 µs | 192 KiB retidos |
| `session.snapshot_bytes()` | 86–90 µs | 197.234 B |
| `session.restore()` | 102 µs | — |
| `session.fork_current()` | 666 µs | 822 KiB RSS |
| `session.create_session()` | 278 µs | 346 KiB RSS |
| `service.dispatch` (sem sessão) | 6,5 µs | — |
| `service.inspect` | 224 µs | — |
| `service.snapshot` | 369 µs | — |
| `service.fork` | 796 µs | — |
| `capture_frame` | 1.267 µs | 320×224×4 B RGBA |
| `clone_vdp` / `clone_cartridge` (1 KiB) | 1,23 µs / 1,03 µs | VDP 328 B + heap |

---

## 5. Achados

### F-01 — Hash canônico completo em cada step domina o custo (99,9%) e inviabiliza 60 fps
- **Descrição:** `Backend::step` entrega `state_hash` de 197 KB de estado por
  instrução. Medido: 91,0 µs de verificação contra 0,173 µs de emulação.
  Consequência: 0,47–1,05 fps por frame NTSC; `session.run` de 1.000 steps leva
  209,5 ms; a meta de emulação interativa é aritmeticamente impossível com o
  contrato atual (orçamento 60 fps = 1,63 µs/step).
- **Evidência:** §3 e §4; `raw/baseline-*.txt`, `raw/iso-hash-full-*.txt`,
  `raw/iso-nohash-*.txt`.
- **Componentes:** `chronofork-megadrive` (state_hash/step), `chronofork-backend-api`
  (`StepOutcome`), consumidores em `chronofork-sessions`, `-service`, `-debug`,
  `-cli`, `-gui`.
- **Condições:** qualquer execução real; agrava com RAM/VRAM maiores.
- **Impacto atual:** bloqueia meta interativa/headless de M8; testes de demo
  dominados por hash (`megadrive_demo_*` 166,9 s no baseline).
- **Impacto futuro:** cada novo dispositivo (Z80, áudio, timers) **aumenta** o
  corpo hasheado; o custo por step cresce junto com o estado.
- **Probabilidade:** 100% (determinístico, medido).
- **Severidade:** blocker · **Confiança:** alta · **Prioridade:** **P0** ·
  **Fase:** M8 (transversal; ADR de contrato).
- **Alternativas:** hash sob demanda (A), granularidade por frame/N steps (C),
  `run_until/run_cycles/run_frame` (F), hash incremental (D), não fazer nada (J).
- **Recomendação:** tirar a verificação do caminho de execução; manter
  `state_hash` disponível **sob demanda** (backend) e como política explícita no
  nível de sessão/serviço. Ganho medido: 5,55 M steps/s em vez de 5,27 k.
- **Custo/risco:** médio; exige ADR (muda `StepOutcome`), adaptação de
  debugger/TAS/trace e testes que hoje afirmam hash por step (31 usos de
  `StepOutcome`, 122 chamadas de `.inspect()` no workspace).
- **Determinismo:** risco baixo — o hash é derivado, não estado; a igualdade
  passa a ser verificada em pontos explícitos (e por replay, ver E).

### F-02 — Quatro alocações de ~460 KB por step dobram o custo via page faults
- **Descrição:** cada step aloca VDP snapshot + corpo + buffer de hash (4 allocs,
  459.970 B). O glibc alterna entre reutilizar heap e devolver páginas ao SO:
  51,4 minor faults/step (~205 KB de páginas novas) e **+116,6 µs/step (55,6%)**.
  Mesmo código com thresholds elevados: 93,2 µs e 82 faults em 50k steps.
- **Evidência:** `raw/alloc-hash-on.txt` vs `raw/alloc-hash-off.txt`;
  `raw/iso-hash-full-*.txt` (minflt_delta_steps=2.568.723) vs
  `raw/iso-hash-tuned-*.txt` (minflt=81).
- **Componentes:** `state_hash`, `state_body`, `hash_state_body`,
  `Vdp::snapshot_bytes`.
- **Condições:** qualquer step com hash; independe de plataforma, mas o
  tamanho exato do efeito depende do alocador (glibc; musl/macOS variam).
- **Impacto atual:** metade do tempo do step; bimodalidade de 2× em benchmarks.
- **Impacto futuro:** com o hash removido o churn desaparece do step (0 allocs),
  mas volta em todo `snapshot`/`inspect`.
- **Probabilidade:** alta (medido e reproduzido) · **Severidade:** alta ·
  **Confiança:** alta · **Prioridade:** **P0** (é o ganho mais barato do
  relatório) · **Fase:** M8.
- **Alternativas:** buffer `scratch` persistente no backend (reuso por
  `clear()+extend` ou array fixo) (B); hashear por streaming com digest
  incremental para escrever direto no `Sha256` (evita 1 buffer); `SmallVec`/
  arena; não fazer nada (J).
- **Recomendação:** B + escrita direta no hasher. Ganho esperado ≥2,25× no pior
  caso atual (medido via proxy) e ~0 allocações por step depois de F-01.
- **Custo/risco:** baixo (local, sem mudança de formato); nenhum risco de
  determinismo se o buffer nunca entrar em snapshot/hash.

### F-03 — `inspect()` paga verificação e cópia de RAM; observação custa 25× o necessário
- **Descrição:** `inspect` calcula o hash completo (197 KB) e copia 64 KiB de
  RAM, além de ~30 `String` de registradores: **96 µs / 60 allocs / 529 KB** por
  chamada; sem hash: 3,85 µs / 56 allocs / 68,7 KB.
- **Evidência:** `backend_inspect_median_seconds` nas seções B/C/E de
  `results-tables.md`; `raw/alloc-hash-{on,off}.txt`.
- **Componentes:** `MegadriveBackend::inspect`, `chronofork-debug` (122 chamadas
  de `.inspect()` no workspace), `chronofork-service::inspect` (224 µs),
  `chronofork-gui` (views por frame).
- **Impacto atual:** debugger/step-a-step, GUI e `session.inspect` pagam
  verificação em cada observação; a 60 fps, 96 µs/frame = 0,6% (aceitável),
  mas em loops de breakpoint multiplica.
- **Severidade:** média · **Confiança:** alta · **Prioridade:** **P1** ·
  **Fase:** M8.
- **Alternativas:** `inspect` sem hash + `state_hash()` separado; janela de RAM
  configurável; retornar hash só quando pedido. Não fazer nada (J).
- **Recomendação:** separar: `inspect` = registradores + janela de memória
  explicitamente pedida; hash por chamada separada.

### F-04 — Checkpoint e fork materializam e copiam o estado várias vezes
- **Descrição:** `snapshot()` = corpo + hash + envelope (7 allocs/657 KB);
  `Session::snapshot` retém mais um payload de 197 KB (+90 µs); envelope
  serializado custa outros 86–90 µs (SHA-256 de novo); `fork_current` produz
  **3 cópias** do mesmo estado e custa 666 µs / 822 KiB, sendo o snapshot do pai
  já existente. `fork` via serviço: 796 µs.
- **Evidência:** §4; `session_snapshot*`, `session_fork_current*`,
  `rss_delta_per_*` nos logs (`raw/iso-hash-full-*.txt`).
- **Componentes:** `chronofork-sessions` (`session.rs`, `manager.rs`),
  `chronofork-service::fork`.
- **Impacto atual:** experimentos com muitos forks/checkpoints pagam ~600 µs e
  ~0,8 MiB por fork; 100 forks = 82 MB e 67 ms (aceitável, mas 2× evitável).
- **Severidade:** média · **Confiança:** alta · **Prioridade:** **P1** ·
  **Fase:** M8/FU.
- **Alternativas:** deduplicação por `state_hash` (H); snapshot CoW/persistente
  (G); fork a partir do snapshot existente sem re-snapshot (evidente no código:
  `initial_state = backend.snapshot()` após `restore`); não fazer nada (J).
- **Recomendação:** começar pelo fork (remover o snapshot redundante e o clone
  adotado; `Arc<BackendSnapshot>`); depois dedup/CoW se experimentos crescerem.

### F-05 — Renderização por frame é obrigatória mesmo sem consumidor de mídia
- **Descrição:** `capture_frame` roda em todo `frame_started`, clona o VDP
  (1,23 µs, irrelevante) e renderiza (1,267 ms). Em headless sem consumidor,
  isso é **42% do custo pós-hash** (49,4 ms de 118,6 ms em 400k steps).
- **Evidência:** `raw/frames-nohash-*.txt` (`frame_step_*` 1,27 ms vs
  `nonframe_step_*` 97 ns, 39 frames/400k steps).
- **Componentes:** `backend.rs:723-731`, `Vdp::finish_frame`, `MediaSource`.
- **Impacto atual:** mascarado pelo hash (0,6% do step), mas vira o maior custo
  não-emulação assim que F-01 for corrigido; headless e CI pagam por frames que
  ninguém lê.
- **Impacto futuro:** com áudio e mais camadas de VDP, cresce.
- **Severidade:** média · **Confiança:** alta · **Prioridade:** **P1** (fazer
  junto de F-01, senão o ganho é reduzido de 5,55 M para 3,37 M steps/s) ·
  **Fase:** M8 8.3/8.5.
- **Alternativas:** render sob demanda no `take_video_frame` (lazy, com o VDP
  congelado no frame_started); flag `capture_video` no factory; fila de 1 frame
  mantida mas render só se consumidor registrado; não fazer nada (J).
- **Recomendação:** render lazy/opt-in preservando determinismo do frame
  (frame é saída não-canônica; o estado no boundary já está definido).

### F-06 — O contrato de step força o consumidor a pagar verificação que ele descarta
- **Descrição:** `AppService::run` descarta `outcome.state_hash`
  (só usa `halted`, [`service.rs:475`](../../../../crates/chronofork-service/src/service.rs));
  loops de CLI/sessão idem. Mesmo assim o hash é calculado. `run_until/
  run_cycles/run_frame` não existem; a única granularidade é 1 instrução com
  hash.
- **Evidência:** o código citado; `service_run_1000` = 209,5 ms vs 0,3 ms
  esperado com hash 1×/requisição.
- **Componentes:** `backend-api::Backend`, `sessions`, `service`, `debug`,
  `gui`, `cli`, `script`.
- **Severidade:** alta · **Confiança:** alta · **Prioridade:** **P0** (é a
  forma de F-01 chegar aos consumidores) · **Fase:** M8.
- **Alternativas:** `StepOutcome.state_hash: Option<StateHash>`; método
  `step_with(policy)`; `Backend::advance(cycles|frame)` com hash opcional;
  manter `step()` como está e adicionar `run_*` no nível de sessão.
  Não fazer nada (J).
- **Recomendação:** política de verificação na **sessão/serviço**
  (`Never | Every(n) | OnDemand | AtCheckpoint`), `Backend::step` sem hash e
  `Backend::state_hash()` explícito. Compatível com breakpoints (que já usam
  eventos do barramento, não hash) e com snapshots (que carregam o hash do
  corpo). Exige ADR e revisão dos 31 pontos que consomem `StepOutcome`.

### F-07 — `session.run` prende o lock global do manager durante todo o loop
- **Descrição:** `run` adquire o `Mutex<SessionManager>` e só o solta ao final
  ([`service.rs:461-491`](../../../../crates/chronofork-service/src/service.rs),
  [`service.rs:695`](../../../../crates/chronofork-service/src/service.rs)).
  Com 100.000 steps por requisição a ~5,3 k steps/s, uma requisição segura o
  lock global por **~19 s**; outras sessões (e o GUI) ficam bloqueadas.
- **Evidência:** código; `service_run_1000` 209,5 ms ⇒ ~19 s extrapolado para
  100k com o hash atual.
- **Componentes:** `chronofork-service`.
- **Severidade:** média (concorrência é da área 04; registro aqui pelo custo)
  · **Confiança:** alta · **Prioridade:** **P1** · **Fase:** M8/FU.
- **Alternativas:** lock por sessão (já existe lease), chunking com liberação
  do lock, `run` assíncrono; reduzir `max_steps_per_request`; não fazer nada (J).
- **Recomendação:** depois de F-01/F-06, o loop de 100k steps cai para ~20 ms e
  o problema perde urgência; até lá, limitar/chunkar.

### F-08 — Clone da ROM por máquina é barato hoje, mas escala com o cartucho
- **Descrição:** `create_machine` clona o `Cartridge` ([`backend.rs:302-303`](../../../../crates/chronofork-megadrive/src/backend.rs)).
  Medido com ROM sintética de 1 KiB: 1,03 µs, footprint 132 KB. **[hipótese]**
  ROM de 4 MiB ⇒ +4 MiB de RSS por sessão e ~0,3–0,5 ms de `memcpy` por máquina;
  100 sessões = 400 MiB.
- **Severidade:** baixa hoje · **Confiança:** média (aritmética; não medido com
  ROM real) · **Prioridade:** **P3** (antes de 8.6/8.7, quando o mapper/SRAM
  reais entrarem) · **Fase:** M8 8.6.
- **Alternativas:** `Arc<[u8]>`/`Arc<Cartridge>` compartilhado e imutável;
  mmap; não fazer nada (J). Ver AGENTS.md: "configuração imutável pode ser
  compartilhada via `Arc`".

### F-09 — Tracing seletivo custa +7,6% da execução pura; irrelevante sob o hash atual
- **Descrição:** trace de 65.536 registros: 186,5 vs 173,2 ns/step (+13,4 ns).
  Memória do ring não medida; 50.000 registros retidos após 50k steps.
- **Evidência:** `raw/iso-nohash-tuned-full-*.txt`.
- **Severidade:** baixa · **Confiança:** alta · **Prioridade:** **P3** ·
  **Fase:** M8.
- **Recomendação:** nenhuma ação agora; quando F-01 sair, o orçamento de trace
  é ~0,13 ms/frame (0,8% de 16,7 ms), então tracing seletivo por região/
  breakpoint é viável. Hooks de debug desligados não foram isolados (custo
  contido nos 173 ns).

### F-10 — Metodologia: o benchmark oficial esconde bimodalidade e não mede memória
- **Descrição:** `bench_step` reporta min/mediana/max de 5 trials de 10k steps,
  mas o fenômeno real é bimodal (1,03 s vs 2,14 s por causa do alocador); não há
  coluna de alocações, memória, frame ou serviço; `--release` é obrigatório mas
  não verificado pelo próprio binário além de `debug_assertions`.
- **Evidência:** §4 e `raw/baseline-*.txt`.
- **Severidade:** baixa · **Confiança:** alta · **Prioridade:** **P2** ·
  **Fase:** M8/FU.
- **Recomendação:** transformar este harness em `review_*`→`bench_*` versionado
  (sem `unsafe`: contador de alocações pode ir para um crate de teste com
  `unsafe` isolado ou usar contadores do alocador do sistema), com colunas de
  p50/p95, allocs/step, RSS/sessão/checkpoint/fork, frames/s e um gate de
  regressão no CI (p50 com tolerância).

---

## 6. Alternativas avaliadas (sem recomendação sem gargalo medido)

Legenda de ganho: **medido** = experimento deste relatório; **estimado** =
aritmética sobre medições. Complexidade/risco: baixa/média/alta.

| # | Alternativa | Ganho esperado | Complexidade | Risco p/ determinismo | Compat. snapshots/TAS | Breakpoints/watchpoints | 1ª divergência |
|---|---|---|---|---|---|---|---|
| A | Separar execução e verificação (hash sob demanda) | **1.050×** no step (5,27 k → 5,55 M/s, medido) | média | baixo (hash é derivado) | total (formato e hash do corpo intactos) | inalterados (usam eventos do barramento) | exige E para localizar |
| B | Buffer/scratch persistente (sem alocar por step) | **2,25×** (209,8→93,2 µs, medido por proxy); depois de A, 0 allocs/step | baixa | nulo (buffer não é estado) | nenhum impacto | nenhum | n/a |
| C | Granularidade configurável (`Never/Every(n)/Frame/OnDemand`) | = A (o hash deixa de ser por step) | baixa | baixo (política explícita) | requer registrar a política no artefato de experimento | inalterados | depende de E |
| D | Hash incremental / dirty pages (Merkle de páginas) | estimado 6–9× no **hash** (197 KB→~8–16 KB/step), ainda ~50× a emulação | alta | **alto** (write perdido = falso negativo) | formato do hash muda (versão) | nenhum | viável por página |
| E | Checkpoints periódicos + replay determinístico | localiza 1ª divergência com custo O(K + replay); checkpoint/frame = 183 µs (1,1% de 16,7 ms, medido) | média | baixo (replay é determinístico por contrato) | precisa de log de inputs (TAS já existe) | bons pontos de parada | **é o mecanismo** |
| F | `run_until/run_cycles/run_frame` na API | amortiza hash por chamada: `session.run` 1000 steps 209,5 ms → ~0,27 ms (medido por ablação) | média | baixo | mantém semântica se parar em boundary | exige sub-passos internos sem hash | com E |
| G | Snapshots copy-on-write / estruturas persistentes | checkpoint 183→~94 µs; fork 666→~300 µs (estimado) | alta | médio (aliasing) | formato pode ficar igual | nenhum | n/a |
| H | Deduplicação de checkpoints por `state_hash` | fork 822→~400 KiB e −~200 µs (estimado); evita 3ª cópia | baixa/média | baixo | IDs de snapshot precisam ser estáveis | nenhum | acelera comparações |
| I | `Arc<[u8]>` para ROM | ROM 4 MiB: −4 MiB/sessão, −~0,4 ms/create (hipótese) | baixa | nulo (imutável) | nenhum | nenhum | n/a |
| J | Não fazer nada | 0,47–1,05 fps; CI e demos dominados por hash; meta interativa inatingível | zero | zero | zero | zero | já funciona (comparação por hash) |

Ordem sugerida (por ganho/risco, não por preferência estética):
**B → F-05 (render opt-in) → A/C/F (contrato) → F-03 (inspect) → F-04/H (fork/dedup)
→ E (replay p/ divergência) → D/G/I (só se medições futuras exigirem).**

---

## 7. Metas realistas e o que é necessário agora vs depois

Premissas medidas: 10.256 steps/frame NTSC (896.040 master cycles, 89,3
master cycles/step); frame PAL 1.070.460 master cycles ⇒ ~11.993 steps.

| Meta | Orçamento | Hoje | Depois de B+A+F-05 | Folga |
|---|---|---|---|---|
| Interativo 60 fps NTSC | 16,67 ms/frame (1,63 µs/step) | 2,15 s/frame (**0,47 fps**) | 1,77 ms exec + 1,27 ms render + 0,09 ms hash/frame ≈ **3,13 ms** (319 fps) | **5,3×** |
| Interativo 50 fps PAL | 20 ms/frame | ~2,5 s/frame | ≈ 2,08 + 1,27 + 0,09 = **3,44 ms** (291 fps) | **5,8×** |
| Headless sem limite | o máximo possível | 5,27 k steps/s | **5,55 M steps/s** (541 fps equivalentes; 3,37 M/s ≈ 329 fps se renderizar todo frame) | ~1.050× |
| Tracing seletivo | ≤1 ms/frame | +7,6% da execução pura = 0,13 ms/frame | idem | 7,7× |
| Múltiplos experimentos | 100 forks < 1 s / < 100 MB | 666 µs e 822 KiB/fork ⇒ 67 ms / 82 MB | ~300 µs e ~400 KiB ⇒ 30 ms / 40 MB | 2× |

Observação: o render (1,27 ms) passa a ser **41% do frame** depois do hash sair;
por isso F-05 é pré-requisito para não trocar um gargalo por outro. A 60 fps o
orçamento restante (13,5 ms) cobre áudio/Z80/timers futuros com folga, mas o
render é o próximo alvo natural (1,27 ms para 320×224 = 17,7 ns/pixel).

**Necessário agora (bloqueia a meta de M8 e o custo de CI):**
1. F-02 — buffers persistentes/streaming no hash (baixo risco, 2,25× medido).
2. F-01/F-06 — ADR + política de verificação fora do step e `state_hash()`
   explícito; adaptar debugger/TAS/trace/testes.
3. F-05 — render opt-in/lazy (senão o ganho de F-01 cai 39%).
4. F-03 — `inspect` sem hash (barato, destrava debugger/GUI).
5. F-04 — remover o snapshot redundante do fork (barato).

**Depois (não bloqueia, ganho incremental):** E (replay para 1ª divergência),
D (dirty pages, só se igualdade por step for requisito de produto), G (CoW),
H (dedup), I (Arc de ROM antes de 8.6/8.7), F-07 (lock por sessão/chunking),
F-10 (bench versionado com gate de regressão no CI).

---

## 8. Limitações, o que não foi verificado e perguntas

**Não verificado / limites de ambiente**
- Sem `perf`, `valgrind`/`callgrind` ou `perf_event_open`: a atribuição é por
  ablação e contadores do kernel (`minflt`, contador de alocações), não por
  símbolo. Ordem de grandeza confiável; frações abaixo de ~2% não resolvidas.
- Host compartilhado (load 2,9–4,0, outros revisores compilando/testando):
  spread de até 2× em trials curtos; todas as conclusões usam medianas de 3
  execuções e efeitos ≥7%.
- Nenhuma ROM comercial foi executada (proibido copiar bytes): o efeito de ROM
  real em F-08 é hipótese aritmética.
- Não medi: custo dos hooks de debug desligados isoladamente, memória do ring
  de trace, áudio/Z80 (não implementados), concorrência real com múltiplas
  sessões simultâneas (área 04), pacing do GUI (`HostClock`), `chronofork
  serve`/HTTP (só `AppService::dispatch`).
- O patch de instrumentação e os harnesses vivem só na cópia isolada
  `/tmp/perf-iso`; nada foi alterado no repositório. Para reproduzir, ver
  [`../benchmarks/README.md`](../benchmarks/README.md).

**Perguntas para o capitão / critic**
1. A igualdade **por step** (`StepOutcome.state_hash`) é requisito de produto
   (artefatos de experimento, TAS, trace) ou bastam checkpoints + replay? Isso
   decide entre A/C/F (recomendado) e D (incremental).
2. Quem é o dono do ADR que muda `Backend::step`/`StepOutcome`? A área 01
   (arquitetura) deve ser consultada antes de F-01/F-06.
3. O plano de M8 8.4–8.8 já prevê `run_until/run_frame`? Se sim, F-06 deve ser
   tratado lá, não como mudança isolada.
4. Para a área 04: o lock global do manager em `session.run` (F-07) deve ser
   reportado como achado de concorrência; o custo aqui é apenas a evidência.
5. Para a área 05 (determinismo): o replay de E pressupõe log de input por step
   (TAS); confirmar que o formato cobre isso antes de recomendar E.
