# ChronoFork — Síntese integrada preliminar (rascunho para crítica)

**Baseline:** `origin/main` `b8d63a1` · **Worktree:** `review-2026-11-20-architecture` ·
**Data:** 2026-11-20 · **Autoria:** product (t9), a partir dos sete relatórios de área
(`areas/01`–`07`) e das respostas do capitão às perguntas cross-area de A6.
**Status:** rascunho — não é o relatório final A–G. Serve ao capitão e ao crítico adversarial.
**Rastreabilidade:** cada afirmação remete ao relatório de área (`A1`–`A7`) e, nele, ao
`path:line`/comando. Nada aqui foi medido de novo; quando não há evidência, está dito.
**Rótulos:** `[F]` fato observado (código/execução), `[E]` experimento, `[H]` hipótese,
`[I]` inferência, `[O]` opinião. Confiança e prioridade vêm das áreas.

---

## 1. Sumário executivo

1. **A base de engenharia é sólida e acima da média para a fase.** 12 crates com direção de
   dependência verificada por compilação, 66,4 k linhas de `src` + 48,9 k de testes, 1.289
   testes verdes, gate local completo, demos byte-determinísticos, envelopes canônicos com
   paridade direta/HTTP/MCP, sandbox de script, GUI sem posse de estado. `[F]` (A1 §2/§7, A5 §1,
   A6 §0)
2. **A premissa central do M8 não tem evidência executada.** Nenhum dos dois ROMs curados passa
   da primeira instrução no baseline. As causas são defeitos concretos e baratos de corrigir —
   ausência do espelho de RAM/truncamento de 24 bits no bus e registradores de controle de I/O
   documentados não implementados —, não hardware exótico. `[F]` (A2-F1/F2/F13; A7 §4)
3. **Há um bug de correção silenciosa**: DMA `memory→VRAM` lê a própria VRAM e "funciona" com a
   fonte errada, violando a regra 8 do `AGENTS.md`. Correção mínima: rejeitar com erro tipado
   hoje; implementar o lado 68000 na fase de compatibilidade. `[F]` (A2-F3)
4. **O custo dominante do passo é o caminho de verificação, não a emulação.** Step mediano
   209,8 µs; execução pura 0,17 µs; SHA-256 puro ~91 µs; alocações/page faults ~117 µs;
   serialização/cópias ~9 µs. O caminho completo (serializar + alocar + hashear) é ~97–99 % do
   step. Sem mudança de contrato, o teto é ~0,5–1 fps; com o caminho fora do step e render sob
   demanda, ~319 fps. `[E]` (A3-F-01/F-02/F-05; A1-01)
5. **O produto agêntico ainda não existe na prática.** Debug/trace/script/TAS/memória-escrita
   não estão sobre o transporte (M6-FU1 confirmado por código e runtime); observar 10 k passos
   via MCP custa ~20 k chamadas, ~5,2 M tokens e ~103 s; não há cancelamento, deadline nem
   cotas. `[E]` (A6-F1/F2/F3/F7; A7 §3)
6. **O serviço serializa todas as sessões.** Um `Mutex<SessionManager>` global mantém sessões
   independentes sem paralelismo (1,00/0,13/0,07/0,03 de eficiência em 1/2/4/8 threads) e um run
   longo bloqueia leitura/criação de todas as outras. O backend é paralelizável (controle
   `experiment.run`: 5,97× em 8 threads); o gargalo é o desenho do serviço. `[E]` (A4-F01/F02;
   A1-06)
7. **A evidência de qualidade não é reproduzível a partir do clone.** O diferencial exige
   oráculo out-of-tree (aborta sem ele), cobre 550 instruções/186 opcodes e **zero** exceções,
   interrupções ou frames de pilha; dois testes pulam em silêncio sem ROMs. `[E]` (A5-Q01–Q05)
8. **A documentação de status é uma fonte ativa de decisão errada.** 89 arquivos/38.165 linhas,
   13 documentos repetindo status, seis versões contraditórias de "rendering", 49 menções a
   `cb05836`. `[F]` (A7 §2; A2-F11)
9. **Conclusão de produto `[O]`:** o maior risco não é "o emulador está incompleto" — isso é
   esperado e documentado com honestidade —, é **o roadmap poder declarar progresso sem software
   executando, sem desempenho medido e sem superfície usável por agente**. Corrigir a medição
   (gates de boot, contrato de verificação, API mínima) vale mais do que qualquer fase nova.

