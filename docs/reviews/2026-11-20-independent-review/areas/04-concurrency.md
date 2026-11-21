# A4 — Concorrência, paralelismo e isolamento de sessões

**Reviewer:** concurrency (agente `concurrency`, time `chronofork-review-2026-11-20`)
**Baseline:** `origin/main` `b8d63a17349e672ba6945eaba3379cec38cf9d1b`
**Worktree:** `/home/stealthc/projects/ChronoFork/.agents/worktrees/review-2026-11-20-architecture`
**Data:** 2026-11-20 (America/Fortaleza)
**Ambiente:** WSL2 Ubuntu ext4, 8 vCPU, `rustc 1.100.0`, `cargo 1.100.0`, release.
`CARGO_HOME=/tmp/t7-cargo`, `CARGO_TARGET_DIR=/tmp/chronofork-review-target`.
**Raw:** [`areas/04-evidence/`](04-evidence/) (logs + fonte do experimento + `SHA256SUMS`).

> Convenção: **fato observado**, **experimento**, **inferência**, **hipótese**, **opinião**.
> Toda afirmação numérica vem de log em `04-evidence/`. Mediana de duas execuções
> (`run2`, `run3`) salvo indicação contrária; o host é compartilhado com outros revisores,
> então valores absolutos oscilam — as razões entre cenários foram estáveis.

---

## 1. Sumário executivo

1. **Todas as operações de sessão são serializadas por um único `Mutex<SessionManager>`**
   (`crates/chronofork-service/src/service.rs:114-122`). `session.run` mantém esse lock
   durante **todo** o laço de passos (`:461-491`), até o limite padrão de **100.000 passos**
   (`crates/chronofork-service/src/limits.rs:37`). Sessões independentes **não executam em
   paralelo**: com 8 sessões/8 threads o throughput agregado é o mesmo de 1 thread
   (≈4.600–5.100 steps/s), `cpu/wall ≈ 0,98` e a parede cresce linearmente. **fato
   observado** (E1), confiança **alta**, prioridade **P1**.
2. **Leituras não bloqueiam por lease, mas bloqueiam pelo lock**: um `session.inspect` em
   **outra** sessão espera 230–737 ms durante um `run` de 1.000 passos e até 1,39 s sob dois
   runners (E2/E7). Não há starvation bound: `std::sync::Mutex` não é justo. **experimento**,
   confiança **alta**, **P1**.
3. **Não existe cancelamento/interrupção.** O catálogo tem 16 operações e nenhuma
   `pause`/`cancel`/`interrupt` (`catalog.rs:300-460`); `session.close` durante um run é
   **rejeitado imediatamente** com `conflict` (lease, E3). Um cliente que desconecta **não
   cancela**: o servidor seguiu computando (E10, +137 ms observados; e um `run` de 100.000
   passos continuou e aplicou estado após o cliente já ter falhado). **experimento**,
   confiança **alta**, **P1**.
4. **O limite de 100.000 passos ≈ 9–24 s de lock global** (medido 20.000 passos = 4,65–4,73 s;
   extrapolação a 100k = 23,3–23,6 s). Isso **excede o read timeout de 5 s do cliente tipado**
   (`client.rs:20-22`): medido, `session.run` de 100.000 passos sobre HTTP falha no cliente em
   **5,07 s**, um `inspect` seguinte falha em **5,12 s**, e o servidor **aplica** o run
   (`tick=8.970.094` após 25 s). Resultado: *unknown outcome* + bloqueio global. **experimento**,
   confiança **alta**, **P1**.
5. **Registro sem remoção**: `Registry` não tem API de remoção (`service.rs:89-95`) e
   `SessionManager` idem; `max_sessions` (1024) é um limite **vitalício**. Demonstrado com
   `max_sessions=4`: após fechar as 4 sessões (`live=0`), `session.create` continua
   `LimitExceeded` (E5). Sessões fechadas **retêm** o backend: **387,5 KiB/sessão** medidos,
   RSS idêntico antes/depois do `close` (E6). **P1** (daemon multiusuário) / **P2** (CLI curta).
6. **Existe um caminho paralelo acidental e correto**: `experiment.run` cria um
   `SessionManager` **privado** e não toca no lock global (`service.rs:619-639`,
   `experiment.rs:30-39`); 8 threads → 51,0 exp/s vs 8,55 exp/s com 1 thread (**5,97×**,
   `cpu/wall 4,42`). `store.load/save/delete` seguem o mesmo padrão (manager privado +
   writer lock de arquivo). Isso prova que o backend **é paralelizável** e isola o problema no
   desenho do serviço. **experimento**, confiança **alta**, **P2 (oportunidade)**.

**Conclusão de produto:** para o caso de uso declarado — “sessões independentes que humanos,
scripts e agentes inspecionam e bifurcam” — o serviço hoje entrega **isolamento lógico** (sem
interleaving, sem corrupção, determinismo preservado) mas **não isolamento de desempenho**:
um agente rodando uma sessão degrada todos os outros. Isso é aceitável para v1 single-agent e
**não é** aceitável para “experimentos simultâneos de vários agentes” (§7).

---

## 2. Modelo de concorrência observado (fatos de código)

### 2.1 Fronteira de sincronização

`AppService` é o único ponto de sincronização (`service.rs:114-122`):

| Campo | Tipo | Escopo do lock | Operações |
|---|---|---|---|
| `manager` | `Mutex<SessionManager>` | **toda** a operação de sessão, inclusive o laço de passos | create, list, inspect, run, fork, snapshot, restore, compare, close, read_memory |
| `registry` | `Mutex<Registry>` | curto; por vezes aninhado dentro do manager | ownership/closed; `create`/`fork`/`close`/`list` |
| `leases` | `Mutex<LeaseTable>` | lookup/insert/remove apenas | mutações (`run`, `fork`, `snapshot`, `restore`, `close`) |

