# A6 — Interfaces agênticas, MCP/HTTP/CLI/GUI e segurança

**Baseline:** `origin/main` `b8d63a17349e672ba6945eaba3379cec38cf9d1b` (worktree
`review-2026-11-20-architecture`, branch `agent/review-2026-11-20-architecture`).
**Revisor:** `interfaces` (A6). **Data:** 2026-11-20 (America/Fortaleza). **Time-box:** ~90 min.
**Escopo:** `chronofork-service`, `chronofork-transport`, superfícies de processo
(`serve`/`api`/`mcp`/`experiment`/`session`/`debug`/`trace`/`script`/`tas`/`gui`), sandbox de
script, limites e trust boundaries. Fora do escopo: internals do 68000/VDP, formato de
snapshot byte a byte, desempenho do emulador (coberto por outras áreas).
**Regras seguidas:** nenhuma edição de produção/docs/commits; experimentos em
`.research/review-2026-11-20/agents/interfaces/`; evidência bruta copiada para
`.research/review-2026-11-20/areas/06-evidence/`; exemplo temporário `review_*` removido ao
final. Rótulos usados: **fato observado**, **experimento**, **inferência**, **hipótese**,
**opinião**; confiança alta/média/baixa; prioridade P0–P3 (implementação), separada de
severidade.

**Classificação:** sem rótulo explícito, medições e saídas de comando são **fato observado/experimento**; alternativas, recomendações e notas de arquitetura são **opinião/julgamento**; compatibilidade com clientes não testados é **inferência**, sinalizada no texto.

**Método:** leitura do código e dos ADRs 0012/0013, da arquitetura de interfaces e das
requirements; dois daemons reais (`--rom` sintético e Mega Drive), JSON-RPC e HTTP crus por
socket, cenário representativo de agente, microbenchmarks, provas de concorrência/
cancelamento, fuzz de corpo/HTTP e de parsers binários, testes de sandbox do script, GUI
local/remota, matriz de dependências resolvida com `cargo metadata` e leitura das
especificações MCP primárias no repositório oficial.

---

## 0. Sumário executivo

**Veredito:** o M6 entrega o que o ADR-0012 promete — um catálogo de 16 operações com
autenticação, ownership, limites e paridade canônica byte a byte — e é **seguro o suficiente
para uso em loopback/ambiente confiável de um único usuário**. Ele **não** é uma interface de
engenharia reversa: 4 das 15 capacidades essenciais que um agente precisa existem apenas na
CLI (confirmando M6-FU1), e as operações existentes não têm cancelamento, progresso nem
deadline de execução. O daemon é, hoje, um orquestrador de sessões, não um depurador.

Achados principais (detalhe nas seções 2–7):

| # | Achado | Severidade | Prioridade |
|---|---|---|---|
| A6-F1 | M6-FU1 confirmado: sem breakpoint/watchpoint/disassembly/trace/script/TAS sobre transporte; um agente MCP não faz RE real | alta | P1 |
| A6-F2 | `session.run` de 100k passos (~9,4 s no Mega Drive) bloqueia **todas** as operações, inclusive leitura e criação de sessão; desconexão do cliente e `notifications/cancelled` não cancelam | alta | P1 |
| A6-F3 | `session.snapshot` não tem quota: 100 snapshots em uma sessão = +21,4 MB de RSS; `store.save` idem em disco (4,3 MB/20 saves). `max_sessions` não limita memória | alta | P1 |
| A6-F4 | Slowloris **pré-autenticação**: 2 conexões truncadas prendem 2 workers (default 4) e `/health` legítimo estoura timeout | alta (bind não-loopback) / média (loopback) | P1 |
| A6-F5 | Loop de accept com poll de 5 ms impõe piso de ~5,16 ms/chamada e teto de ~200 req/s por cliente / 763 req/s com 4 workers | média | P2 |
| A6-F6 | MCP preso em `2025-03-26` (vigente: `2026-07-28`), sem `MCP-Protocol-Version`, sem `Mcp-Method/Mcp-Name`, sem `server/discover`; validação de `Origin` **ausente** (MUST da spec) | média-alta | P1/P2 |
| A6-F7 | Faltam primitivas de alto valor: escrita de memória (backend suporta, nenhuma superfície expõe), busca de padrões, diff de regiões, disassembly/trace/frame sobre transporte, sequências de input | média | P2 |
| A6-F8 | Tokens estáticos sem TLS/rotação/expiração/auditoria; arquivo de token 0644 é aceito sem aviso; store/artefatos 0644/0755; ownership durável inexistente (M6-FU2) | média | P2 |

**O que está bom e foi verificado:** paridade e determinismo do envelope canônico
(`experiment run` duas vezes → mesmos bytes, SHA-256 `b97b1df6…`); sandbox de script bloqueia
`..`, caminho absoluto e symlink para fora; 47 corpos MCP malformados + 160 execuções de
parsers binários malformados sem crash/panic/timeout; autenticação e ownership funcionam
(reader → `forbidden`); nenhum token aparece em stdout/stderr/GUI/`Debug`.

---

## 1. Mapa das superfícies e estado real do M6-FU1

### 1.1 O que existe hoje

`crates/chronofork-cli/src/main.rs:78-108` roteia: `demo`, `megadrive`, `backend`, `serve`,
`api`, `mcp`, `experiment`, `gui`, `session`, `tas`, `trace`, `script`, `debug`.

| Superfície | Forma | O que cobre | Evidência |
|---|---|---|---|
| **CLI** (`session`, `tas`, `debug`, `trace`, `script`) | um processo por invocação, blocos `record=`/`key=value` | sessões duráveis, TAS completo (record/play/verify/compare/studio/edit), debugger (breakpoints condicionais, watchpoints de barramento, eventpoints, logpoints, inspect/memory/disasm/vdp/interrupts/bus), trace (capture/info/query/compare/export), script DSL | `debug.rs:1455+`, `trace.rs:870+`, `script.rs:300+`, `tas.rs:1075+`; execuções em §2 |
| **HTTP** | `GET /health`, `GET /v1/tools`, `POST /v1/ops` | catálogo de 16 operações | `crates/chronofork-transport/src/http.rs:120-135`; `POST /v1/ops` real em §3 |
| **MCP** | `POST /mcp`, JSON-RPC 2.0 stateless | `initialize`, `notifications/initialized`, `ping`, `tools/list`, `tools/call`; 1 tool por operação do catálogo | `transport/src/mcp.rs:29,79-152`; `tools/list` real = 16 tools |
| **GUI** | `chronofork gui run` dirigido por stdin, local ou `--remote` | `create`, `list`, `attach`, `detach`, `focus`, `layout`, `panel`, `step`, `run`, `pulse`, `pause`, `handoff`, `html`, `quit`; apresentação texto/ANSI/HTML | `gui/src/command.rs:1327-1328`; execução real em §3.4 |
| **Script** | `chronofork script run --file` | DSL com `open/step/fork/checkpoint/restore/select/register/read/tick/state_hash/trace_*/tas_*/report`; sandbox opt-in | `script/src/eval.rs:620-800`; execuções em §6.3 |
| **Serviço** | catálogo de 16 operações | `capabilities`, `session.*` (10), `store.*` (5), `experiment.run` | `service/src/catalog.rs:297-460` |

