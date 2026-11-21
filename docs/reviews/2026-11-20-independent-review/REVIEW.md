# ChronoFork — Revisão independente de arquitetura, engenharia, desempenho e planejamento

**Data:** 2026-11-20 (America/Fortaleza)
**Baseline:** `origin/main` em `b8d63a17349e672ba6945eaba3379cec38cf9d1b`
**Worktree:** `.agents/worktrees/review-2026-11-20-architecture`
**Branch:** `agent/review-2026-11-20-architecture`
**Equipe:** 8 agentes (7 especialistas independentes + 1 crítico adversarial), coordenados pelo capitão.
**Status:** relatório final da revisão. Nenhum código de produção, doc versionada, dependência ou plano de fases foi alterado. Nada foi commitado, publicado ou integrado.

> **Idioma e convenção.** Este relatório é dirigido ao proprietário em português brasileiro. Se alguma recomendação for aprovada, o registro durável (ADR, arquitetura, requisitos) deve ser escrito em inglês, conforme `AGENTS.md` e `docs/README.md`.
>
> **Evidência detalhada.** Os sete relatórios de área, a crítica final e os dados brutos estão preservados em `docs/reviews/2026-11-20-independent-review/` nesta worktree (`areas/`, `benchmarks/`, `critique-final.md`, `SYNTHESIS-DRAFT.md`, `baseline.md`). O relatório abaixo é a consolidação; os anexos contêm o `path:line`, comandos e logs que sustentam cada afirmação.
>
> **Classificação.** `[F]` fato observado (código/execução), `[E]` experimento desta revisão, `[H]` hipótese, `[I]` inferência, `[O]` opinião técnica. Prioridade P0–P3 é prioridade de implementação; severidade é impacto teórico. As duas são diferentes.

---

## A. Resumo executivo

### A.1 O que o ChronoFork é hoje

O ChronoFork tem uma base de engenharia **acima da média** para a fase: 12 crates com direção de dependência verificada por compilação, 66,4 mil linhas de `src` e 48,9 mil de testes, 1.289 testes verdes, gate local completo, demos byte-determinísticos, snapshots versionados e validados num único ponto, sessões/fork/lineage, TAS/replay, debugger, tracing, DSL determinístico com limites e sandbox, serviço com paridade canônica HTTP/MCP e uma GUI que não possui estado canônico. `[F]` (A1 §2/§7; A5 §1; A6 §0)

O que **não** existe ainda é o produto que o projeto promete: um emulador de Mega Drive utilizável e uma plataforma em que um agente consiga conduzir engenharia reversa de ponta a ponta. Nenhuma das duas ROMs comerciais curadas progride no bootstrap no baseline; o MCP não expõe debug/trace/script/TAS; o serviço serializa todas as sessões; e o custo de verificação por passo impede tempo real. `[F/E]` (A2; A6; A4; A3)

### A.2 Veredito

**A arquitetura é viável e não deve ser reescrita.** O caminho responsável é uma **consolidação curta e cirúrgica** antes de novas fases: corrigir o mapa de bus/boot, rejeitar o DMA incorreto, separar execução de verificação, destravar a API agêntica e a concorrência por sessão, e tornar a evidência reproduzível. O maior risco não é “o emulador está incompleto” — isso é esperado e documentado com honestidade —; é **o roadmap declarar progresso sem software executando, sem desempenho medido e sem superfície usável por agente**. `[O]` (síntese + crítica final)

### A.3 Os sete achados que mudam decisão

1. **P0 — nenhuma ROM comercial boota.** Chakan cai no handler fatal no primeiro passo; Kid Chameleon aborta o primeiro passo ao preparar a moldura de exceção. Causas concretas: o 68000 não trunca o endereço para 24 bits, o espelho de RAM `0xE00000..0xFFFFFF` não existe, os registradores de controle `0xA10008..0xA1000F` não estão mapeados e o plano 8.5 ainda trata a porta 3 como erro. `[F]` (A2-F1/F2/F13/F14; A7 §4)
2. **P0 — DMA `memory→VRAM` lê a própria VRAM e não erra.** Viola a regra 8 do `AGENTS.md` (“nunca reinterpretar silenciosamente”). Correção mínima: rejeitar com erro tipado até existir o lado 68000. `[F]` (A2-F3)
3. **P0/P1 — o custo dominante do passo é a verificação, não a emulação.** Step mediano 209,8 µs; emulação pura ~0,17 µs; SHA-256 puro 84,7–86,9 µs (~38–41% do passo); corpo/cópias ~4 µs; churn de alocações/page faults ~116,6 µs (bimodal). O caminho completo de observabilidade é ~99% do passo mediano, mas **as parcelas não são aditivas** (medianas de harnesses diferentes; o churn é uma diferença experimental, não um perfil exclusivo). `[E]` (A3; A1-01; crítica C1)
4. **P1 — o produto agêntico não existe na prática.** O catálogo tem 16 operações; `debug.run`, `trace.capture`, `script.run` e `tas.record` não estão no MCP. Observar 10 mil passos via MCP custa ~20 mil chamadas, ~20,7 MB, ~5,2 milhões de tokens (extrapolação bytes/4) e ~103 s — contra ~1,04 s de emulação. Não há cancelamento, deadline nem cotas. `[E]` (A6-F1/F2/F3/F7)
5. **P1 — o serviço serializa todas as sessões.** Um `Mutex<SessionManager>` global mantém a execução sem paralelismo; a eficiência medida por throughput é 1,00 / 0,446 / 0,227 / 0,114 em 1/2/4/8 threads (não 1/0,13/0,07/0,03; essa era a leitura errada da síntese). O backend é paralelizável (controle `experiment.run`: 5,97× em 8 threads); o gargalo é o desenho do serviço. `[E]` (A4-F01; crítica C2)
6. **P1 — a evidência de qualidade não é reproduzível a partir do clone.** O diferencial exige oráculo out-of-tree e aborta sem ele; cobre 550 instruções/186 opcodes e **zero exceções, `RTE` ou frames de pilha no corpus diferencial** (não “zero cobertura total”: há testes unitários de exceções). Dois testes pulam em silêncio sem ROMs. `[E]` (A5-Q01–Q05; crítica C3)
7. **P1/P2 — a documentação de status é uma fonte ativa de decisão errada.** 89 arquivos/38.165 linhas, 13 documentos repetindo status, seis versões contraditórias de “rendering”, 49 menções a `cb05836`. O status correto só sai de código/testes. `[F]` (A7 §2; A2-F11)

### A.4 O que fazer agora (recomendação)

1. **Hardening de boot e bus (agora, ~1 semana):** truncamento de 24 bits no bus, espelho de RAM, `0xA10008–0xA1000F` como registradores de controle (porta 3 incluída), open bus determinístico no que não é decodificado, rejeição tipada de DMA `memory→VRAM`, correção das frases técnicas erradas na documentação. Gates de boot específicos por fixture (G-BOOT-0/1), não “não crashou”.
2. **ADR do contrato de verificação (agora):** `step` deixa de devolver hash obrigatório; `state_hash()` explícito; buffers persistentes; `status()` barato; render na fronteira (não no estado futuro); política de verificação configurável. Preservar TAS/replay com um caminho verificado por passo.
3. **Serviço e API agêntica (curto prazo, após o ADR):** lock por sessão (não mutex global), cancelamento cooperativo, deadline, cotas, ciclo de vida do registry, persistência/exportação de sessão viva, e uma API mínima de 9 operações compostas com batching — não espelhar os ~30 verbos da CLI.
4. **Qualidade e evidência (paralelo):** commitar as ROMs sintéticas legadas + teste de fixture; cobrir exceções/`RTE`/frames no diferencial; controle de mutação no emulador; remover pulos silenciosos; CI em camadas; fonte única de status.
5. **Roadmap:** preservar o trabalho de 8.4; concluir Z80/bus (que também é pré-requisito de boot); decidir com o responsável por 8.4 se o áudio termina antes ou depois de 8.5 (timers/controles); só então 8.6a (compatibilidade), 8.6b (desempenho) e 8.6c (consolidação de produto). Adiar 8.7, 8.8, M9, TLS/multiusuário e async.

### A.5 O que **não** fazer

