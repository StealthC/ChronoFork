# A2 — Engenharia de emuladores e correção do hardware (Mega Drive, baseline `b8d63a1`)

**Reviewer:** `emulator` · **Data:** 2026-11-20 (America/Fortaleza) · **Worktree:**
`/home/stealthc/projects/ChronoFork/.agents/worktrees/review-2026-11-20-architecture`
(branch `agent/review-2026-11-20-architecture`, HEAD `b8d63a1`).
**Escopo:** organização do 68000, decodificação/execução, modos de endereçamento, exceções,
modelo de ciclos, VDP (render/DMA/FIFO/timing/H-V), bus/arbitragem, CPU–VDP–Z80–áudio,
contenção, PAL/NTSC, controles, save/restore, comparação de modelos de execução, nível de
precisão necessário, referências upstream e build-vs-integrate.
**Método:** leitura do código do baseline + demos sintéticos executados + benchmark release
próprio + tentativa de boot dos dois fixtures comerciais + pesquisa das referências.
Nenhum código de produção, doc versionada, commit ou dependência foi alterado; nenhum byte de
ROM foi copiado/impresso/commitado. Achados brutos em `areas/02-evidence/`.

**Classificação usada:** fato observado (FO) / hipótese (H) / experimento (E) / inferência (I) /
opinião (O). Cada achado traz confiança (alta/média/baixa) e prioridade (P0–P3).

---

## 1. Resumo executivo

O backend do baseline é um **modelo de tempo por instrução com timeline de master-clock por
instrução**, e não um emulador por ciclos: cada passo executa **uma** instrução, custa o valor
tabelado da instrução × 7 master cycles, e só então avança VDP e scheduler
(`backend.rs:632-685`). Essa escolha é coerente, determinística e barata de raciocinar, mas
impõe um teto estrutural de precisão que **nenhuma otimização posterior remove** sem trocar o
modelo (contenda de barramento, latência de interrupção, espera em flags do VDP no meio da
instrução, DMA real).

Três obstáculos dominam o caminho até "jogo comercial rodando" e são independentes entre si:

1. **O decodificador de barramento não tolera regiões não modeladas** (P0). Os dois fixtures
   comerciais morrem na primeira instrução (Kid Chameleon) ou no tratador de exceção que a
   própria ROM instalou (Chakan), porque `TST.L $00A10008` — leitura de uma janela de I/O não
   modelada — vira exceção de endereço do guest em vez de *open bus* como no hardware real.
   Detalhe em §4 e A2-F1/A2-F2.
2. **DMA memory→VRAM lê da VRAM** (P0). O modo de DMA mais usado por jogos é aceito e executa a
   transferência com a fonte errada, silenciosamente; isso viola diretamente a regra 8 de
   `AGENTS.md` ("Never silently reinterpret"). §5, A2-F3.
3. **O hash canônico completo obrigatório a cada passo** consome ~38 % do passo medido e o
   throughput fica em **5.190 instruções/s** contra ~1,9 M instruções/s necessários para tempo
   real — ~366× mais lento (§3, A2-F4). É um problema de contrato de API, não de micro-otimização.

Além disso, o frame é composto **inteiro no fim do frame** a partir do estado final
(`backend.rs:723-731` + `render.rs:263`), o que torna impossível qualquer efeito raster
(HBlank/line scroll/paleta por linha) e é incompatível com o objetivo de "compatibilidade
comercial" da fase 8.6 (A2-F5).

Nada disso invalida o que as fases 8.2/8.3 entregaram: o 68000 documentado está implementado e
tem evidência diferencial 11/11; o VDP tem um compositor determinístico e testado. O que existe
**não é** um emulador de Mega Drive jogável, e a documentação já diz isso — mas algumas
afirmações específicas de `docs/architecture/megadrive-slice.md` sobre espelhamento de RAM e sobre
origem de DMA estão erradas em relação ao código (§9, A2-F11).

---

## 2. Método e ambiente

- `CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=/tmp/chronofork-review-target`.
- `cargo build -p chronofork-cli --release --locked` (cache quente, exit 0).
- Baseline declarado verificado por mim: `git rev-parse HEAD` = `b8d63a1…`; worktree limpa
  exceto `scripts/__pycache__/` (pré-existente, não minha).
- Demos: `chronofork demo` e `chronofork megadrive demo` (ambos exit 0) — E1.
- Benchmark próprio: `bench_step` × 3 + probe de hash (E2, E3).
- ROMs: `debug run/inspect/memory/disasm/interrupts/vdp/trace` com caminho absoluto em
  `/home/stealthc/projects/ChronoFork/.research/roms/` (E4).

---

## 3. Modelo de tempo e ciclos

### 3.1 O que o código faz (FO)

- `MASTER_CYCLES_PER_LINE = 3420`, `MASTER_CYCLES_PER_68000_CYCLE = 7`,
  NTSC 262/224, PAL 313/240, relógios 53.693175/53.203424 MHz (`cycles.rs:24-52`).
- Um `step` = 1 instrução: `deliver_pending_interrupt` → `cpu.step` → `cycles*7 + fifo_wait*7`
  → `scheduler.advance_master` → `vdp.advance_master` → `drain_fifo` → `take_dma_cycles`
  (valor descartado) → `capture_frame` se cruzou frame → `sync_vdp_status` → `state_hash`
  (`backend.rs:632-685`).
- O custo da instrução vem de uma **tabela estática** derivada do manual (`cpu/timing.rs`),
  com custo de EA somado uma vez por operando (`ea_cycles`, `timing.rs:33-59`).
- Três contabilidades de tempo coexistem: `tick` (backend), `scheduler.ticks` e a posição
  line/dot do VDP. Todas avançam com o mesmo delta, mas são três cópias do mesmo fato; o
  backend ainda re-sincroniza o latch de VBlank do VDP a partir do scheduler
  (`sync_vdp_vblank`, `backend.rs:515-518`).

### 3.2 Consequências (I, confiança alta)

- Nada observa o barramento **dentro** de uma instrução: um `MOVE` de 4 ciclos vale 28 master
  cycles atomicamente. Logo:
  - latência e ordem de interrupção são quantizadas ao limite de instrução;
  - um laço que *espera um flag do VDP* (`btst/tst` sobre `$C00004`) só vê o flag no próximo
    limite de instrução — exatamente a limitação que a documentação do Exodus identifica como
    necessária para jogos que fazem *wait loop* de sub-opcode;
  - não existe contenda 68000↔VDP↔Z80: o único custo de barramento modelado é o FIFO cheio
    (`vdp/mod.rs:1286-1320`), e o FIFO é drenado **integralmente a cada passo**
    (`vdp/mod.rs:1374`), então ele nunca acumula entre instruções — na prática o modelo
    subestima o *stall* real de escrita em VRAM (hardware: ~16+ ciclos por palavra em display
    ativo, dependente de HBlank/DMA).
- O DMA é **eager** (toda a transferência dentro do passo) e `take_dma_cycles()` é chamado e
  **descartado** (`backend.rs:671-676`): a CPU **não** perde os ciclos do DMA. Um DMA de 20 KiB
  que no hardware custa ~milhares de ciclos de 68000 custa zero aqui.