Ordem documentada em `service.rs:7-17`: leases → manager → registry. Verifiquei por inspeção
que **não há inversão**: `authorize` (`:653-666`) pega `registry` e **libera** antes de
`acquire_lease` (`:669-684`); `create` (`:372-392`), `list` (`:394-429`), `fork` (`:493-514`)
e `close` (`:585-600`) pegam manager e depois registry, sempre nessa ordem. **Nenhum deadlock
ou inversão encontrado**; mutex envenenado vira `Internal` (`:762-764`) em vez de panic.
O `Lease` é declarado **antes** do guard do manager (`:465-466`), então é liberado **depois**
(Campo `Drop` `:146-154`) — o comentário de `:136-139` está correto. **fato observado.**

### 2.2 Escopo exato do lock em `session.run`

```rust
let _lease = self.acquire_lease(session)?;   // :465  falha rápido em conflito
let mut manager = self.lock_manager()?;      // :466  lock global até o fim do bloco
while performed < steps {                    // :470
    let outcome = manager.step(session, input)?;
    ...
}
let view = manager.inspect(session)?;        // :480  ainda sob o lock
```

- O lock cobre `steps` passos + o `inspect` final. `steps ≤ max_steps_per_request =
  100_000` (`limits.rs:37`).
- Parse do request e encoding canônico da resposta ficam **fora** do lock
  (`dispatch` `:226-242`: parse em `:227`, `to_canonical_bytes` em `:239-241`). Isso é bom e
  reduz o hold time para operações de resposta grande.
- Nenhum lock é mantido através de I/O de socket: `http::respond` chama `dispatch` e só
  depois `finish`/`write_response` (`http.rs:589-628`, `:620`). A afirmação de
  `docs/architecture/overview.md:202` (“never held across socket I/O”) **procede**.
  **fato observado.**

### 2.3 Leases: o que realmente garantem

- Lease é **por sessão e por mutação**; a segunda mutação concorrente falha em ~10 µs com
  `Conflict` + `retryable=true` (E3). **experimento.**
- Leitura **não** pega lease (`service.rs:17`) — logo, durante um `run` na **mesma** sessão:
  `inspect` **não** retorna `conflict`; ele **bloqueia** no lock global (medido 377–400 ms,
  E3). O ADR-0012 §5 diz que leituras “remain available while a mutation runs”; isso é
  verdadeiro apenas no sentido de que não são rejeitadas — na prática elas **esperam o run
  inteiro** (E2/E3). **fato observado** vs. **inferência**: a frase do ADR é otimista.
- `session.close` é mutação → durante um run recebe `Conflict` (E3, latência 0,00–0,01 ms).
  Ou seja, **a única operação de ciclo de vida não interrompe nada**, apenas é recusada.

### 2.4 Registry, leases e retenção

- `Registry` é `BTreeMap<SessionId, SessionRecord>` com `owner` e `closed`; **não há
  `remove`** (`service.rs:73-104`). `registered_count` conta **incluindo fechadas**
  (`:89-95`); `live_count` filtra `closed` (`:97-103`). `limits.rs:18-23` documenta
  honestamente que o limite é vitalício — mas nenhum caminho o mitiga.
- `Session` guarda `Box<dyn Backend>` + `initial_state` + `snapshots` (`session.rs:57-65`);
  `close` só troca o estado (`session.rs:283-286`). Nada é liberado.
- `SessionManager` também não tem remoção (`manager.rs:56-61`, sem `fn remove`).

### 2.5 Transporte (não-Tokio, thread-per-connection, blocking)

- Worker pool fixo de threads (`http.rs:281-343`), default **4** (`cli/src/agent/serve.rs:27`),
  fila de aceitação limitada a **16** (`http.rs:42`), `Connection: close` por request
  (`http.rs:4`, `:806`), sem keep-alive, sem SSE/streaming. Classificação: **I/O-bound** no
  socket, **CPU-bound** no `dispatch`.
- Cada request autenticado faz `service.dispatch` síncrono no worker (`http.rs:697`); MCP
  idem (`mcp.rs:188-224`). Não há offload para outro thread pool, fila de comandos por
  sessão nem prioridade.
- Portanto o desenho é: **N threads de conexão → 1 mutex global**. As threads são reais
  (paralelismo de I/O), mas a seção crítica é única.

### 2.6 Domínios de lock extra (paralelos ao manager)

| Domínio | Onde | Alcança sessões? |
|---|---|---|
| writer lock de arquivo por chave durável | `durable.rs:186-225` (`with_writer_lock`) | não usa o manager global (manager privado) |
| `Mutex<Queue>` + `Condvar` do transporte | `http.rs:167-183` | não |
| manager **privado** de `experiment.run` | `experiment.rs:46-49` | não é registrado no serviço |
| manager **privado** de `store.load/save` | `durable.rs:102-135`, `:139-225` | não é registrado no serviço |

Isso significa que **duas classes de operação já rodam em paralelo com o resto**:
`experiment.run`, `store.*` e `capabilities`/`tools` (sem lock). Todo o resto serializa.

### 2.7 Tabela operação × sincronização

| Operação | Lock global? | Lease | Paralelizável com outra sessão? | Observação |
|---|---|---|---|---|
| `capabilities`, `tools` | não | – | sim | `:303-335` sem lock |
| `session.create` | sim (+registry) | – | não | também consome cota vitalícia |
| `session.list` | sim, por **N inspects** | – | não | O(n) sob lock (`:394-429`) |
| `session.inspect` | sim | – | **não** (medido) | `:431-459` |
| `session.run` | sim, por `steps` | sim | **não** | `:461-491` |
| `session.fork` | sim (snapshot+restore) | sim | não | `:493-514` |
| `session.snapshot` | sim (serializa estado) | sim | não | `:516-539` |
| `session.restore` | sim | sim | não | `:541-559` |
| `session.compare` | sim (2 inspects) | – | não | `:561-583` |
| `session.close` | sim | sim | não | `:585-600` |
| `session.read_memory` | sim | – | não | `:602-617` |
| `experiment.run` | **não** | – | **sim** | manager privado |
| `store.list/load/save/delete` | não | writer lock de arquivo | sim (por chave) | `durable.rs` |