O catálogo tem exatamente 16 operações e nenhuma delas é debug/trace/script/TAS. Isso foi
verificado de três formas independentes: leitura de `catalog.rs`, `tools/list` real (16
nomes) e quatro chamadas `tools/call` para `debug.run`, `trace.capture`, `script.run`,
`tas.record` → todas `-32602 unknown tool` (73 bytes cada). O estado de M6-FU1 é, portanto,
**real e consistente**: `docs/architecture/agent-interfaces.md:481-489` e
`docs/requirements/functional.md:115-116` declaram a limitação corretamente; não há
publicidade enganosa.

### 1.2 O que isso significa para um agente

O agente que fala MCP tem um **handle de sessão persistente** mas nenhuma ferramenta de
observação fina; o agente que fala CLI tem as ferramentas finas mas **nenhum handle**: cada
comando recria a máquina a partir do ROM e de um `--steps N` fixo. Para inspecionar o passo
1.000.000 pela CLI é preciso re-executar 1M passos (~104 s a 9.609 passos/s medidos), e não há
como "anexar" a CLI a uma sessão criada via MCP. O pior dos dois mundos para fluxo agêntico:
`session.inspect` em uma sessão Mega Drive custa 1.789 bytes e não existe um comando que
continue a partir dali com breakpoint.

---

## 2. Um agente consegue fazer engenharia reversa de verdade?

Capacidades pedidas, confrontadas com as quatro superfícies (fato observado; CLI executada
com `review.rom` = `interrupt_rom(3)`, saída entre 236 e 878 bytes por comando; MCP medido no
daemon Mega Drive; script e GUI pelo código e execuções):

| Capacidade | CLI | MCP/HTTP | GUI | Script |
|---|---|---|---|---|
| Breakpoint condicional (reg/mem) | **sim** (`debug run --breakpoint 0x202` parou em `reason=event`, tick 3458; condição `reg:`/`mem:` documentada) | não | não | não (só `if register(...)`) |
| Watchpoint de barramento (r/w/rw, byte) | **sim** (`--watchpoint 0xFF0000-0xFF0FFF:w`) | não | não | não |
| Inspeção de registradores | **sim** (`debug inspect`: PC, SR, flags, VDP, scheduler, mapper, cartucho) | **parcial** (`session.inspect` devolve mapa completo d0–d7/a0–a7/PC/SR/IRQ/VDP/scheduler; 1.789 B) | sim (painel `registers`) | **sim** (`register("pc")`) |
| Leitura de memória | **sim** (`debug memory`, janela ROM/RAM) | **parcial** (`session.read_memory` ≤4.096 B; I/O mapeado falha: `0x00A10003` → `backend_error` "outside any mapped window") | sim (painel `memory`) | **sim** (`read(addr,len)`) |
| **Escrita de memória** | **não** | **não** (backend tem `write_memory`, `capabilities.memory_write=true`, mas nenhum op do catálogo) | não | não |
| Disassembly | **sim** (`debug disasm --count 6`, 236 B) | não | não (bloqueado por M6-FU1, `gui/src/presentation.rs:14`) | não |
| Tracing estruturado (CFTRACE1) | **sim** (capture/query/compare/export) | não | não (painel bloqueado) | **sim** (`trace_capture/records/export/compare`) |
| Busca de padrões em memória/ROM | **não existe em lugar nenhum** (grep por `search` nas interfaces = 0) | não | não | não |
| Comparação de regiões de memória | não (só `session.compare` tick+hash e `trace compare` de registros) | **parcial** (`session.compare` = tick + state hash, 387 B) | não | não |
| Execução até evento (PC/watch/IRQ/VDP) | **sim** (`--eventpoint interrupt:requested[:level]`, `vdp:`, `controller:`, `bus-write`) | não (só `steps` fixos) | não | não |
| Fork | não na CLI | **sim** (`session.fork`; determinismo confirmado: dois forks com o mesmo input → `equivalent=true`) | não | **sim** (`fork()`) |
| Comparação de sessões | TAS `compare` (filmes) / trace `compare` | **parcial** (tick+hash) | não | não (só trace/tas compare) |
| Manipulação de inputs | **sim** (TAS record/edit/play/verify; portas 1–3, eixos, teclas) | **mínimo** (`buttons` u32 fixo por `session.run`/`experiment.run`) | não | **não** (`step` usa `InputFrame::NEUTRAL`, `script/src/host.rs:101`) |
| Captura de frame (pixels) | não | não | só texto/HTML determinístico; `rendering=false` em toda configuração v1 | não |
| Cancelamento / progresso | não (um comando por processo) | **não** (notificação `notifications/cancelled` → `202` e ignorada; desconexão não interrompe) | `pause` cooperativo local | deadline só entre operações (§6.3) |

**Resposta:** com CLI+script, sim, para análise em lote e offline (o script roda 10.000
passos + 10.000 leituras de registrador em 3,5 s de CPU, ~2.857 passos/s, contra ~5 ms por
chamada MCP). Com MCP apenas, **não**: faltam breakpoints/watchpoints, disassembly, trace,
escrita de memória, busca, diff de regiões, frame e input timeline. O mínimo para tornar o
MCP útil é uma API pequena e composável, não o espelhamento dos ~30 verbos da CLI
(proposta em §3.5).

**A6-F1 — M6-FU1 é o bloqueio central da proposta "agent-native"**
- *Componentes:* `chronofork-service` (catálogo), `chronofork-transport`, CLI.
- *Condições:* qualquer agente que só disponha de MCP/HTTP (o caso de uso declarado do M6).
- *Impacto atual:* o agente não consegue observar o passo a passo, parar em condição,
  desmontar, tracejar nem alterar memória; só orquestra `run`/`fork`/`compare`/`read_memory`.
- *Impacto futuro:* quanto mais o produto se posicionar como plataforma para agentes, mais
  visível fica que a análise real continua fora do contrato.
- *Probabilidade:* certa (fato). *Severidade:* alta. *Confiança:* alta.
- *Alternativas:* (a) não fazer nada e documentar (é o estado atual); (b) expor tudo da CLI
  (superfície enorme, ~30 verbos, difícil de limitar); (c) **API mínima de 8–10 operações
  composáveis** (recomendada); (d) MCP stdio empacotando a CLI (resolve cliente, não resolve
  handle/limites).
- *Recomendação:* (c), faseada: P1 = `run` com `stop_on`+`inputs`+`max_wall_ms`+cancelamento/
  progresso; P2 = `memory.write`, `debug.disassemble`, `trace.*`, `frame.capture`,
  `memory.diff`; P3 = import de TAS.