---

## 2. Estado real e principais qualidades

| Camada | O que existe de fato | Qualidade | Evidência |
|---|---|---|---|
| Contratos e crates | `Backend`/`Session`/`Snapshot` pequenos, snapshot versionado validado num único ponto, dispatch dinâmico só na composição; sem `unsafe` | boa; 3 mudanças pontuais antes de M9 (hash, inspeção, timebase) | A1 §3/§7 |
| Sessões/TAS/replay | checkpoint durável, forks com lineage, movie v1/v2, rewind, edição de branch, replay verificado | entregue e verificado (M2/M3); fork retém 4× o estado (773,5 KiB) e registry não remove | A1-05; A4-F03/F04 |
| Debug/trace/script | breakpoints/watchpoints/eventpoints/logpoints, `CFTRACE1`, DSL determinístico com limites e sandbox | entregue (M4/M5); `inspect` por passo custa ~1 step; disassembler tem lacunas (`0x4AB9` → `DC.W`) | A1-02; A7 §3 |
| Serviço/transporte | 16 operações, token/escopo/ownership, HTTP/1.1 e MCP `2025-03-26`, paridade byte a byte, artefatos `cfexp` | entregue (M6), honesto e testado; sem cancelamento/deadline/cotas e sem as operações de RE | A6 §0–§3 |
| GUI | apresentação text/HTML, oito painéis, pacing, lease cooperativo; nunca possui estado | entregue (M7) | A6 §1; A7 §4.3 |
| Emulador | 68000 documentado com diferencial 11/11; VDP com compositor, DMA/FIFO/timing; cartucho/TMSS; controle 3-button TH-high | correto no que cobre; modelo por instrução (não por ciclo), frame composto no fim, sem Z80/áudio; **não** roda jogo comercial | A2 §3–§6/§12 |
| Determinismo | mesma entrada ⇒ mesmo hash; replay/rewind; cross-mode ainda não existe (8.7) | forte na semântica; caro no caminho (hash por passo) | A1-01; A3-F-01 |
| Segurança/robustez | loopback, Bearer, escopos, ownership, sandbox; fuzz sem crash | suficiente para v1 usuário único; não multiusuário | A6 §6/§9 |

**O que deve ser preservado `[O]`:** o contrato `Backend` opaco e versionado; a separação
apresentação/serviço/backend; o envelope canônico e a paridade entre superfícies; o DSL com
limites; os demos como contrato de determinismo. Não reescrever a arquitetura para "ficar
moderna" (async/Tokio, toolkit nativo, multi-backend) antes de o fluxo de valor existir.

---

## 3. Top problemas consolidados (P0–P3)

