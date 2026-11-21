# A7 — Produto, planejamento e governança documental

**Revisão:** ChronoFork, baseline `b8d63a1` (`origin/main`, 2026-11-20).
**Worktree:** `.agents/worktrees/review-2026-11-20-architecture`.
**Autor:** product (revisor independente, área 7).
**Método:** leitura de visão/escopo/non-goals, `README.md`, `AGENTS.md`, `docs/README.md`,
`docs/planning/*`, `docs/roadmap/*`, `docs/requirements/*`, `docs/epics/*`, ADRs, specs/planos
M8; estado real do GitHub via `gh`; execução do CLI `release` do próprio baseline contra os
fixtures locais (read-only, sem copiar/commitar bytes) e contra o backend sintético.
**Classificação usada:** **[fato]** observado/executado, **[experimento]** rodado para medir,
**[hipótese]**, **[inferência]**, **[opinião]**. Confiança: alta/média/baixa.
**Tempo:** ~75 min. Nenhum arquivo versionado foi alterado.

---

## 0. Sumário executivo

1. **O fluxo KidEngine não é executável em software real hoje.** Nenhum dos dois ROMs curados
   passa da *primeira* instrução no baseline: *Chakan* morre no vetor de erro do convidado e
   estaciona em `BRA *` (`PC=0x00000220`); *Kid Chameleon* falha com "68000 bus error: unmapped
   write at 0xFFFFF7FD" na primeira exceção. **[fato, confiança alta]** (comandos na §4).
   A justificativa do M8 ("ROMs comerciais jogáveis") tem, portanto, **zero evidência executada**
   e fases 8.4–8.8 estão planejadas sem nenhum gate que meça progresso em ROM real.

2. **A plataforma (sessões, checkpoints, forks, artefatos) funciona — mas só no backend
   sintético.** O `chronofork demo` prova checkpoint → 3 forks → mutação de memória →
   `compare` com divergência → replay; o `script run` prova checkpoint → 2 forks → hashes →
   restauração no backend Mega Drive. **[fato]** O que não existe é o caminho de *software
   real*: injeção de input por passo e escrita de memória não têm superfície no CLI/DSL/serviço
   para o backend real; checkpoint durável de sessão real só existe via `serve`, e `store.save`
   não persiste uma sessão viva. **[fato, ver §3]**

3. **Desempenho é um gate de produto, não um detalhe de 8.6.** Medido no baseline: **1,19 s por
   frame** NTSC de um ROM real (12.799 instruções/frame) ⇒ **≈0,84 fps**, ~**71× abaixo** de
   60 fps. **[experimento, confiança alta]** 8.7 (pacing/captura) e 8.8 (janela nativa) são
   inúteis antes de um piso de desempenho explícito e medido.

4. **A documentação de status é a maior fonte de risco de decisão do projeto.** 89 arquivos
   Markdown versionados, 38.165 linhas; **13 arquivos** repetem estado de milestone e **15**
   repetem status de fase em prosa; `cb05836` aparece **49×**, `0c4e9f0` **24×**, `4c595a2`
   **18×**; o estado do 8.3/rendering aparece em **6 versões diferentes**, incluindo
   `README.md:83` ("phase 8.2"), `README.md:199` ("rendering=false"),
   `docs/architecture/overview.md:178,181` ("no rendering/DMA"), `docs/epics/EPIC-0002…:252`
   (diz que rendering/DMA continuam abertos) e `docs/README.md:7` ("M0–M5 complete").
   **[fato]** Para responder "roda jogo comercial?" um agente lê ~6 arquivos, ~2.700 linhas e
   recebe 3 respostas contraditórias; a resposta correta só sai de código/testes.

5. **Recomendação de roadmap (não "continuar como está"):** manter 8.4; **revisar o escopo de
   8.5** para cobrir a janela de I/O inteira (incluindo *leituras* e semântica de open bus);
   **dividir 8.6** em compatibilidade (com gate de boot por fase) e desempenho; **inserir um
   milestone de consolidação de produto** com 5 jornadas de usuário e piso de fps; antecipar as
   interfaces agênticas que já existem mas estão presas no CLI (M6-FU1); **adiar 8.8 (window
   nativo) e M9** para depois de o valor ponta a ponta estar provado.

6. **Governança:** uma única fonte de status (`docs/roadmap/milestones.md`), prosa de status
   substituída por *links* + um bloco de status gerado; ADRs e planos de aceitação viram
   documentos históricos imutáveis com banner; `STATE.md` reduzido a um handoff de 10 linhas
   com branch/PR/next action; `docs/quality/*acceptance*` deixa de parecer status atual.
   **[opinião, confiança alta]** Detalhe na §7 e decisões na §9.

---

## 1. Escopo, fontes e o que foi executado

Lido integralmente: `docs/vision/{product-vision,scope,non-goals}.md`,
`docs/planning/{ROADMAP,MILESTONE,STATE}.md`, `README.md`, `docs/README.md`, `AGENTS.md`,
`docs/roadmap/{roadmap,milestones}.md` (seções de status, M8, M9, H1–H5, Riscos),
`docs/requirements/functional.md`, `docs/architecture/overview.md`,
`docs/epics/EPIC-0002-megadrive-slice.md`, `docs/adr/README.md` + ADR-0011/0012/0013,
specs/planos 8.5/8.6/8.8, `docs/superpowers/specs/2026-11-17-milestone-8-megadrive-functional-design.md`,
`docs/quality/rom-fixtures.md`.

Executado (binário `release` do baseline em `/tmp/chronofork-review-target/release/chronofork`,
`CARGO_HOME=/tmp/t7-cargo`; ROMs lidos por caminho absoluto, read-only):

```text
megadrive cartridge --rom ...            (2 ROMs: header/checksum OK)
megadrive boot --rom ... --frames 1      (Chakan: 1,19 s; Kid: erro)
debug inspect/run/memory/disasm --rom    (estado, frame de exceção, disasm)
script run --rom ...                     (checkpoint, 2 forks, restore, report)
serve --rom + api call                   (session.create/run/snapshot/fork/inspect
                                          /store.save/experiment.run)
tas record/verify --rom                  (10 passos)
gui run --rom                            (create/attach/step/frame)
demo, experiment run                     (backend sintético)
gh issue/pr/milestone list               (projeção GitHub)
```