- *Custo/risco:* 1,5–3k LOC e 1–2 fases; risco de inflar o catálogo — mitigar com limites e
  testes de paridade por operação. *Contratos:* `chronofork-service` catalog + arquitetura de
  interfaces + testes de aceitação M6; requer ADR (sucessor do 0012) por tocar o transporte.
- *Prioridade:* P1. *Milestone:* M6-FU1/M8.

---

## 3. Economia da API: cenário MCP real

### 3.1 Cenário representativo (22 chamadas, daemon sintético)

Daemon `chronofork serve` (backend sintético, store/artefatos habilitados), MCP cru
(`tools/call`), `operator` (read+write). Sequência: create → inspect → run 10 → snapshot →
fork×2 → run×2 → compare → read_memory → experiment.run → store.save/list/load/delete →
list → 4 tentativas M6-FU1 → 2 erros. **Total: 22 chamadas, 8.564 bytes de corpo (~2.141
tokens por chars/4), 3 envelopes de erro tipados** (`conflict`, `not_found`,
`limit_exceeded`). Chamadas individuais: `session.create` 378 B, `session.inspect` **989 B**
(sintético; no Mega Drive o mesmo op devolve 1.789 B, §2), `session.run` 266 B,
`session.compare` 387 B, `read_memory` 255 B, `experiment.run` 1.054 B, `store.list` 590 B,
`session.list` 1.379 B. `tools/list` = **4.956 B** (3.390 B só de schemas; 16 tools) — pago
por cliente que lista tools na conexão.

### 3.2 Custo de observar passo a passo

Laço sintético de 200 × (`run` 1 passo + `inspect`): **400 chamadas, 253.236 bytes
(~63,3k tokens), 2,06 s → 1.266 B (~317 tokens) e 10,3 ms por passo**. No Mega Drive, laço de
50 × (`run` 1 + `inspect`): **100 chamadas, 103.594 bytes, 514 ms → 2.072 B (~518 tokens) e
10,3 ms por passo** (a inspeção Mega Drive devolve 1.789 B contra 989 B do sintético).
Extrapolando o Mega Drive: 10.000 passos = 20.000 chamadas, 20,7 MB, ~5,2M tokens, ~103 s —
para um emulador que executa esses 10.000 passos em ~1,04 s (9.609 passos/s medidos). Um laço
equivalente **dentro do script** (10.000 × `step(1)` + `register("pc")`) roda em 3,5 s de CPU
e ~8,0 s de wall, sem tokens. `experiment.run` de 1.000 passos com 3 checkpoints custa **1
chamada / 1.068 bytes / 6,9 ms**, mas devolve só hashes — não dá PC, registrador nem
disassembly.

### 3.3 Piso de latência do transporte

`GET /health` (resposta de 47 B), 200 requisições sequenciais: **1.031 ms → 5,16 ms/req**;
4 threads × 50: 262 ms → **763 req/s**. A causa dominante é o loop de accept: quando a fila
está vazia ele dorme `ACCEPT_POLL = 5 ms` (`transport/src/http.rs:44,354,369`), então uma
conexão recém-chegada espera até um poll inteiro antes de ver um worker. O custo do handler é
irrelevante em comparação. Isso não é só desempenho: **cada chamada MCP de um agente tem um
piso de ~5 ms**, e o cenário de 400 chamadas acima leva 2,06 s quase todo em espera de accept.

### 3.4 Comparação de superfícies (medido)

| Caminho | Chamadas | Bytes | Tempo | Observação |
|---|---|---|---|---|
| MCP sintético, 200 × (1 passo+inspect) | 400 | 253.236 | 2,06 s | 1,27 kB/passo |
| MCP Mega Drive, 50 × (1 passo+inspect) | 100 | 103.594 | 0,51 s | 2,07 kB/passo |
| HTTP `/v1/ops` inspect (sintético) | 1 | 811 | ~5 ms | mesma sessão via MCP: 989 B (texto JSON dentro de string, 1,22×) |
| CLI `debug inspect --steps 100` | 1 processo | 449 | 0,02 s | mas re-executa 100 passos; a 1M de passos ≈ 104 s |
| CLI `debug run` com breakpoint/watch/event | 1 processo | 448 | 0,02 s | melhor ergonomia por comando, sem handle |
| script 10k passos + 10k `register` | 1 processo | relatório 64 KiB máx. | 3,5 s CPU | melhor custo/passo, sem breakpoint/watchpoint |
| `experiment.run` 1.000 passos | 1 | 1.068 | 6,9 ms | só hashes |

**A6-F2 — Sem cancelamento, progresso ou deadline de execução; uma execução longa congela
todo o serviço**
- *Evidência (experimento):* um `session.run` de 100k passos no Mega Drive levou 9,42 s; durante
  ele, `session.create` (9.124 ms), `session.list` (9.125 ms), `session.inspect` de **outra**
  sessão (9.127 ms) e `session.read_memory` de outra sessão (9.127 ms) ficaram todos
  bloqueados; o lock do manager é mantido por todo o laço (`service/src/service.rs:461-491`;
  `lock_manager` em `:691-693`). Fechar o socket do cliente 0,2 s depois do início **não**
  interrompeu a execução: a inspeção seguinte esperou 9.345 ms e o tick avançou até o fim.
  `notifications/cancelled` recebe `202` e é ignorada (`mcp.rs:113-116`). Não há `progress`
  (o `progressToken` em `params._meta` nunca é lido).
- *Condições:* qualquer cliente `write`; um agente que peça 100k passos (o máximo permitido) ou
  que abandone a conexão. *Impacto atual:* o daemon inteiro fica serializado por ~10 s por
  execução; quatro execuções concorrentes = ~40 s de fila. *Impacto futuro:* piora com M8
  (passos mais caros) e impede um agente de operar em paralelo.
- *Probabilidade:* alta (é o caminho normal de `session.run`). *Severidade:* alta. *Confiança:*
  alta. *Alternativas:* (a) nada; (b) reduzir `max_steps_per_request` (paliativo, não resolve
  cancelamento); (c) **deadline + cancelamento + progresso por requisição** (recomendada);
  (d) lock por sessão em vez de lock global (mudança arquitetural maior, melhora paralelismo).
- *Recomendação:* (c) agora e avaliar (d) em M8/M9: `max_wall_ms` por operação, `notifications/
  cancelled` honrada, `session.cancel`/`session.progress` in-band (sem SSE), e `run` fatiado
  pelo cliente quando precisar de mais que o deadline. *Custo/risco:* médio (thread de
  execução cancelável + lease/lock); contratos de `run` e testes de concorrência mudam.
  *Prioridade:* P1. *Milestone:* M6/M8.

### 3.5 Proposta de API mínima de alto valor (não espelhar a CLI)

Nove operações compostas, todas com envelope canônico e limites explícitos:

| Tool | Entrada (resumo) | Saída | Substitui na CLI | Ganho |
|---|---|---|---|---|
| `session.state` | `{session}` | tick, hash, halted, PC/SR/IRQ, registradores (já existe como `inspect`) | `inspect`/`vdp`/`interrupts` | mantém |
| `session.run` **estendido** | `steps`, `inputs` (array limitado de u32 ou pares run-length ≤1.024), `stop_on` `{pc:[a,b], watch:{address,length,access}, event:"interrupt|vdp|controller|bus-write", halt:true}`, `max_wall_ms`, `observations:{max:64}` | executados, tick, hash, `stop:{reason,...}`, observações limitadas | `debug run` (break/watch/event/log) + `session.run` | fecha F1/F2/F7 |
| `session.cancel` / `notifications/cancelled` | `{request_id}` | `cancelled` | — | fecha F2 |
| `session.progress` | `{token}` | passo/tick atual (polling in-band, sem SSE) | — | progresso sem M6-FU4 |
| `memory.read` (existe) / `memory.write` | `{session,address,length}` / `{...,bytes}` ≤4 KiB | bytes/lido | `debug memory` (escrita não existe) | fecha lacuna de patch |
| `memory.diff` | `{left,right,address,length}` | primeiras faixas divergentes (≤16) | — | comparação de regiões |
| `debug.disassemble` | `{session,address,count ≤256}` | linhas addr/opcode/texto | `debug disasm` | RE sobre transporte |
| `trace.capture` / `trace.query` | `{session,steps,kinds,capacity≤64k}` / `{trace,filter,limit}` | id + metadados / registros limitados | `trace capture|query` | tracing sobre transporte |
| `frame.capture` | `{session}` | `{width,height,format,sha256,data≤96 KiB base64}` | — (não existe) | agente "vê" a tela |

Regras de projeto: (i) parâmetros limitados e recusados antes do trabalho; (ii) resposta
sempre ≤ `max_response_bytes`; (iii) toda operação mutante com lease + deadline; (iv)
`stop_on`/`inputs` são um único verbo em vez de dez; (v) TAS de edição continua CLI até haver
demanda. Custo estimado: 1,5–3k LOC + testes de paridade e limites; 1–2 fases.

---

## 4. Conformidade MCP/HTTP e versão vigente

Requisitos primários citados em `areas/06-evidence/mcp-spec-excerpts.md`. Situação em
2026-11-20: o repositório oficial lista as revisões `2024-11-05`, `2025-03-26`, `2025-06-18`,
`2025-11-25`, **`2026-07-28` (vigente)** e `draft`; o changelog de `2026-07-28` remove
sessões e o handshake `initialize`, adiciona `server/discover`, `_meta` por requisição e
`Mcp-Method`/`Mcp-Name`. O ChronoFork implementa `2025-03-26` (`transport/src/mcp.rs:29`).

| Requisito | Implementação | Resultado |
|---|---|---|
| Servidor **MUST** validar `Origin` (2025-03-26 e 2026-07-28) | nenhuma referência a `Origin` no transporte; requisição com `Origin: http://evil.example` em `/mcp`, `/v1/ops` e `/health` → 200 | **não conforme** |
| Sessão `Mcp-Session-Id` opcional; stateless permitido | nenhum header emitido/validado; `Mcp-Session-Id` inventado é ignorado | **conforme** 2025-03-26 |
| POST responde `application/json` **ou** `text/event-stream` | responde `application/json; charset=utf-8`; `Accept: text/event-stream` sozinho é aceito e respondido em JSON | **conforme** (servidor pode escolher) |
| GET deve ser SSE ou 405 | `GET /mcp` → 405 + `Allow: POST` (119 B) | **conforme** |
| DELETE pode ser 405 | `DELETE /mcp` → 405 + `Allow: POST` | **conforme** |
| Notificação (sem `id`) → 202 sem corpo | `notifications/initialized` e qualquer notificação → 202, 0 B | **conforme** |
| `MCP-Protocol-Version` obrigatório após init (2025-06-18+) | header é ignorado; requisição com `2025-06-18` → 200 | **não implementado** (versão antiga não exigia; clientes novos enviam) |
| Negociação de versão no `initialize` | sempre devolve `2025-03-26`, inclusive quando o cliente pede `2025-06-18` | aceitável para 2025-03-26; **insuficiente** para clientes 2025-06-18+/2026 |
| 2026-07-28 (POST-only, sem handshake, `server/discover`, `_meta`, `Mcp-Method/Mcp-Name`) | não implementado; `resources/list` → `-32601`; endpoint aceita POST sem `initialize` (o que confunde a detecção "moderna" do cliente) | **não conforme** com a revisão vigente |
| Batches | `-32600` + 400 | aceitável (2025-03-26 permitia; 2025-06-18 removeu) |
| Códigos JSON-RPC | `-32700` parse, `-32600` invalid, `-32601` método, `-32602` params/tool | **conforme** |
| `Accept` do cliente | não validado (leniente) | sem impacto prático; não é violação do servidor |

**Interoperabilidade:** não há teste com cliente real/SDK oficial — os testes de transporte
usam o cliente próprio (`transport/tests/mcp.rs`, `transport/src/client.rs`). Um cliente
2025-06-18+ tende a funcionar porque a maioria aceita servidores 2025-03-26; um cliente
2026-07-28 pode falhar na detecção (nosso servidor responde a `tools/list` sem handshake, mas
não implementa `server/discover`, `_meta` nem os erros `UnsupportedProtocolVersionError`).
*Inferência, confiança média* — não foi possível instalar um SDK real no tempo disponível.

**A6-F6 — Versão MCP defasada e validação de Origin ausente**
- *Evidência:* `mcp.rs:29`; execuções em §4; `mcp_probe.json` (chaves `conformance`).
- *Impacto atual:* erosão de compatibilidade com hosts atuais; violação de um MUST de
  segurança da spec (defesa em profundidade contra DNS rebinding).
- *Impacto futuro:* alto — o produto quer ser interoperável com agentes; a spec muda em
  ciclos anuais.
- *Alternativas:* (a) ficar em 2025-03-26 (barato, piora); (b) **adicionar já `Origin`,
  `MCP-Protocol-Version` e testes de interop, e planejar o modo duplo 2025-03-26/2026-07-28
  para M8** (recomendada); (c) adotar `rmcp` como servidor (migração alta).
- *Recomendação:* (b). Validar `Origin` quando presente (403 se inválido; permitir ausente
  para clientes não-browser, como fazem SDKs), responder 400 a `MCP-Protocol-Version`
  desconhecido, aceitar `initialize` e/ou requisições com `_meta` num período de transição,
  implementar `server/discover`, `Mcp-Method`/`Mcp-Name` e `UnsupportedProtocolVersionError`.
  Adicionar `rmcp` como **dev-dependency** para testes de conformidade (impacto zero no
  binário).