- `Vdp::advance_master` usa aritmética absoluta `end/line_cycles` comparada com `self.line`
  (`vdp/timing.rs:126-160`): na virada de frame, `hblank`/`vblank` do `VdpTick` podem ser
  reportados para um intervalo que na verdade virou linha 0. O scheduler não consome
  `VdpTick.hblank/vblank` (gera os eventos por contagem de linhas,
  `interrupts.rs:466-499,636-653`), então **hoje não há bug observável**, mas há duas fontes de
  verdade para o mesmo evento esperando divergir.

### 3.3 Comparação de modelos de execução (E/O)

| Modelo | Onde aparece | Prós | Contras | Adequação ao ChronoFork |
|---|---|---|---|---|
| Por instrução (atual) | este baseline | simples, determinístico, tabela verificável por oráculo, rápido de implementar | sem contenda, sem latência fina, sem efeitos sub-instrução; impede 32X/Mega CD depois | **bom para M8.1–8.5**, insuficiente para 8.6 e para "precisão de hardware" como diferencial |
| Por ciclo (cycle-stepped, bus-cycle exact) | Exodus, BlastEm (parcial), jgenesis | modela DTACK/espera, FIFO, DMA, IRQ no ponto certo | 5–20× mais complexo; exige reescrever o 68000 (barramento por acesso), o VDP e o scheduler; custo alto em determinismo de performance, não em determinismo de resultado | necessário apenas se o produto vender "precisão de hardware" |
| Event-driven por domínio de clock (master clock + eventos agendados) | MAME (parcial), Ares (por chip), abordagem usual de 16-bit | mantém o custo perto do modelo por instrução e resolve a maior parte dos sintomas (DMA, FIFO, HBlank no dot certo, IRQ no evento) | exige disciplina de "nenhum estado derivado do host" e uma fila de eventos serializada no snapshot | **recomendado como alvo**: híbrido instrução + eventos |
| Híbrido (instrução para o 68000, ciclo para o VDP/Z80/áudio) | Genesis Plus GX, maioria dos emuladores maduros | equilíbrio real: 68000 por instrução com contenda aproximada; VDP/Z80/áudio por ciclo | precisa de "custo de barramento por acesso" injetado na instrução | **é o que o M8 precisa**; o baseline já tem 80 % da estrutura (timeline master, scheduler, VDP com line/dot) |

---

## 4. 68000 (`crates/chronofork-megadrive/src/cpu/`)

### 4.1 Organização (FO)

- `mod.rs` (2003 linhas): estado, `step`, exceções, helpers de flags; `decode.rs` classifica o
  opcode em `InstructionClass`; `ea.rs` implementa os modos de endereçamento; `exec/` divide por
  grupo (move/arith/logic/shift/bit/control/system); `timing.rs` é a tabela.
- `step` (`mod.rs:735-820`): busca opcode, grava trace, classifica, despacha; converte
  `IllegalOpcode`/`Unsupported` em vetor 4 e `AddressError` em vetor 3; qualquer outro erro é
  erro de host.
- Modos de endereçamento completos e com custo de EA tabelado (`ea.rs`, `timing.rs:33-59`);
  `MOVEM`/`MOVEP`, `EXG`, BCD, shifts de memória, `TAS`, `CHK`, `TRAP`, `LINK/UNLK`, `STOP`,
  `RESET`, `USP` e `MOVE to/from SR` presentes.
- Endianness big-endian correta; escritas multi-byte validam o intervalo inteiro antes de
  mutar (`bus.rs:455-482`), o que é uma boa propriedade de segurança de estado.
- Exceções: molduras de 6/14 bytes, `RTE` valida a moldura antes de mexer em estado
  (`mod.rs:857-884`), violação de privilégio para `MOVE to SR`/`STOP`/`RESET`/`RTE`/`USP`.

### 4.2 O que falta / limites (FO + I)

- **Bus error (vetor 2) e trace (vetor 9)** não são entregues ao guest; um fetch em região não
  mapeada continua sendo erro de host (`docs/architecture/megadrive-slice.md:283-292`). Para
  jogos comerciais isso é irrelevante; para fidelidade de exceções é uma lacuna.
- **Leitura não mapeada = exceção de endereço** (`ea.rs:311-317`): no Mega Drive real, endereços
  não decodificados devolvem *open bus* e **não** geram exceção. É o achado A2-F2 e é o motivo
  pelo qual os dois fixtures comerciais não saem do lugar.
- **Timing**: um único número por instrução, sem penalidade de *double bus cycle* otimizado,
  sem prefetch, sem `MOVE from SR` real (a tabela casa com o oráculo por decisão explícita,
  documentada em `megadrive-slice.md:120-127`). Para lógica de gameplay é suficiente; para TAS
  determinístico *do próprio emulador* é ótimo (é reprodutível); para "divergência de timing
  contra hardware" é insuficiente por construção.

---

## 5. VDP (`crates/chronofork-megadrive/src/vdp/`)

### 5.1 O que existe e está testado (FO)

- VRAM 64 KiB flat, CRAM 64 palavras, VSRAM 40, portas data/control/HV, auto-incremento,
  byte-lane, registro 0x00..0x17, status derivado do dot/linha (`mod.rs:553-586`).
- Compositor com planos A/B, window, scroll full/column/line, 80 sprites com colisão/overflow,
  prioridade de 6 níveis, shadow/highlight, H40/H32 (`render.rs`, testes `vdp_*`).
- DMA fill/copy/memory, FIFO de 4 palavras com *backpressure* contabilizado, pendências
  VBlank/HBlank pertencem ao VDP e são gated pelo registro 0x00.
- Snapshot `CFVDP002` com validação atômica (`vdp/mod.rs:1134+`).

### 5.2 Problemas concretos

**A2-F3 (P0) — DMA memory→VRAM lê a própria VRAM.** `start_dma` aceita `MemoryToVram` e, para
cada palavra, faz `self.read_vram_word(source + offset*2)` (`vdp/mod.rs:885-901`). A fonte do DMA
do Mega Drive nos modos 0/1 é um endereço de **memória 68000** (RAM/ROM), não VRAM. O resultado
não é erro: é VRAM copiada para VRAM, ou seja, gráficos plausíveis e errados. Um jogo que faz DMA
de tilemap/sprite na VBlank corrompe a VRAM silenciosamente. Isso viola a regra 8 de `AGENTS.md`
("Reject incompatible input; never silently reinterpret bytes") e contamina qualquer evidência
de pixels da fase 8.6.

**A2-F6 (P1) — não há *stall* de CPU por DMA nem por escrita no VDP.** `take_dma_cycles()` é
descartado no `step` (`backend.rs:671-676`); a escrita no data port custa o que a instrução custa.
No hardware, o custo de acesso ao VDP é a principal fonte de contenda do sistema e a base de
efeitos raster (Direct Color DMA, etc.). Sem ele, o jogo roda "rápido demais" em seções de DMA e
qualquer jogo que dependa de tempo de DMA para sincronismo diverge.

**A2-F5 (P1) — o frame é composto inteiro no fim do frame, a partir do estado final.** `capture_frame`
clona o VDP e chama `finish_frame()` (`backend.rs:723-731`), que compõe as 224/240 linhas com o
estado *daquele instante* (`render.rs:263-300`). Consequências: mudanças de VSRAM/CRAM/registros
feitas por handlers de HBlank no meio do frame **não** aparecem; o frame é retroativamente
"recombinado" com o estado final. É internamente determinístico (bom) e funcionalmente errado
para efeitos raster (água do Sonic, Gunstar, Castlevania, muitos jogos de corrida). A doc diz "o
compositor é uma função pura do estado e da timeline", o que é verdade e ao mesmo tempo enganoso:
não há *line buffer* histórico.