- Não reescrever a arquitetura Rust, os crates, o contrato opaco de snapshot, o modelo de forks ou a separação apresentação/serviço/backend. `[O]` (crítica final)
- Não adotar Tokio/async/atores como pré-requisito; o menor passo correto é lock/owner por sessão. `[O]`
- Não trocar SHA-256 por BLAKE3/xxh3 como “solução de performance”: continua serializando 197 KB por passo. `[O]`
- Não adotar COW/dedup/hash incremental agora sem evidência de uso; `Arc` no payload/initial_state e `reap()` resolvem o caso imediato. `[O]`
- Não embutir núcleo GPL/não comercial, ROM comercial ou firmware; Exodus continua apenas como oráculo externo. `[F]` (A2 §10; A5-Q01)
- Não declarar multiusuário, bind não-loopback, TLS ou MCP 2026-07-28 como prontos; a v1 é loopback/usuário único até decisão explícita. `[O]` (A6 §7)

### A.6 Respostas diretas ao critério de sucesso

| Pergunta | Resposta curta | Evidência |
|---|---|---|
| Está caminhando para ser útil? | A infraestrutura sim; o produto ainda não. Falta boot real e superfície agêntica. | A2; A6; A7 |
| A arquitetura permite desempenho adequado? | Sim, se a verificação sair do step e os buffers forem reutilizados. A emulação pura é rápida; o caminho de observabilidade é o teto. | A3; A1 |
| Os contratos preservam determinismo sem custo desnecessário? | Preservam determinismo, mas impõem custo desnecessário por passo. `StepOutcome.state_hash` é o principal acoplamento. | A1-01; A3 |
| A estratégia de emulação é sustentável? | O modelo por instrução é coerente e barato; não é cycle-exact. Para compatibilidade comercial, faltam bus/I-O, DMA real, raster e áudio. | A2 |
| É possível executar experimentos paralelos? | Pelo serviço, não; o mutex global serializa. Pelo backend, sim (5,97× em 8 threads). | A4 |
| Estamos reinventando componentes? | Em parte: JSON/HTTP/MCP/DSL/GUI são próprios. A justificativa de determinismo vale para o estado canônico e o DSL; para MCP/JSON externo, a decisão deve ser reavaliada, mas não é o gargalo atual. | A1; A6 |
| As interfaces são adequadas para agentes? | Não para RE interativa: faltam debug/trace/memória-escrita/batching/cancel. | A6 |
| Os testes demonstram correção? | Demonstram consistência interna e um diferencial útil, mas limitado; não demonstram “68000 completo” nem “VDP completo”. | A5 |
| As decisões continuarão adequadas ao crescer? | Não sem mudar hash/inspect/timebase e o contrato de inspeção para multi-CPU. | A1; A3 |
| O que resolver agora? | Boot/bus, DMA, contrato de verificação, serviço/API, evidência reproduzível, fonte única de status. | C |
| Caminho mais curto para valor real? | Hardening + uma ROM real bootando + fluxo KidEngine (checkpoint → forks → compare → memória/disasm/trace → relatório) antes de 8.7/8.8/M9. | E |

---

## B. Relatório técnico

### B.1 Desempenho, hashing e modelo de execução

**O que o contrato faz.** `Backend::step(&mut self, InputFrame) -> StepOutcome` devolve `state_hash` em toda chamada (`crates/chronofork-backend-api/src/traits.rs:12-33`). O step Mega Drive fecha com `state_hash()` (`crates/chronofork-megadrive/src/backend.rs:632-685`), que serializa CPU, 64 KiB de RAM, snapshot completo do VDP (VRAM/CRAM/VSRAM/regs/FIFO/DMA/timing), bancos do mapper, SRAM, TMSS e scheduler, e calcula SHA-256. `Vdp::snapshot_bytes` aloca e escreve o VDP inteiro por chamada (`vdp/mod.rs:1081+`); `state_body` aloca outro buffer; `hash_state_body` aloca um terceiro; `state_hash_for` hasheia. `[F]`

**Decomposição medida (release, mediana de 3 execuções).** Workload `interrupt_rom(3)`, NTSC, 10.256 passos/frame. `[E]` (A3 §3; A1 §4; crítica C1)

| Componente | Mediana | Observação |
|---|---:|---|
| Emulação CPU+VDP+scheduler | **0,17 µs** | `REVIEW_HASH=0`; 0 alocações/step |
| Corpo/cópias do estado | **~4,0 µs** | construção de `state_body` |
| SHA-256 puro | **84,7–86,9 µs** | 197.058 B; SHA-NI ~2,1–2,4 GB/s |
| Churn (alocações/page faults) | **~116,6 µs** (bimodal) | 51,4 minor faults/step; cai para 82/50k com thresholds elevados |
| Residual | **~2 µs** | fechamento da soma |
| **Passo mediano** | **209,8 µs** | 5.185–5.269 steps/s |

**Correção dos denominadores (aplicada da crítica final).**
- SHA-256 puro = **37,8–41,4% do passo** (84,7/224 na A2; 86,9/209,8 na A3). Dizer “hash é 99,9% do step” é **impreciso**.
- O valor 99,9% (na verdade 99,81%) é `91,0/(91,0+0,173)`, isto é, **verificação/(verificação+emulação)**, não o passo inteiro.
- O caminho completo (corpo + SHA + churn) é **~98,95%** de `(91,0+116,6)/209,8`. Não somar 91+116,6+9=216,6 (103,24% do passo): as parcelas vêm de harnesses diferentes e o churn é uma diferença experimental, não uma parcela disjunta. `[E/I]` (crítica C1)

**Fator dez corrigido.** 9 µs/step = **111 mil steps/s ≈ 10,8 fps** a 10.256 passos/frame; não 10–11 mil steps/s. Remover **apenas o SHA-256** do baseline deixa, por subtração ilustrativa, 209,8 − 86,9 = **122,9 µs** (~8,1 mil steps/s, ~0,79 fps), porque o churn continua. Com alocador estabilizado, 93,2 − 86,9 ≈ **6,3 µs** (~159 mil steps/s, ~15,5 fps), ainda longe de 60 fps nesse workload. `[E/I]` (crítica C1)

**Projeção de 319 fps.** 10.256 × 0,173 µs (emulação) + 1,267 ms (render) + 0,091 ms (um hash por frame) ≈ **3,132 ms/frame ≈ 319 fps**. Isso é **aritmética**, não um frame de jogo corrigido: o workload é sintético, o render é o atual (clone na fronteira), e não inclui Z80/áudio/scanline/contenda. O 0,84 fps “de ROM real” mede Chakan preso no handler, não um perfil representativo. Render opt-in **não é pré-requisito** para 60 fps nessa projeção: 1,267 ms é ~7,6% do orçamento de 16,67 ms. `[E/I/O]` (A3; A7; crítica C5)

**O que isso significa.** Existem **três intervenções distintas**, não uma:
1. Trocar o algoritmo de hash (ganho pequeno; não resolve a serialização).
2. Evitar serialização/cópias/alocações (buffer persistente, `status()` barato, sem corpo por passo).
3. Deixar de solicitar o caminho inteiro em passos que não precisam de verificação (hash opt-in, verificação em fronteiras, replay para localizar divergência).

O patch experimental `REVIEW_HASH=0` faz a terceira (retorna antes de construir o corpo); o ganho dele **não** pode ser atribuído à primeira. A recomendação correta ataca (1)+(2)+(3) juntos, com um caminho verificado separado para TAS/replay. `[E/I]` (A3; A1; crítica C1)

**Alternativas avaliadas.** Hash sob demanda; granularidade configurável; `run_until`/`run_cycles`/`run_frame`; hash incremental/dirty pages; estruturas persistentes; snapshots COW; dedup; checkpoints periódicos + replay. O hash incremental e o COW ficam para depois de evidência de uso; o ganho imediato vem de buffers + hash opt-in + batching. `[O]` (A3 §6)

**Conclusão.** A emulação pura é rápida o suficiente; o teto atual é o caminho de observabilidade. A meta realista não é “5 M steps/s universal” (passos têm custos diferentes e o host é compartilhado): é um orçamento por frame e uma regressão relativa controlada, medida em workload pinado, com hashes iguais nas fronteiras. `[E/O]` (A3; crítica C5)