- *Custo/risco:* 200–600 LOC + ADR; risco de duplicar caminhos — mitigar com um único
  adaptador de versão e testes de matriz. *Prioridade:* P1 (Origin/header) e P2 (2026-07-28).
- *Milestone:* M6-FU4/M8.

---

## 5. Alternativas tecnológicas (matriz)

Contagens de dependências transitivas medidas com `cargo metadata` em 2026-11-20
(`/tmp/review-deps`, features: `serde` derive; `hyper` server+http1; `axum`; `tokio`
rt-multi-thread+net+macros; `rmcp` server+streamable-http-server; `rhai`; `mlua` lua54
vendored). “Superfície de ataque” é julgamento, não medida.

| Opção | Versão / licença | Trans. | Maturidade | Superfície de ataque | Encaixe/perf | Portabilidade | Testes | Migração |
|---|---|---|---|---|---|---|---|---|
| **Atual** (JSON próprio + HTTP/MCP à mão) | workspace | **0** | primeira parte, 864+961+279+432 LOC, testes 1.356+594+375+797 | parser próprio sem corpus de fuzz; sem time de segurança | ~5 ms/chamada por poll do accept; 763 req/s | excelente (std) | fortes no repo; sem interop real | — |
| `serde`/`serde_json` | 1.0.229 / 1.0.154, MIT/Apache | 6 / 10 | altíssima | maduro, muito auditado | similar | excelente | fuzz/ecossistema | **média**: não expressa o perfil canônico (chaves duplicadas, floats, faixa ±2^53, ordenação, sem whitespace) sem visitor + canonicalizador; ganho duvidoso |
| `tiny_http` | 0.12.0, MIT/Apache | 4 | alta, baixo nível, blocking | pequeno; **fork mantido `tiny_http_dh` anuncia "CVE fixes"** (checar CVEs antes) | blocking, thread/conn — bom encaixe; removeria o poll de 5 ms | boa | maduro | média: substitui `http.rs` e parte dos testes de framing |
| `hyper` | 1.12.0, MIT | 22 (+tokio 13) | altíssima | grande ecossistema; async | excelente throughput, HTTP/2/TLS | boa | excelente | **alta**: runtime async + serviço síncrono (`&mut SessionManager`, mutex) exigiria `spawn_blocking` e refatoração |
| `axum` | 0.8.9, MIT | 57 | altíssima | maior que hyper | excelente | boa | excelente | muito alta; overkill para 4 rotas |
| `rmcp` (SDK oficial Rust) | 3.11.0, **Apache-2.0**, 106 trans. (0.8.5 era MIT, 98) | 106 | alta e oficial | grande; tokio+serde+schemars | boa; sessões/SSE prontos | boa | alta | **alta** como servidor; **baixa** como dev-dependency de conformidade |
| `rhai` | 1.27.0, MIT/Apache | 53 (inclui MPL-2.0+ e CC0 no closure) | alta, pure Rust | média | boa para DSL | excelente | alta | alta: perderia o relatório/limites determinísticos e os bindings atuais |
| `mlua` (Lua) | 0.12.2, MIT | 37 + toolchain C (`lua-src` vendored) | alta | FFI/unsafe na dependência; CVE de Lua | boa | pior (C cross-compile) | alta | alta; conflita com “sem unsafe/sem C” |

**Leitura:** redução de dependências **não é métrica isolada**. O código à mão tem 0
dependências transitivas, mas também tem um parser HTTP de primeira parte sem corpus de fuzz
e sem interop testada; `tiny_http` (4 transitivas) é a alternativa de melhor relação
risco/ganho para o HTTP; `rmcp` tem 106 transitivas mas é o SDK oficial e resolveria
conformidade/streaming, ao custo de async e de inverter a arquitetura síncrona. O codec
canônico é um **requisito de protocolo** (paridade byte a byte), não duplicação: substituí-lo
por `serde_json` exigiria reimplementar as mesmas regras em um visitor, com pior clareza.
**Recomendação:** manter o codec e o servidor atual; corrigir o poll do accept e adicionar
`cargo-fuzz`; adicionar `rmcp` como dev-dependency para testes de interop; reavaliar
`tiny_http` apenas se o HTTP ganhar TLS/streaming/multiplexação. Não adotar Lua. O DSL
próprio continua justificado por determinismo e limites, mas seu custo de manutenção deve
ser monitorado.

**A6-F5 — Poll de accept impõe piso de latência e teto de vazão**
- *Evidência:* 200 `/health` = 1.031 ms (5,16 ms/req); 4×50 em 262 ms; `http.rs:44,354,369` (polls) e `http.rs:377-403` (worker).
- *Impacto:* toda chamada de agente custa ≥1 poll; o cenário de 400 chamadas leva 2,06 s.
- *Alternativas:* (a) nada; (b) reduzir `ACCEPT_POLL` para 0,5 ms (barato, ainda polling);
  (c) **accept bloqueante + self-pipe/condvar para shutdown** (recomendado; remove o piso);
  (d) trocar por `tiny_http`.
- *Recomendação:* (c) — ~50 LOC, sem dependência, mantém o modelo de threads. *Custo/risco:*
  baixo; exige teste de shutdown e de fila. *Prioridade:* P2. *Milestone:* M6-FU4/M8.

---

## 6. Segurança

### 6.1 Trust boundaries

```text
[hostil?] rede/loopback ──▶ socket (pré-auth: read_head, /health)
                                   │ token Bearer (estático, arquivo)
                                   ▼
                          parser HTTP/JSON (bounded)
                                   │
                                   ▼
              AppService: scopes, ownership, leases, limites
                     │ manager lock        │ storage/artifacts
                     ▼                     ▼
              sessões em memória      arquivos 0644/0755 sob --store-root/--artifacts
                     ▲
   CLI ──▶ arquivos do host (ROM, token, movie, trace, script; leitura bounded)
   Script ──▶ sandbox canônico (somente trace_export/trace_compare/tas_*)
   GUI ──▶ cliente HTTP (token) ou serviço in-process; não escuta
```

Fronteiras reais: (1) rede→daemon, protegida só por token Bearer estático e limites de
framing; (2) requisição→serviço, protegida pelo parser + validação de campos; (3)
serviço→sessão, com ownership por principal; (4) serviço→disco, com confinamento de caminho,
mas sem quota nem permissões restritas; (5) script→host, **sandbox de capacidades
cooperativo**, não fronteira de SO contra código hostil (declarado em `script/src/lib.rs:13-17`);
(6) arquivos de entrada da CLI: parsers de primeira parte, com limites de tamanho, mas sem
fuzz contínuo.

**Operações simultâneas e recuperação.** Toda mutação de sessão toma um *writer lease* por
sessão antes do lock global (`service/src/service.rs:668-680`): uma segunda mutação
**sobreposta** da mesma sessão falha com `conflict` retryable, enquanto leituras não tomam
lease e, ainda assim, esperam o lock global do manager (é o que produz o convoy de F2). O
armazenamento publica checkpoints/artefatos por arquivo temporário + rename atômico
(`experiment.rs:137-168`), `store.list` reporta `anomalies.count` sem reparar, mutex
envenenado é tolerado (`PoisonError::into_inner`) e um worker que panicar não é substituído
(S7). Sessões vivas não sobrevivem a restart (M6-FU2) e o daemon não tem supervisão de
processo; a recuperação é “recomeçar do store durável”, não “retomar a sessão”.