---

## 3. Experimento: protocolo

- **Workload único e determinístico:** backend Mega Drive real
  `MegadriveFactory::with_scheduler_config(interrupt_rom(3), SchedulerConfig::ntsc())`
  (o mesmo do `bench_step` do baseline), via `AppService::dispatch` com documentos canônicos.
- **Harness:** crate de scratch fora do workspace, em `.research/.../agents/concurrency/review_conc/`
  (workspace próprio, sem tocar `crates/`), arquivado em `04-evidence/review_conc-source/`.
- **Sem `tokio`/async:** o serviço e o transporte são `std::sync::Mutex` + `std::thread`
  bloqueantes; as medições usam `Instant` e `/proc/self/{stat,statm}`.
- **Cenários:** E1 escala 1/2/4/8 sessões; E2 leitura concorrente; E3 conflito/cancelamento;
  E4 controle (`experiment.run`); E5 esgotamento de registry; E6 retenção de estado; E7
  starvation; E8 custo fixo; E9 ladder de custo por passo; E10 HTTP/MCP reais
  (`chronofork_transport::serve` + `HttpClient` + `McpClient`); E11 run longo.
- **Comando exato** em `04-evidence/README.md`; logs `run2/run3`, `transport2..6`, `ladder1`.

**Sanidade contra o baseline:** `bench_step` do baseline registra 5.009 instruções/s
(2,196 s / 10.000 passos). Meu single-thread com `run` de 500 passos deu **5.035–5.098
steps/s** (run2/run3) — concorda dentro de ~1%, apesar do caminho de serviço. **fato
observado** ⇒ o harness mede o mesmo custo de máquina do baseline.

**Aviso metodológico (importante):** o custo por passo **não é constante**. O ladder (E9)
mostra 96–100 µs/passo no início da máquina subindo para ~210–236 µs/passo conforme a
máquina envelhece (série de 40 batches de 100 passos: primeiros 9,6–15,8 ms, últimos
21,0–22,2 ms). Portanto: (a) comparações válidas são **dentro** do mesmo cenário; (b)
throughput absoluto do emulador é assunto de A2/A3, aqui só uso steps/s como unidade de
trabalho; (c) o texto não extrapola steps/s entre cenários com idades diferentes.

---

## 4. Resultados

### 4.1 E1 — escala entre sessões independentes (release, run2/run3, mediana)

4 runs de 500 passos por thread, cada thread com **sua própria** sessão, todas contra o
**mesmo** `AppService`; 8 vCPU.

| Sessões/threads | parede (s) | steps/s agregado | cpu/wall | run p50 (ms) | p95 (ms) | máx (ms) | eficiência* |
|---|---|---|---|---|---|---|---|
| 1 | 0,394 | **5.066** | 0,965 | 115,6 | 117,1 | 117,1 | **1,00** |
| 2 | 0,886 | 4.514 | 0,975 | 146,2 | 387 | 387 | **0,13** |
| 4 | 1,743 | 4.593 | 0,980 | 403,4 | 471 | 758 | **0,07** |
| 8 | 3,457 | 4.629 | 0,975 | 640,3 | 1.547 | 2.150 | **0,03** |

\* eficiência = throughput(T) / (T × throughput(1)).

**Leitura:** throughput agregado **constante**, `cpu/wall ≈ 1` (um único core ocupado), parede
cresce linearmente, latência p50 cresce ~T e a cauda explode (p95 1,5 s com 8). Não há
paralelismo entre sessões; o custo do lock é pequeno, o custo é a **serialização da execução**.
**experimento**, confiança **alta**.

### 4.2 E2/E7 — leitura concorrente, independência e starvation

| Cenário | operação | n | p50 (ms) | p95 (ms) | máx (ms) | >50 ms |
|---|---|---|---|---|---|---|
| E2: 1 runner (run de 1.000 passos em loop) | `inspect` em **sessão ociosa** | 5–7 | 230,3 | 463–737 | 737 | 100% |
| E2: idem | `inspect` na **sessão em execução** | 5–7 | 234 | 241–705 | 705 | ~100% |
| E7: 2 runners | `inspect` em **sessão canário** | ~7 | 466–919 | 919–1.389 | **1.389** | 100% |

- Um `inspect` que custa **0,217 ms** isolado (E8) passa a custar **230–1.389 ms** quando
  outra sessão executa. **A independência de sessão não existe na latência.**
- `blocked_over_50ms = todas` as amostras; a razão leitura/s cai para ~2 por 3 s.
- Não há limite de espera nem fairness: o máximo observado (1,39 s) é ~7,5× a duração de um
  run de 1.000 passos; `std::sync::Mutex` pode favorecer quem já rodou (barging).
  **experimento**, confiança **alta** para o bloqueio; **média** para “starvation ilimitada”
  (não rodei janela longa o bastante para provar divergência).

### 4.3 E3 — conflito de escrita e tentativa de cancelamento

```
same_session_second_run  Err(Conflict, retryable=true)       0,01 ms
close_during_run         Err(Conflict)                       0,00 ms
inspect_during_own_run   Ok                                  377–400 ms (bloqueou o run inteiro)
first_run_result         Ok
```

**experimento**, confiança **alta**: (a) exclusão de escrita por sessão funciona e é
fail-fast; (b) **não há forma de interromper** um run: `close` é recusado, `pause`/`resume`
existem na camada de sessões (`session.rs:269-286`) mas **não estão no catálogo do serviço**;
(c) leitura na mesma sessão é serializada silenciosamente, não rejeitada.

### 4.4 E4 — controle: `experiment.run` (manager privado, sem lock global)

| threads | parede (s) | exp/s | cpu/wall | speedup |
|---|---|---|---|---|
| 1 | 0,117 | 8,55 | 0,94 | 1,00× |
| 4 | 0,121 | 33,14 | 3,19 | 3,87× |
| 8 | 0,158 | **51,04** | **4,42** | **5,97×** |