| ID | Problema | Evidência | Conf. | Impacto atual | Impacto futuro | P |
|---|---|---|---|---|---|---|
| **S-01** | Boot comercial bloqueado por mapa de bus: espelho de RAM `0xE00000..0xFFFFFF` ausente; 68000 não trunca endereço para 24 bits; controles de porta `0xA10008..0xA1000F` não implementados; plano 8.5 mantém porta 3 como erro | A2-F1/F2/F13/F14; A7 §4 | alta | os 2 ROMs morrem na 1ª instrução; M8 sem evidência | 8.6 inteira depende; qualquer jogo morre no init | **P0** |
| **S-02** | DMA `memory→VRAM` lê a própria VRAM em silêncio | A2-F3 | alta | dados errados sem erro; viola regra 8 | compatibilidade e confiança | **P0** |
| **S-03** | Verificação (serialização+hash+alocações) obrigatória por passo; `inspect` re-hasheia; render sempre; fork copia 4× | A1-01/02/05; A3-F-01..F-05; A2-F4 | alta | 0,47–1,05 fps; debug/trace 2×; CI caro | estado cresce em 8.4/8.5 e o custo cresce junto | **P0** |
| **S-04** | Superfície agêntica ausente (M6-FU1) + economia de chamadas: sem batching/cancel/deadline/cotas; 10 k passos MCP ≈ 20 k chamadas/5,2 M tokens/103 s; sem `memory.write` | A6-F1/F2/F3/F7; A4-F02; A7 P-02 | alta | agente não faz RE; um run longo bloqueia e não cancela | tese "agent-native" não se sustenta | **P1** |
| **S-05** | `Mutex<SessionManager>` global serializa sessões; registry vitalício; `max_sessions` não limita memória; sessão fechada retém backend | A4-F01/F03/F04; A1-06 | alta | sem paralelismo real; 1.024 creates e ~397 MiB | multi-agente inviável | **P1** |
| **S-06** | Evidência de qualidade não reproduzível; zero cobertura de exceções/interrupções; controles negativos provam o comparador; pulos silenciosos; teste de 167 s mede run-vs-run | A5-Q01–Q05/Q12 | alta | confiança nas afirmações "68000/VDP completo" | regressões passam; auditoria externa não reproduz | **P1** |
| **S-07** | Precisão de hardware para 8.6: frame composto no fim (sem raster), CPU não paga DMA/escrita VDP, HBlank no fim da linha, três timelines | A2-F5/F6/F7/F8 | alta | jogos com efeitos raster/contenda ficam errados | matriz de compatibilidade não fecha | **P1** |
| **S-08** | Drift documental de status e de fatos técnicos (espelho de RAM, DMA "pré-validado") | A7 §2; A2-F11 | alta | decisões erradas; custo de leitura por sessão | cresce a cada fase | **P1/P2** |
| **S-09** | Serviço/segurança: slowloris pré-auth, quotas, MCP preso em `2025-03-26`, `Origin` ausente, tokens estáticos, accept poll 5 ms | A6-F4/F5/F6/F8; capitão (a)/(c)/(d) | alta | DoS entre agentes/processos locais; conformidade | multiusuário/exposto inviável | **P2** (Origin P2/P3) |
| **S-10** | Higiene de código: 493 itens `pub` no megadrive, `StateView` duplicado, `SessionError` misturando storage, 50 parsers manuais, fachadas service/script duplicadas | A1-07/08/09/12/13/14 | alta | manutenção | não bloqueia | **P3** |

Prioridade ≠ severidade: S-03 é P0 de severidade e **P1 de implementação** (entra antes de 8.5
porque o estado cresce); S-09 é P2 com bind loopback; S-10 é P3.

---

## 4. Conflitos entre áreas e resolução

### 4.1 Conflito obrigatório 1 — "hash é 37,8 %" vs "hash é 99,9 %"

Os dois números são verdadeiros com **denominadores diferentes**:

| Medição | Numerador | Denominador | Resultado | Fonte |
|---|---|---|---|---|
| A2-F4 | SHA-256 sobre 197.058 B (83–93 µs) | passo mediano (213–254 µs) | **36,5–39,6 %** (mediana ~37,8 %) | A2 §12; A1 §4 |
| A3-F-01 linha "99,9 %" | caminho de verificação (91,0 µs) | **execução pura + verificação** (91,173 µs) — não o passo inteiro | 99,9 % (502× a emulação) | A3-F-01 |
| Caminho completo no passo | hash 91,0 + alocações/page faults 116,6 + serialização/cópias ~9 µs (parcelas com sobreposição; fecham em ~210) | passo 209,8 µs | **~97–99 %** | A3 §3/F-02; A1 §4 |

**Conclusão registrada:** SHA-256 puro ≈ **40 %** do passo; o **caminho completo de
observabilidade ≈ 97–99 %**. A frase "hash é 99,9 % do step" é imprecisa (o 99,9 % é
verificação/emulação); a frase "o problema é só o SHA-256" também é falsa. O que domina é o
conjunto serializar + alocar + hashear, e o SHA é sua parte estável e mensurável (~40 %).

### 4.2 Conflito obrigatório 2 — "sobram 14–19 µs de CPU+VDP" vs "execução pura 0,18 µs"

| Componente | Mediana | Como foi medido | Fonte |
|---|---|---|---|
| (a) CPU+VDP+scheduler | **0,17 µs** | `REVIEW_HASH=0`: 0 alocações/step | A3-F-02 |
| (b) serialização/cópias do corpo | **~9 µs** | 93,2 µs (hash+alocador estável) − 84 µs hash − 0,17 µs | A3 §3; A1 §4 |
| (c) alocações/page faults | **0–116,6 µs** (bimodal) | 51,4 minor faults/step; cai para 82 faults em 50 k com thresholds elevados | A3-F-02 |
| (d) SHA-256 puro | **91,0 µs** (83–93) | hash sobre 197.058 B, SHA-NI ~2,1–2,4 GB/s | A1 §4; A3 §3 |
| **Passo observado** | **93–210 µs** | mediana 209,8; mínimo 102 | A3; A1 |