**A2-F7 (P2) — HBlank é gerado no fim da linha, não no início do blanking.** O scheduler conta
linhas concluídas (`interrupts.rs:636-653`) e emite HBlank quando a linha `< active` termina; o
hardware levanta HBlank em ~2560 master cycles dentro da linha (ou no dot 0xB6 do contador H). A
diferença é de até uma linha inteira na chegada da IRQ — inofensivo para jogos que só usam HBlank
para trocar paleta no início do blanking, fatal para efeitos com janela estreita.
Junto com o status HBlank derivado de `dot >= 2560` (`mod.rs:567-569`) e o contador H aproximado
(`vdp/timing.rs:166-180`, documentado), o conjunto é honesto mas limita "precisão de hardware".

**Nota (sem ação) — "um pendente por nível".** O scheduler coalesce pedidos do mesmo nível e guarda o
tick original (`interrupts.rs`, documentado). É uma aproximação razoável (o hardware tem latch por
fonte), mas significa que uma IRQ de HBlank perdida é definitivamente perdida — comportamento
correto — e que duas VBlanks não podem ficar pendentes — comportamento também correto. **Sem
ação**; registrado para não ser "descoberto" como bug depois.

---

## 6. Bus, arbitragem e relação CPU–VDP–Z80–áudio

- `bus.rs` (1143 linhas): ROM + mapper Sega + SRAM 64 KiB + TMSS + version register + RAM 64 KiB
  + VDP + dois registradores de controle. Todo o resto é `UnmappedRead`/`UnmappedWrite` tipado
  (`bus.rs:355-530`).
- **Janela de RAM**: `RAM_BASE = 0x00FF0000`, `RAM_END = 0x01000000` (`bus.rs:51-53`), e o teste
  de faixa é `(RAM_BASE..RAM_END).contains(&address)`. Todo endereço ≥ `0x01000000` — inclusive
  o espelho de hardware `0xE00000..0xFFFFFF` — é **não mapeado**. A máscara de 16 bits usada para
  indexar (`address & 0xFFFF`) só é alcançável dentro da própria janela de 64 KiB, logo **não
  existe espelhamento algum**. `docs/architecture/megadrive-slice.md:44` afirma "mirrored by
  address bits 15–0", o que é falso no código (A2-F11).
- **Arbitragem**: ausente por completo (não há Z80). `0xA00000`/`0xA11100`/`0xA11200` e o PSG
  `0xC00011` são erros tipados (`bus.rs:13`, teste em `bus.rs:833`). Para áudio/Z80 a fase 8.4
  terá de introduzir um domínio de clock novo (Z80 3.58 MHz, 1 master cycle a cada ~15) e um
  modelo de *bus request/grant* — não é uma extensão incremental do `step` atual, é um segundo
  timeline.
- **Controles**: só as leituras de byte em `0x00A10003/0x00A10005`, subconjunto TH-alto de
  3 botões; `A`/`Start` inertes (`docs/architecture/megadrive-slice.md`, testes `controller.rs`).
  Sem `Start` não há como sair de uma tela de título: **jogabilidade real exige a fase 8.5**.
- **PAL/NTSC**: constantes corretas e testadas (`cycles.rs`), status PAL derivado do padrão de
  vídeo, 313/240 linhas. O que falta é o *timing* PAL real de linha (3420 master cycles é usado
  para ambos; o hardware PAL usa ~3432) — aproximação documentada.

---

## 7. Save/restore (segurança e custo)

- Payload `CFMD68K5` v5 com RAM 64 KiB + VDP completo + cartucho (SRAM 64 KiB, bancos, SRAM
  control, TMSS) + scheduler; `restore` valida identidade, versão, config hash, magic, tamanho,
  snapshot do VDP, snapshot do scheduler e **recalcula o hash de estado** antes de comitar
  (`backend.rs:781+`, doc §Backend integration). Restore falho constrói o novo bus/scheduler antes
  de trocar — boa propriedade (estado antigo intacto).
- Custo: **197.234 bytes** por checkpoint (medido, E2) e 88,9 µs de serialização / 100,4 µs de
  restore. 64 KiB de SRAM e 64 KiB de VRAM entram sempre, mesmo quando não mudaram — 2/3 do
  payload é estático na maioria dos passos. Para rewind/fork em massa (a proposta de valor do
  projeto) isso é caro: 1 fork = 197 KB + 100 µs, e o `fork_current` faz `snapshot` +
  `fork_from_snapshot` + `snapshot()` de novo (`sessions/src/manager.rs:184-227`).
- **Recomendação**: separar estado "estrutural/imutável" (ROM, bancos, tamanho) do mutável e
  comprimir/deduplicar SRAM-zerada são otimizações de baixo risco, mas **só depois** de resolver
  A2-F8 (hash por passo), que é 100× mais impactante.

---

## 8. Níveis de precisão necessários

| Objetivo | Precisão mínima real | O que falta no baseline | Veredito |
|---|---|---|---|
| (a) Rodar jogos comerciais | decodificação de barramento tolerante (open bus), RAM espelhada, Z80+PSG, timers, Start/6-button, DMA real | A2-F1, A2-F2, A2-F3 + fases 8.4/8.5 | **hoje: 0 de 2 fixtures saem da primeira instrução/larva** |
| (b) Lógica de gameplay | 68000 completo + IRQ de VBlank/HBlank + RAM/ROM/SRAM corretos | quase tudo pronto; falta só não morrer em I/O não modelada | alcançável após A2-F1/F2 |
| (c) Interação CPU–hardware | custo de acesso ao VDP, FIFO com drenagem temporal, DMA com stall, contadores H/V úteis | A2-F6, A2-F7 | **maior lacuna estrutural**; exige modelo híbrido |
| (d) TAS determinístico | nada de host no estado canônico + snapshot completo + replay | **já atendido** (é o ponto forte do baseline) | excelente |
| (e) Divergências de timing | modelo por ciclo/eventos + tráfego de barramento sub-instrução | ausente por construção | decisão de produto, não bug |
| (f) Sistemas futuros (32X, Mega CD, MS) | barramento com múltiplos mestres e janelas; dispatcher por device | ausente | o modelo por instrução será o teto |

Compromissos (O): precisão × desempenho não são o trade-off dominante aqui — **o hash por passo
custa mais que a emulação** (§3/§9). O trade-off real é **precisão × complexidade × manutenção**:
ir a cycle-stepped agora multiplicaria o custo de 8.4–8.8 sem entregar jogabilidade. A rota de
menor risco é híbrida (instrução + eventos), com custo de barramento injetado por acesso.

---

## 9. Desempenho medido (E) — o primeiro obstáculo de engenharia

Comandos e saídas completas em `02-evidence/E2-bench.txt` e `E3-hash-probe.txt`.

- `bench_step` × 3 (release): **5.183 / 5.218 / 5.190 instruções/s** (mediana ≈ 5.190);
  `step_median` 2,135–2,170 s por 10.000 passos ⇒ **~214 µs por instrução**.
  `serialized_checkpoint_bytes = 197.234`, `final_tick = 87.052`, hashes idênticos entre runs.