### B.2 Engenharia de emuladores e correção do hardware

**Modelo de tempo.** O backend é **por instrução**, com uma timeline de master-clock por instrução: `deliver_pending_interrupt` → `cpu.step` → `cycles*7 + fifo_wait*7` → `scheduler.advance_master` → `vdp.advance_master` → `drain_fifo` → `take_dma_cycles` (valor descartado) → `capture_frame` na virada → `sync_vdp_status` → `state_hash`. É determinístico e barato de raciocinar, mas não observa o barramento **dentro** da instrução; contenda, latência de interrupção, espera em flags do VDP no meio da instrução e DMA real não são representáveis sem trocar o modelo. `[F/I]` (A2 §3)

**Boot comercial — redação precisa.** Os dois fixtures **não progridem no bootstrap**. Chakan: reset em `PC=0x0000333E`, primeira instrução `4AB9 00A10008 = TST.L $00A10008`; o bus não mapeia `0xA10008`; a exceção cai no handler fatal em `0x220`. Kid Chameleon: `SSP=0xFFFFF7FE`, `PC=0x000004B0`; o **primeiro passo solicitado** falha ao preparar a moldura de exceção, após **zero instruções completadas**. O endereço reportado (`0xFFFFF7FD`) é o **fim do intervalo validado**, não evidência de escrita parcial. O SSP é legítimo (24 bits), mas a integridade externa do dump não foi verificada. `[F]` (A2 §16; E4; crítica C3)

**Causas e correções.**
- **24 bits:** o 68000 tem 24 linhas de endereço; `0xFFFFF7FE & 0x00FFFFFF = 0x00FFF7FE` (RAM). A normalização deve ocorrer no **bus**, num único ponto, para `read8/16/32`, `write8/16/32`, `peek8`, `write_memory` e o endereço gravado na moldura de exceção. **Não** normalizar o valor do registro A7. ~10 linhas. `[F]` (A2-F13)
- **Espelho de RAM:** `0xE00000..0xFFFFFF` deve espelhar a work RAM; hoje só `0x00FF0000..0x01000000` está mapeado. `[F]` (A2-F1)
- **I/O:** `0xA10008/09`, `0A/0B`, `0C/0D`, `0E/0F` são registradores de **controle** das portas 1/2/3/EXT, legíveis/escritíveis, reset 0. `0xA1000C` é o controle da porta 3, exatamente o segundo endereço que o Chakan lê. Dados 3/EXT continuam erro tipado; controle 3/EXT é stub 0 legível/escritível. O plano 8.5 promete `0xA10008–0xA1000F` mas mantém “porta 3 e além” como erro — **corrigir o plano antes de executar**. `[F]` (A2-F2/F14; `docs/superpowers/plans/2026-11-17-8.5-timers-controllers.md:119`)
- **DMA:** `start_dma` aceita `MemoryToVram` e copia VRAM→VRAM silenciosamente. Rejeitar com erro tipado até implementar o lado 68000; depois contabilizar os ciclos de DMA no step (hoje `take_dma_cycles()` é descartado). `[F]` (A2-F3/F6)
- **Open bus:** para o que é de fato não decodificado, definir um valor determinístico (não uma exceção do guest); não usar open bus como desculpa para não mapear registradores documentados. `[F/I]` (A2-F2)

**Precisão necessária (compromissos).**

| Objetivo | Precisão necessária | O que falta |
|---|---|---|
| Rodar jogos comerciais | bus tolerante/aberto, RAM espelhada, 24 bits, controles/timers, Z80+PSG, DMA real, raster por scanline, contenda | S-01/S-02/S-07; 8.4/8.5/8.6a |
| Reproduzir lógica de gameplay | instrução + interrupções/controles corretos; áudio/vídeo podem ser aproximados | 8.5; controle completo |
| Investigar interação CPU–hardware | modelo por ciclos/eventos no barramento (DMA, VDP wait, arbitragem) | mudança de modelo, não micro-otimização |
| TAS determinístico | instrução determinística + replay; cycle-exact só para timing sub-instrução | já funciona no escopo; verificado por TAS |
| Divergências de timing | modelo por ciclos/eventos; caso contrário, documentar limite | 8.6a |
| Outros sistemas | contrato multi-CPU/clock domains/address spaces | M9; não implementar agora |

**Build vs integrate.** Manter o núcleo Rust próprio (CPU/VDP/sessões) — a arquitetura e o determinismo são o diferencial; não reescrever. Para **áudio**, `ymfm` (BSD-3) é candidato de baixo atrito, mas ainda **não** foi feito o audit de licença primária, pin de commit, build, precisão e serialização de estado; `Nuked-OPN2` (LGPL-2.1+) é opcional e exige ADR/análise de relink. Exodus (MIT) continua **apenas** como oráculo externo por processo; não embutir núcleo GPL/não comercial. Um adaptador temporário (ex.: `exodus-mcpd`) pode validar o fluxo de valor, mas não deve ser adotado como arquitetura. `[F/O]` (A2 §10/§11; crítica C7)

**Conclusão.** O que 8.2/8.3 entregaram é real e testado no escopo, mas **não é um emulador jogável**. O próximo obstáculo não é “mais uma instrução”: é bus/I-O + um gate de boot honesto. `[F/O]` (A2)

### B.3 Rust e arquitetura de sistemas

**Contratos.** `Backend` é pequeno e opaco (`descriptor`, `step`, `snapshot`, `restore`, `inspect`, `read_memory`, `write_memory`, `debug_access`, `media_mut`), com `Send` e sem estado mutável compartilhado. `BackendFactory` é `Send + Sync`. A direção de dependência é verificada por compilação; os 12 crates são justificados (cada aresta é um teste de compilação da direção). `[F]` (A1 §3/§7)

**Problemas de contrato.**
- `StepOutcome.state_hash` obrigatório acopla execução e verificação (A1-01; A3).
- `inspect()` recomputa o hash e copia 64 KiB (`backend.rs:883-977`); tracing e debugger chamam `inspect` **por passo** (`trace/src/capture.rs:256`, `debug/src/debugger.rs:440`) → ~2× custo. `[F/E]` (A1-02)
- `MachineInspection` é plano (`string → u64` + uma janela de memória); não acomoda Z80 em 8.4 nem multi-CPU/address spaces em M9. “pc” é string mágica em três crates; o serial do cartucho é empacotado em `u64`. `[F]` (A1-03)
- `fork_current` retém ~773,5 KiB por fork (4× o estado) e 0,8–1,36 ms; sem API de remoção e com `max_sessions=1024` isso chega a ~800 MiB. `[E]` (A1-05; A4-F04)
- Superfície pública de `megadrive`: 493 itens `pub`; `StateView` duplicado; `SessionError` mistura storage; 50 parsers manuais no CLI. `[F]` (A1-07/08/09/12/13/14)

**Multi-backend (M9).** O contrato atual é genérico o suficiente para um segundo sistema simples, mas **não** para múltiplas CPUs, espaços de endereçamento distintos, memória virtual, clock domains ou granularidades diferentes. As mudanças necessárias são conhecidas e pequenas: `StepOutcome` sem hash, `MachineStatus`/views tipadas, timeline explícita, `run_until`/`run_cycles`. Não implementar M9 agora; não congelar o contrato num formato que só o Mega Drive cabe. `[I/O]` (A1-03; A2 §8)

**Conclusão.** A arquitetura Rust é boa e não precisa de reescrita. As três mudanças antes de M9 são: hash/step, inspeção barata/tipificada, e timebase única. `[O]` (A1)

### B.4 Concorrência, paralelismo e isolamento

**Fatos.** `AppService` tem `Mutex<SessionManager>`, `Mutex<Registry>` e `Mutex<LeaseTable>` (`service.rs:114-122`). `session.run` segura o manager durante todo o loop (`:461-491`), até 100.000 passos (`limits.rs:37`). O registry não tem remoção; `max_sessions` é vitalício; sessão fechada retém o backend. `close` durante run vira `conflict`; não há `cancel`/`pause` no catálogo; desconexão do cliente não cancela; o cliente tipado tem read timeout de 5 s (`client.rs:20-22`), menor que o run máximo. `[F/E]` (A4 §2)