**experimento**, confiança **alta**: o backend escala ~linearmente até 4 threads e ~6× em 8
(limite: 8 vCPU compartilhados + hyperthreading + hash por passo). Isto é a prova de que o
gargalo é **o lock global do serviço**, não a máquina emulada. É também o padrão de desenho
que já funciona neste repositório.

### 4.5 E11 — run longo e tamanho do hold do lock

| medida | run2 | run3 |
|---|---|---|
| 20.000 passos, parede | 4,73 s | 4,65 s |
| µs/passo | 236,4 | 232,7 |
| `inspect` em outra sessão disparado em t=1 s | bloqueou **3,728 s** | **3,654 s** |
| extrapolação para 100.000 passos | **23,6 s** | **23,3 s** |

**experimento + inferência** (a extrapolação assume custo médio por passo constante, o que o
ladder mostra ser aproximado; mesmo no extremo barato observado, 90 µs/passo, 100k passos =
**9 s**). Confiança **alta** para “9–24 s de lock global por request no limite padrão”.

### 4.6 E10 — caminho real HTTP/MCP (`chronofork_transport::serve`, 4 workers)

| medida | transport2 | transport3 | transport4 | transport6 |
|---|---|---|---|---|
| `run` 500 passos, 1 cliente | 52,2 ms | 51,1 ms | 49,0 ms | 50,6 ms |
| 7 runs concorrentes em 7 sessões, parede | 0,332 s | 0,329 s | 0,314 s | 0,333 s |
| throughput agregado | 10.537 | 10.634 | 11.130 | 10.498 steps/s |
| latências individuais | 55,5 / 102,1 / 148,1 / 194,3 / 240,6 / 286,0 / 331,6 ms | 51,4 → 328,5 | 45,3 → 313,6 | 50,7 → 332,5 |
| 4 `tools/call` MCP concorrentes | 0,190 s | 0,189 s | 0,184 s | 0,191 s |
| `inspect` ocioso via HTTP | 4,59 ms | 5,78 ms | 1,40 ms | 4,39 ms |
| desconexão do cliente durante um run: `inspect` seguinte | **138,0 ms** | **138,0 ms** | 132,2 ms | 136,9 ms |
| `run` de 100.000 passos via cliente tipado | – | – | – | **falha em 5.069 ms** (`os error 11`) |
| `inspect` seguinte ao timeout | – | – | – | **falha em 5.120 ms** |
| estado após 25 s | – | – | – | **aplicado**: `tick=8.970.094` |

**Leitura (experimento, confiança alta):**
1. As latências HTTP individuais formam uma **escada perfeita** (+46–47 ms por request):
   as requests **não** se sobrepõem, apesar de 4 worker threads e 7 clientes. O worker pool
   é irrelevante; o mutex é o recurso.
2. O `inspect` via HTTP custa 1,4–5,8 ms contra 0,217 ms in-process: **overhead fixo de
   ~1–5 ms por request** (TCP connect + `Connection: close` + framing), pago em **toda**
   chamada de agente.
3. **Desconectar não cancela**: depois que o cliente fecha o socket, um `inspect` em outra
   sessão ainda espera 132–141 ms — o run continuou até o fim. Não há detecção de peer
   fechado no caminho de execução.
4. **`run` de 100.000 passos é inutilizável pelo cliente tipado**: o read timeout de 5 s
   (`client.rs:20-22`) dispara, a request **aplica mesmo assim** (tick avançou), e uma
   chamada de observação também falha em 5 s. O agente recebe “erro de I/O” sem saber se o
   estado mudou — exatamente o pior modo de falha para automação. **P1.**

### 4.7 E5/E6 — registry vitalício e retenção de estado

```
E5 (max_sessions=4):
  created 4 → live=4;  create #5 → LimitExceeded "session limit reached"
  close 4 → live=0
  create após fechar tudo → LimitExceeded "session limit reached"     ← limite VITALÍCIO

E6 (48 sessões, max_sessions=1024):
  rss antes 93.112 KiB → após criar 111.716 KiB → após fechar 111.716 KiB
  Δ criação 18.604 KiB (387,6 KiB/sessão)   Δ depois de fechar 18.604 KiB (0 liberado)
```

**experimento**, confiança **alta**. Consequências: (a) qualquer agente pode **esgotar** o
daemon permanentemente com 1.024 creates (custo trivial: ~397 MiB e ~0,4 s); (b) com os ROMs
de M8 (clones por backend), o custo por sessão retida cresce. `limits.rs:18-23` reconhece o
fato, mas nada no produto o corrige nem o expõe em `capabilities` (só o número `max_sessions`).

### 4.8 E8 — custos fixos (release, mediana)

| operação | p50 | p95 | máx |
|---|---|---|---|
| `session.create` | 0,371–0,378 ms | 0,42–0,55 ms | 0,53–0,90 ms |
| `session.inspect` | 0,217 ms | 0,249 ms | 0,34–0,44 ms |
| `session.run` 100 passos | 9,58–9,60 ms | 9,97–10,07 ms | 15,6–15,8 ms |

`create` faz `factory.create()` + `backend.snapshot()` completo (serialização de ~197 KiB),
por isso ~0,4 ms. `inspect` de 0,217 ms inclui o `state_hash()` da máquina; com 1.024 sessões
vivas, um único `session.list` sob o lock executa **1024 `inspect`s** ⇒ ~0,2 s de lock global
(**inferência** a partir de `service.rs:394-429` + E8; não medido com 1.024 sessões).

---

## 5. Classificação CPU-bound vs I/O-bound

| Componente | Classe | Evidência |
|---|---|---|
| `session.run` (1 passo = 1 instrução + VDP/scheduler + `state_hash` de 197 KiB) | **CPU-bound** | E1 `cpu/wall 0,96–0,98`; E11 236 µs/passo |
| `session.snapshot`/`fork`/`restore` (serializa/restaura 197 KiB; fork cria backend + restore) | **CPU-bound** | `bench_step` baseline: serialize 88 µs, restore 99 µs; `fork_current` = snapshot+restore+snapshot |
| `session.inspect` | CPU-bound leve | 0,217 ms (hash do estado) |
| HTTP/MCP | **I/O-bound** no socket; CPU-bound no dispatch | E10: 1–5 ms de overhead de rede vs 0,2 ms de inspect |
| `store.*` | **I/O-bound** (arquivo + writer lock) | `durable.rs` |