**Resolução:** os 14–19 µs de A1 (passo mínimo 102 − hash 83 − cópias ~5) **não são CPU**: são
construção do corpo (~9 µs) + variância de alocador/page faults (~6–9 µs) + 0,2 µs de execução.
A leitura "CPU+VDP custa 14–19 µs por instrução" é **incorreta**. `[F/E]`
**Implicação para a recomendação:** "tirar o hash" isolado deixa o corpo sendo construído
(~9 µs/step ⇒ ~10–11 k steps/s, ~1 fps) — **insuficiente**; mas remover o caminho inteiro
(sem hash, sem corpo, buffers persistentes, render sob demanda) leva a ~0,17 µs/step + 1,27 ms
de render/frame ≈ **319 fps** — suficiente com folga. A recomendação correta ataca (a)+(b)+(c)+(d)
juntos e **precisa ser validada com workload de jogo real**, não só com `interrupt_rom(3)`.

### 4.3 Outros conflitos e resoluções

| Tema | Divergência | Resolução adotada |
|---|---|---|
| Ordem 8.4 (Z80/áudio) vs 8.5 (timers/controles) | A2 recomenda **inverter** (8.5 antes); A7 recomendava manter 8.4 | Endurecimento barato **agora** (S-01/S-02), depois **8.5**, depois **8.4**; áudio é o bloco mais caro e isolável, e sem controle/timer o jogo não sai do título. Decisão D1 |
| Causa da morte do *Kid Chameleon* | A7 levantou "dump suspeito" (SSP `0xFFFFF7FE`) | **A7 corrigido por A2-F13**: a ROM é válida; `0xFFFFF7FE & 0x00FFFFFF = 0x00FFF7FE` (RAM). O bug é do backend (truncamento 24 bits + espelho) |
| Política de I/O não mapeado | A7 propôs "open bus" para `0xA10008` | **Corrigido**: `0xA10008–0xA1000F` são registradores de **controle** documentados, não open bus (A2-F2/§16); open bus determinístico só para o que é de fato não decodificado |
| Prioridade do hash | A1: severidade P0, prioridade **P1**; A3: **P0**; A2: fase 8.7 | Severidade **P0**; implementação **P1 e antes de 8.5** (contrato + buffers); programa completo de performance em 8.6b |
| "1289 testes" vs cobertura real | A5: número verdadeiro, cobertura de exceções zero | Ambos verdadeiros; registrar "1.289 testes, 550 instruções/186 opcodes no diferencial, zero exceções" como a frase honesta |
| `Origin`/MCP | A6 sugeria P1/P2 | **Capitão**: com Bearer em todas as rotas exceto `/health` e bind loopback, é **P2/P3** (endurecimento), não P0; não há exploração demonstrada. Versão MCP vigente `2026-07-28` não é bloqueador do M8 |
| Multiusuário | ADR-0012 documenta serialização; A4 mede que ela impede multi-agente | Honesto no ADR, mas **não** suporta a promessa "experimentos simultâneos"; declarar v1 usuário único/loopback até D3/D4 |

---

## 5. Decisões que precisam do proprietário

### D1 — Ordem das fases de hardware e gates de boot
**Problema:** o M8 pode fechar 8.4/8.5 sem nenhum ROM executando; a ordem atual coloca áudio
(8.4) antes do que desbloqueia o boot (8.5).
**Alternativas:** (a) manter 8.4→8.5; (b) endurecimento barato agora + 8.5 → 8.4; (c) 8.4 e 8.5
em paralelo; (d) nada.
**Recomendação:** **(b)**. Agora (≈1 semana): truncamento 24 bits no bus, espelho
`0xE00000..0xFFFFFF`, `0xA10008–0xA1000F` como registradores (controle de porta 3 incluído),
open bus determinístico no resto, rejeitar DMA `memory→VRAM` com erro tipado, corrigir as duas
frases de doc (A2-F11), e instalar **G-BOOT-0** (300 k passos sem erro de host) e **G-BOOT-1**
(PC fora da área de reset, registradores VDP ≠ 0, VRAM não nula). Depois 8.5, depois 8.4.
**Risco:** inverter a ordem atrasa o áudio e a fase 8.4 já tem plano/design; mitigação: manter
8.4 com plano aprovado e começar pelo Z80/bus (que também é pré-requisito de boot), com áudio
como sub-bloco.
**Verificação:** G-BOOT-0 passa nos dois fixtures; G-BOOT-1 passa no *Chakan*; nenhum teste de
hash/snapshot quebra; documentação corrigida.