**Escala medida (release, 1/2/4/8 sessões).** Throughput agregado ~5.066/4.514/4.593/4.629 steps/s; eficiência **1,00 / 0,446 / 0,227 / 0,114** (corrigida da crítica C2; a síntese dizia 1/0,13/0,07/0,03). CPU/wall ~0,96–0,98, ou seja, serialização. Um run de 500 passos tem p95 de 117 ms → 1,5 s; 20.000 passos = 4,65–4,73 s; 100.000 ≈ 23 s de lock. `[E]` (A4 §4)

**Controle.** `experiment.run` usa um manager privado e não toca no lock global; 8 threads → 5,97× (51,0 exp/s vs 8,55 exp/s). Isso prova que o **backend é paralelizável** e que o gargalo é o desenho do serviço. `[E]` (A4 §4.4)

**Impacto.** Um agente rodando degrada todos os outros; `list`/`inspect`/`create` de outras sessões esperam; não há deadline/cancelamento; um run longo pode deixar *unknown outcome* (o cliente falha, o servidor aplica estado). `[E]` (A4-F01/F02/F06)

**Conclusão.** Para a tese “experimentos simultâneos de vários agentes”, o serviço atual não serve. O menor passo correto é **lock/owner por sessão** + cancelamento cooperativo + deadline + cotas + `reap()`; ator/worker por sessão é uma evolução, não o pré-requisito. Não adotar Tokio/async por padrão. `[O]` (A4 §6/§7; crítica C6)

### B.5 Determinismo, testes diferenciais e qualidade

**O que existe.** 1.289 testes em 108 alvos, gate local completo (`fmt`, `check`, `clippy -D warnings`, `rustdoc -D warnings`), testes de propriedade nos parsers, testes de concorrência no serviço, matriz de rejeição de snapshots, demos byte-determinísticos e um diferencial com Exodus que já encontrou defeitos reais (decodificação `MOVE.L (abs).L,Dn`, `ABCD`, tabelas de ciclos). `[F]` (A5 §1)

**O que o diferencial prova.** A cobertura medida é **550 instruções, 186 palavras de opcode, 145 executadas uma única vez**; **nenhuma exceção, `TRAP`, `RTE`, `LINK` ou frame de pilha é comparada ao oráculo**. Isso é cobertura diferencial limitada, **não** “zero cobertura total”: há testes unitários de exceções. A frase honesta é “o subconjunto documentado está coberto nesses 11 programas”, não “68000 completo”. `[E]` (A5-Q02; crítica C3)

**Reprodutibilidade.** O harness exige checkout out-of-tree pinado e binários C++ compilados; sem eles **aborta** (exit 2), sem caminho de skip. A verificação “fixture commitada == ROM gerada” fica dentro desse caminho inalcançável; não há teste Rust equivalente. 8/8 fixtures commitadas conferem; 3 ROMs legadas (`rom_alu`, `rom_mem_vdp`, `rom_masked`) não estão commitadas e seus marcadores gerados hoje divergem +2 dos documentados. `[E]` (A5-Q01)

**Controles negativos.** Os três controles corrompem a **projeção**; provam o comparador, não que uma regressão do emulador seja detectada. Falta um controle de mutação no emulador (ex.: +1 ciclo em `MOVE.W SR,<ea>`). `[E]` (A5-Q03)

**Pulos silenciosos.** Dois testes de cartucho fazem `eprintln!("skipping…"); return;` sem `.research/roms/`; o CI nunca tem os ROMs, então o número efetivo em CI é menor que 1.289 e não há guarda “zero testes = falha”. `[F]` (A5-Q05)

**Custo de teste.** `megadrive_demo_output_is_deterministic` roda o demo 2× em debug (167 s) comparando run-vs-run, sem golden; o stdout é idêntico entre debug e release (sha256 `fe254e5...`), e release leva 7,7 s. Fixar o golden converte 167 s em 8 s e ganha cobertura cross-profile. `[E]` (A5-Q12)

**CI e anti-slop.** O `ci.yml` real difere da descrição em `quality-gates.md` (push em todas as branches/cancelamento); a lista de gates aparece em 16 arquivos; a varredura de `allow(` dispara em 53/86 testes por causa de `#![allow(clippy::expect_used)]`. A política anti-slop **detecta defeitos reais** (catálogo A1–A11), mas ~40% do volume é procedimento que não muda decisão; a simplificação deve preservar baseline-first, `cmp`, proibição de enfraquecer gate e AS-02/03/12, e remover duplicação/contagens monotônicas. `[F/O]` (A5 §6; crítica C7)

**Conclusão.** A qualidade é boa para consistência interna, mas não sustenta “completo”. O próximo ganho é um corpus reproduzível com exceções/`RTE`/frames, um controle de mutação no emulador e a remoção de pulos silenciosos — não mais testes do mesmo tipo. `[O]` (A5)

### B.6 Interfaces agênticas, MCP/HTTP/CLI/GUI e segurança

**Estado real.** 16 operações no catálogo; `debug.run`, `trace.capture`, `script.run` e `tas.record` **não** estão no MCP (`-32602 unknown tool`). O agente MCP não consegue fazer RE interativa; CLI/script conseguem, mas são one-shot e não anexam à sessão MCP. `[F/E]` (A6-F1)

**Economia de chamadas.** 22 chamadas MCP reais = 8.564 B (~2.141 tokens). Um passo observado sintético = 1.266 B (~317 tokens)/10,3 ms; Mega Drive = 2.072 B (~518 tokens)/passo; 10 mil passos Mega Drive ≈ **20 mil chamadas, 20,7 MB, ~5,2 M tokens (extrapolação bytes/4), ~103 s** (emulação ~1,04 s). `experiment.run` de 1.000 passos = 1 chamada/1.068 B/6,9 ms. `[E]` (A6 §3.2; crítica C6)

**Operação.** Sem cancelamento/deadline; `run` de 100 mil passos = 9,42 s sob lock; outras sessões esperam 9,12–9,13 s; fechar o socket não cancela; `notifications/cancelled` → 202; `progressToken` ignorado. `[E]` (A6-F2)

**Recursos.** 100 snapshots = +21,4 MB de RSS; 20 `store.save` = 4,3 MB em 62 arquivos `0644`; `max_sessions` não limita memória. `[E]` (A6-F3)

**Segurança.** Slowloris pré-auth: 2 conexões truncadas com `--workers 2` prendem o pool; `/health` legítimo estoura 4,002 s. `accept` poll de 5 ms → piso de 5,16 ms/chamada, 763 req/s com 4 workers. MCP em `2025-03-26` (vigente `2026-07-28`), sem `Origin`, sem `MCP-Protocol-Version`/`Mcp-Method`/`server/discover`; sem teste de interop com SDK real. Com Bearer em todas as rotas exceto `/health` e bind loopback, o `Origin` é **P2/P3** (endurecimento/lacuna de spec), não P0; não há exploração autenticada demonstrada. O slowloris pré-auth, porém, é indisponibilidade **demonstrada**. `[E/O]` (A6-F4/F5/F6/F8; crítica C6)

**Pontos fortes.** Sandbox bloqueia `../`, caminho absoluto e symlink; 47 corpos MCP e 160 parsers binários malformados sem crash; paridade canônica (experiment 2× byte-idêntico); auth/ownership sem vazamento de token. `[F]` (A6 §6)

**Conclusão.** O diagnóstico M6-FU1 é forte, mas a frase “produto agêntico não existe” apaga o valor real de CLI/script/experiment. A formulação correta é **“jornada interativa de RE sobre MCP incompleta”**. A API mínima deve ser composta (não espelhar ~30 verbos), incluir persistência/restauração de **sessão viva ramificada**, definir formato/paginação de frame (96 KiB base64 não cabe RGBA 320×224 = 286.720 B), e tratar *unknown outcome* com `performed`/tick/hash ou operação consultável. `[O]` (A6; crítica C6)

### B.7 Produto, roadmap e governança documental