Fora de escopo por desenho: editar código/docs versionados, commits, alterar fixtures.

---

## 2. Documentação × estado real (drift) e custo de navegação

### 2.1 Drift material (afeta decisão)

| # | Local | Afirmação | Estado real / evidência | Gravidade |
|---|---|---|---|---|
| D1 | `README.md:83` | "M8 … In progress (phase 8.2)" | 8.3 completa e mergeada (PR #29, 2026-11-19; `docs/roadmap/milestones.md:51`) | média |
| D2 | `README.md:199` | "the backends still report `rendering=false` and `audio=false`" | Mega Drive reporta `rendering=true` (`crates/chronofork-megadrive/src/backend.rs:256`; `session.inspect` executado) | média |
| D3 | `README.md:206-210` | "Rendering, DMA, audio …, ROM header/region detection and cartridge mapping … still missing" | rendering/DMA entregues no 8.3 (FR-MD-007, PR #29); cartucho/header no 8.1 | alta |
| D4 | `docs/architecture/overview.md:178` | "VDP status/control/VRAM port window — Implemented (no rendering/DMA)" | rendering/DMA implementados (8.3) | alta |
| D5 | `docs/architecture/overview.md:181` | "VDP rendering/DMA, audio, Z80, controller protocol … Not implemented" | rendering/DMA implementados; áudio/Z80/controller parcial (correto) | alta |
| D6 | `docs/README.md:7` | "Bootstrap foundation and milestones M0–M5 are complete and merged" | M0–M7 mergeados (`cb05836`, PR #6) — o mesmo arquivo registra M6/M7 como completos mais adiante | média |
| D7 | `docs/epics/EPIC-0002-megadrive-slice.md:252-256` | "epic remains open for … rendering and frame timing, VDP data reads, DMA …" | 8.3 fechou rendering/DMA; epic virou documento histórico sem banner | média |
| D8 | `docs/requirements/functional.md:65` (FR-BACKEND-007) | "Partial — minimal 68000 + ROM/RAM slice implemented" | 8.2/8.3 entregues; a evidência da mesma linha está desatualizada | baixa |
| D9 | `docs/adr/0013-human-facing-gui.md:430` | "No rendering and no audio. Both backends report `rendering=false`…" | decisão (sem toolkit) segue válida; a *descrição* foi superada pelo 8.3 e não há nota de emenda | média |
| D10 | `docs/planning/STATE.md:8-14` | "8.3 … o PR é o registro de integração" (escrito antes do merge) | PR #29 mergeado 2026-11-19T22:16Z | baixa |
| D11 | `docs/quality/*acceptance*.md`, `quality-gates.md` | várias linhas "rendering=false", "no rendering/audio" | corretas **na data**; sem banner "HISTÓRICO — não é status atual", são lidas como estado corrente | média |
| D12 | `docs/planning/STATE.md` (todo) | único handoff; não representa trabalho não-mergeado | checkout pai está em `agent/8.4-z80-audio` com 14 commits locais; de `main` isso é invisível e um novo sessão leria "next: start 8.4" | alta (coordenação) |

Não é drift: `docs/README.md:22` e FR-MD-009 dizem que a entrega de *bus error* (vetor 2) e
*trace* (vetor 9) ao convidado está ausente; o experimento com *Chakan* confirma que o acesso
não mapeado a `0xA10008` vira **address error (vetor 3)**, exatamente como FR-MD-009 descreve.
**[fato]**

### 2.2 Duplicação e custo de navegação (quantificado)

| Métrica | Valor | Fonte |
|---|---|---|
| Arquivos `*.md` versionados | **89** | `git ls-files '*.md'` |
| Linhas Markdown versionadas | **38.165** | `wc -l` |
| Linhas em `docs/` | **37.000** | idem |
| Documentos que repetem status de milestone/fase em prosa | **13** (e **15** citam "phase 8.x", incluindo specs/planos históricos) | grep |
| Menções a `cb05836` / `0c4e9f0` / `4c595a2` | **49 / 24 / 18** | grep |
| Fontes de "o que está pronto" | **3 camadas**: lifecycle (`docs/planning/`), registro de capacidade (`docs/roadmap/`), prosa (README, docs/README, AGENTS, vision, requirements, overview, epics) | leitura |
| Respostas diferentes para "rendering implementado?" | **6 documentos** divergem | D2–D5, D7, D9 |
| Custo para responder "roda ROM comercial?" | 6 arquivos, ~2.700 linhas, 3 respostas contraditórias, decisão final exige código/teste | [inferência, confiança alta] |
| Custo de revalidar status por sessão nova | ~20–40 min de leitura + risco de decidir com texto velho | [inferência, confiança média] |

**Conclusão [opinião, confiança alta]:** o problema não é "documentação demais"; é **status
duplicado em prosa** em 13 lugares com ciclos de atualização diferentes. O registro canônico
(`milestones.md`) está correto e bem mantido; o resto é cópia parcial que envelhece. O custo é
pago em toda sessão nova, humana ou agente, e já produziu pelo menos uma decisão errada no
próprio README (subestimar 8.3) e uma superestimação no overview (negar rendering).

---

## 3. Fluxo KidEngine ponta a ponta: o que existe, o que falta

Pergunta de produto: *carregar ROM → rodar até gameplay → checkpoint → N forks → alterar
inputs/variáveis → rodar cenários → comparar divergências → inspecionar
registradores/memória/instruções → identificar rotina/condição → relatório reproduzível.*

| # | Etapa | Estado no baseline | Superfície | Evidência / limite |
|---|---|---|---|---|
| 1 | Carregar ROM | **Existe** | `--rom` em `debug`, `trace capture`, `tas *`, `script run`, `serve`, `gui run`; `megadrive cartridge` valida header | header/checksum OK nos 2 ROMs; não há comando único "abrir jogo" |
| 2 | Rodar até gameplay | **Ausente** | só `--steps N`/`--frames N` (este último só no `megadrive boot`, que é sintético); `session.run` limitado a 100.000 passos | nenhum "run until PC/condição/frame"; `boot` espera o *marcador sintético* `0x224` e aceita ROM arbitrário sem avisar |
| 3 | Checkpoint | **Parcial** | `script checkpoint()`, `serve session.snapshot`; CLI `session save` é só sintético (não aceita `--rom`); `store.save` não recebe sessão de origem | snapshot real medido: 197.250 bytes, hash `f8ef45…` |
| 4 | Múltiplos forks | **Parcial** | `script fork()`, `serve session.fork`; não há fork no CLI de sessão | experimento: 2 forks de 5.000 passos → hashes idênticos `72b32f…` (sem sinal porque não há input/mutação) |
| 5 | Alterar inputs/variáveis | **Ausente no backend real** | `InputFrame` existe; `script step()` usa input **neutro**; não há builtin de input nem de escrita; `debug` não tem `write`; `session.write_memory`/`Backend::write_memory` existem na biblioteca, usados só no `demo` sintético | contradiz diretamente o item 6 da visão ("run different inputs or memory mutations in each branch") |
| 6 | Rodar cenários | **Parcial** | DSL com `repeat/while/if`; TAS record/play | sem run-to-condition, sem batch de cenários, sem paralelismo |
| 7 | Comparar divergências | **Existe** | `tas verify/compare` (primeiro tick divergente), `trace compare` (primeiro registro), `session.compare`, `script trace_compare/tas_compare` | funciona; sem sinal no ROM real porque os ramos são idênticos |
| 8 | Inspecionar registradores/memória/instruções | **Existe, com defeito** | `debug inspect/memory/disasm/vdp/interrupts/bus`; `script register/read`; `serve session.inspect/read_memory` | **disassembler não decodifica instruções válidas**: `0x4AB9` (`TST.L abs.L`) sai como `DC.W $4AB9` (visto em 0x333E/0x4B0) — o mesmo encoding que o CPU executa |
| 9 | Identificar rotina/condição | **Parcial** | breakpoints PC/range + condição `reg:`/`mem:`; watchpoints; eventpoints (interrupt/VDP/controller/bus-write); logpoints | sem símbolos, CFG, call-stack, step-over; sem trace de exceções/`address error`, que é justamente o que mata *Chakan* |
| 10 | Relatório reproduzível | **Parcial/assimétrico** | `script run` imprime `record=script_report` determinístico; `serve experiment.run` gera `chronofork.experiment.v1` (funciona com `--rom`); `experiment run` do CLI é **só sintético** | sem PNG/WAV (8.7); sem relatório de sessão com trilha de inspeção; mesma capacidade existe em uma superfície e não na outra |
| 11 | Compartilhar artefatos | **Parcial** | `.cfmovie`, `.cftrace`, `.cfexp.json`; TSV one-way | sem bundle de evidência (trace + movie + relatório + hashes) e sem schema de relatório de investigação |

### 3.1 Assimetria CLI × serviço × DSL (fonte de fricção de produto)

- `experiment run` (CLI): sintético apenas; `experiment.run` (serviço): funciona com `--rom`
  (artefato `81b680…`, checkpoints em 0/50). **[fato]**
- `session.*` (serviço): create/run/fork/snapshot/compare/inspect funcionam com ROM; o CLI
  `session save` **não** aceita `--rom`. **[fato]**
- Store durável: `serve --store-root` habilita `store.*`, mas `store.save` deriva a sessão da
  *factory* + `steps`/`buttons`; não há parâmetro de sessão viva/fork/snapshot. Logo, um
  histórico de investigação ramificado **não é persistível**. **[fato, `crates/chronofork-service/src/service.rs:280-290`, `durable::save`]**
- Capacidade `memory_write=true` é anunciada (`session.inspect`) mas nenhuma superfície de
  produto escreve memória no backend real. **[fato]**

---

## 4. Evidência executada com os ROMs locais (read-only)

Fixture: `/home/stealthc/projects/ChronoFork/.research/roms/` (nunca copiado/commitado).

### 4.1 *Chakan (USA, Europe)* — morre na 1ª instrução

```text
$ chronofork megadrive cartridge --rom "…/Chakan (USA, Europe).bin"
system=SEGA GENESIS  region=U  checksum_stored=0x4BBD  checksum_computed=0x4BBD  checksum_ok=true  PASS

$ chronofork debug inspect --rom "…/Chakan…bin" --steps 1
pc=0x00000220  sr=0x2700  tick=238  state_hash=ffbecf03df919e2744671d197c4c0c8226fba9cb55872fb027168f6892596ad7

$ chronofork debug disasm --rom "…/Chakan…bin" --steps 0 --address 0x333E --count 3
0000333E  4AB9  DC.W $4AB9      ← na verdade TST.L ($00A10008).L
```

Vetor de reset: `SSP=0x00FFB436`, `PC=0x0000333E`; vetores 2..N apontam para `0x220`, cujo
conteúdo é `BRA *` (stub fatal). Frame de exceção lido em `0x00FFB428` após 1 passo:

```text
00FFB428  27 00 | 00 00 33 44 | 00 A1 00 08 | 4A B9 | 00 15 | 00 00
          SR=0x2700  PC=0x3344   acesso=0x00A10008  IR=0x4AB9  status  função
```

**[fato, confiança alta]** A primeira instrução `TST.L $00A10008` (leitura da janela de I/O não
mapeada, além dos data ports `0xA10003`/`0xA10005`) gera exceção de grupo 0 entregue ao
convidado (vetor 3, *address error*, conforme FR-MD-009), e o jogo cai no handler fatal
`0x220`. 200.000 passos depois ainda está em `0x220` (tick 14.000.168, 18,45 s). A política
"todo outro acesso de I/O é erro tipado" (`crates/chronofork-megadrive/src/bus.rs:9-11`) é
incompatível com a primeira instrução do fixture curado.

### 4.2 *Kid Chameleon (USA, Europe)* — vetor de reset anômalo

```text
$ chronofork megadrive boot --rom "…/Kid Chameleon…bin" --frames 1
chronofork: backend execution error: 68000 bus error: unmapped write at 0xFFFFF7FD   (exit 1)
$ chronofork debug memory --rom … --address 0 --length 8
00000000  FF FF F7 FE 00 00 04 B0   ← SSP=0xFFFFF7FE (fora da work RAM), PC=0x4B0
```

Header/checksum válidos (`checksum_ok=true`, região JUE). **[fato]** O vetor 0 não é um SSP
plausível (esperado `0x00FFxxxx`); a primeira exceção empurra o frame para `0xFFFFF7FD`, fora
do mapa. **[hipótese, confiança média]** o dump tem os 4 primeiros bytes corrompidos ou é uma
variante patcheada; não validei contra um hash externo. Independentemente disso, a lição de
produto é que **fixtures nunca foram bootados**: se fossem, o problema apareceria no 8.1, não na
revisão.

### 4.3 O que *funciona* com o ROM real (para não subestimar a base)

- `tas record/verify --rom` (10 passos) → `verified=1`, hashes idênticos. **[fato]**
- `script run --rom` com `open/step(20000)/register("pc")/state_hash/checkpoint/fork×2/restore`
  → `status=passed`, `steps=30000`, `report.restored_pc=544`, hashes de fork idênticos. **[fato]**
- `serve --rom` + HTTP: `session.create` → `run 20000` → `snapshot` (197.250 B) → `inspect` →
  `fork` (lineage depth 1) → `experiment.run` (artefato com 2 checkpoints). **[fato]**
- `gui run --rom`: create/attach/step/frame; `attach` diz `rendering=true`, rodapé do frame diz
  `rendering=unavailable` (v1 não configura fonte de vídeo). **[fato]**

### 4.4 Desempenho medido (release, 3 corridas)

```text
megadrive boot --rom Chakan --frames 1 → 12.799 instruções, 896.098 master cycles
run1 wall=1.18 s | run2 wall=1.19 s | run3 wall=1.19 s
```

**≈0,84 fps** contra 60 fps ⇒ **~71× mais lento**; o baseline bench (5.009 instr/s, hash de
estado completo a cada passo, checkpoint 197.234 B) é ainda mais lento. **[experimento,
confiança alta]** Qualquer promessa de "jogável", 8.7 e 8.8 depende de resolver isto.

---

## 5. Casos de uso end-to-end propostos (pequenos, demonstráveis, com valor)

| ID | Jornada | Critério de sucesso executável | Bloqueio atual |
|---|---|---|---|
| **UC-1 Boot smoke** | Carregar 1 ROM, rodar 1 frame, reportar PC, frame hash, VDP status | sai da região de reset; `PC` não é stub fatal; frame RGBA com hash; artefato `cfexp` | já falha nos 2 fixtures; falta gate por fase |
| **UC-2 Divergência de input** | checkpoint no frame N → fork A/B → input distinto por K passos → comparar | `session.compare` com ticks/hashes distintos e primeiro tick divergente no relatório | input por passo inexistente no backend real |
| **UC-3 Caça à rotina de input** | breakpoint condicional em leitura de `0xA10003`/`0xA10005`, trace de 300 instruções, export TSV | relatório com PC, opcode, tick, sequência de acessos e hash reproduzível | trace não tem exceções; disassembler falha em `0x4AB9` |
| **UC-4 Mutação de variável** | checkpoint → fork → escrever 1 byte de variável (ex.: vidas) → rodar → comparar | hash muda e a diferença é atribuída ao endereço escrito | escrita de memória ausente do produto |
| **UC-5 Determinismo cross-mode** | mesmo cenário unpaced/paced/step | 3 hashes de frame/áudio/estado iguais + PNG/WAV pinados | 8.7 não existe |

**Valor:** UC-1 é o gate que impede o projeto de "fechar" M8 sem software real; UC-2/UC-4 são a
tese do produto (fork + mutação + comparação); UC-3 é o que um engenheiro reverso compra; UC-5 é
a garantia de qualidade que diferencia de emuladores de GUI. Todos são pequenos o suficiente para
caber em uma fase.

---

## 6. Avaliação do roadmap

### 6.1 Ordem atual e problemas

Ordem vigente: 8.4 (Z80/áudio) → 8.5 (timers/controllers) → 8.6 (compatibilidade+perf) → 8.7
(modos/captura) → 8.8 (janela nativa) → M9 (multi-backend). Problemas:

1. **Nenhum gate de ROM real antes de 8.6.** As fases 8.4 e 8.5 podem fechar "verdes" com os
   fixtures ainda morrendo na 1ª instrução. O DoD do M8 mede fases, não jornadas. **[fato]**
2. **8.5 não cobre leituras da janela de I/O.** O design fala em *writes* aos control registers
   `0xA10008–0xA1000F` e no version register, mas *Chakan* faz **read** `TST.L $A10008` e morre.
   Sem open bus/leitura modelada, 8.5 "completa" e o ROM continua morto. **[fato + inferência,
   confiança alta]**
3. **8.6 assume que os ROMs bootam** ("boot reaches the controller-response probe on both
   fixtures"). Hoje o primeiro passo do plano é impossível; a fase inteira é uma aposta.
   **[fato]**
4. **Desempenho entra tarde e sem número.** 8.7/8.8 dependem de ~60 fps; o baseline mede
   0,84 fps. O roadmap trata performance como item de 8.6, sem piso, sem orçamento, sem
   benchmark de regressão no CI. **[fato + opinião]**
5. **A interface agêntica — o diferencial — está congelada.** Debug/trace/TAS/script ficaram
   CLI-local (M6-FU1) e não estão agendados em nenhuma fase do M8; o KidEngine é exatamente um
   consumidor agêntico. Antecipar M6-FU1 (pelo menos `session.inspect/read_memory`, trace e
   debug read-only) entrega valor de produto antes do fim do emulador. **[opinião, confiança alta]**
6. **M9 é prematuro** enquanto o primeiro sistema não é jogável; e **8.8 (janela nativa) é o
   item de menor valor por unidade de risco** para o público-alvo declarado (agentes/engenharia
   reversa), além de exigir ADR que supersede ADR-0013.

### 6.2 Alternativas avaliadas

| Alternativa | Prós | Contras | Veredito |
|---|---|---|---|
| A. Continuar como está | zero replanejamento; fases já destrinchadas | mantém aposta sem evidência; 8.6 pode falhar tarde; perf e valor agêntico no fim | **rejeitar** |
| B. Reordenar sem milestone novo (gates dentro de 8.5/8.6) | barato; preserva numeração | não cria compromisso de valor; fácil de contornar | aceitável como mínimo |
| C. **Reordenar + milestone de consolidação (recomendado)** | valor demonstrável cedo; força UC-1..UC-5 e piso de fps; separa "plataforma" de "precisão do core" | +1 checkpoint; exige owner decidir DoD por valor | **recomendado** |
| D. Trocar o core por emulador existente | precisão comercial rápida | abandona a identidade e o DoD do M8; licença/proveniência; não resolve forks/snapshots | **rejeitar** |
| E. Adaptador temporário de um emulador existente só para provar o fluxo (investigar) | valida plataforma+UCs com software real antes do core | risco de escopo/licença/determinismo; pode virar dependência | **investigar, não adotar** |

### 6.3 Proposta de sequência (mínima, com gates)

```text
8.4 Z80/áudio (mantém)
8.5 Timers/controllers + janela de I/O COMPLETA (reads+writes, open bus, IRQ)
    + gate UC-1 (boot smoke) rodando em CI local com fixtures ignorados
8.6a Compatibilidade: matriz por título, cenários boot/intro/play, UC-1 por fase
8.6b Desempenho: orçamento explícito (ex.: ≥30 fps headless no fixture, ≥60 fps com
     skip de hash por passo), benchmark de regressão no CI, sem quebrar determinismo
8.6c CONSOLIDAÇÃO DE PRODUTO (novo): UC-2/UC-3/UC-4 + artefato reproduzível +
     relatório de investigação + piso de fps + documentação de status reconciliada
8.7 Modos/captura (PNG/WAV)    → 8.8 janela nativa (último, opcional)
M9 multi-backend (somente após o valor acima provado)
M6-FU1 (interfaces agênticas: debug/trace/TAS no serviço) antecipado para junto de 8.6c
```

### 6.4 Critérios de conclusão por valor (substituir contagem de fases)

Proposta de DoD do milestone de consolidação: (i) UC-1 e UC-2 passam em ROM real com evidência
versionada; (ii) UC-3 produz relatório reproduzível com trace+hashes; (iii) UC-4 muda estado por
mutação explícita e compara; (iv) piso de desempenho medido e benchmark no CI; (v) os cinco
artefatos (movie/trace/exp/report/frame) são byte-idênticos em reexecução; (vi) uma sessão nova
consegue reproduzir tudo lendo **um** documento. **[opinião]**

### 6.5 Backend existente / adaptador temporário (investigação, não adoção)

- `exodus-for-ais` (`linux-headless`, MIT no root, `ec005615`) já é usado como **oráculo**
  out-of-process (`exodus-probe`/`exodus-mcpd`) e expõe `System::SaveState/LoadState` para
  arquivo (`System/System.h:74-75`) — logo, um adapter atrás de `Backend` poderia, em tese,
  fazer snapshot/fork. **[fato, inspeção read-only]**
- Riscos: determinismo (host/áudio/timing do processo), serialização binária opaca, versão,
  licença (inventário por arquivo ainda é pergunta aberta em `docs/licensing/provenance.md:128`),
  e o risco político de o adaptador virar o produto. **[inferência, confiança alta]**
- **Recomendação:** spike time-boxed de 2–3 dias, fora da árvore, **somente** para rodar UC-1/UC-2
  e medir o valor do produto; não entra no DoD do M8, não é dependência, não é commitado. Se o
  spike mostrar que o fluxo é valioso, a decisão seguinte é investir no core próprio (não no
  adaptador). Alternativa mais barata: usar o oráculo apenas para *validação de boot* do core
  (comparar PC/frame após N ciclos), sem adapter de runtime.

---

## 7. Governança documental: divisão de responsabilidades proposta

### 7.1 Problema

Três camadas (`planning/` execução, `roadmap/` capacidade, prosa) e 13 arquivos repetem o mesmo
status com ciclos de atualização diferentes. O resultado já custou decisões erradas (D2–D5) e
torna cada sessão nova mais caras. O antídoto não é mais documentação: é **um dono por fato**.

### 7.2 Regra de ouro proposta

> **Cada fato tem um único dono. Os demais artefatos linkam, nunca copiam.**
> Status/integração/evidência → `docs/roadmap/milestones.md`. Intenção de estágio →
> `docs/roadmap/roadmap.md` (1 página). Execução da fase ativa → `docs/planning/ROADMAP.md`.
> DoD do milestone ativo → `docs/planning/MILESTONE.md`. Handoff → `docs/planning/STATE.md`.
> Decisão → ADR. Requisito → `docs/requirements/`. Evidência de fase → acceptance plan datado.
> Trabalho → Issue/PR. Scratch → `.planning/`.

### 7.3 Papel de cada artefato (o que pode e o que não pode conter)

| Artefato | Dono do quê | Pode conter | **Não pode** conter | Atualização |
|---|---|---|---|---|
| `docs/roadmap/milestones.md` | **único** status, integração, evidência, follow-ups | tabela de status + blocos por milestone + follow-ups | prosa de intenção de estágio (link) | por fechamento de fase/milestone |
| `docs/roadmap/roadmap.md` | intenção de estágio | escopo, exit criteria conceituais | status, SHAs, "in progress" | raro (decisão de rumo) |
| `docs/planning/ROADMAP.md` | execução do milestone ativo | fases abertas, issues, planos | histórico de milestones fechados (1 linha + link), evidência | por fase |
| `docs/planning/MILESTONE.md` | DoD do milestone ativo | DoD, modelo de execução | status de fases (link) | ao definir/ajustar DoD |
| `docs/planning/STATE.md` | handoff | **≤10 linhas**: fase, branch/PR, próxima ação, blocker, trabalho não-mergeado visível | histórico, prosa de status, SHA de merges antigos | ao pausar/retomar |
| `docs/adr/NNNN-*.md` | decisão na data | contexto, decisão, consequências, **emendas datadas** | status corrente de capacidade | imutável; emenda aditiva |
| `docs/requirements/*.md` | requisito + status + link de evidência | 1 linha de status, 1 link | narrativa histórica de verificação (link p/ acceptance) | quando o requisito muda |
| `docs/quality/*acceptance*.md`, `quality-gates.md` | evidência datada | resultados brutos, contagens, hashes | qualquer afirmação de status atual | **imutável**, com banner |
| `docs/superpowers/{specs,plans}` | plano da fase ativa | design/plano | status realizado (vai para milestones) | ao abrir/fechar fase |
| `README.md` | vitrine | status em **1 bloco gerado** + 1 link | SHAs repetidos, listas de capacidades em prosa | junto com milestones |
| `docs/README.md` | índice | 1 linha de status + mapa | seção de status longa (remover) | raro |
| `AGENTS.md` | instruções de agente | 1 parágrafo de estado + link; regras | status detalhado por fase | raro |
| `docs/vision/*` | visão/escopo/non-goals | princípios | status de entrega (link) | raro |
| `docs/epics/*` | histórico do epic | critérios originais | status atual (banner "histórico") | congelar |
| Issues/PRs | trabalho | discussão, aceite | fonte de verdade copiada de volta | fluxo normal |

### 7.4 Simplificações concretas (baratas)

1. **Bloco de status gerado.** Em `milestones.md`, um bloco `<!-- status:begin -->…<!-- status:end -->`
   com a tabela M0–M9; um script (`scripts/check-docs-status.sh`) verifica que o mesmo bloco em
   `README.md`/`docs/README.md`/`AGENTS.md` é **byte-idêntico** (ou que não há cópia). Falha no
   `check.sh` quando divergir.
2. **Banner histórico obrigatório** em `docs/quality/*acceptance*`, `quality-gates.md`,
   `EPIC-0001/0002` e ADRs antigos: "HISTÓRICO — status atual em milestones.md".
3. **Remover SHAs de prosa.** Um link para o PR/milestone substitui `cb05836` repetido 49×.
4. **`STATE.md` ≤10 linhas + tabela "unmerged work"** (branch, base, commits, dono, risco de
   colisão). Isso resolve D12 sem inventar sistema novo.
5. **`roadmap.md` reduzido a 1 página** (ou fundido em `milestones.md` com âncoras). Hoje são
   675 + 1.013 linhas com sobreposição grande.
6. **Uma regra de aceitação por fase:** "o fase só fecha quando o UC/gate correspondente passa";
   evidência vive no acceptance plan, não na prosa.
7. **Congelar epics** (são históricos) e parar de atualizá-los.

**[opinião, confiança alta]** Isso corta ~30–40% da prosa de status sem perder informação,
elimina as 6 versões de "rendering", e transforma o custo de navegação de "ler 6 arquivos" em
"ler 1 bloco + 1 link".

---

## 8. Achados estruturados

> Formato: id, título, descrição, evidência, componentes, condições, impacto atual/futuro,
> probabilidade, severidade, confiança, alternativas, recomendação, custo/risco, efeitos em
> testes/contratos, prioridade, milestone/fase.

### P-01 — Nenhum ROM curado chega a gameplay; DoD do M8 sem evidência executada
- **Descrição:** *Chakan* morre na 1ª instrução (`TST.L $00A10008` → exceção de grupo 0 →
  stub fatal `0x220`); *Kid Chameleon* falha na 1ª exceção (SSP `0xFFFFF7FE` → push em
  `0xFFFFF7FD`). Não há gate que meça boot de ROM em nenhuma fase.
- **Evidência:** §4.1/4.2; comandos e saídas citados; `pr-details` do M8 ("curated commercial
  ROMs play") e `milestones.md:51/706-775`.
- **Componentes:** backend Mega Drive (bus/I-O), fixtures, roadmap M8.
- **Condições:** baseline `b8d63a1`, ROMs locais read-only.
- **Impacto atual:** a afirmação central do M8 não tem prova; 8.6 está construída sobre uma
  premissa não verificada. **Futuro:** se 8.5 fechar sem gate, o projeto descobre o problema
  tarde, com o cronograma já consumido.
- **Probabilidade:** certa (observado). **Severidade:** blocker de produto. **Confiança:** alta.
- **Alternativas:** (a) seguir e descobrir em 8.6; (b) gate UC-1 já no 8.4/8.5; (c) redefinir
  "jogável" para um conjunto menor de títulos.
- **Recomendação:** (b) — adicionar gate de boot smoke por fase, rodando localmente com fixtures
  ignorados; registrar no acceptance plan.
- **Custo/risco:** baixo (script + 1 teste por fase); risco de ruído se o ROM for inválido —
  validar o fixture primeiro.
- **Efeitos:** novo teste de integração opcional (ignorado sem ROM), DoD do M8 ganha evidência.
- **Prioridade:** **P0**. **Milestone/fase:** M8 / 8.4–8.6.

### P-02 — Fluxo KidEngine não é executável no backend real (input/memória/checkpoint durável)
- **Descrição:** faltam superfícies de produto para (i) input por passo, (ii) escrita de memória,
  (iii) checkpoint durável de sessão real, (iv) fork no CLI. A visão promete "run different
  inputs or memory mutations in each branch"; isso só existe no demo sintético.
- **Evidência:** §3 tabela linhas 3–5; `script step()` neutro (ADR-0011 §3.5); ausência de
  `write` no CLI/DSL/serviço; `store.save` sem parâmetro de sessão viva
  (`crates/chronofork-service/src/service.rs:280-290`); experimento §4.3.
- **Componentes:** `chronofork-cli`, `chronofork-script`, `chronofork-service`, `chronofork-sessions`.
- **Condições:** sessão Mega Drive com ROM.
- **Impacto atual:** o produto não demonstra sua tese em software real; agentes não conseguem
  rodar o caso de uso central. **Futuro:** cada consumidor (agente/humano) reimplementa por fora
  ou desiste.
- **Probabilidade:** certa. **Severidade:** alta. **Confiança:** alta.
- **Alternativas:** (a) nada; (b) adicionar builtins `input(mask)`/`poke(addr,bytes)` no DSL
  (limites já existem) e um `session.write_memory` no serviço; (c) expor TAS como único caminho
  de input.
- **Recomendação:** (b) — DSL + 2 operações no serviço, reutilizando `Backend::write_memory`
  e `InputFrame`; documentar no ADR-0011 como emenda.
- **Custo/risco:** médio (contrato/serialização/hashes: input não entra no snapshot — já é
  assim); risco de "script vira linguagem de emulador" — manter escopo de 2 builtins.
- **Efeitos:** +2 operações no catálogo (17→18), novos testes de determinismo, atualização de
  acceptance M5/M6.
- **Prioridade:** **P1** (P0 para o valor do produto). **Milestone/fase:** consolidação / 8.6c.

### P-03 — Desempenho: 0,84 fps medido; nenhum piso/orçamento no roadmap
- **Descrição:** 1,19 s/frame para *Chakan*; 12.799 instruções/frame; ~71× abaixo de 60 fps.
  O bench registra 5.009 instr/s por causa do hash completo por passo (197 KB).
- **Evidência:** §4.4; `docs/quality/anti-slop-refactor-evidence.md` §7.6/§8.5 (banda histórica).
- **Componentes:** backend (hash/serialização), roadmap 8.6/8.7/8.8, CI.
- **Impacto atual:** 8.7/8.8 não podem entregar experiência; "jogável" é falso. **Futuro:**
  retrabalho grande se a arquitetura de hash for assumida como contrato.
- **Probabilidade:** certa. **Severidade:** alta. **Confiança:** alta.
- **Alternativas:** (a) otimizar depois; (b) orçamento + benchmark de regressão já em 8.6b;
  (c) separar hash canônico opcional do caminho de execução.
- **Recomendação:** (b)+(c): piso explícito (ex.: ≥30 fps headless no ROM, ≥60 fps com hash
  amostrado) e bench obrigatório no CI antes de 8.7.
- **Custo/risco:** médio; risco de quebrar a promessa "hash a cada passo" — decidir se o hash
  por passo é contrato público ou ferramenta de verificação.
- **Efeitos:** NFR/contrato de step, testes de determinismo cross-mode.
- **Prioridade:** **P1**. **Milestone/fase:** M8 / 8.6b.

### P-04 — Status duplicado em 13 documentos com 6 versões de "rendering"
- **Descrição:** README/overview/docs-README/epic/ADR-0013/requirements divergem sobre 8.3 e
  rendering; 49 menções a `cb05836`.
- **Evidência:** §2.1/2.2.
- **Componentes:** documentação, AGENTS/README, ADRs, epics.
- **Impacto atual:** decisões erradas, retrabalho de leitura por sessão. **Futuro:** cresce a
  cada fase; agentes novos herdam fatos errados.
- **Probabilidade:** certa. **Severidade:** média-alta. **Confiança:** alta.
- **Alternativas:** (a) deixar; (b) prosa + links + bloco gerado (recomendado); (c) congelar
  todos os docs e manter só milestones.
- **Recomendação:** (b) com drift guard no `check.sh`.
- **Custo/risco:** baixo-médio (1 passe de edição + 1 script); risco de edição ampla — fazer
  por arquivo com revisão.
- **Efeitos:** `scripts/check.sh`, convenção `AGENTS.md` §Lifecycle, README/docs-README.
- **Prioridade:** **P1**. **Milestone/fase:** consolidação.

### P-05 — `megadrive boot` é sintético mas aceita ROM arbitrário sem avisar
- **Descrição:** o comando espera o marcador `0x224` do `cartridge_boot_rom()`; com ROM
  comercial ele falha (Kid) ou relata `reached_marker=false` (Chakan) sem indicar que o
  contrato é sintético; o help diz "no commercial image is needed" mas não "não use uma".
- **Evidência:** `chronofork megadrive boot --help`; execuções §4.1/4.2.
- **Componentes:** CLI (`crates/chronofork-cli/src/main.rs:288-442`).
- **Impacto atual:** usuário/agente conclui que "o emulador não boota" ou, pior, que "bootou"
  quando o marcador não foi atingido. **Futuro:** confusão em demos/documentação.
- **Probabilidade:** alta. **Severidade:** média. **Confiança:** alta.
- **Alternativas:** (a) documentar; (b) renomear para `boot-fixture`/`boot-synthetic`;
  (c) fazer `--rom` comercial mudar o contrato (boot até N frames + relatório de PC/frame).
- **Recomendação:** (c): separar `boot` (fixture) de `open` (ROM real: roda N frames, reporta
  PC/tick/VDP/frame hash, sem marcador). É a base do UC-1.
- **Custo/risco:** baixo; mexe em CLI/docs/testes.
- **Efeitos:** novos testes de CLI, help, README; nenhum contrato de backend.
- **Prioridade:** **P2**. **Milestone/fase:** consolidação / 8.6a.

### P-06 — Fixtures curados nunca foram bootados; proveniência de Kid suspeita
- **Descrição:** o README de fixtures e o milestone aceitam Kid/Chakan como alvo de
  compatibilidade, mas o vetor 0 de Kid é implausível e nenhum teste executa os arquivos.
- **Evidência:** §4.2; `docs/quality/rom-fixtures.md`.
- **Impacto atual:** 8.6 pode perseguir um dump inválido. **Futuro:** conclusões de
  compatibilidade contaminadas.
- **Alternativas:** (a) manter; (b) validar hash/estrutura e substituir o fixture se necessário;
  (c) escolher 2–3 títulos com dumps verificados.
- **Recomendação:** (b)+(c), registrando SHA-256 e origem (sem bytes no repo).
- **Prioridade:** **P1** (barato e evita retrabalho). **Milestone/fase:** M8 / 8.4.

### P-07 — Roadmap sem milestone de consolidação e com critérios de fase, não de valor
- **Descrição:** a ordem 8.4→8.8 trata o valor de produto como consequência automática; 8.8
  (janela nativa) e M9 são planejados antes de existir um fluxo real ponta a ponta.
- **Evidência:** §6; `ROADMAP.md:97-186`; M8 DoD em `milestones.md:752-764`.
- **Impacto atual:** risco de "M8 completo" sem software real e sem usuário. **Futuro:** alto
  custo de correção e perda de confiança.
- **Alternativas:** §6.2 (A–E). **Recomendação:** C (reordenar + consolidação), adiando 8.8/M9.
- **Custo/risco:** replanejamento e possível desconforto do proprietário; sem custo técnico.
- **Efeitos:** `ROADMAP.md`, `MILESTONE.md`, `milestones.md`, issues #17–#21/#7–#11.
- **Prioridade:** **P0** (decisão de rumo). **Milestone/fase:** M8 (planejamento).

### P-08 — Estado do 8.3/README subestima o entregue; overview nega rendering
- **Descrição:** agrupamento de D1–D5: a documentação de entrada contradiz o registro canônico.
- **Evidência:** §2.1. **Severidade:** média. **Confiança:** alta.
- **Recomendação:** corrigir no mesmo passe de P-04; adicionar ao drift guard frases proibidas
  ("no rendering", "phase 8.2") fora de documentos históricos.
- **Prioridade:** **P2**. **Milestone/fase:** consolidação.

### P-09 — Assimetria CLI × serviço × DSL para as mesmas operações
- **Descrição:** `experiment.run` e `session.*` existem no serviço com ROM, mas não no CLI;
  `session save` não aceita `--rom`; `experiment run` do CLI é sintético.
- **Evidência:** §3.1.
- **Impacto:** humano e agente têm capacidades diferentes sem justificativa de segurança
  (o serviço já roda local). **Recomendação:** unificar a seleção de backend nas superfícies
  humanas (`--rom` em `session`, `experiment`) e documentar as diferenças restantes.
- **Prioridade:** **P2**. **Milestone/fase:** consolidação.

### P-10 — Disassembler não decodifica instruções que o CPU executa
- **Descrição:** `TST.L abs.L` (`0x4AB9`) e provavelmente outras famílias aparecem como
  `DC.W`; o usuário não consegue ler a rotina que causou o fault.
- **Evidência:** §4.1 (`debug disasm`), FR-DEBUG-005 admite "68000-subset disassembler".
- **Impacto:** UC-3 (caça à rotina) fica capenga; depuração real exige ler hex.
- **Recomendação:** completar o decodificador para o conjunto já implementado (8.2) e testar
  contra o decode audit; ou rotular claramente as lacunas no output.
- **Prioridade:** **P2**. **Milestone/fase:** M8 / 8.6c ou antes (dependência de UC-3).

### P-11 — `STATE.md` não representa trabalho não-mergeado
- **Descrição:** um handoff de sessão não vê os 14 commits de 8.4 no checkout pai; outra sessão
  de `main` leria "next: start 8.4".
- **Evidência:** §2.1 D12; `git worktree list`.
- **Recomendação:** tabela "unmerged work" (branch/base/commits/dono) no STATE + draft PR como
  sinal externo.
- **Prioridade:** **P1** (coordenação). **Milestone/fase:** imediato.

### P-12 — Evidência histórica sem banner é confundida com status atual
- **Descrição:** `docs/quality/*acceptance*` e `quality-gates.md` contêm `rendering=false` etc.
  sem marcação; leitura ingênua conclui estado atual.
- **Evidência:** §2.1 D11. **Recomendação:** banner + mover para `docs/quality/history/` ou
  prefixar `HISTORICAL`.
- **Prioridade:** **P3**. **Milestone/fase:** consolidação.

---

## 9. Decisões que precisam do proprietário

| # | Decisão | Opções | Recomendação | Por quê |
|---|---|---|---|---|
| **D-1** | Aceitar um **milestone de consolidação de produto** entre 8.6 e 8.7 (UC-1..UC-5 + piso de fps + artefato reproduzível) | sim / não / só gates dentro de 8.6 | **sim** | transforma "fases verdes" em valor demonstrável; força o fluxo KidEngine |
| **D-2** | **Adiar 8.8 (janela nativa) e M9** até a consolidação passar | sim / não | **sim** | menor valor por risco para o público-alvo; libera foco para compatibilidade+desempenho |
| **D-3** | **Antecipar M6-FU1** (debug/trace/TAS no serviço) para junto da consolidação | sim / não | **sim** | é o diferencial agêntico; já está implementado, só não exposto |
| **D-4** | **Piso de desempenho** contratual (ex.: ≥30 fps headless no fixture; ≥60 fps paced com hash amostrado) e benchmark no CI | fixar número / adiar | **fixar** | sem número, 8.7/8.8 são inverificáveis; 0,84 fps hoje |
| **D-5** | **Política de I/O não mapeado**: manter "erro tipado" ou adotar open bus/no-op de hardware | mudar / manter | **mudar para open bus + erro só onde o hardware falha** | a 1ª instrução de *Chakan* prova que a política atual mata software real; manter exige modelar toda a janela antes de qualquer ROM |
| **D-6** | **Investigar adaptador temporário** de emulador existente (exodus-mcpd, MIT) por 2–3 dias, sem adotar | autorizar / recusar | **autorizar o spike, não o produto** | valida plataforma+UCs com software real antes do core; riscos de licença/determinismo controlados fora da árvore |
| **D-7** | **Governança de status**: uma fonte (`milestones.md`) + bloco gerado + banner histórico + STATE ≤10 linhas | aceitar / ajustar | **aceitar** | corta ~30–40% da prosa de status e elimina as 6 versões de "rendering" |
| **D-8** | **Fixtures**: validar/reprovar *Kid Chameleon* e escolher o conjunto curado com hashes e origem | validar / substituir | **validar e substituir se necessário** | evita perseguir dump inválido em 8.6 |

---

## 10. O que não pude verificar / perguntas para o capitão

- **Validade do dump de *Kid Chameleon*:** não comparei com hash externo; o vetor 0 é
  implausível. Precisa de confirmação de origem (hash/região) antes de virar evidência de 8.6.
- **Intenção do design 8.5 sobre leituras de I/O:** o texto cobre *writes* aos control registers
  e o version register; não afirma leituras/open bus. Confirmar com o autor do plano se a
  leitura de `0xA10008` está no escopo.
- **Custo real das otimizações de hash:** não medi variantes (hash amostrado, cache de
  serialização); apenas o baseline. Números de 8.6b precisam de experimento dedicado (área de
  desempenho).
- **`store.save` e sessões vivas:** confirmei pela assinatura/implementação que não há parâmetro
  de sessão; não testei todos os fluxos de continuação (`session_key`) por limite de tempo.
- **Cross-area:** o achado P-01 (ROMs morrem na 1ª instrução) é insumo direto para
  arquitetura/emuladores (política de I/O), desempenho (0,84 fps), determinismo (hash por passo)
  e interfaces (superfícies ausentes). O achado P-10 (disassembler) cruza com ferramentas.
- **Escopo não coberto:** revisão profunda de ADR-0013/0012 (arquitetura), áudio/Z80 (8.4),
  qualidade/CI, segurança de transporte — pertencem às outras áreas.