### D2 — Contrato de verificação e programa de desempenho
**Problema:** `StepOutcome.state_hash` obrigatório + `inspect`/render/fork caros impedem
qualquer meta interativa (0,47–1,05 fps; 57–129× acima do orçamento).
**Alternativas:** (a) micro-otimizar mantendo o contrato; (b) ADR com hash **opt-in**
(`state_hash()` explícito), buffers persistentes, `status()` barato, render opt-in, fork sem
cópias; (c) hash incremental/dirty; (d) nada.
**Recomendação:** **(b)** agora, com (c) reavaliado só se hash por passo virar requisito de
produto. Metas: piso intermediário ≥ 60 k instr/s; alvo de hardware ~1,9 M instr/s (no workload
medido, 60 fps exige 0,6–0,8 M steps/s); ≥ 60 fps após render opt-in; benchmark de regressão no
CI. Validar em ROM real, não só no loop sintético.
**Risco:** muda contrato público (`StepOutcome`) e ~31 usos + 122 chamadas de `inspect`; testes
golden de hash por passo precisam migrar. Mitigação: teste de invariante "hash sob demanda ==
hash completo" em todo o workspace antes/depois.
**Verificação:** execução pura ≥ 5 M steps/s; frame de ROM real dentro do orçamento declarado;
demos byte-idênticos; `.cfmovie`/snapshots inalterados.

### D3 — Dono único da API agêntica e do redesenho do serviço
**Problema:** M6-FU1 (sem RE sobre transporte), sem batching/cancel/deadline/cotas e lock global
são o **mesmo** problema de desenho; hoje três áreas os tratam separadamente.
**Alternativas:** (a) espelhar ~30 verbos da CLI no MCP; (b) **API mínima de 9 operações** com
batching (`run` com `stop_on`/`inputs`/`max_wall_ms`/`cancel`/`progress`, `memory.write/diff`,
`disassemble`, `trace`, `frame`) + lock por sessão + cancelamento + cotas, sob um dono; (c)
manter CLI-only e adiar; (d) Tokio/async.
**Recomendação:** **(b)**, com a lista de A6 §3.5 como **entrada, não compromisso fechado**, e
**dono único na fase de consolidação de produto (8.6c)**, junto com A1/A4. Não adotar
Tokio/async por padrão. A atualização de protocolo MCP (`2026-07-28`) entra depois do
M8, sem bloquear. Bind não-loopback fica **fora de escopo da v1** (restrição de segurança
documentada), salvo pedido explícito do proprietário.
**Risco:** muda a fronteira do serviço e o contrato de `run` enquanto o emulador muda; mitigação:
fazer o desenho depois do ADR de D2 e antes de 8.6c.
**Verificação:** 10 k passos via transporte em **uma** chamada com deadline/cancel e < 2 s;
4 sessões escalam > 2×; nenhuma operação bloqueia `list`/`inspect` de outra sessão; teste de
interop com cliente MCP oficial em dev.

### D4 — Modelo de concorrência e de confiança (multi-agente?)
**Problema:** o produto se apresenta para "humanos, scripts e agentes", mas um agente rodando
degrada todos; `max_sessions` é vitalício e sessões fechadas retêm memória.
**Alternativas:** (a) "nada" (aceitável só para v1 single-agent); (b) lock por sessão +
cancelamento + ciclo de vida do registry + cotas; (c) processo por sessão; (d) async.
**Recomendação:** **(b)** agora; (c) como escape hatch documentado; declarar formalmente **v1
loopback/usuário único** até TLS + ownership durável (M6-FU2/3).
**Verificação:** throughput 1→8 sessões escala; `close` libera memória e conta só sessões vivas;
run de 100 k passos cancelável; timeout cliente/servidor coerente (sem *unknown outcome*).