### 6.2 Achados de segurança (estrutura pedida pelo baseline)

**A6-S1 — Snapshots sem quota: memória cresce sem limite por cliente autenticado**
- *Evidência (experimento):* 100 × `session.snapshot` em uma sessão Mega Drive: RSS 4.224 →
  25.652 kB (+21,4 MB, ~214 kB/snapshot) sem nenhum erro; `Session::snapshot` insere em
  `BTreeMap` sem limite (`sessions/src/session.rs:151-158`); `ServiceLimits` não tem quota de
  snapshots (`service/src/limits.rs:33-42`); `max_sessions` só conta sessões
  (`service/src/service.rs:372-377`).
- *Condições:* token com escopo `write` (qualquer agente). *Impacto:* OOM do daemon derruba
  todas as sessões (não há persistência de sessões vivas); 1.024 sessões × snapshots
  ilimitados. *Futuro:* pior com M8 (estado maior, ROM real por sessão).
- *Probabilidade:* alta (é um loop trivial). *Severidade:* alta. *Confiança:* alta.
- *Alternativas:* (a) nada; (b) **quota de snapshots por sessão e orçamento global de
  bytes de checkpoint no `ServiceLimits`** (recomendada); (c) limite só de contagem (não
  limita bytes). *Recomendação:* (b): `max_snapshots_per_session` (ex. 64),
  `max_checkpoint_bytes_total` (ex. 256 MB) recusando antes de capturar, e `retention`
  documentada. *Custo/risco:* baixo (limites + erros tipados + testes); altera contrato de
  limites (documentar). *Prioridade:* P1. *Milestone:* M6/M8.

**A6-S2 — Store durável sem quota: disco cresce sem limite**
- *Evidência:* 20 × `store.save` no mesmo root = 4,3 MB / 62 arquivos, todos 0644, sem erro;
  `max_store_entries` só limita a **listagem** (`limits.rs:40`), não a escrita; não há quota
  de bytes nem GC.
- *Impacto:* um cliente `write` enche o disco do host; checkpoints podem conter bytes
  derivados do cartucho (o checkpoint Mega Drive tem ~197 kB > RAM+VRAM), então também é
  confidencialidade em diretórios compartilhados. *Severidade:* alta/média. *Confiança:* alta.
- *Recomendação:* quota de bytes por root + permissões 0700/0600 (ver S4) + erro tipado
  `limit_exceeded`; documentar retenção. *Prioridade:* P1/P2.

**A6-S3 — Slowloris pré-autenticação esgota o pool de workers**
- *Evidência (experimento):* daemon `--workers 2`; duas conexões enviando 1 byte a cada 4 s
  (< `io_timeout` 5 s) prendem os dois workers; `GET /health` legítimo falha por timeout em
  4.002 ms; após liberar, `/health` volta a 200. `http.rs:406-407` aplica timeout **por
  leitura**, não por requisição; `read_head` roda antes de rota/autenticação
  (`http.rs:405-407,441-489,589`).
- *Condições:* qualquer processo que alcance a porta; default é loopback, mas `--bind`
  aceita `0.0.0.0` com apenas um warning (`cli/src/agent/serve.rs:90-94`).
- *Impacto:* negação de serviço não autenticada; com 4 conexões, todo o daemon (default
  `workers=4`). *Severidade:* alta se não-loopback; média em loopback multiusuário.
- *Alternativas:* (a) nada; (b) deadline total de requisição (ex. 10 s) além do timeout por
  leitura; (c) limite de conexões por IP + accept blocking; (d) proxy TLS com timeouts.
- *Recomendação:* (b)+(c): deadline absoluto medido no `handle_connection`, fechando com 408,
  e um teto de conexões por peer. *Custo:* baixo/médio; testes de slowloris. *Prioridade:* P1.

**A6-S4 — Arquivos sensíveis com permissões padrão e sem verificação**
- *Evidência:* token file 0644 aceito sem aviso (daemon iniciou normalmente); store e
  artefatos criados 0644 em diretórios 0755 (`umask 022`); nenhuma checagem de modo/symlink em
  `read_token_table` (`cli/src/agent/mod.rs:207-218`). `/home/stealthc` é 750 neste host, o
  que mitiga localmente, mas o CLI aceita qualquer caminho (ex. `/tmp`, volumes de CI).
- *Impacto:* leitura de bearer tokens e de checkpoints por outros usuários/containers;
  tokens estáticos não têm rotação/expiração/auditoria (M6-FU3, documentado).
- *Recomendação:* recusar (ou avisar alto) token file com bits de grupo/outros; criar
  store/artefatos 0700/0600; documentar que o modelo é loopback/usuário único até TLS+OIDC.
  *Prioridade:* P2 (defesa em profundidade) / P1 se o bind não-loopback for suportado.

**A6-S5 — Ownership durável ausente (M6-FU2)**
- *Evidência:* `store.list/load/save/delete` exigem apenas escopo, sem checar principal
  (`service/src/service.rs` store ops); sessões em memória têm owner, objetos duráveis não
  (`docs/architecture/agent-interfaces.md:430-435`). Um `write` pode apagar sessões duráveis
  de outro principal; um `read` pode ler todas.
- *Impacto:* em um daemon multiusuário/equipe, isolamento inexistente no disco.
  *Severidade:* média. *Recomendação:* persistir owner no metadata (M6-FU2) ou documentar
  formalmente que o store é compartilhado por confiança. *Prioridade:* P2.

**A6-S6 — Robustez de autenticação/HTTP (baixa)**
- Comparação de token por `String::eq` (`auth.rs:124-129`) — não é constant-time (documentado,
  exploração remota por timing é ruidosa; baixa).
- `Authorization` duplicado: vence o **primeiro** (bom→200, ruim→401); RFC 9110 exige
  combinação/rejeição; risco de smuggling com proxy. `Bearer  token` (dois espaços) é aceito
  por `trim_matches`. `Host` ausente ou forjado é aceito (HTTP/1.1 exige Host).
- Corpo > limite: o servidor responde 413 e fecha enquanto o cliente ainda escreve → o
  cliente vê `ECONNRESET`, não o 413 (baixo, ergonomia de API).
- `read_memory` em I/O mapeado devolve `backend_error` com endereço (sem path) — bom.
- *Recomendação:* rejeitar `Authorization` duplicado, exigir Host, aceitar só um espaço,
  documentar o reset de corpo grande. *Prioridade:* P3.