**Fluxo KidEngine (caso de uso central).** Carregar ROM → rodar até gameplay → checkpoint → múltiplos forks → alterar inputs/variáveis → rodar cenários → comparar divergências → inspecionar registradores/memória/instruções → identificar rotina → relatório reproduzível. Hoje: carregar/checkpoint/fork/compare/read_memory existem; **input por passo no DSL é neutro**; não há `memory.write` no CLI/serviço; `session.save` não aceita `--rom`; `store.save` não persiste sessão viva; debug/trace/TAS não estão no MCP. O fluxo **não é executável** no backend real. `[F/E]` (A7 §3)

**Documentação.** 89 arquivos/38.165 linhas; 13 documentos repetem status; 6 versões contraditórias de “rendering”; 49 menções a `cb05836`; `docs/architecture/overview.md` ainda diz “sem rendering/DMA” enquanto 8.3 está concluída. A resposta correta só sai de código/testes. `[F]` (A7 §2; A2-F11)

**Roadmap.** O M8 pode fechar fases sem nenhum ROM executando; 8.6 assume um boot que não existe; 8.7/8.8 dependem de um emulador que ainda não roda. A consolidação de produto deve vir **antes** de 8.7/8.8/M9, com casos de uso verificáveis (UC-1..UC-4; UC-5 de captura fica para depois de 8.7). `[O]` (A7; crítica C4)

**Conclusão.** O produto precisa de um gate de valor (boot real + fluxo de RE) antes de mais infraestrutura. A documentação deve ter uma fonte única de status e um bloco gerado; o `STATE.md` deve listar o trabalho não-mergeado (hoje o 8.4 com 14 commits é invisível). `[O]` (A7)

### B.8 Pesquisa externa e limitações

**Referências.** Exodus (MIT; oráculo), Genesis Plus GX (licença restritiva; não reutilizar), BlastEm (GPLv3), jgenesis, MAME (licença complexa), Ares, BizHawk (heterogênea), `ymfm` (BSD-3, candidato), Nuked-OPN2 (LGPL-2.1+). Nenhum código foi copiado; a tabela de licenças/URLs está em A2-E5. `[F]` (A2 §10)

**Limitações desta revisão.**
- Baseline `b8d63a1`; o trabalho 8.4 (14 commits + mudanças não commitadas) foi lido apenas como contexto, não buildado nem testado.
- Benchmarks em host compartilhado, workload sintético `interrupt_rom(3)`; sem `perf`/`valgrind`/`perf_event`; atribuição por ablação, minflt e contador de alocações.
- Nenhuma medição pós-correção; as projeções (319 fps) são aritmética, não capacidade demonstrada em jogo.
- Sem hardware real, sem validação externa dos dumps, sem Windows/macOS runtime, sem SDK MCP real, sem audit de licença do `ymfm`.
- O diferencial com Exodus não foi reexecutado neste ambiente; a análise é de código + binário local + fixtures.

---

## C. Registro de recomendações

### C.1 Tabela priorizada

| ID | Problema | P | Recomendação | Custo/risco | Fase/gate |
|---|---|---|---|---|---|
| **S-01** | Boot/bus: 24 bits, espelho de RAM, I/O `0xA10008–0xA1000F`, open bus | **P0** | Corrigir o bus num único ponto; mapear controles 1/2/3/EXT; corrigir o plano 8.5 (porta 3); gates G-BOOT-0/1 específicos | Baixo (~1 semana); muda comportamento canônico → versionar/regenerar goldens; risco de regressão de snapshot | Agora; antes de 8.5 |
| **S-02** | DMA `memory→VRAM` lê VRAM em silêncio | **P0** | Rejeitar com erro tipado agora; implementar o lado 68000 em 8.6a; contabilizar ciclos | Baixo (5 linhas); nenhum contrato público | Agora; 8.6a |
| **S-03** | Verificação por passo: hash+cópias+alocações; inspect re-hasheia; render sempre; fork 4× | **P0/P1** | ADR: `step` sem hash, `state_hash()` explícito, buffers persistentes, `status()` barato, render na fronteira, fork `Arc`+`reap`; caminho verificado para TAS | Médio (40–60 call sites); muda API pública; testes golden migram; risco de divergência silenciosa → invariante hash sob demanda == completo | Agora (ADR); antes de 8.4/8.5 |
| **S-04** | M6-FU1 + economia de chamadas + sem cancel/deadline/cotas | **P1** | API mínima de 9 ops compostas com batching, `stop_on`, inputs, `max_wall_ms`, cancel/progress; `memory.write/diff`, disasm, trace, frame, sessão viva; lock por sessão; cotas | Médio-alto (1,5–3k LOC, 1–2 fases); muda fronteira do serviço | Consolidação 8.6c; após ADR D2 |
| **S-05** | Mutex global, registry vitalício, sessão fechada retém backend | **P1** | Lock/owner por sessão; `reap()`; cotas; deadline/cancel; sem Tokio por padrão | Médio; testes de concorrência | Agora/curto prazo |
| **S-06** | Diferencial não reproduzível; zero exceções/RTE/frames no corpus; pulos silenciosos; demo 167 s | **P1** | Commitar ROMs sintéticas legadas + teste de fixture; oráculo vendorizado/pinado ou skip explícito; ROM de exceção+RTE; controle de mutação; guarda de contagem; golden do demo | Baixo-médio; licença das fixtures sintéticas é do projeto | 8.5/CI |
| **S-07** | Frame no fim (sem raster), sem contenda DMA/VDP, HBlank no fim da linha | **P1** | Render por scanline com line buffer canônico; contabilizar DMA/escrita VDP; HBlank/VBlank no dot correto; hashes dourados versionados | Médio; muda frame/hashes | 8.6a |
| **S-08** | Drift documental de status e fatos técnicos | **P1/P2** | Fonte única (`docs/roadmap/milestones.md`), bloco de status gerado, banner histórico, `STATE.md` ≤10 linhas com trabalho não-mergeado, drift guard no `check.sh` | Baixo | Agora/contínuo |
| **S-09** | Slowloris pré-auth, quotas, MCP 2025-03-26, Origin, accept 5 ms | **P2** | Timeout/limite pré-auth; quotas; `Origin` agora; versão MCP após M8; accept poll menor; loopback v1 | Baixo-médio | 8.6c/plataforma |
| **S-10** | Superfície pública 493 itens, `StateView` duplicado, erros mistos, 50 parsers | **P3** | Higiene incremental; sem reescrita | Baixo | Depois |

### C.2 Alternativas descartadas (e por quê)

- **Reescrever o emulador ou integrar um núcleo inteiro:** descartado. A arquitetura Rust/sessões é o diferencial; licença/estado/determinismo e custo de migração não se justificam. `[O]`
- **Tokio/async/atores agora:** descartado como pré-requisito; lock/owner por sessão resolve o caso imediato; async adiciona complexidade e risco de determinismo. `[O]`
- **BLAKE3/xxh3 como solução principal:** descartado isoladamente; continua serializando 197 KB por passo; pode ser otimização opcional depois do contrato. `[O]`
- **COW/dedup/hash incremental agora:** adiados; `Arc` no payload/initial_state + `reap()` resolvem o caso imediato; COW exige evidência de uso e cuidado com versionamento. `[O]`
- **Commitar ROMs comerciais / embutir BIOS:** nunca. `[F]` (AGENTS.md; licensing)
- **TLS/multiusuário/bind não-loopback na v1:** fora de escopo; documentar como restrição. `[O]`
- **“Anti-slop” com contagens monotônicas/limites rígidos de linhas:** descartado; preservar o que detecta defeito real, remover duplicação. `[O]` (A5; crítica C7)

### C.3 O que preservar

Rust e crates atuais; backend opaco/versionado; estado mutável exclusivo por máquina; separação core/serviço/transporte/apresentação; emulação sem relógio de host; hash canônico disponível e formatos preservados; verificação por passo onde TAS/replay exigirem; rejeição explícita de DMA incorreto; gates de valor e diferencial reproduzível; nenhuma migração genérica para async/atores/core externo. `[O]` (crítica final)

---

## D. Proposta de evolução arquitetural

### D.1 Princípios

1. **Execução e verificação são responsabilidades diferentes.** O passo executa; a verificação é uma política.
2. **Determinismo não depende de calcular hash.** O hash detecta divergência; o log de inputs + replay localiza a primeira divergência.
3. **Observação não altera estado canônico.** `status`, `inspect`, `trace` e `frame` são leituras; render lazy não pode renderizar o estado futuro.
4. **Uma sessão, um dono de execução.** Nada de lock global no caminho de execução.
5. **Contratos pequenos e versionados.** Não congelar o contrato no Mega Drive; não generalizar para necessidades hipotéticas.
6. **Sem reescrita.** Mudanças cirúrgicas, com invariantes e migração.