- Probe próprio (exemplo temporário removido): dos 224 µs do passo, **84,7 µs são o SHA-256 do
  corpo de estado (37,8 %)** e **88,4 µs com a cópia intermediária (39,5 %)**; o hash roda a
  **2,3 GB/s** (SHA-NI ativo via `sha2`). Ou seja: ~40 % do passo é a observabilidade obrigatória,
  e o restante inclui construir o corpo (RAM 64 KiB + VRAM 64 KiB + SRAM 64 KiB + VDP snapshot
  alocado por passo) mais a instrução em si.
- Necessidade para tempo real: 7,67 MHz / 59,92 fps ≈ 128 mil ciclos/frame ≈ **~1,9 M
  instruções/s**. O baseline está **~366× abaixo**. Mesmo removendo 100 % do hash, ficaria ~3×
  abaixo do alvo por passo — porque o custo por instrução também está inflado por alocação.
- Efeito prático já observado: 500.000 instruções = **95,7 s** de parede (E4). Uma matriz de
  compatibilidade de 2 ROMs × 60 s emulados ≈ 105 mil frames ≈ 3,3 M instruções/ROM ≈ **10 min por
  ROM** mesmo sem áudio. Viável para laboratório, inviável para "interativo" e caro para CI.

**A2-F4 (P0) — o contrato `Backend::step -> StepOutcome { state_hash }` torna o hash obrigatório.**
`backend.rs:488-513` monta o corpo a cada passo; `state_body` (`backend.rs:1211-1245`) copia
197 KB; `hash_state_body` (`:1256-1260`) copia mais; `vdp.snapshot_bytes` (`vdp/mod.rs:1081-1125`)
aloca e escreve o VDP inteiro. Alternativas: (i) hash opt-in (`step_unhashed` + hash sob demanda,
testes de determinismo comparam hashes em fronteiras escolhidas); (ii) hash incremental por região
suja (RAM/VRAM pages + cabeçalho) com `state_hash = H(regions)` — mantém o contrato e corta ~95 %;
(iii) estado "imutável" fora do hash (SRAM imutável quando write-protected, etc.).
Recomendação: (i) + (ii) na fase 8.7; (iii) em paralelo. Custo/risco: mudança de trait público →
ADR + revisão; efeito em testes: todos os testes que dependem de hash por passo passam a chamar
uma API explícita.

---

## 10. Referências upstream (pesquisa; URLs, licença e maturidade)

Nenhum código foi copiado. Licenças verificadas na fonte indicada; "fonte" indica onde li a
licença, não o repositório completo.

| Projeto | URL | Versão/commit observado | Licença (verificada) | Maturidade | Instrumentação | Controle de execução/timing | Serialização de estado | Determinismo/replay/forks | Custo de integração |
|---|---|---|---|---|---|---|---|---|---|
| **Exodus** (Nemesis) | https://github.com/Nemesis/exodus · https://www.exodusemulator.com | clone local `08f388f` (2024-02-16); fork headless `ec005615` (2026-08-13, oracle pinado) | **MIT** desde 2024-01-23 (`License.txt` local; [anúncio](https://exodusemulator.com/about/news/39-return-to-active-development-now-mit-licensed)) | alta em precisão; projeto pequeno/1 autor; C++/Windows; VDP *cycle-exact*, 68000 ainda sub-opcode (declarado pelo autor) | excelente (é um debugger: trace, breakpoints, workspace) | **cycle-stepped**, o mais preciso da lista | sim (via core) | determinístico por construção; não é Rust | **alto** (C++, plugin Windows, sem build headless canônico — o fork headless do oráculo é o caminho) |
| **BlastEm** | https://www.retrodev.com/blastem/ · https://github.com/libretro/blastem | 1.0.0 (2026-09-11); upstream Mercurial | **GPL-3.0-or-later** ([licença](https://github.com/libretro/blastem)) | muito alta; referência de cycle-accuracy (Titan Overdrive 2) | debugger integrado + GDB remote | por ciclo, "fast enough for old hardware" | save states | determinístico; GPL contamina | **alto**; licença incompatível com MIT |
| **Genesis Plus GX** | https://github.com/ekeeke/Genesis-Plus-GX | master (Copyright até 2026) | **proprietária "non-commercial"** ([LICENSE.txt](https://github.com/ekeeke/Genesis-Plus-GX/blob/master/LICENSE.txt)): proíbe venda/uso comercial, exige fonte de derivados | altíssima (libretro, 8/16-bit) | limitada | híbrido, muito maduro | sim | **licença incompatível com projeto MIT e com uso comercial**; só como oráculo/referência de comportamento | **proibitivo para código** |
| **jgenesis** | https://github.com/jsgroth/jgenesis | 0.14.1 (2026-08-22) | **GPL-3.0** (README atual); a wiki registra "GPLv3, formerly MIT" — versões antigas MIT **precisam ser verificadas** antes de qualquer uso | alta e crescente; passa Nemesis VDP FIFO / direct color DMA / Titan Overdrive | tem "VDP State" view e ferramentas de debug de Genesis desde 0.12 | por ciclo, Rust | save states, rewind | determinístico; **única referência em Rust** | **médio-alto**: arquitetura mais próxima, mas GPL atual |
| **MAME** | https://github.com/mamedev/mame | 0.289 (2026-07-30) | projeto **GPL-2.0+**, >90 % dos arquivos **BSD-3-Clause**, incluindo `src/devices/cpu/m68000` (Musashi, `// license:BSD-3-Clause`, credit Karl Stenerud) ([docs](https://docs.mamedev.org/license.html)) | altíssima em cobertura, média em MD | excelente (debugger, `-debug`) | event-driven/agendado por device | save states completos | determinístico | **alto** (C++/framework); só arquivos BSD são reutilizáveis e com "approval from specific developers" explícito |
| **Ares** | https://github.com/ares-emulator/ares | v148 (2026) | **ISC** ([LICENSE](https://github.com/ares-emulator/ares/blob/master/LICENSE)) | alta; multi-sistema, foco em preservação | limitada | por chip, scheduler próprio | save states | determinístico | **alto** (C++/nall-hiro); licença permissiva é o ponto positivo |
| **BizHawk** | https://github.com/TASVideos/BizHawk | clone local `d148601` (2026-11-10) | **heterogênea**; o próprio `LICENSE` diz "should be considered an illegal combination of several incompatible GPL licenses" | altíssima para TAS | excelente (é a referência de TAS) | frontend hospeda cores de terceiros | savestates | é a referência de determinismo/TAS | **proibido** para reuso (política do projeto); usar como referência conceitual |
| **Nuked-OPN2** (YM2612) | https://github.com/nukeykt/Nuked-OPN2 | master (26 commits) | **LGPL-2.1-or-later** (cabeçalho de `ym3438.c/.h`) | altíssima (die-shot, cycle-accurate) | n/a | clock a clock (`OPN2_Clock`, 1 internal clock = 6 master) | estado interno exposto em struct | determinístico | **médio**: LGPL exige cuidados (linking/relink) e é C |
| **ymfm** (YM2612/PSG alternativo) | https://github.com/aaronsgiles/ymfm | — | **BSD-3-Clause** (citado por dois consumidores independentes) | alta | n/a | clock a clock | sim | determinístico | **baixo-médio**: BSD é compatível; C++ moderno |
| **Nuked-PSG** | https://github.com/nukeykt/Nuked-PSG | — | **GPL-2.0-or-later** (citado por consumidor) | alta | n/a | clock a clock | sim | determinístico | **alto** (GPL) — preferir ymfm/implementação própria para PSG |
| **Musashi / MAME m68000** | `mame/src/devices/cpu/m68000` | MAME master | **BSD-3-Clause** (`m68kcpu.cpp` header) | altíssima | limitada | por instrução com ciclo por acesso | estado em struct | determinístico | **médio**: é a referência de semântica e de tabela; traduzir para Rust é trabalho real mas bem definido |

