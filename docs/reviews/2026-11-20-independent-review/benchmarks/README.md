# Benchmark harness — área 03 (desempenho, hashing e modelo de execução)

Evidência executada para [`areas/03-performance.md`](../areas/03-performance.md).
Tudo aqui foi produzido **somente** na worktree
`review-2026-11-20-architecture` (baseline `b8d63a1`) e em uma cópia isolada
descartável dela; nenhum arquivo versionado do repositório foi alterado.

## Ambiente

| Item | Valor |
|---|---|
| Host | WSL2 Ubuntu em ext4, kernel `6.18.58.1-microsoft-standard-WSL2` |
| CPU | AMD Ryzen 9 7950X3D (VM: 4 núcleos físicos × SMT = 8 vCPU), `sha_ni` e AVX-512 presentes |
| RAM | 30 GiB (`MemTotal: 32095892 kB`) |
| Rust | `cargo 1.100.0 (3acc79c0c 2026-11-05)`, `rustc 1.100.0 (0ef1bc386 2026-11-09)`, edition 2024 |
| Build | `--release --locked`, sem `debug-assertions`; `CARGO_HOME=/tmp/t7-cargo` |
| Baseline | `b8d63a17349e672ba6945eaba3379cec38cf9d1b` (`origin/main`), worktree `agent/review-2026-11-20-architecture` |
| Carga do host | load average 2.9–4.0 durante as medições (outros revisores rodando `cargo test`) |
| Pinagem | `taskset -c 5/6/7` por execução; nenhuma medição usa `perf` |

Ferramentas de perfil indisponíveis: `perf` não está instalado
(`/usr/lib/linux-tools/6.18.54.1-microsoft-standard-WSL2/` vazio, sem
`/usr/bin/perf`) e `perf_event_paranoid=2` bloquearia contadores de hardware;
`valgrind`/`callgrind` também ausentes. A atribuição de tempo foi feita por
**ablação controlada** (cópia isolada com o hash desligável) + **contador de
alocações** + **minor faults** do kernel, não por amostragem de símbolos.

## Reprodução em um comando

`scripts/run_benchmarks.sh` reconstrói a cópia isolada, aplica o patch, compila os
três harnesses e regenera os logs em `OUT` (padrão `/tmp/perf-runs`). Requer a
crate scratch `/tmp/perf-alloc` para o passo de alocações (ver §5). O script não
toca em nenhum arquivo versionado.

## Workload

O mesmo do `crates/chronofork-megadrive/examples/bench_step.rs`:
`interrupt_rom(3)`, NTSC, 3.420 master cycles/linha, um step = uma instrução
68000 + avanço do VDP + scheduler + (no baseline) hash canônico completo.
Medido: ~89,3 master cycles por step e ~10.256 steps por frame
(896.040 master cycles/frame), 39 frames em 400.000 steps.

## Como foi medido (receita reproduzível)

1. **Baseline intacto** (`raw/baseline-*.txt`), 3 execuções do binário da worktree:

   ```sh
   CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=/tmp/chronofork-review-target \
     cargo build -p chronofork-megadrive --example bench_step --release --locked
   for i in 1 2 3; do
     taskset -c 6 /tmp/chronofork-review-target/release/examples/bench_step \
       > raw/baseline-$i.txt
   done
   ```

2. **Cópia isolada descartável** (`git archive HEAD` → `/tmp/perf-iso`), com o
   patch de instrumentação [`patches/review-instrumentation.patch`](patches/review-instrumentation.patch):
   - `REVIEW_HASH=0` faz `MegadriveBackend::state_hash()` devolver zeros, o que
     isola a **execução pura** (o hash não é estado; nenhum outro caminho muda);
   - métodos `review_*` medem serialização do corpo, hash isolado, clone do
     VDP/cartucho, `capture_frame` e footprint.
   O patch é de 74 linhas, só em `crates/chronofork-megadrive/src/backend.rs`,
   e **não** foi aplicado em nenhum arquivo versionado.

3. **Harness** [`src/review_perf.rs`](src/review_perf.rs) copiado para
   `crates/chronofork-megadrive/examples/review_perf.rs` na cópia isolada:

   ```sh
   cd /tmp/perf-iso
   CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=/tmp/perf-iso-target \
     cargo build -p chronofork-megadrive --example review_perf --release --locked
   for i in 1 2 3; do taskset -c 6 \
     /tmp/perf-iso-target/release/examples/review_perf full > raw/iso-hash-full-$i.txt; done
   # execução pura
   for i in 1 2 3; do REVIEW_HASH=0 taskset -c 6 \
     /tmp/perf-iso-target/release/examples/review_perf steps > raw/iso-nohash-steps-$i.txt; done
   # mesmos passos com o allocator glibc estabilizado (ver abaixo)
   for i in 1 2 3; do MALLOC_MMAP_THRESHOLD_=1048576 MALLOC_TRIM_THRESHOLD_=1048576 \
     taskset -c 6 /tmp/perf-iso-target/release/examples/review_perf steps > raw/iso-hash-tuned-$i.txt; done
   ```