**Consequência de desenho:** o gargalo do produto não é I/O nem a rede; é **uma seção crítica
CPU-bound longa**. Async/Tokio não resolveria nada aqui — só moveria o bloqueio de thread
para bloqueio de task. A solução correta é **paralelismo real entre sessões** (§6), não
concorrência cooperativa de I/O.

---

## 6. Alternativas avaliadas

Critério central: preservar o **isolamento determinístico por máquina** — uma `Session` só
pode ser tocada por **uma** thread de cada vez, e a ordem das operações sobre ela é a ordem
observada. Paralelismo **dentro** de uma máquina (CPU/VDP/áudio simultâneos) é outro problema,
com risco de determinismo muito maior, e **não** é necessário para multiplicar sessões.

| # | Alternativa | Como | Veredito | Custo/risco | Determinismo |
|---|---|---|---|---|---|
| A1 | **Locks independentes por sessão** (recomendado) | `manager: Mutex<BTreeMap<SessionId, Arc<Mutex<Session>>>>`; operações pegam o `Arc`, soltam o mapa e lockam só a sessão; leases passam a ser o próprio mutex (ou `try_lock` para manter o `conflict` fail-fast) | **Adotar (P1)**. Resolve F01/F02/F05 inteiros; ~150–300 linhas em `chronofork-service` + testes; `SessionManager` continua síncrono e existente para CLI | Médio; toca `service.rs`, `durable.rs` (usa manager privado, não muda), testes de concorrência; contrato externo inalterado | **Preservado**: 1 writer por máquina; sem interleaving |
| A2 | **Worker/thread por sessão (owner thread)** | cada sessão tem uma thread e um canal de comandos; operações enfileiram | Bom, mas mais caro que A1 (thread por sessão, 1024 threads, ciclos de vida) | Alto; reescreve dispatch; precisa de shutdown e backpressure | Preservado |
| A3 | **Actor model / fila de comandos por sessão** | cada sessão é um ator; respostas por oneshot | Equivalente a A2 em semântica; melhor se houver muitas operações lentas e cancelamento; overkill sem dependência async (seria hand-rolled) | Alto | Preservado |
| A4 | **Worker pool global com filas** (N workers, jobs por sessão, serialização por chave) | pool + hash de sessão → fila | Intermediário: paralleliza sessões, mas não dá cancelamento nem leitura não-bloqueante; complexidade de scheduler própria | Médio-alto | Preservado (se 1 job por sessão por vez) |
| A5 | **Sharding por processo** (N daemons, sessões distribuídas) | roteamento por sessão | Só faz sentido depois de A1; não resolve nada dentro de um daemon | Médio (operacional) | Preservado |
| A6 | **Processo por sessão** | fork/exec por sessão; IPC | **Escape hatch** para paralelismo real de CPU e isolamento de falhas; hoje sem protocolo IPC e sem TLS. Não recomendado como primário | Alto (IPC, vida, observabilidade) | Preservado, mas replay/artefatos precisam de coordenação |
| A7 | **Escalonamento cooperativo** dentro de `run` | yield a cada N passos, com fila justa | Complementa A1 para longos runs; sozinho não paraleliza e adiciona jitter não determinístico ao **host** (não ao estado) | Baixo-médio | Preservado no estado; cuidado para não introduzir dependência de host time |
| A8 | **Paralelizar componentes internos da máquina** | CPU/VDP/áudio em threads | **Rejeitar agora** | Muito alto | **Risco alto**: sincronização intra-máquina, ordem de hashing, replay; ganho incerto vs. A1 |
| A9 | **Nada (do nothing)** | manter o lock global, documentar | Aceitável **apenas** para v1 single-agent; com 2+ agentes degrada para fila e o limite de 5 s do cliente quebra | Zero | Preservado |

**Recomendação geral:** A1 como base, A7 como complemento (cancelamento cooperativo),
A6 como opção operacional documentada (não implementar agora), A2/A3/A4 rejeitadas por
complexidade enquanto A1 resolve o gargalo medido, A8 rejeitada explicitamente.

**Por que A1 é suficiente (inferência forte + evidência E4):** `experiment.run` já executa
máquinas completas em paralelo, com hashing e tudo, escalando 5,97× em 8 threads. A única
diferença para `session.run` é **onde** a `Session` mora e quem a locka. Não há nada no
backend que impeça o paralelismo entre sessões.

---

## 7. O que é necessário para “experimentos simultâneos de vários agentes”

### P0 — pré-requisito para declarar suporte multi-agente

1. **Nenhuma operação pode bloquear por segundos o serviço inteiro.** Fixar A1 (locks por
   sessão) ou, no mínimo, documentar e impor um teto de passos por request compatível com a
   latência tolerada (ex.: 2.000–5.000 passos) e **não** vender 100.000 como default utilizável.
2. **Cancelamento/interrupção de operação longa.** Sem isso, um agente que erra o `steps` (ou
   um cliente que cai) mantém CPU ocupada e, no desenho atual, todos os outros agentes
   esperando. Mínimo viável: flag atômica de cancelamento verificada no laço de `run` a cada
   N passos, retornando `Conflict`/`Cancelled` tipado com o prefixo aplicado; expor
   `session.cancel` (ou `deadline_steps`/`budget_ms` no request). Determinismo: o resultado é
   sempre um prefixo determinístico do run, então é seguro.
3. **Timeout coerente entre cliente e servidor.** O read timeout de 5 s do cliente
   (`client.rs:20-22`) é menor que o run máximo; hoje o cliente falha com *unknown outcome*
   e o servidor continua. Ou o cliente espera o `max_steps` (com timeout configurável), ou o
   servidor limita `steps` por request ao que cabe no timeout, ou existe `deadline` explícito.

### P1 — necessário para operação multiusuário previsível