Notas de honestidade: (i) não abri o código-fonte de BlastEm/Genesis Plus/jgenesis/MAME nesta
revisão além do cabeçalho de licença do m68000 do MAME; as colunas "instrumentação",
"serialização" e "controle de execução" vêm de documentação pública e do conhecimento geral, não
de execução — **confiança média**. (ii) O valor de commit exato das referências remotas não foi
pinado (sem rede para `git ls-remote`); os commits listados são os dos clones locais existentes.

---

## 11. Build vs integrate (recomendação)

**Recomendação: continuar build (Rust nativo) para 68000/VDP, integrar (via tradução ou
reimplementação guiada) apenas os blocos de áudio e o Z80, e nunca colar código GPL/non-commercial.**
Justificativa com evidência:

1. O 68000 já existe, está diferencial-testado 11/11 e é a parte de menor risco. Substituir por
   uma tradução do Musashi BSD trocaria um ativo por uma dívida (re-verificar 11 ROMs, contexto de
   exceção, privilégio).
2. O VDP atual tem um defeito *estrutural* (DMA errado, frame no fim do frame) mas também uma
   arquitetura de estado/ snapshot que é exatamente o que o projeto vende. Reescrever para
   cycle-exact agora custa a fase 8.4–8.6 e não é pré-requisito para jogar.
3. Áudio é o único bloco em que "escrever do zero" é claramente pior que integrar: um YM2612
   correto exige engenharia reversa (die shots). `ymfm` (BSD-3) é o candidato de menor risco
   legal; `Nuked-OPN2` (LGPL) é o de maior precisão e exige decisão explícita de licença
   (o projeto é MIT; LGPL é aceitável se isolada em crate própria com linking dinâmico, mas isso
   precisa de ADR e de aval do proprietário).
4. **Adaptador temporário** (a hipótese do enunciado): sim, vale — mas não como emulador
   embutido. O adaptador de maior valor/risco é um **oráculo externo por processo** (Exodus
   headless, já pinado como oráculo do projeto) para gerar *golden* de frame/áudio/estado e para
   diagnosticar divergências, mantendo o caminho canônico em Rust. Isso entrega valor antes da
   precisão total sem violar licença (MIT), sem integrar código de terceiros e sem quebrar o
   determinismo (o oráculo não entra no caminho de execução). O que **não** recomendo é embrulhar
   BlastEm/Genesis Plus como backend "temporário": GPL/non-commercial contaminam o produto e o
   adaptador viraria permanente por custo afundado.

---

## 12. Achados

Formato: **id — título**; descrição; evidência; componentes; condições; impacto atual/futuro;
probabilidade; severidade; confiança; alternativas; recomendação; custo/risco; efeito em
testes/contratos; prioridade; fase.

**A2-F1 — O espelho de RAM `0xE00000..0xFFFFFF` não existe; Kid Chameleon morre na 1ª instrução.**
`RAM_BASE..RAM_END = 0x00FF0000..0x01000000` e o teste de faixa exclui todo o resto (`bus.rs:51-53,395,465-482`);
não há máscara de espelho alcançável. O SSP de reset da ROM é `0xFFFFF7FE`; a primeira instrução
(`TST.L $00A10008`) gera exceção de endereço e o empilhamento da moldura de 14 bytes falha em
`0xFFFFF7FD` (“unmapped write”). **Evidência:** E4 (`debug run` exit 1; `debug memory` do vetor de
reset; `debug disasm 0x4B0`). **Componentes:** `bus.rs`, `cpu/mod.rs:390-450`,
`docs/architecture/megadrive-slice.md:44`. **Impacto:** nenhum jogo comercial que use o topo do
espelho (a maioria) sai do reset; a doc afirma o contrário do que o código faz. **Severidade:**
alta. **Confiança:** alta. **Alternativas:** (a) mapear `0xE00000..0xFFFFFF` e `0x000000`-mirror
corretamente (é o mapa documentado do 68000 no MD); (b) só aceitar o espelho no `validate_write_range`
(remendo, mantém assimetria); (c) não fazer nada — bloqueia a fase 8.6 inteira. **Recomendação:**
(a), incluindo o alias `0x000000`-`0x3FFFFF` já tratado como ROM e o espelho de RAM por bits
15..0 dentro de `0xE00000..0xFFFFFF`. **Custo:** baixo (tabela de janelas + 10 testes).
**Efeito em testes:** os testes de bus que exigem erro acima de `RAM_END` precisam ser reescritos
com o novo mapa — é uma mudança de contrato observável. **Prioridade: P0 — fase 8.6 (ou já em 8.5).**

**A2-F2 — A janela de I/O `0xA10000..0xA1001F` é tratada como erro fatal; os registradores de
controle `0xA10008..0xA1000F` (documentados) não existem, e leituras de endereços realmente não
decodificados deveriam ser open bus, não exceção.**
`controller_port` só mapeia `0xA10003`/`0xA10005` (`bus.rs:762-771`); `ea.rs:311-317` converte
`UnmappedRead`/`WriteProtected` em `CpuError::AddressError` (vetor 3).
**Correção de escopo em relação à minha primeira leitura (e à hipótese da t7):** a instrução que
trava o Chakan, `TST.L $00A10008`, **não** é uma leitura de área “morta”: `0xA10008` e `0xA1000C`
são os registradores de **controle das portas 1 e 3** (documentados no mapa de I/O do Mega Drive,
byte a byte `0xA10008..0xA1000F` = controle porta 1/2/3/EXT). A causa correta é “registrador
documentado não implementado”, não “open bus”. O problema geral de open bus continua real para
áreas de fato não decodificadas (ex.: `0xA10010..0xA1001F`, PSG/Z80 antes de 8.4), mas **não** é o
que trava o Chakan.
**Evidência:** E4 (execução, vetores 2–5 = `0x220`, VDP com todos os registradores 0x00–0x17 = 0
após 20.000 e 500.000 passos, traço com 19.998/20.000 registros em `0x220`/`0x60FE`); plano 8.5
Task 3, `docs/superpowers/plans/2026-11-17-8.5-timers-controllers.md:119` (promete
`0xA10008–0xA1000F`). **Impacto:** qualquer jogo que leia registradores de controle de porta (é o
caminho normal para configurar TH/6-button) morre no vetor 3 antes de tocar o VDP. **Severidade:**
alta. **Confiança:** alta. **Alternativas:** (a) implementar `0xA10008..0xA1000F` como
registradores reais (protocolo de 8.5) — é o que o plano já prevê; (b) devolver open bus
determinístico em toda a janela não implementada, adiando o protocolo; (c) manter o erro (bloqueia
8.6). **Recomendação:** (a) na fase 8.5 **e** (b) como regra geral para o resto da janela e do
mapa (com contadores de diagnóstico por janela). Ver A2-F14 para um conflito no plano 8.5.
**Prioridade: P0 — fase 8.5 (parte do caminho de boot).**