**A6-S7 — Worker morto não é substituído; sem isolamento de panic**
- *Evidência:* `worker_loop` não usa `catch_unwind`; um panic em handler reduz o pool
  permanentemente (`http.rs:377-403`); mutex envenenado é tolerado
  (`unwrap_or_else(PoisonError::into_inner)`). O fuzz não encontrou panic (47 corpos MCP +
  framing + 160 parsers binários), então é robustez latente. *Prioridade:* P3.

**A6-S8 — Parsers de primeira parte sem fuzz contínuo**
- *Evidência:* 160 execuções de `trace info`, `tas verify`, `debug inspect --rom` com 40
  arquivos aleatórios (0–4 KiB): todas exit 1 tipado, 0 sinais, 0 timeouts
  (`fuzz_binary_parsers.json`). Cobertura pontual, não contínua; não há alvos `cargo-fuzz` no
  repositório. *Recomendação:* alvos de fuzz para `CFSNAP`, `CFMOVIE2`, `CFTRACE1` e para o
  parser JSON/HTTP; corpus versionado. *Prioridade:* P2 (assurance).

**A6-S9 — Sandbox de script: contenção correta, mas deadline não limita um builtin**
- *Evidência positiva:* `trace_export("../escape")`, caminho absoluto e symlink para fora →
  `failure=sandbox_path`; export válido em `sub/out.cftrace` funciona
  (`script_escape*.out`, `script_ok_export.out`); `Sandbox` canonicaliza raiz e rejeita
  `.`/`..`/symlink (`script/src/sandbox.rs:86-199`).
- *Evidência negativa:* script `open(); step(100000)` com `--wall-time-ms 100` executou
  **10,41 s de CPU / 17,97 s de wall** e só então falhou por wall-time; o deadline é checado
  a cada 256 operações e entre statements (`eval.rs:210,1058`), não dentro do builtin
  `step`. Com `--max-steps 100.000.000` (máximo) isso pode durar horas.
- *Impacto atual:* CLI local (auto-DoS/CI travado). *Impacto futuro:* **bloqueador de
  qualidade para expor script via MCP (M6-FU1)** — seria DoS remoto trivial.
- *Recomendação:* checar deadline dentro do laço de passos do host e/ou impor
  `max_wall_ms` por builtin; ao expor via transporte, executar em thread cancelável com
  deadline rígido. *Prioridade:* P2 agora, P0 antes de M6-FU1.

### 6.3 Registro estruturado (campos exigidos pelo baseline)

Para não repetir prosa, a tabela consolida os campos que faltam nos blocos acima.
**Mapeamento dos achados do sumário:** A6-F3 = S1 (memória) + S2 (disco); A6-F4 = S3;
A6-F7 = proposta de §3.5 + lacunas da matriz de §2; A6-F8 = S4 + S5.

| ID | Condições | Prob. | Severidade | Confiança | Alternativas | Recomendação | Custo/risco | Prior. | Milestone |
|---|---|---|---|---|---|---|---|---|---|
| S2 | token `write`; store configurado | alta | alta/média | alta | (a) nada; (b) quota de bytes + GC; (c) permissões 0600/0700 sem quota | (b)+(c): `max_store_bytes` por root, erro tipado antes de gravar, 0700/0600 | baixo; altera limites e testes de store | P1/P2 | M6/M8 |
| S3 | porta alcançável; ≤4 conexões truncadas | alta se não-loopback | alta/média | alta | (a) nada; (b) deadline total + 408; (c) limite por peer; (d) proxy TLS | (b)+(c), e impedir `0.0.0.0` sem flag explícita | baixo/médio; testes de slowloris e de shutdown | P1 | M6 |
| S4 | token/store em caminho legível por terceiros | média | média | alta | (a) documentar; (b) recusar modo inseguro; (c) avisar | (b)+(c): recusar token 0644 e criar store/artefatos 0600/0700 | baixo; documentação operacional | P2 | M6-FU3 |
| S5 | daemon multiusuário/equipe | média | média | alta | (a) documentar store compartilhado; (b) owner no metadata (M6-FU2) | (b) se houver multiusuário; senão (a) explícito | médio (muda formato M2) | P2 | M6-FU2 |
| S6 | proxy/hostil na mesma rede | baixa | baixa | alta | (a) nada; (b) endurecer | (b): rejeitar `Authorization` duplicado e Host ausente; 413 legível; um espaço | baixo | P3 | M6 |
| S7 | panic em handler (não observado) | baixa | baixa | média | (a) nada; (b) `catch_unwind` + worker substituto | (b) se o pool for crítico | baixo | P3 | M8 |
| S8 | parser de formato novo/regressão | média | média | alta | (a) fuzz pontual; (b) `cargo-fuzz` contínuo no CI | (b) para JSON/HTTP/CFSNAP/CFMOVIE2/CFTRACE1 | médio (infra de CI) | P2 | M8 |
| S9 | script local/CI; exposição futura | alta (local) | média (P0 se exposto) | alta | (a) documentar; (b) deadline dentro do builtin; (c) thread cancelável | (b) agora; (c) antes de M6-FU1 | baixo/médio | P2 (P0 antes de expor) | M6-FU1 |



### 6.4 Postura geral

O produto é **seguro para loopback/ambiente confiável de usuário único** (é o que a
documentação diz) e **não é seguro** para: bind não-loopback (sem TLS, tokens estáticos,
slowloris, 644), host multiusuário com token em caminho compartilhado, ou múltiplos agentes
não cooperativos com escopos amplos (sem quota, sem auditoria, sem ownership durável). Nada
no código se apresenta como sandbox de código hostil; a documentação do script é honesta. A
classificação “sandbox de capacidades cooperativo vs fronteira real” está correta no código e
nos docs — o risco é operacional (alguém rodar `--bind 0.0.0.0`), mitigado por um warning
que não impede a operação.

---

## 7. Determinismo: estado emulado vs representação externa

**Determinismo necessário ao estado emulado (verificado):** dois forks do mesmo estado com o
mesmo input produzem `session.compare.equivalent=true`; `experiment run --steps 100
--checkpoints 0,50,100` executado duas vezes gera **bytes idênticos** (SHA-256
`b97b1df6a2948de1e909a42a3e5f5dc1b4a98005c015c5bfb7507b9e2519c73a`); hashes de estado, tick
e snapshots não contêm tempo de host, PID, path ou ordem de chegada (headers de resposta não
incluem `Date`; inspeção Mega Drive só tem campos do backend). Sessões são independentes;
o lock global serializa mas não altera bytes.