4. **Ciclo de vida do registry**: remover a sessão no `close` (e liberar o backend), ou
   contar apenas sessões vivas para `max_sessions` e implementar retenção com TTL/limite.
   Caso contrário o daemon “morre” após 1.024 creates.
5. **Latência de leitura sob execução**: com A1, `inspect` de outra sessão deixa de esperar;
   se A1 não for feito, ao menos separar “leitura de metadados” de “leitura com hash”.
6. **`session.list` O(n) sob lock** (`service.rs:394-429`): com A1 vira custo local; sem A1,
   limitar o número de sessões inspecionadas por resposta ou paginar.

### P2 — pode esperar evidência de uso real

7. **Actor/worker-por-sessão**: só se a latência de enfileiramento de A1 se mostrar
   insuficiente com carga real; A1 já entrega paralelismo e cancelamento com muito menos
   máquina.
8. **Processo por sessão / sharding**: só quando houver necessidade de isolar falhas ou
   exceder 1 core por sessão em produção; hoje é uma decisão operacional documentável.
9. **Keep-alive no HTTP / worker pool adaptativo**: economiza 1–5 ms por chamada e threads;
   relevante quando houver muitos agentes de baixa latência, não para correção.
10. **Fairness explícita (FIFO)**: só se medições com carga real mostrarem starvation
    relevante; com A1 a contenção é por sessão e o problema muda de natureza.

### O que **não** fazer agora

- Paralelizar CPU/VDP/áudio **dentro** de uma máquina (A8): risco de determinismo, sem
  demanda medida; o ganho de throughput vem de sessões, não de um passo mais rápido.
- Introduzir `async`/Tokio: não resolve seção crítica CPU-bound; contraria ADR-0012 e o
  princípio de dependências mínimas.
- Trocar o lock por `RwLock`: leituras continuariam bloqueadas durante um `run` (write), e
  `inspect` precisa de `&mut` no manager; não resolve nada.

---

## 8. Findings

### F01 — Execução de sessões independentes é globalmente serializada
- **Descrição:** um `Mutex<SessionManager>` cobre `session.run` inteiro; N sessões não
  executam em paralelo. Throughput agregado constante de 1→8 sessões, `cpu/wall≈0,98`,
  eficiência 1,00/0,13/0,07/0,03.
- **Evidência:** `service.rs:114-122`, `:461-491`; `limits.rs:37`; E1 (`run2/run3`),
  E10 (escada 51→332 ms), E11.
- **Componentes:** `chronofork-service` (`AppService`), `chronofork-transport` (indireto).
- **Condições:** ≥2 agentes/sessões executando ao mesmo tempo; qualquer `steps` > alguns
  milhares.
- **Impacto atual:** fila global; com 8 sessões, p50 de um run de 500 passos sobe de 116 ms
  para 640 ms e p95 para 1,5 s. **Impacto futuro:** o produto se posiciona para agentes
  simultâneos (M6/M7, epics de agentes); com M8 (frames/áudio) o custo por passo cresce e o
  hold piora.
- **Probabilidade:** certa (determinística, não é corrida).
- **Severidade:** alta. **Confiança:** alta. **Prioridade:** **P1**.
- **Alternativas:** A1 (recomendada), A2/A3/A4, A6, A9.
- **Recomendação:** adotar A1 (locks por sessão) antes de declarar suporte multi-agente;
  enquanto isso, reduzir `max_steps_per_request` default e documentar a fila.
- **Custo/risco:** médio (serviço + testes); risco de regressão em ownership/lease; contrato
  de wire inalterado.
- **Testes/contratos:** novo teste de concorrência (2 sessões, 2 threads, throughput > 1,5×
  single-thread), mantendo os testes de paridade direto/HTTP/MCP; ADR-0012 §5 precisa de
  emenda se A1 for adotado.
- **Fase:** M8/M9 (serviço), não bloqueia 8.4 em andamento.

### F02 — Não há cancelamento/interrupção de operação longa; desconexão não cancela
- **Descrição:** catálogo sem `pause`/`cancel`; `close` durante run → `Conflict` imediato;
  leitura bloqueia; cliente desconectado não aborta o run; 100k passos ≈ 9–24 s.
- **Evidência:** `catalog.rs:300-460`; `service.rs:461-491`, `:585-600`; E3, E10
  (desconexão: +137 ms; 100k: cliente falha em 5,07 s, estado aplicado, `tick=8.970.094`).
- **Componentes:** `chronofork-service` (run), `chronofork-transport` (timeouts).
- **Condições:** `steps` grande, cliente impaciente/agente com deadline, erro de rede.
- **Impacto atual:** trabalho órfão, lock global preso, cliente sem saber o resultado.
- **Impacto futuro:** com múltiplos agentes, um pedido mal dimensionado degrada todos.
- **Probabilidade:** alta em uso agêntico real.
- **Severidade:** alta. **Confiança:** alta. **Prioridade:** **P1**.
- **Alternativas:** flag cooperativa no laço (recomendada), A7, deadline no request,
  processo por sessão (mata o processo), A9.
- **Recomendação:** `cancel` explícito + `deadline_steps` opcional; retorno `Cancelled` com
  `steps` aplicados; alinhar timeout do cliente ao máximo aceito.
- **Custo/risco:** baixo no serviço; precisa de teste de determinismo do prefixo (o estado
  após cancelar == estado após rodar exatamente `k` passos).
- **Testes/contratos:** novo teste “cancelar em k passos == run de k passos”; novo código de
  erro ou uso de `Conflict` documentado; paridade HTTP/MCP.
- **Fase:** M8 (serviço) — habilitador de agentes.

### F03 — `max_sessions` é um limite vitalício e o registry não remove entradas
- **Descrição:** `Registry` sem remoção; fechar não devolve cota; daemon esgota após 1.024
  creates.
- **Evidência:** `service.rs:73-104`; `limits.rs:18-23`; E5 (`LimitExceeded` após fechar
  todas).