**A2-F3 — DMA memory→VRAM lê a VRAM (reinterpretação silenciosa).**
`vdp/mod.rs:885-901`. Ver §5.2. **Severidade:** alta. **Confiança:** alta (código). **Alternativas:**
(a) implementar o lado 68000 do DMA (o VDP precisa de uma porta de leitura da memória do host no
momento do DMA, o que exige callbacks ou um “barramento do VDP” na máquina); (b) rejeitar
`DmaMode::MemoryToVram` com erro tipado até (a) existir (mantém a regra 8 e falha alto); (c) manter
como está (inaceitável). **Recomendação:** (b) imediatamente — é uma mudança de 5 linhas que troca
“pixels errados em silêncio” por “erro tipado” — e (a) na fase 8.6. **Custo:** (b) trivial;
(a) médio (introduz uma dependência VDP→memória que deve ser resolvida no backend, não no VDP, para
preservar a direção de dependências). **Efeito em testes:** `vdp_dma` precisa de um caso de rejeição;
a doc de capabilities deve dizer “DMA fill/copy apenas”. **Prioridade: P0 — patch imediato.**

**A2-F4 — O hash canônico obrigatório por passo custa ~38 % e limita o produto a 5.190 instr/s.**
Ver §9. **Severidade:** alta (produto: não há como atingir 8.6/8.7/8.8). **Confiança:** alta
(medido). **Alternativas:** (i) hash opt-in; (ii) hash incremental por região suja; (iii) cache do
corpo serializado com *dirty flags*; (iv) fazer nada (366× abaixo de tempo real).
**Recomendação:** (i)+(ii)+(iii) combinados na fase 8.7, com um teste de invariante
“hash incremental == hash completo” em todos os testes existentes. **Custo:** médio;
**risco:** o hash é a base do determinismo — qualquer otimização precisa do teste de igualdade
como rede. **Efeito em testes/contratos:** muda `Backend` (traço público) → ADR; todos os testes de
determinismo passam a comparar em fronteiras explícitas. **Prioridade: P0 — fase 8.7 (bloqueia
8.6 na prática, por custo de laboratório).**

**A2-F5 — Frame composto inteiro no fim do frame a partir do estado final.**
`backend.rs:723-731`, `render.rs:263-300`. Sem efeitos raster (HBlank/line/coluna/paleta por
linha); HBlank handlers que mudam VSRAM/CRAM no meio do frame não aparecem no frame. **Severidade:**
alta para 8.6 (compatibilidade/qualidade visual), média para as fases atuais.
**Confiança:** alta. **Alternativas:** (a) renderizar por scanline no evento de linha (line buffer
canônico, 320×240 bytes = 77 KB extra no snapshot ou derivável); (b) manter o render “pós-hoc” e
documentar que efeitos raster não são suportados; (c) renderizar por linha apenas quando o VDP
mudou no meio do frame (complexidade sem ganho). **Recomendação:** (a), guardando o line buffer
como estado derivado canônico (precisa entrar no snapshot para fork/rewind — decisão de ADR:
77 KB ou recomputar). **Custo:** médio; **efeito:** `vdp_frame`/`vdp_compose` ganham casos raster;
a doc de capabilities muda de “frame determinístico” para “frame determinístico por scanline”.
**Prioridade: P1 — fase 8.6.**

**A2-F6 — CPU não paga DMA nem escrita em VDP (sem contenda).**
`backend.rs:671-676` descarta `take_dma_cycles()`; `write_data_port` custa a instrução.
**Severidade:** média-alta. **Confiança:** alta. **Alternativas:** (a) contabilizar o custo do DMA
no `step` (simples: somar `dma_cycles` ao delta, como já é feito com `fifo_wait`);
(b) modelo de barramento por acesso (cycle-stepped); (c) não fazer nada. **Recomendação:** (a)
agora — é a mesma fórmula já usada para FIFO e remove uma inconsistência grave (DMA grátis) com
risco mínimo de determinismo; (b) fica para a decisão de precisão. **Custo:** baixo.
**Efeito:** o tick avança diferente ⇒ **todos os hashes dourados mudam** ⇒ exige bump de versão e
re-geração de evidência (custo de coordenação, não de engenharia). **Prioridade: P1 — 8.6.**

**A2-F7 — HBlank gerado no fim da linha; status/contadores H/V aproximados.**
`interrupts.rs:636-653`, `vdp/mod.rs:567-569`, `vdp/timing.rs:15-26,166-180`. **Severidade:** média.
**Confiança:** alta. **Alternativas:** (a) emitir HBlank no dot 2560 e VBlank na linha 224 (evento
por linha, não por “linha concluída”); (b) manter e documentar. **Recomendação:** (a) junto com
A2-F5 (mesmo mecanismo de evento por linha). **Custo:** baixo-médio. **Efeito:** hashes dourados
mudam; testes de scheduler precisam de nova fronteira. **Prioridade: P2 — 8.6.**

**A2-F8 — Três timelines redundantes e estado de VBlank duplicado.**
`tick` (backend), `scheduler.ticks`, posição line/dot do VDP, mais `sync_vdp_vblank`
(`backend.rs:515-518`) e `sync_vdp_vint_pending`. **Severidade:** baixa hoje, média no futuro
(divergência silenciosa quando 8.4 adicionar o Z80). **Confiança:** alta (código).
**Alternativas:** (a) uma timeline única (master cycle no scheduler) e posições derivadas;
(b) manter e adicionar teste de invariante `tick == scheduler.ticks == f(vdp position)`.
**Recomendação:** (b) agora (1 teste), (a) quando 8.4 introduzir o segundo domínio.
**Prioridade: P3.**

**A2-F9 — `Cartridge` clonado por máquina; hash de ROM refeito por criação.**
`backend.rs:302-323` clona a `Cartridge` (ROM) por backend. **Severidade:** baixa (memória 8×ROM
por fork). **Confiança:** alta. **Alternativas:** `Arc<Cartridge>`/`Arc<[u8]>` com copy-on-write do
estado mutável (SRAM/bancos/TMSS). **Recomendação:** `Arc` na fase de performance (8.6/8.7),
porque com forks em massa e 4 MiB de ROM isso escala mal. **Prioridade: P2.**

**A2-F10 — Sem Z80/PSG/timers, nenhum jogo comercial progride além do init de áudio.**
`bus.rs:13`, `bus.rs:833`. **Severidade:** alta (produto) / esperada (fase 8.4). **Confiança:** alta.
**Alternativas:** nada a decidir agora, exceto a **ordem**: recomendo 8.5 (timers/controles)
**antes** de 8.4 (áudio), porque sem `Start` não há jogabilidade e sem timers metade dos jogos não
avança do título; o áudio é o bloco mais caro e mais isolável. **Prioridade: P1 — decisão de fase.**

**A2-F11 — Deriva de documentação em afirmações técnicas específicas.**
(a) `megadrive-slice.md:44` afirma RAM “mirrored by address bits 15–0” — falso (§6/A2-F1);
(b) `megadrive-slice.md:210-219` afirma “DMA memory-to-VRAM … pre-validated with typed errors” e
“The DMA source is a VRAM address in this slice” — o código aceita o modo e copia VRAM→VRAM
(A2-F3), o que é diferente de rejeitar; um leitor da doc conclui “não suportado com erro”, o código
faz “suportado com dados errados”. **Severidade:** média (governança/documentação). **Confiança:**
alta. **Recomendação:** corrigir as duas frases e adicionar uma linha “limitações silenciosas”
na página canônica; o padrão do repositório (uma fonte de verdade por afirmação) já existe, o
problema é a afirmação não bater com o código. **Prioridade: P1 (rápido, alto valor).**