### D.2 Diagrama: hoje vs proposta

```text
HOJE
  Backend::step ──► StepOutcome { tick, state_hash }   ← serializa 197 KB + SHA a cada instrução
  AppService ──► Mutex<SessionManager> ──► run loop    ← todas as sessões serializadas
  MCP ──► 16 ops (sem debug/trace/script/TAS/memória-escrita)
  inspect ──► hash + 64 KiB de RAM por chamada

PROPOSTA
  Execução:  step ──► StepOutcome { tick, halted }      ← sem hash
             run_until / run_cycles / run(batch)        ← até condição/evento/frame
  Verificação: state_hash() explícito; VerificationPolicy (EveryStep | EveryN | OnDemand | Checkpoint)
             step_verified() para TAS/replay; replay a partir do checkpoint para 1ª divergência
  Observação: status() barato (tick, pc, halted); inspect() sob demanda; memória em janelas
  Serviço:   SessionRegistry com Mutex/owner por sessão; run com cancel/deadline/progress/quotas
  Transporte: 9 ops compostas com batching; MCP 2026-07-28 após M8; loopback v1
  Backend:   bus 24 bits + RAM espelhada + controles de I/O; DMA rejeitado até 8.6a;
             render na fronteira (não no futuro); Arc no payload/initial_state; reap()
```

### D.3 Mudanças de contrato

| Contrato | Hoje | Proposta | Compatibilidade |
|---|---|---|---|
| `StepOutcome` | `{tick, state_hash, halted}` | `{tick, halted}` + `Backend::state_hash()` explícito | Quebra pública; ADR; migrar 31 usos; testes golden; versão de política de hash |
| `step` | 1 instrução | `step`, `step_verified`, `run_until`, `run_cycles`, `run` (batch) | Aditivo; `step_verified` preserva TAS |
| `inspect` | hash + RAM | `status()` barato + `inspect()`/`read_memory` sob demanda | `MachineInspection` vira `MachineStatus` + views; adaptar serviço/GUI |
| `MachineInspection` | flat string→u64 + 1 janela | `MachineStatus` + mapa tipado/por CPU + janelas | Necessário para Z80/M9; preservar views atuais como adaptador |
| Snapshot | payload v5 (main) | v6/v7 com Z80/áudio; hash de estado separado | Rejeitar versões antigas com erro tipado; regenerar goldens |
| TAS | hashes por passo | política + hashes em intervalos; replay verificado | Filmes antigos: replay com `step_verified`; novos: política registrada |
| Concorrência | mutex global | lock/owner por sessão; cancel/deadline; cotas | API do serviço muda; testes de concorrência |
| Multi-backend | `Tick` único, `InputFrame` fixo | timeline explícita, CPUs/views, input por backend | M9; não implementar agora |

### D.4 Modelo de execução e verificação

- **Execução normal:** `step` não calcula hash. `run_until(condition)` / `run_cycles(n)` / `run_frame()` executam em lote, com `stop_on` (PC, memória, evento, tick) e `max_wall_ms` (política de host, não estado canônico).
- **Verificação:** `state_hash()` explícito; `VerificationPolicy`:
  - `EveryStep` (TAS verify, depuração de divergência);
  - `EveryN`/`Checkpoint` (experimentos, CI);
  - `OnDemand` (inspeção).
- **Primeira divergência:** verificar em granularidade ampla; se divergir, restaurar o último checkpoint e replay com `EveryStep` para localizar o passo exato. Preservar log de inputs e tick.
- **TAS:** manter um caminho `step_verified` para verificação; filmes novos registram a política; filmes antigos continuam reproduzíveis com verificação por passo.
- **Render:** renderizar na fronteira do frame (como hoje) ou capturar o estado de fronteira e renderizar sob demanda; nunca renderizar o estado futuro no `take`. Sem consumidor, pular render é seguro **se** o frame puder ser reconstruído do estado de fronteira.
- **Inspect:** `status()` retorna tick/pc/halted/registradores baratos; `read_memory`/views são sob demanda; trace/debug não chamam `inspect` por passo.

### D.5 Serviço, concorrência e API agêntica

- **Sessões:** `SessionRegistry` com `RwLock`/shard e `Mutex<Session>` por sessão; `run` segura apenas a sessão alvo; `list` lê metadados baratos (sem `inspect` de cada sessão); `reap()` remove fechadas e libera memória.
- **Cancelamento:** `CancelToken` cooperativo checado em fronteiras de instrução; `run` retorna `performed`, `tick`, `halted`, `state_hash` (se verificado); `notifications/cancelled` e `progressToken` com semântica real.
- **Unknown outcome:** operação com `operation_id` consultável + `performed`/tick/hash; cliente pode reconciliar após timeout; não prometer exactly-once sem protocolo.
- **Cotas:** max snapshots/sessão, max bytes/sessão, max store, max sessions com `reap`; memória estimada, não só contagem.
- **API mínima (9 operações compostas, entrada: A6 §3.5):**
  1. `run` (batch; `stop_on` PC/memória/evento/tick/frame; inputs; `max_wall_ms`; cancel/progress).
  2. `memory.read` / `memory.write` / `memory.diff` / `memory.search` (uma família).
  3. `disassemble`.
  4. `trace.capture` / `trace.query`.
  5. `debug` (breakpoints/watchpoints/eventpoints).
  6. `frame.capture` (formato/paginação; não 96 KiB para RGBA bruto).
  7. `session.save` / `session.load` (sessão viva ramificada; exportação).
  8. `fork` / `compare`.
  9. `capabilities` / `cancel` / `progress` (controle).
  Nomes e agrupamento são decisão da equipe; a aceitação é o fluxo KidEngine, não a contagem.
- **Transporte:** manter HTTP/1.1 bounded loopback; corrigir slowloris (timeout/limite pré-auth), accept poll, `Origin`; MCP 2026-07-28 depois do M8; sem TLS/SSE/keep-alive na v1.

### D.6 Snapshot, fork e memória

- `Arc<Vec<u8>>` no payload/initial_state e no snapshot adotado; clone vira refcount; `reap()` remove sessões fechadas.
- COW/dedup só com evidência de uso; versionar se mudar o formato.
- Checkpoint continua completo e versionado; hash do estado separado do payload.
- Fork: evitar as 3 cópias atuais (snapshot do pai + payload do filho + initial_state); reutilizar `Arc`.

### D.7 Multi-backend (M9)

- Não implementar agora. Preparar o contrato: `Tick` com timeline explícita, `MachineStatus` por CPU, `InputFrame` por backend, `run_until`/`run_cycles` genéricos, views tipadas.
- Não criar abstrações para sistemas hipotéticos; um segundo backend simples valida o contrato. `[O]` (A1-03; A2 §8)

### D.8 Migração

1. **Hardening (behavior-bearing):** bus/DMA/docs; versionar snapshots e regenerar goldens; separar de otimizações.
2. **ADR de verificação:** `step`/`state_hash`/`step_verified`; atualizar 31 usos e testes; invariante “hash sob demanda == completo”; medir.
3. **Serviço:** lock por sessão, cancel/deadline, quotas, `reap`; testes de concorrência.
4. **API agêntica:** 9 ops, batching, sessão viva, frame; interop MCP.
5. **Render/fork:** render na fronteira/lazy; `Arc`/`reap`; COW depois.
6. **Qualidade:** corpus reproduzível, exceções/RTE, mutação, CI.

---

## E. Proposta de atualização do roadmap

### E.1 Agora (0–2 semanas, sem features novas)

- **S-01/S-02:** bus 24 bits, espelho de RAM, `0xA10008–0xA1000F`, open bus, rejeição de DMA, correções de doc; gates G-BOOT-0/1 específicos por fixture.
- **D2:** ADR do contrato de verificação; buffers persistentes; `status()` barato; render na fronteira.
- **D5:** fonte única de status; banner histórico; `STATE.md` ≤10 linhas com trabalho não-mergeado; drift guard.
- **Qualidade:** commitar ROMs sintéticas legadas + teste de fixture; remover pulos silenciosos; golden do demo.