**Determinismo apenas da representação externa:** ordem de chaves, ausência de whitespace,
inteiros decimais, hex minúsculo, listas ordenadas (`session.list` ordenado por id),
envelope `{"op","result"}` e o embrulho MCP `content[0].text` (JSON dentro de string). O
embrulho MCP aumenta o corpo em 1,17–1,67× conforme o payload (inspect sintético: 811 B no
`/v1/ops` vs 989 B no MCP = 1,22×; inspect Mega Drive: envelope de 1.522 B → 1.789 B = 1,18×)
mas, depois de decodificar a string, os bytes são idênticos — é o contrato de paridade do M6 e está
correto. Esse determinismo de representação é valioso para testes e para artefatos
reproduzíveis, mas **não deve ser confundido** com o determinismo do emulador: nenhuma
propriedade do estado depende dele. O risco inverso (representação não determinística) é
baixo: o codec ordena chaves, os mapas de registradores chegam como objetos e o catálogo tem
ordem fixa; nenhuma iteração de `HashMap` vaza para a saída.

**Recomendação:** manter o contrato de bytes no envelope do serviço (paridade), mas testar a
igualdade **decodificada** no nível MCP (não bytes de transporte, que incluem `id`, headers e
`isError`). Ao adicionar streaming/SSE, a ordem de mensagens passa a fazer parte da
representação e precisa de teste próprio.

---

## 8. Limitações deste estudo

- **Não testei um cliente MCP real/SDK oficial** (sem instalação no ambiente): a avaliação de
  interop 2025-06-18/2026-07-28 é inferência a partir das specs primárias.
- Não revisei byte a byte os parsers de `CFSNAP`/`CFMOVIE2`/`CFTRACE1`; apenas fuzz black-box
  (160 execuções) e leitura de limites.
- Não medi constant-time de tokens nem tentei explorar timing.
- Não testei TLS (não existe), OAuth, nem múltiplos hosts.
- Benchmarks em máquina compartilhada (8 vCPU, WSL2); números de latência são de uma sessão,
  não um benchmark estatístico. O `ACCEPT_POLL` de 5 ms é diagnóstico (código + medida), não
  uma regressão comparativa com outra stack.
- O ROM usado é sintético (`interrupt_rom(3)`, 1.024 B), não um cartucho comercial; conclusões
  sobre memória não incluem o custo de ROMs reais (que agrava S1/S2).
- Não avaliei `tas`/`studio` em profundidade (fora do escopo de interfaces; coberto por A?).
- O `cargo metadata` de alternativas usa features escolhidas por mim; contagens transitivas
  variam com features.

---

## 9. Prioridades e decisões para o proprietário

### Prioridades de implementação (separadas de severidade)

| P | Item | Por quê |
|---|---|---|
| **P1** | Cancelamento + deadline + progresso em `session.run`; deadline total de requisição no HTTP | sem isso, um agente não controla execuções de ~10 s e um cliente pode negar serviço |
| **P1** | Quotas de snapshots/checkpoints (memória) e de store (disco) | DoS trivial por cliente autenticado; cresce com M8 |
| **P1** | Validação de `Origin` + `MCP-Protocol-Version` + plano de versão 2026-07-28 + testes de interop com `rmcp` (dev) | conformidade e interoperabilidade são o produto |
| **P1** | API mínima de RE (run com stop_on/inputs, memory.write, disassemble, trace, frame) | é o que torna “agent-native” verdadeiro |
| **P2** | Corrigir accept (blocking + self-pipe); limites por peer; permissões 0700/0600; Host/Authorization duplicado | desempenho e defesa em profundidade |
| **P2** | Alvos `cargo-fuzz` para JSON/HTTP e formatos binários; deadline dentro de builtins do script | assurance |
| **P3** | Polir erros (413 legível, `Bearer` estrito), supervisão de worker, auditoria/rotação de token | robustez |

### Decisões (com recomendação)

- **D1 — Expor debug/trace/script/TAS via MCP?** Recomendo **sim, por uma API mínima
  composável** (9 operações, §3.5), não espelhando a CLI; TAS fica CLI até haver demanda.
  Critério de aceitação: toda operação com limite, deadline cancelável, paridade decodificada
  e teste de abuso (corpo grande, timeout, cancelamento).
- **D2 — Subir a versão MCP?** Recomendo **modo duplo**: manter 2025-03-26 aceito, adicionar
  `Origin`/`MCP-Protocol-Version` já, e implementar `2026-07-28` (POST-only, sem handshake,
  `_meta`, `server/discover`, `Mcp-Method`/`Mcp-Name`) em M8, com `rmcp` como cliente de
  conformidade em testes. Não adotar `rmcp` como servidor agora (async + 106 transitivas +
  mudança de arquitetura).
- **D3 — Trocar a stack HTTP/codec?** Recomendo **não** trocar o codec (é requisito de
  protocolo) e **não** trocar o servidor agora; corrigir o poll e fuzzar. Reavaliar
  `tiny_http` (4 transitivas, mas checar CVEs/`tiny_http_dh`) se TLS/streaming entrarem.
- **D4 — Modelo de confiança.** Declarar formalmente “loopback/usuário único” até existir
  TLS+ownership durável (M6-FU2/FU3); impedir bind não-loopback sem flag explícita
  (ex. `--i-understand-no-tls`) ou exigir TLS. Enquanto isso, o produto não deve ser oferecido
  como multiusuário.

---

## Anexo — evidência bruta

Diretório: `.research/review-2026-11-20/areas/06-evidence/` (scripts e saídas; os mesmos
arquivos estão em `agents/interfaces/` como scratch). Principais:

- `mcp_probe.json` — cenário MCP (22 chamadas), conformidade (Origin/GET/DELETE/OPTIONS/
  versão/batch), segurança (auth/ownership), fuzz MCP (47 casos), economia.
- `mcp_megadrive.json` — superfície Mega Drive (registradores, memória, I/O não mapeado),
  throughput 100k passos, convoy do lock, cancelamento, loop de 50 passos.
- `snapshot_growth.txt` — RSS 4.224→25.652 kB em 100 snapshots.
- `slowloris.txt` — 2 workers presos, `/health` timeout em 4.002 ms.
- `fuzz_binary_parsers.json` — 160 execuções, 0 anomalias.
- `script_escape{1,2,3}.out`, `script_ok_export.out`, `script_step_long.{out,err}`,
  `script_loop.{out,err}`.
- `dbg_*.out`, `tr_*.out`, `gui_*.out`, `cli_*.out`, `exp1.json`/`exp2.json`,
  `mcp-spec-excerpts.md`.
- Scripts: `mcp_probe.py`, `md_probe.py`, `loop.cfs`, `escape*.cfs`.

Comandos-chave (todos executados na worktree; binário release em
`/tmp/chronofork-review-target/release/chronofork`, `CARGO_HOME=/tmp/t7-cargo`):
`serve --token-file … --bind 127.0.0.1:0 [--rom review.rom]`, `python3 mcp_probe.py`,
`python3 md_probe.py`, `script run --sandbox …`, `debug …, trace …, gui run --remote …`,
`experiment run` (×2, `cmp` idêntico), `cargo metadata` em projeto temporário.

**Estado do checkout:** nenhuma alteração em arquivos versionados; o helper
`crates/chronofork-megadrive/examples/review_dump_rom.rs` foi usado para gerar o ROM sintético
e removido ao final; daemons encerrados.