**A2-F12 — `VdpTick.hblank/vblank` calculados em aritmética absoluta podem reportar o intervalo
errado na virada de frame** (`vdp/timing.rs:126-160`). Hoje sem consumidor (o scheduler conta
linhas), mas é uma armadilha para quem for implementar A2-F5/F7. **Severidade:** baixa;
**Confiança:** média (não escrevi um teste que exponha; a leitura do código é clara).
**Recomendação:** derivar os flags de `(start, end)` relativos e adicionar teste de fronteira.
**Prioridade: P3.**

**A2-F13 — O 68000 não trunca o endereço para 24 bits antes do acesso ao bus.**
O `M68000` mantém endereços em `u32` e o bus recebe o valor cru (`bus.rs:355-530`); não há
`& 0x00FF_FFFF` em ponto algum. No hardware o 68000 tem 24 linhas de endereço (A1–A23 + UDS/LDS),
portanto **não é possível** colocar um endereço >`0xFFFFFF` no barramento: o truncamento é
propriedade elétrica da CPU, não decisão de decodificação.
**Evidência:** E4 (SSP de Kid Chameleon `0xFFFFF7FE`; rejeição em `0xFFFFF7FD`); `bus.rs:51-53,455-482`.
**Efeito demonstrado por aritmética (fato):** `0xFFFFF7FE & 0x00FFFFFF = 0x00FFF7FE`, dentro da
janela `0x00FF0000..0x01000000`; logo o mesmo push passaria a cair em RAM `0xFFF7FA` e a moldura de
exceção seria escrita. **Onde deve ocorrer:** no **bus** (fronteira elétrica), num único ponto
aplicado a `read8/16/32`, `write8/16/32`, `peek8`, `write_memory` e ao endereço gravado na moldura de
exceção — e **não** na escrita de registradores, porque `A7 = 0xFFFFF7FE` é um valor de registro
legítimo. O bit 0 é preservado, então os testes de paridade/alinhamento não mudam.
**Impacto:** sozinho **não** faz nenhuma das duas ROMs bootar (Kid Chameleon passaria a entrar no
próprio handler em `0x2D0`: `MOVE.W #3,D0; RTE` — e re-executaria a instrução faltosa para
sempre, ~34 ciclos por exceção), mas remove o *host abort* e é pré-requisito de qualquer boot.
**Severidade:** alta. **Confiança:** alta. **Alternativas:** (a) máscara no bus (recomendado);
(b) máscara na CPU (duplica o ponto e deixa o debugger inconsistente); (c) só alargar a janela de
RAM para `0xE00000..0xFFFFFF` sem máscara (não cobre `0xFFFFF7FE`? cobre, mas não cobre PC/leituras
em `0x01000000..` e mantém a assimetria). **Recomendação:** (a) + mapa de janelas de A2-F1.
**Custo:** baixo. **Efeito em testes:** novos testes de bus para `0x01000000`→`0x00000000`
(ROM, `WriteProtected`), `0xFFFFF7FE`→RAM, e um teste de CPU para push em `A7` alto.
**Prioridade: P0 — imediato (8.4/8.5).**

**A2-F14 — O plano 8.5 mapeia os registradores de controle mas mantém “porta 3 e além” como erro.**
`docs/superpowers/plans/2026-11-17-8.5-timers-controllers.md:119` promete
`0xA10008–0xA1000F` e, na mesma frase, “port 3 and beyond stay typed errors”.
`0xA1000C` (controle da porta 3) está **dentro** de `0xA10008–0xA1000F` e é exatamente um dos dois
endereços que o Chakan lê na primeira instrução (`TST.L $00A10008`, depois `TST.W $00A1000C`) —
ver E4. Se “porta 3” virar erro, o Chakan continua morrendo. **Severidade:** média (é um plano, não
código). **Confiança:** alta. **Recomendação:** separar no plano “registrador de **controle** de
porta 3/EXT = stub legível/escritível que devolve 0” de “**dados** de porta 3/EXT = erro tipado
enquanto o hardware não existir”; e adicionar um passo de teste com o padrão exato do Chakan
(leitura long de `0xA10008` e word de `0xA1000C`). **Prioridade: P1 — antes de executar 8.5.**

---

## 13. O que não consegui verificar (limitações)

1. **Não** executei o branch `agent/8.4-z80-audio` (proibido pelo enunciado) — o contexto dele não
   entra em nenhum achado acima; a avaliação é do baseline.
2. **Não** medi o custo isolado da execução da instrução (sem hash/serialização): o probe mede o
   hash e a cópia, e o restante é atribuído por subtração. Precisaria de um `step` sem hash (que é
   justamente A2-F4) ou de instrumentação no crate.
3. **Não** validei os contadores H/V contra hardware nem contra o oráculo: a doc já declara
   aproximação; um teste diferencial exigiria o oráculo expor os contadores.
4. **Não** comparei pixels com nenhum emulador externo (o oráculo headless não expõe VRAM/CRAM
   segundo a própria documentação de evidência).
5. Licenças de referências remotas: verifiquei por página oficial/consumidores independentes;
   não fixei commits remotos (sem `git ls-remote` no ambiente). Os commits de Exodus/BizHawk são
   dos clones locais.
6. Não testei PAL em máquina real nem o caminho `MediaSource` de dois frames.

## 14. Perguntas ao capitão/critic

1. **Ordem 8.4 vs 8.5**: recomendo inverter para 8.5 (timers/controles) antes de 8.4 (áudio) —
   concordam? Isso muda o roadmap e é decisão do proprietário.
2. **Licença de áudio**: `ymfm` (BSD-3) vs `Nuked-OPN2` (LGPL-2.1+). O projeto é MIT; integrar
   LGPL exige ADR e decisão explícita. Preciso do direcionamento antes de qualquer recomendação
   final na área de áudio.
3. **Política de "open bus"** (A2-F2): aceitar leitura determinística em janelas não implementadas
   é mudança de contrato observável. Confirmam que isso deve entrar como requisito de 8.6?
4. **Hash por passo**: a mudança de `Backend` é transversal (sessions/service/transport/GUI). Se a
   área de arquitetura (t1) já propõe rever o trait `Backend`, este achado deve ser consolidado lá
   e não duplicar ADR.

## 15. Evidência executada (índice)

| Arquivo | Conteúdo |
|---|---|
| `02-evidence/E1-demos.txt` | `chronofork demo` e `chronofork megadrive demo` completos (exit 0) |
| `02-evidence/E2-bench.txt` | `bench_step` × 3 (release) + ambiente |
| `02-evidence/E3-hash-probe.txt` | probe de hash/cópia (saída + fonte do exemplo temporário, removida da árvore) |
| `02-evidence/E4-rom-boot.txt` | tentativa de boot Kid Chameleon/Chakan, vetores de reset, disasm, saída de `debug run`, estado VDP/interrupções, traço |
| `02-evidence/E5-references.md` | tabela de licenças/URLs com a fonte de cada licença |