### E.2 Trilhas

**Trilha A — backend (sequencial, com decisão do proprietário sobre 8.4/8.5):**
1. **8.4 (preservar):** concluir Z80/bus (pré-requisito de boot); o áudio é um sub-bloco separável e pode terminar antes ou depois de 8.5, conforme o status real de 8.4 (decisão D4). Não descartar o trabalho existente.
2. **8.5:** timers, controles completos (TH/6-button, Start), I/O `0xA10008–0xA1000F`, correção do plano (porta 3); G-BOOT-1.
3. **8.6a compatibilidade:** DMA real, frame por scanline, contenda, HBlank/VBlank no dot; G-BOOT-2 (frame+áudio por ROM, local-only).
4. **8.6b desempenho:** hash fora do step, buffers, render sob demanda, fork `Arc`, inspect barato; meta por frame, bench no CI.
5. **8.6c consolidação de produto:** UC-1..UC-4; API agêntica; lock por sessão; relatório reproduzível; piso de fps.

**Trilha B — plataforma (paralela, dono único):**
- lock/owner por sessão; cancel/deadline/quotas; `reap`; API mínima 9 ops; MCP `Origin`/versão; loopback v1.

**Trilha C — qualidade/evidência (paralela):**
- diferencial reproduzível; exceções/RTE/frames; controle de mutação; fuzz; CI em camadas; anti-slop lite.

### E.3 Adiar

8.7 (modos/captura PNG/WAV), 8.8 (janela nativa), M9 (multi-backend), TLS/multiusuário, async/Tokio, hash incremental, COW/dedup, GUI com conteúdo de inspetor. `[O]`

### E.4 Casos de uso de consolidação (UC)

- **UC-1 Boot smoke:** ROM sintética redistribuível + (se disponível) um fixture comercial; avançar além do handler/reset; PCs/transições VDP específicos; sem erro de host. **Não** “não crashou”.
- **UC-2 Fork/compare:** checkpoint → 2 forks → alterar input → rodar → comparar → restaurar; hashes determinísticos.
- **UC-3 RE interativa via MCP:** run com `stop_on` PC/evento; memory read/write/diff; disasm; trace; breakpoint; 10 mil passos em **uma** chamada < 2 s.
- **UC-4 Relatório reproduzível:** artefato do cenário KidEngine com inputs, ticks, hashes, achados; reexecutável.
- **UC-5 (pós-8.7):** captura PNG/WAV determinística; não bloquear a consolidação anterior.

### E.5 Gates

- **G-BOOT-0:** 300 mil passos sem erro de host nos fixtures (necessário, não suficiente; um `BRA *` passa).
- **G-BOOT-1:** marcos observáveis por fixture (PCs de init, transições VDP, saída do handler), não “todos os registradores ≠ 0”.
- **G-BOOT-2:** frame+áudio com hash por ROM; local-only; skip sem fixture (nunca CI verde por ausência).
- **G-PERF:** orçamento por frame em workload pinado; regressão relativa; runner calibrado; não um gate absoluto universal de 5 M steps/s.

### E.6 Dependências e aprovações

- S-01/S-02 antes de 8.5; D2 antes de 8.4/8.5 (o estado cresce); D3/D4 antes de promessa multi-agente; 8.6c depois de 8.5+8.6a (precisa de ROM executando), mas UC-2/UC-3 podem usar o sintético.
- Aprovações do proprietário: escopo v1, contrato de verificação, sequência 8.4/8.5, áudio/licença, orçamento de CI/qualidade, MCP não-loopback.

---

## F. Registro de decisões pendentes

> **Regra.** O proprietário decide **escopo, arquitetura, licença, compatibilidade e objetivos**. A equipe decide detalhes técnicos reversíveis (mutex por sessão, tamanho de batch, enum de política, nomes de tools, organização de scripts) e registra em ADR quando exigido.

### D1 — Público e escopo da v1

**Problema:** o produto se apresenta para humanos, scripts e agentes, mas o serviço serializa sessões e o MCP não faz RE interativa.
**Alternativas:** (a) v1 single-agent, loopback, MCP mínimo; (b) v1 multi-agente com locks/cotas/TLS; (c) adiar MCP e manter CLI/script.
**Recomendação:** **(a)**. Loopback/usuário único; MCP mínimo com batching/cancel; multi-agente/TLS depois. **Risco:** limita a promessa “agentes simultâneos”; **verificação:** 10 mil passos em 1 chamada <2 s; 4 sessões >2×; nenhuma operação bloqueia `list` de outra sessão. **Dono:** proprietário (escopo); equipe (implementação).

### D2 — Contrato de verificação e desempenho

**Problema:** hash obrigatório por passo, inspect/fork/render caros; 0,47–1,05 fps.
**Alternativas:** (a) micro-otimizar mantendo contrato; (b) ADR de hash opt-in + buffers + status + render na fronteira + caminho verificado para TAS; (c) hash incremental/dirty; (d) nada.
**Recomendação:** **(b)** agora; (c) só se hash por passo virar requisito de produto. **Risco:** quebra pública, 40–60 call sites, goldens; **mitigação:** invariante hash sob demanda == completo; versão de política; migração faseada. **Verificação:** execução pura medida; frame de ROM real no orçamento; demos byte-idênticos; `.cfmovie`/snapshots compatíveis. **Dono:** proprietário aprova a quebra; equipe desenha/implementa.

### D3 — Serviço, concorrência e API agêntica

**Problema:** mutex global, sem cancel/deadline/cotas, registry vitalício, M6-FU1, sem sessão viva no transporte.
**Alternativas:** (a) espelhar ~30 verbos da CLI; (b) API mínima 9 ops + lock por sessão + cancel/deadline/cotas + sessão viva + unknown-outcome; (c) CLI-only; (d) Tokio/async.
**Recomendação:** **(b)**, com a lista A6 §3.5 como entrada, não compromisso; dono único; sem async por padrão. **Risco:** muda fronteira do serviço; **verificação:** 10 mil passos/1 chamada <2 s; 4 sessões >2×; cancelamento efetivo; `close` libera memória; `performed`/tick/hash reconciliam timeout. **Dono:** proprietário (escopo/quebra); equipe (desenho).

### D4 — Sequência do roadmap e consolidação

**Problema:** M8 pode fechar sem ROM executando; 8.4/8.5 em ordem incerta; 8.7/8.8/M9 dependem de um emulador que não roda.
**Alternativas:** (a) manter 8.4→8.5; (b) hardening agora + preservar 8.4 + concluir Z80/bus + 8.5 + áudio; (c) 8.4 e 8.5 em paralelo; (d) nada.
**Recomendação:** **(b)**, condicionada ao status real de 8.4; não descartar o trabalho; consolidação 8.6c antes de 8.7/8.8/M9. **Risco:** atrasa áudio; **verificação:** G-BOOT-0/1; 8.4 preservado; UC-1..UC-4. **Dono:** proprietário, com o responsável por 8.4 informando o que está pronto.

### D5 — Áudio e licença

**Problema:** escolha entre `ymfm` (BSD-3) e `Nuked-OPN2` (LGPL-2.1+); precisão vs atrito.
**Alternativas:** (a) `ymfm` com ADR; (b) `Nuked` com ADR/análise LGPL; (c) implementar próprio; (d) adiar áudio.
**Recomendação:** **(a) como candidato**, após audit de licença primária, pin de commit, build, precisão e serialização; `Nuked` opcional; não adotar agora. **Risco:** licença/relink/estado; **verificação:** notices, provenance, snapshot determinístico, portabilidade. **Dono:** proprietário (licença); equipe (avaliação técnica).

### D6 — Qualidade, evidência e governança

**Problema:** diferencial não reproduzível; zero exceções no corpus; pulos silenciosos; docs duplicados; anti-slop burocrático.
**Alternativas:** (a) manter; (b) ROMs sintéticas commitadas + teste de fixture + oráculo vendorizado/pinado ou skip explícito; ROM de exceção/RTE; mutação; guarda de contagem; fonte única; anti-slop lite; (c) contratar serviço externo.
**Recomendação:** **(b)**. **Risco:** custo de CI; **verificação:** clone limpo roda o diferencial ou falha explícita; nenhum teste passa por ausência; docs sem status contraditório. **Dono:** proprietário (orçamento); equipe (execução).