- **Componentes:** `chronofork-service`; `chronofork-sessions` (sem `remove`).
- **Condições:** daemon de vida longa + agentes que criam/fecham sessões.
- **Impacto atual:** indisponibilidade permanente até reiniciar; DoS acidental trivial.
- **Impacto futuro:** pior com mais agentes e sessões efêmeras.
- **Probabilidade:** alta sob uso real. **Severidade:** alta (disponibilidade).
- **Confiança:** alta. **Prioridade:** **P1**.
- **Alternativas:** remover no close (recomendada), contar só vivas + TTL/tombstone,
  `SessionManager::remove` + drop, A9 (documentar).
- **Recomendação:** `close` remove do registry e do manager (drop do backend), mantendo
  linhagem/ids monotônicos; teste de “criar→fechar→criar” com `max_sessions=1`.
- **Custo/risco:** baixo-médio; cuidado com snapshots/lineage de sessões fechadas e com
  `session.list` (já as esconde).
- **Testes/contratos:** teste de cota reciclada; `capabilities` pode reportar
  `sessions_registered` vs `sessions_live`.
- **Fase:** M8.

### F04 — Sessões fechadas retêm o backend inteiro (memória)
- **Descrição:** `close` só marca estado; `Session` mantém `Box<dyn Backend>` + estado
  inicial + snapshots. 387,6 KiB/sessão medidos; 0 liberado após close.
- **Evidência:** `session.rs:57-65`, `:283-286`; `manager.rs:56-61`; E6.
- **Componentes:** `chronofork-sessions`, `chronofork-service`.
- **Condições:** sessões efêmeras, sessões grandes (M8 com ROM/VRAM).
- **Impacto atual:** até ~397 MiB com 1.024 sessões (1024 × 387,6 KiB); com ROMs maiores, mais.
- **Impacto futuro:** cresce com M8.
- **Probabilidade:** certa. **Severidade:** média hoje, alta com M8. **Confiança:** alta.
- **Prioridade:** **P2** (sobe a P1 junto com F03).
- **Alternativas:** drop no close (recomendada, junto com F03), manter só metadados,
  snapshot em disco antes de liberar, A9.
- **Recomendação:** fundir com F03.
- **Custo/risco:** baixo. **Testes:** RSS/contagem de sessões após N ciclos.
- **Fase:** M8.

### F05 — `session.list` e `session.compare` fazem trabalho O(n) sob o lock global
- **Descrição:** `list` percorre o registry e chama `manager.inspect` por sessão (hash
  completo) com o lock do manager preso; `compare` faz 2 inspects.
- **Evidência:** `service.rs:394-429`, `:561-583`; E8 (`inspect` 0,217 ms) ⇒ ~0,22 s para
  1.024 sessões (**inferência**).
- **Componentes:** `chronofork-service`.
- **Condições:** muitas sessões vivas + um agente listando.
- **Impacto atual:** um `list` grande congela todos por ~0,2 s; com A1 o custo deixa de ser
  global.
- **Impacto futuro:** moderado.
- **Probabilidade:** média. **Severidade:** média. **Confiança:** média (não medido com
  1.024). **Prioridade:** **P2**.
- **Alternativas:** A1; paginar; cachear `state_hash` (já disponível no último `StepOutcome`);
  `list` sem hash por sessão (novo campo opcional), A9.
- **Recomendação:** com A1, manter; sem A1, limitar o trabalho por resposta.
- **Custo/risco:** baixo. **Testes:** latência de `list` com N sessões.
- **Fase:** M8/M9.

### F06 — Timeout do cliente (5 s) é menor que o run máximo e o resultado é desconhecido
- **Descrição:** `run` de 100.000 passos falha no cliente em 5,07 s, o `inspect` seguinte
  falha em 5,12 s e o servidor aplica o run (`tick` avançado).
- **Evidência:** `client.rs:20-22`, `:137`; `limits.rs:37`; E10/transport6.
- **Componentes:** `chronofork-transport` (cliente), `chronofork-service`.
- **Condições:** qualquer `run` > ~5 s (≈20–25k passos no estado atual).
- **Impacto atual:** automação recebe erro sem saber se o estado avançou; retry cego duplica
  execução (mudando o estado).
- **Impacto futuro:** é o modo de falha mais perigoso para agentes.
- **Probabilidade:** alta com o default de 100k.
- **Severidade:** alta. **Confiança:** alta. **Prioridade:** **P1** (ou P0 se 100k for
  mantido como default).
- **Alternativas:** timeout configurável/proporcional a `steps` (recomendada), limitar
  `steps` por request ao timeout, `deadline` explícito, operação assíncrona com `job id`
  (fora do escopo de v1), A9.
- **Recomendação:** derivar o read timeout do cliente de `max_steps_per_request` (ou aceitar
  `timeout_ms` no cliente) e documentar; melhor ainda com F02 (cancelamento por deadline).
- **Custo/risco:** baixo; contrato do cliente ganha parâmetro.
- **Testes:** teste de integração com `run` que excede o timeout → erro tipado e
  `deadline`/cancelamento; teste de “não duplicar execução”.
- **Fase:** M6-FU/M8.

### F07 — Overhead fixo por request HTTP/MCP (sem keep-alive) e worker pool fixo
- **Descrição:** `Connection: close` em toda request; `inspect` 0,217 ms in-process vs
  1,4–5,8 ms via HTTP; 4 workers por default; um run longo ocupa um worker por segundos.
- **Evidência:** `http.rs:4`, `:42`, `:281-343`, `:806`; `serve.rs:27`; `client.rs:137`; E10.
- **Componentes:** `chronofork-transport`, `chronofork-cli`.
- **Condições:** agentes com muitas chamadas pequenas; runs longos concorrentes.
- **Impacto atual:** 20× de overhead em leituras pequenas; fila de workers com 8+ runs.
- **Impacto futuro:** moderado (M6-FU2 já prevê keep-alive? — verificar).
- **Probabilidade:** certa. **Severidade:** baixa-média. **Confiança:** alta.
- **Prioridade:** **P2**.
- **Alternativas:** keep-alive (recomendada, já prevista como follow-up), pool adaptativo,
  A9.