Comandos principais (todos em `../../../../` = worktree raiz):

```sh
CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=/tmp/chronofork-review-target \
  /tmp/chronofork-review-target/release/chronofork demo
CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=/tmp/chronofork-review-target \
  /tmp/chronofork-review-target/release/chronofork megadrive demo
CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=/tmp/chronofork-review-target \
  cargo run -q -p chronofork-megadrive --example bench_step --release --locked
... debug run --rom "/home/stealthc/projects/ChronoFork/.research/roms/Kid Chameleon (USA, Europe).bin" --steps 20000
... debug run --rom "/home/stealthc/projects/ChronoFork/.research/roms/Chakan (USA, Europe).bin" --steps 500000
```

---

## 16. Confirmação cruzada com a área de produto (t7), a pedido do capitão

Recebi a pergunta de confirmação com três pontos. Reproduzi cada um. Respostas:

**(0) Confirmações.** (i) `Chakan` PC de reset `0x0000333E`, primeira instrução `4AB9 00A10008` =
`TST.L $00A10008`: **confirmado** (E4). (ii) `Kid Chameleon` SSP `0xFFFFF7FE`, PC `0x000004B0`:
**confirmado** (E4). (iii) O bus só mapeia `0xA10003`/`0xA10005` na janela de I/O
(`bus.rs:762-771`) e só `0x00FF0000..0x01000000` de RAM (`bus.rs:51-53`): **confirmado**.
(iv) “exceção de endereço, vetor 3, stub em PC=0x220”: **confirmado com uma precisão** — o vetor 3
do Chakan aponta para `0x00000220` (`debug memory --address 0x08 --length 16`), e o traço mostra o
CPU em `0x220` com opcode `0x60FE` (`BRA.S -2`) desde o índice 16; os vetores 2, 4 e 5 apontam para
o mesmo `0x220`, ou seja, é o *stub de trap da própria ROM*, não código do jogo.
(v) “o push em `0xFFFFF7FD` vira bus error”: **parcialmente** — não houve escrita parcial: o
`raise_exception` valida o intervalo **antes** de escrever (`cpu/mod.rs:407-411` →
`bus.rs:455-482`), e o endereço reportado (`0xFFFFF7FD`) é o **fim** do intervalo
(`end = 0xFFFFF7F0 + 14 - 1`), não o endereço de uma escrita que falhou. Correção importante para
não sugerir corrupção de estado.

**(a) O 68000 deve mascarar o endereço para 24 bits antes de qualquer acesso? Onde?**
**Sim** — e é uma propriedade da CPU, não uma escolha de decodificação: o 68000 tem 24 linhas de
endereço, então valores acima de `0xFFFFFF` não existem no barramento. **Onde aplicar:** no **bus**,
num único ponto de normalização usado por `read8/16/32`, `write8/16/32`, `peek8`, `write_memory` e
pelo endereço gravado na moldura de exceção (o hardware grava o endereço truncado). **Não** na
escrita de registradores: `A7 = 0xFFFFF7FE` é estado de registro legítimo e deve continuar assim.
Efeito verificado por aritmética: `0xFFFFF7FE & 0x00FFFFFF = 0x00FFF7FE` (RAM). Ver A2-F13.
Correção associada: o mapa de janelas (A2-F1) precisa cobrir `0xE00000..0xFFFFFF` explicitamente,
porque mascarar e depois decodificar só uma janela de 64 KiB ainda funciona para o Kid Chameleon
(`0xFFFFF7FE` → `0x00FFF7FE`) mas deixa `0xFF0000..0xFFFFFF` sem alias, contra o mapa do hardware.

**(b) `0xA10008–0xA1000F` precisa ser mapeado como registrador legível para o boot do Chakan?
O plano 8.5 cobre isso de fato?**
**Sim, precisa — e são registradores de CONTROLE, não “open bus”.** `0xA10008/09`, `0xA1000A/0B`,
`0xA1000C/0D`, `0xA1000E/0F` são os controles das portas 1, 2, 3 e EXT (byte a byte), documentados
no mapa de I/O do Mega Drive, e são legíveis/escritíveis no hardware (reset = 0). O `TST.L` do
Chakan lê controle 1+2; o `TST.W` lê controle 3+EXT. **O plano 8.5 cobre em parte:** a Task 3
promete exatamente `0xA10008–0xA1000F` (`docs/superpowers/plans/2026-11-17-8.5-timers-controllers.md:119`),
mas a mesma frase diz que “port 3 and beyond stay typed errors”, e `0xA1000C` é controle da porta 3
— conflito real, registrado como A2-F14. **Discordo, portanto, da hipótese “mapear como open bus”**:
o comportamento correto aqui é implementar o registrador; open bus é a regra para o que é de fato
não decodificado (A2-F2, parte (b) da recomendação).

**(c) Impacto em 8.4/8.5/8.6 e gates de boot recomendados.**
- **8.4 (Z80/bus/áudio) — impacto alto.** O boot de qualquer jogo comercial passa por upload do
  driver de som: escrita em `0xA00000..` (janela do Z80), `0xA11100` (bus request) e `0xC00011`
  (PSG), hoje todas erros tipados (`bus.rs:13,833`). Sem isso o jogo para no init de áudio. Além
  disso, 8.4 deve incluir a normalização de 24 bits (A2-F13) e a política de open bus (A2-F2),
  porque são pré-requisitos baratos e transversais.
- **8.5 (timers/controles) — impacto alto e é o desbloqueio mais próximo.** Controle de porta
  1/2/3/EXT + `Start`/`A` no latch TH. Sem `Start` não há como sair da tela de título; sem os
  controles de porta o Chakan nem sai da primeira instrução.
- **8.6 (compatibilidade) — é onde o custo aparece.** Precisa de DMA real (A2-F3), frame por
  scanline (A2-F5), contenda por acesso ao VDP (A2-F6), HBlank no dot certo (A2-F7) e o hash
  opt-in/incremental (A2-F4) para tornar a matriz de compatibilidade executável em tempo aceitável
  (hoje: ~10 min por ROM por passe de 3,3 M instruções).
- **Gates de boot recomendados (por fase, executáveis e baratos):**
  - **G-BOOT-0 (8.4, hoje):** para cada fixture, `debug run --rom <abs> --steps 300000` termina com
    `status=stopped reason=step-limit` (**nenhum erro de host**). Critério atual: **falha**
    (Kid Chameleon aborta na 1ª instrução).
  - **G-BOOT-1 (8.5):** no mesmo orçamento, `pc` final **fora** de `0x000000..0x0002FF`,
    `vdp_register_00..07` **diferentes de zero** (o jogo inicializou o VDP) e pelo menos um
    `VRAM` não nulo. Hoje: Chakan falha (todos os registradores 0, PC `0x220`).
  - **G-BOOT-2 (8.6):** frame renderizado com hash registrado + áudio não silencioso + matriz
    por ROM, em script **local-only** que sai com `skip` quando os fixtures não existem
    (nunca em CI, nunca commitando ROM).
  - **G-PERF (transversal, 8.7):** `bench_step ≥ 60 x 10^3 instr/s` como piso intermediário e
    ≥ 1,9 M como alvo de tempo real; declarar o número no relatório de fase.
  Não recomendo um gate “boot completo em N frames” antes de 8.6: com 5,19 k instruções/s o
  orçamento de parede inviabiliza o CI e mede a coisa errada.