### D5 — Governança, evidência reproduzível e build-vs-integrate
**Problema:** status em 13 documentos; diferencial não reproduzível; anti-slop duplicado; decisão
de áudio/licença pendente.
**Alternativas:** (a) manter; (b) fonte única de status (`milestones.md`) + bloco gerado +
banner histórico + `STATE.md` ≤10 linhas com trabalho não-mergeado; diferencial reproduzível
(oráculo vendorizado/pinado **ou** skip explícito com guarda de contagem); anti-slop "lite";
áudio: integrar `ymfm` (BSD-3) com ADR, manter `Nuked-OPN2` (LGPL) opcional; adaptador Exodus
**somente** como oráculo externo por processo.
**Recomendação:** **(b)**.
**Verificação:** clone limpo roda o diferencial ou falha explicitamente (nunca em silêncio);
`check.sh` detecta drift de status; nenhum código GPL/não comercial embutido; `docs/` sem
afirmações técnicas falsas.

---

## 6. Evolução arquitetural e de roadmap (alto nível)

```text
AGORA (0–1 semana, sem features novas)
  S-01/S-02 hardening + G-BOOT-0/1 + correções de doc
  D2: ADR do contrato de verificação + buffers persistentes + status() barato
  D5: fonte única de status + banner histórico

TRILHA A — backend (sequencial)
  8.5 timers/controles/I-O completo  →  G-BOOT-1 estável
  8.4 Z80/bus/áudio (ymfm c/ ADR)    →  G-BOOT-2 (frame+áudio+matriz)
  8.6a compatibilidade (DMA real, frame por scanline, contenda, HBlank)
  8.6b desempenho (hash fora do step, render opt-in, fork, inspect)
  8.6c CONSOLIDAÇÃO DE PRODUTO (UC-1..UC-5, artefato, piso de fps, docs)

TRILHA B — plataforma (paralela, dono único)
  lock por sessão + cancelamento + cotas + ciclo de vida (D3/D4)
  API mínima de RE com batching sobre HTTP/MCP (entrada: A6 §3.5)
  MCP: Origin/MCP-Protocol-Version agora; 2026-07-28 depois do M8

TRILHA C — qualidade/evidência (paralela)
  diferencial reproduzível + cobertura de exceções/RTE/frames
  controle de mutação no emulador + fuzz + anti-slop lite

DEPOIS (adiável, sem valor antes do acima)
  8.7 modos/captura PNG/WAV  →  8.8 janela nativa  →  M9 multi-backend
  TLS/multiusuário, async/Tokio, hash incremental, CoW de fork, keep-alive
```

**Dependências:** D2 antes de 8.4/8.5 (o estado cresce com Z80/áudio e o hash junto); S-01/S-02
antes de 8.5; gates G-BOOT por fase; D3/D4 antes de qualquer promessa multi-agente; 8.6c depois
de 8.5+8.6a (precisa de um ROM executando), mas pode usar o sintético para UC-2/UC-3 desde já.
**O que pode ser adiado `[O]`:** 8.8 (janela nativa), M9, TLS/multiusuário, async, hash
incremental, CoW/dedup, atualização completa do MCP, GUI com conteúdo de inspetor.

---

## 7. Separação explícita: fato, experimento, hipótese, inferência, opinião

**Fatos observados (código/execução reproduzível):** hash de 197 KB por instrução; `inspect`
re-hasheia e copia 64 KiB; bus só mapeia RAM `0x00FF0000..0x01000000` e `0xA10003/05`; 68000 sem
máscara 24 bits; DMA memory→VRAM copia VRAM; frame composto no fim; `Mutex<SessionManager>` no
`session.run`; catálogo de 16 ops sem debug/trace/TAS no MCP; registry sem remoção; gate verde e
1.289 testes; os dois ROMs morrem na 1ª instrução; 89 md/38.165 linhas com 13 fontes de status.

**Experimentos (medidos nesta revisão):** step 209,8 µs mediano/5,27 k steps/s; `REVIEW_HASH=0`
0,173 µs e 0 alocações; alocador estável 93,2 µs; 51,4 minor faults/step; frame 0,95–2,15 s;
escada 1/2/4/8 sessões (eficiência 1,00/0,13/0,07/0,03); run 20 k = 4,65–4,73 s; MCP 10 k passos
≈ 20 k chamadas/5,2 M tokens/103 s; diferencial 550 instruções/186 opcodes/0 exceções; fuzz
47+160 casos sem crash; demo debug vs release byte-idêntico.

**Hipóteses (não provadas):** frequência real de DMA memory→VRAM nos jogos; impacto de
`musl`/macOS no churn de alocador; que o estado de 8.4 cresça o suficiente para piorar o hash
(provável, não medido); que o *Chakan* avance além do init após S-01; que a matriz de
compatibilidade caiba em CI local; que `Origin`/slowloris sejam exploráveis no modelo de ameaça
declarado (não demonstrado).