### D7 — MCP, transporte e superfície de rede

**Problema:** MCP 2025-03-26, sem `Origin`, sem interop real; slowloris pré-auth; bind loopback.
**Alternativas:** (a) manter; (b) `Origin`/timeout agora, versão 2026-07-28 após M8, loopback v1; (c) expor não-loopback/TLS agora.
**Recomendação:** **(b)**. **Risco:** baixo; **verificação:** teste de interop com SDK em dev; `/health` sob slowloris; nenhum bind não-loopback sem decisão. **Dono:** proprietário (não-loopback); equipe (endurecimento/versão).

---

## G. Baseline de qualidade e desempenho

### G.1 Ambiente e commit

- WSL2 Ubuntu, ext4; 8 vCPU; 30 GiB RAM; 797 GiB livres.
- `cargo 1.100.0 (3acc79c0c 2026-11-05)`; `rustc 1.100.0 (0ef1bc386 2026-11-09)`.
- Baseline: `b8d63a17349e672ba6945eaba3379cec38cf9d1b` (`origin/main`).
- `CARGO_HOME=/tmp/t7-cargo`, `CARGO_TARGET_DIR=/tmp/chronofork-review-target`.
- Worktree limpa; nenhum arquivo versionado alterado; evidência em `docs/reviews/2026-11-20-independent-review/`.

### G.2 Gate

```sh
CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=/tmp/chronofork-review-target \
  cargo test --workspace --locked --no-fail-fast
```

- **1.289 passaram, 0 falharam, 0 ignorados, 108 alvos** (workspace, debug).
- Gate de qualidade (A5): `fmt` 0, `check` 0, `clippy -D warnings` 0, `rustdoc -D warnings` 0.
- Hot spots: `megadrive_demo_*` 166,91 s; `debug_observation` 59,06 s; unidade megadrive 49,06 s; `cycle_timeline` 35,97 s; `cartridge` CLI 34,10 s.
- 2 testes de cartucho pulam silenciosamente sem `.research/roms/`; o número efetivo no CI é menor.

### G.3 Benchmark de passo (release, 3 execuções)

Workload `interrupt_rom(3)`, NTSC, 10.256 passos/frame; 1 warmup + 5 trials × 10.000 passos. `[E]`

| Métrica | run 1 | run 2 | run 3 | mediana |
|---|---:|---:|---:|---:|
| step p50 (s/10k) | 2,17386223 | 2,13847319 | 2,14062625 | **2,14062625** |
| instructions/s | 5.166 | 5.248 | 5.185 | **5.185** |
| checkpoint bytes | 197.234 | 197.234 | 197.234 | **197.234** |
| serialization p50 | 90,2 µs | 87,6 µs | 90,9 µs | **90,2 µs** |
| restore p50 | 118,5 µs | 99,8 µs | 107,1 µs | **107,1 µs** |

Decomposição (A3/A1, mediana): emulação 0,17 µs; corpo ~4,0 µs; SHA 84,7–86,9 µs; churn ~116,6 µs; residual ~2 µs; passo 209,8 µs. SHA puro 37,8–41,4% do passo; caminho completo ~98,95%; o 99,9% é verificação/(verificação+emulação), não o passo. As parcelas não são aditivas. `[E/I]` (crítica C1)

### G.4 Memória, fork e inspeção

- Checkpoint de sessão: 183 µs / ~192 KiB; payload de backend 197.058 B.
- `fork_current`: 666 µs; RSS +822 KiB por fork (A3) / 773,5 KiB (A1, 200 forks); 3 cópias do mesmo estado; sem `reap`.
- `inspect`: 95,9 µs com hash; 3,85 µs sem hash (A3); 211–240 µs em A1 (workload/denominador diferente).
- 100 snapshots = +21,4 MB RSS; 20 `store.save` = 4,3 MB/62 arquivos `0644`.
- Sessão fechada retém ~387,6 KiB; `max_sessions=1024` ≈ 397 MiB.

### G.5 Concorrência (release, mediana de 2 execuções)

| Sessões/threads | Throughput agregado (steps/s) | Eficiência |
|---:|---:|---:|
| 1 | 5.066 | 1,000 |
| 2 | 4.514 | 0,446 |
| 4 | 4.593 | 0,227 |
| 8 | 4.629 | 0,114 |

Eficiência = throughput(N)/(N × throughput(1)); corrigida da crítica C2. Um run de 500 passos tem p95 117 ms → 1,5 s; 20.000 passos = 4,65–4,73 s; 100.000 ≈ 23 s de lock. `experiment.run` (manager privado) escala 5,97× em 8 threads. `[E]`

### G.6 MCP / transporte

- 16 operações; `debug.run/trace.capture/script.run/tas.record` → `-32602 unknown tool`.
- 22 chamadas reais = 8.564 B (~2.141 tokens); passo sintético = 1.266 B (~317 tokens)/10,3 ms; passo Mega Drive = 2.072 B (~518 tokens); 10 mil passos ≈ 20 mil chamadas/20,7 MB/~5,2 M tokens (extrapolação bytes/4)/~103 s; emulação ~1,04 s.
- `experiment.run` 1.000 passos = 1 chamada/1.068 B/6,9 ms.
- `accept` poll 5 ms → piso 5,16 ms/chamada, 763 req/s com 4 workers; MCP 2025-03-26; sem `Origin`; sem interop com SDK real.
- Slowloris: 2 conexões truncadas com 2 workers; `/health` 4,002 s.

### G.7 Boot

- Chakan: reset `PC=0x0000333E`, primeira instrução `TST.L $00A10008`; bus não mapeia `0xA10008`; handler fatal em `0x220`; 200 mil passos depois, ainda lá.
- Kid: `SSP=0xFFFFF7FE`, `PC=0x000004B0`; primeiro passo falha ao preparar a moldura de exceção, zero instruções completadas; endereço reportado é o fim do intervalo validado, não escrita parcial.
- Nenhum dos dois progride no bootstrap; o SSP do Kid é legítimo; integridade externa do dump não verificada.

### G.8 Como reproduzir

```sh
# na worktree de revisão
CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=/tmp/chronofork-review-target \
  cargo test --workspace --locked --no-fail-fast
CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=/tmp/chronofork-review-target \
  cargo run -p chronofork-megadrive --example bench_step --release --locked
```

Evidência detalhada: `areas/` (7 relatórios), `benchmarks/` (logs, tabelas, harnesses e patch), `critique-final.md`, `SYNTHESIS-DRAFT.md` e `baseline.md`.

### G.9 Limitações do baseline

- Host compartilhado; valores absolutos oscilam; use medianas e razões.
- Workload sintético; a projeção de 319 fps não é frame de jogo.
- Sem medição pós-correção; sem hardware real; sem Windows/macOS; sem SDK MCP real; sem audit de licença do `ymfm`; sem validação externa dos dumps.
- O trabalho 8.4 não foi buildado/testado; a revisão avaliou o baseline e leu o 8.4 como contexto.

---

## Anexos (evidência preservada)

- [A1 — Rust e arquitetura](areas/01-rust-systems.md)
- [A2 — Emuladores e hardware](areas/02-emulator-engineering.md)
- [A3 — Desempenho e hashing](areas/03-performance.md)
- [A4 — Concorrência](areas/04-concurrency.md)
- [A5 — Determinismo e qualidade](areas/05-determinism-quality.md)
- [A6 — Interfaces e segurança](areas/06-interfaces-security.md)
- [A7 — Produto e roadmap](areas/07-product-planning.md)
- [Crítica final](critique-final.md)
- [Síntese preliminar (com correções da crítica)](SYNTHESIS-DRAFT.md)
- [Benchmarks: tabelas](benchmarks/results-tables.md) · [README](benchmarks/README.md)
- [Baseline compartilhado](baseline.md)

> Os anexos estão nesta pasta (`docs/reviews/2026-11-20-independent-review/`). Não commite ROMs, BIOS ou logs com caminhos privados; os logs brutos de benchmark permanecem no worktree de revisão (`review-2026-11-20-architecture`).