4. **Experimento do allocator.** `MALLOC_MMAP_THRESHOLD_=1048576` e
   `MALLOC_TRIM_THRESHOLD_=1048576` (variáveis do glibc) impedem que as
   alocações transitórias de ~197 KiB virem `mmap`/`brk` novo a cada step. É um
   **proxy controlado** para um buffer persistente: mesmo código, mesmo hash,
   só muda o comportamento do alocador. O harness lê `minflt` de
   `/proc/self/stat` para provar a causa.

5. **Contador de alocações** [`src/review_alloc.rs`](src/review_alloc.rs):
   crate scratch fora do workspace (`/tmp/perf-alloc`, `[workspace]` próprio,
   `unsafe_code = "allow"` só para o `GlobalAlloc`), com `path` deps para a
   cópia isolada. O workspace proíbe `unsafe`, por isso não é um `example` do
   repositório:

   ```sh
   cd /tmp/perf-alloc && CARGO_HOME=/tmp/t7-cargo \
     CARGO_TARGET_DIR=/tmp/perf-alloc-target cargo build --release
   taskset -c 6 /tmp/perf-alloc-target/release/review-alloc   # e REVIEW_HASH=0
   ```

6. **Serviço** [`src/review_service.rs`](src/review_service.rs) em
   `crates/chronofork-cli/examples/` na cópia isolada, porque só a CLI compõe o
   `AppService` com o backend Mega Drive:

   ```sh
   CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=/tmp/perf-iso-target \
     cargo build -p chronofork-cli --example review_service --release --locked
   for i in 1 2 3; do taskset -c 6 \
     /tmp/perf-iso-target/release/examples/review_service > raw/service-hash-on-$i.txt; done
   ```

7. **Tabelas**: `python3 scripts/summarize.py raw > results-tables.md` (cada
   execução listada + mediana).

## Arquivos

| Caminho | Conteúdo |
|---|---|
| `raw/baseline-{1,2,3}.txt` | `bench_step` intacto, 3 execuções (seção A) |
| `raw/iso-hash-full-{1,2,3}.txt` | fases completas, hash por step, allocator padrão (B) |
| `raw/iso-hash-full-4-sizes.txt` | tamanhos de `state_body`, VDP, cartucho |
| `raw/iso-hash-tuned-{1,2,3}.txt` | steps, hash por step, thresholds glibc elevados (C) |
| `raw/iso-tuned-full-{1,2}.txt`, `raw/iso-tuned-trace-1.txt` | fases completas com thresholds elevados (C) |
| `raw/iso-nohash-steps-{1,2,3}.txt` | execução pura, allocator padrão (D) |
| `raw/iso-nohash-tuned-full-{1,2,3}.txt` | execução pura + trace + fases, thresholds elevados (E) |
| `raw/frames-nohash-{1,2}.txt` | 400k steps, 39 frames, custo do frame (F) |
| `raw/service-hash-on-{1,2,3}.txt` | `AppService::dispatch` (G) |
| `raw/alloc-hash-{on,off}.txt` | alocações por step/snapshot/inspect |
| `raw/iso-baseline-sha.txt` | SHA-256 dos arquivos da cópia isolada |
| `src/review_perf.rs`, `src/review_alloc.rs`, `src/review_service.rs` | harnesses |
| `patches/review-instrumentation.patch` | patch mínimo da cópia isolada |
| `scripts/summarize.py` | gera `results-tables.md` a partir de `raw/` |
| `results-tables.md` | todas as execuções + mediana, por seção |

## Metodologia e limitações

- Cada número da seção de fases é a **mediana** de 50–2.000 amostras dentro de
  uma execução; cada seção é repetida em 3 execuções independentes e a tabela
  mostra a mediana entre execuções.
- O host é compartilhado; o spread min/max chega a 2× em trials de 10k steps.
  Reportamos **p50 e a distribuição completa**, nunca apenas a média.
- Sem `perf`/`perf_event_open`, a fração por componente vem de ablação:
  `hash_only` (hash sobre estado fixo), `body_only` (serialização),
  `REVIEW_HASH=0` (execução) e `REVIEW_HASH` + thresholds do glibc (execução +
  hashing sem churn de páginas). Os componentes somam ~100% do step medido,
  o que fecha o balanço dentro do erro de medição (ver área 03 §3).
- ROM comercial (`/home/stealthc/projects/ChronoFork/.research/roms/`) **não**
  foi usada: bytes nunca entram em relatório e o backend só carrega ROM sintética
  neste slice. O efeito de ROM real é estimado por aritmética e marcado como
  hipótese.