**Inferências:** o caminho completo é ~97–99 % do step; "tirar o hash" isolado não basta (~9 µs
de corpo); sem gates, o M8 pode fechar sem software real; a API atual não sustenta a tese
agent-native; o backend é paralelizável (controle `experiment.run` 5,97×) e o gargalo é o serviço.

**Opiniões (julgamento):** prioridades D1–D5; ordem 8.5→8.4; API mínima de 9 operações;
governança de fonte única; adiar 8.8/M9; integrar `ymfm` em vez de `Nuked-OPN2`; declarar v1
single-user; não trocar stack/async.

---

## 8. Lacunas de evidência e o que ainda precisa ser investigado

1. **Pós-correção S-01/S-02:** os ROMs passam de "morre na 1ª instrução" para onde? G-BOOT-0/1
   executados após o patch; medir PC/registradores VDP/VRAM e o próximo bloqueio (Z80? timers?).
2. **Desempenho em ROM real após D2:** os 319 fps são projeção de `interrupt_rom`; medir frame
   de *Chakan* com render sob demanda e comparar com o orçamento de 60 fps.
3. **Cobertura de exceções/interrupções no diferencial:** criar corpus com `TRAP`, `RTE`,
   `STOP`, frames de pilha, IRQ nível 4/6; validar contra o oráculo.
4. **Mapa de I/O completo:** listar todos os registradores documentados da janela
   `0xA10000–0xA1001F` e separar "registrador" de "open bus"; verificar com o padrão exato de
   *Chakan* (`TST.L 0xA10008`, `TST.W 0xA1000C`).
5. **Matriz de compatibilidade:** validar os fixtures (hash/origem) e definir o conjunto curado;
   *Kid Chameleon* deve ser reavaliado após A2-F13 (não é dump suspeito).
6. **Service/API:** protótipo das 9 operações com batching e cancelamento; medir 10 k passos em
   1 chamada, latência p95 e ganho de tokens; interop com cliente MCP oficial. Confirmar com o
   crítico que `Origin` permanece **P2/P3** sob Bearer + loopback (lacuna de endurecimento, sem
   exploração demonstrada).
7. **Concorrência:** lock por sessão medido (1/2/4/8), custo de cancelamento e cotas; verificar
   que `run` cancelado é sempre um prefixo determinístico.
8. **Áudio/licença:** avaliar `ymfm` (build, precisão, tamanho) e o custo de ADR; confirmar que
   `Nuked-OPN2` fica fora do núcleo MIT.
9. **Ambiente:** sem `perf`/`valgrind`; atribuição por ablação. Repetir em outra máquina/OS para
   separar efeitos de alocador.
10. **Não verificável nesta revisão:** branch `agent/8.4-z80-audio` (proibido), oráculo Exodus
    (ausente no worktree para reexecução), hardware real, cross-platform, e a validade externa
    dos dumps (sem comparação com hash público).

---

## 9. Rastreabilidade dos top problemas

| Síntese | Áreas | IDs |
|---|---|---|
| S-01 boot | A2, A7 | A2-F1, A2-F2, A2-F13, A2-F14, A7-P-01/P-06 |
| S-02 DMA | A2 | A2-F3 |
| S-03 verificação/perf | A1, A2, A3 | A1-01/02/05, A2-F4, A3-F-01..F-05 |
| S-04 API agêntica | A6, A4, A7 | A6-F1/F2/F3/F7, A4-F02, A7-P-02 |
| S-05 concorrência | A4, A1 | A4-F01/F03/F04, A1-06 |
| S-06 qualidade | A5, A2, A7 | A5-Q01..Q05/Q12, A2-F11, A7-P-04 |
| S-07 precisão 8.6 | A2 | A2-F5/F6/F7/F8 |
| S-08 docs | A7, A2 | A7 §2/D1–D12, A2-F11 |
| S-09 segurança/serviço | A6 | A6-F4/F5/F6/F8 |
| S-10 higiene | A1 | A1-07/08/09/12/13/14 |

**Correções explícitas ao rascunho de produto (A7):** (i) *Kid Chameleon* não é dump suspeito —
a causa é a ausência de truncamento de 24 bits/espelho de RAM (A2-F13); (ii) `0xA10008` não é
open bus — é registrador de controle documentado (A2-F2/§16); open bus vale para o que é de fato
não decodificado.