- **Recomendação:** keep-alive + tamanho de pool configurável; medir com carga real de
  agente antes de dimensionar.
- **Custo/risco:** médio (framing HTTP/1.1); contrato de transporte.
- **Testes:** paridade de bytes já existe; adicionar teste de latência/throughput do cliente.
- **Fase:** M6-FU.

### F08 — Ausência de fairness e de limite de espera
- **Descrição:** `std::sync::Mutex` não é justo; com 2 runners, um `inspect` em sessão
  independente esperou até 1,39 s (7,5× a duração de um run).
- **Evidência:** E2, E7; `service.rs:695-701`.
- **Componentes:** `chronofork-service`.
- **Condições:** concorrência contínua.
- **Impacto atual:** latência de cauda imprevisível; um agente pode ficar “sem resposta” por
  segundos mesmo com operações leves.
- **Impacto futuro:** com A1, a contenção passa a ser por sessão; o problema muda de escala.
- **Probabilidade:** média-alta. **Severidade:** média. **Confiança:** média (janela curta).
- **Prioridade:** **P2/P3**.
- **Alternativas:** FIFO explícito por sessão (com A1), tickets, A9.
- **Recomendação:** reavaliar após A1 com carga real; não construir scheduler próprio agora.
- **Custo/risco:** baixo. **Testes:** latência p99 de leitura sob carga.
- **Fase:** pós-M8.

---

## 9. O que não verifiquei / limitações

- **Carga real de múltiplos agentes** (LLM-driven, chamadas pequenas e irregulares): usei
  workload sintético determinístico; minhas conclusões de fila valem para execução, não para
  a distribuição real de tamanhos de request.
- **MCP**: testei 4 `tools/call` concorrentes (serialização confirmada), não testei
  `initialize`/`tools/list` sob carga nem clientes com sessão.
- **GUI local/remoto** e o *controller lease* humano/agente (M7) ficaram fora deste escopo;
  o lease da GUI é outro mecanismo (`chronofork-gui`), não o writer lease do serviço.
- **`store.*`** foi lido, não medido; a concorrência ali é dominada por writer lock de
  arquivo e não pelo manager. A interação `store.save` concorrente + `store.load` (leitura sem
  writer lock) merece teste de A5/A6.
- **`session.snapshot`/`fork`/`restore`** não foram medidos concorrentemente; a serialização
  vem do mesmo lock, e o custo por operação é maior (serializa/restaura 197 KiB + cria
  backend no fork), então o hold é pior que o de um `run` curto — **inferência**.
- **Números absolutos** oscilam no host compartilhado (o `bench_step` do baseline registra
  5.009 steps/s enquanto o mesmo build mediu 7.340–7.570 historicamente). Usei medianas de 2
  execuções e razões; não usei valores de uma única amostra.
- **Custo por passo varia com a idade da máquina** (E9: 96→236 µs); a extrapolação de 100k
  passos (23 s) é uma estimativa superior; no extremo barato seriam ~9 s — em ambos os casos
  > 5 s de timeout.
- **`ulimit`/cgroups**: não testei contenção de memória com 1.024 sessões; E6 sugere
  ~397 MiB.
- Não rodei `cargo test`/`cargo clippy` neste trabalho de revisão (nenhuma alteração de
  código de produção; o baseline de testes do capitão é a referência).

---

## 10. Perguntas para o capitão / critic

1. **Decisão de produto (D1):** o dono quer que “sessões independentes” signifiquem também
   **desempenho independente**? Se sim, A1 (locks por sessão) é pré-requisito de qualquer
   claim multi-agente. Se não, o `README`/ADR devem dizer explicitamente “execução
   serializada por processo; use `serve` por experimento para paralelismo”.
2. **Decisão de contrato (D2):** manter `max_steps_per_request = 100_000` sabendo que um
   request pode durar 9–24 s e exceder o timeout do próprio cliente? Recomendo reduzir o
   default (ex.: 5.000) **e** adicionar deadline/cancelamento (F02/F06).
3. **Decisão de retenção (D3):** `close` deve liberar estado e cota? Recomendo sim (F03/F04);
   isso muda a semântica de “sessão fechada” (hoje inspecionável? `inspect` de fechada
   retorna `Closed` por `ensure_open` — `session.rs:122-132`), então precisa de ADR curto.
4. **Cross-area (A3/A5/A6):** o custo de `state_hash` por passo (197 KiB) é o que transforma
   “1 instrução” em 100–236 µs e, portanto, o que faz o lock global doer. Se A3/A5 reduzirem
   o custo por passo, F01 fica menos grave — mas não desaparece (a serialização continua).
   Vale o capitão cruzar as recomendações: **não** otimizar hash para “resolver”
   concorrência; são decisões independentes.
5. **Cross-area (A6):** `store.load`/`store.save` usam managers privados e **não registram**
   a sessão no serviço; um agente que carrega um snapshot não ganha uma sessão inspecionável
   por `session.inspect`. Isso é intencional? Se não, é um bug de interface/isolamento, não
   de concorrência.

---

## 11. Reprodução

Ver `04-evidence/README.md` para comandos exatos, ambiente e hashes. Resumo:

```sh
# build do harness de experimento (release; fora do workspace)
CARGO_HOME=/tmp/t7-cargo CARGO_TARGET_DIR=/tmp/chronofork-review-target \
  cargo build --release --manifest-path \
  .research/review-2026-11-20/agents/concurrency/review_conc/Cargo.toml

BIN=/tmp/chronofork-review-target/release/review-concurrency
$BIN            # E8, E1, E4, E2, E7, E3, E11, E5, E6
$BIN ladder     # E9 (custo por passo)
$BIN transport  # E10 (HTTP/MCP reais, desconexão, timeout de 100k)
```

Nenhum arquivo em `crates/` ou `docs/` foi alterado **por esta revisão** (verificado com
`git status`; os dois `examples/review_*.rs` não rastreados que aparecem no worktree são de
outro revisor e não foram tocados).
