# A1 — Revisão de Rust e arquitetura de sistemas

**Área:** contratos, crates, traits, ownership e abstrações (Rust/sistemas).
**Baseline:** `b8d63a1` (`origin/main`), worktree `review-2026-11-20-architecture`, branch
`agent/review-2026-11-20-architecture`.
**Escopo:** `chronofork-core`, `chronofork-backend-api`, `chronofork-sessions`,
`chronofork-service`, `chronofork-megadrive`, grafo de crates/Cargo.toml, superfície
pública, tipos de erro, dynamic dispatch, clones/alocações, visibilidade, duplicação.
**Fora do escopo (tratado por outras áreas):** acurácia/engenharia de emulação, análise de
desempenho do interpretador 68000/VDP, segurança de transporte/auth, roadmap de produto.
Quando toco nesses temas, faço só a ponte necessária e marco como cross-area.

Método: leitura integral dos arquivos de contrato e das fronteiras; censo automatizado de
superfície pública/erros/`Capabilities`; um experimento próprio de decomposição de custo
(`Backend::step` vs hash vs serialização vs `inspect` vs criação vs fork), executado 4× em
release com a mesma cache compartilhada do baseline. Nenhum arquivo de produção, doc,
planejamento ou de outro revisor foi editado; o exemplo temporário foi arquivado e removido
(ver `.research/review-2026-11-20/agents/rust-arch/README.md`).

---

## 1. Resumo executivo

1. **O contrato `Backend` é pequeno, honesto e adequado ao produto de longo prazo nos
   pontos que importam** — `step`/`snapshot`/`restore` opacos, snapshot versionado com
   envelope validado em um único lugar (`backend-api/src/snapshot.rs:105`), dispatch
   dinâmico só na fronteira de composição. Não recomendo reescrevê-lo. **Recomendo três
   mudanças pontuais de contrato antes de M9** e uma delas já é necessária em 8.4/8.5.
2. **O acoplamento `step` → `state_hash` completo é o achado estrutural mais caro.**
   Medido: um `StateHash` sobre o corpo canônico de 197 048 B custa **83–93 µs**; o passo
   mediano custa **213–254 µs**. Ou seja, **36–40 % do passo mediano** e **~81 % do passo
   mais rápido observado (102 µs)** é SHA-256 do estado inteiro. Não existe caminho de
   contrato para "avance N passos e me dê o hash só na fronteira".
3. **`inspect()` recalcula o hash e copia 64 KiB de RAM** (`megadrive/src/backend.rs:969`),
   e é o único jeito de ler `tick`/`pc`. Os dois loops de host que mais rodam passos —
   tracing (`trace/src/capture.rs:256`) e debugger (`debug/src/debugger.rs:440`) — chamam
   `inspect()` **por passo**. Efeito: debug/tracing custam ~2× o passo normal (medido:
   `inspect` = 211–240 µs vs `step` = 213–254 µs), e `session.list`/`session.compare`
   pagam um hash completo por sessão (`sessions/src/manager.rs:393-408`, `:372-391`).
4. **`MachineInspection` é plano demais para o futuro imediato, não só para M9.**
   Um `BTreeMap<String,u64>` + uma janela de memória, sem identidade de CPU e sem
   seletor de espaço de endereçamento (`backend-api/src/inspection.rs:12-26`); o backend
   Mega Drive já empilha CPU + VDP + IRQ + mapper + cartucho no mesmo mapa e codifica o
   serial do cartucho em `u64` (`backend.rs:883-976`). O contrato de "registrador `pc`" é
   uma **string mágica compartilhada por três crates** (`trace/src/capture.rs:28`,
   `debug/src/debugger.rs:24`, `gui/src/view.rs:1475`). Em 8.4 entra o Z80: o segundo
   `pc` não cabe nesse contrato sem convenção inventada.
5. **O ownership da sessão está certo (`Box<dyn Backend>` por sessão), mas o ownership do
   *agendamento* está errado para o produto.** `AppService` guarda
   `Mutex<SessionManager>` (`service/src/service.rs:114-122`) e `session.run` segura esse
   lock global por até 100 000 passos (`limits.rs:37`) — ≈21 s na mediana medida. Sessões
   independentes, que são o diferencial do produto, não conseguem rodar em paralelo.
6. **`fork_current` retém ~790 KiB por fork para um estado de 197 KiB (4×) e ~0,8–1,4 ms**;
   o `SessionManager` não tem API de remoção, e `max_sessions = 1024` ⇒ regime de ~800 MiB
   só de forks fechados (`service/src/limits.rs`; medido, §7). Forkar é a primitiva central
   do produto; 4 cópias por fork é acidente de implementação, não requisito.
7. **12 crates / 115,7 k linhas totais estão justificados** — mas a premissa do baseline
   ("~123 k LOC") está errada: são **66,4 k linhas de `src` + 48,9 k de testes de
   integração**. O grafo reflete exatamente a direção de dependência que o `AGENTS.md`
   exige e cada aresta é um teste de compilação. O problema de concentração não é o número
   de crates: é `chronofork-sessions` (11,7 k) acumular ciclo de vida + storage + formato
   `.cfmovie` + player + replay + edição, e `chronofork-megadrive` expor **493 itens
   públicos** para servir seus próprios testes de integração.
8. **Complexidade acidental identificada é pequena e barata de remover** (duplicação do
   `StateView`, `Capabilities` com flags que nunca são `true`, clone de payload em
   `restore`, `SessionError` misturando storage). Nenhuma delas justifica refatoração ampla;
   todas cabem em PRs pequenos e independentes.

Severidade ≠ prioridade: o item de hash tem **severidade P0** (bloqueia a meta "jogável" de
M8 junto com o custo do interpretador) mas **prioridade de implementação P1** (entra com
8.4/8.5, antes do trabalho de performance que outra área vai propor). O item de
`MachineInspection` tem **severidade P1** e **prioridade P2** (antes de M9; em 8.4 já dói).

---

## 2. Correções de premissa do baseline (verificadas)

| Premissa | Verificação | Conclusão |
|---|---|---|
| "12 crates e ~123 k LOC" | `find crates -name '*.rs' \| xargs cat \| wc -l` = **115 690**; `src` = **66 423**; `tests/` = **48 890**; `examples` = 377. `sessions/src` = 11 731, `megadrive/src` = 18 228, `cli/src` = 8 991, `gui/src` = 8 629. | Produção é 58 % do total; o número do baseline soma testes. Não é sinal de hipertrofia. |
| "`MegadriveFactory::create_machine` clona o `Cartridge` inteiro por backend" (`backend.rs:303`) | Medido com ROM de 1 MiB: `create_machine` = **2,0–2,5 µs** (mediana). | Fato, mas **irrelevante** para custo. Não é problema; registrar para não virar falso achado. |
| "VDP `snapshot_bytes` aloca e escreve o VDP inteiro a cada chamada" | Medido: `Vdp::snapshot_bytes()` = **1,15–1,26 µs**. | Fato, custo desprezível face ao hash. A alocação por passo é xarope, mas 0,6 % do passo. |
| "Testes gastam a maior parte no hash por passo" | Medido: hash 83–93 µs vs passo 213–254 µs, e `inspect` ≈ passo. | Confirmado no mecanismo; ver §4/§5. |
| "Só dependência `sha2`" | `Cargo.toml` do workspace + 12 crates: dependências externas = `sha2` (só em `core` e `sessions`); `proptest` em `dev-dependencies`. | Confirmado. `chronofork-sessions` depende de `sha2` diretamente (`sessions/Cargo.toml`), o que é discutível: hash de estado deveria vir de `core`. |

---

## 3. O contrato `Backend` frente ao multi-backend (pergunta 1)

### O que está bom e deve ser preservado

- **Superfície mínima e opaca**: `descriptor`, `step`, `snapshot`, `restore`, `inspect`,
  `read_memory`, `write_memory` + dois seams opcionais com default `None`
  (`debug_access`, `media_mut`) — `traits.rs:28-83`. Backend concreto não conhece sessão,
  storage, transporte nem GUI; nenhuma crate genérica depende de `megadrive` (verificado
  nos 12 `Cargo.toml`).
- **Envelope de snapshot versionado com validação única** (`snapshot.rs:13-141`):
  `backend_id`/`backend_version`/`format_version`/`config_hash` checados em ordem, primeiro
  erro nomeado, payload opaco. É exatamente o que M9 precisa para rejeitar bytes
  incompatíveis em vez de reinterpretá-los (regra 8 do `AGENTS.md`).
- **`Backend: Send` e não `Sync`** (`traits.rs:28`) combinado com `&mut self` deixa a
  exclusividade explícita e permite mover uma máquina entre threads sem inventar
  sincronização interna. Correto.
- **`BackendFactory: Send + Sync`** com `descriptor` sem construir instância
  (`traits.rs:89-99`) permite ao host decidir capabilities antes de alocar máquina.
- **Tipos de erro tipados e sem `String` como transporte de controle** (`error.rs:14-45`),
  com `From<CoreError>` só para `TickOverflow`. Bom.

### Lacunas concretas (e o que muda antes de M9)

**A1-03 — `MachineInspection` não representa múltiplas CPUs nem múltiplos espaços.**

Fato observado: `MachineInspection { tick, state_hash, halted, registers: BTreeMap<String,u64>,
memory_base: u64, memory: Vec<u8> }` (`inspection.rs:12-26`). O backend Mega Drive preenche
esse mapa com registradores de CPU, VDP, scheduler, mapper e cartucho
(`megadrive/src/backend.rs:883-976`) e empacota o serial do cartucho em 6 bytes ASCII
big-endian dentro de um `u64` (`:925-945`, com comentário admitindo o truque). `debug`
e `trace` exigem que exista uma chave `"pc"` (`trace/src/capture.rs:28`,
`debug/src/debugger.rs:24`); o CLI acessa `registers["pc"]` diretamente
(`cli/src/main.rs:1308,1330,1344`). `read_memory`/`write_memory` recebem só
`(address, length)` (`traits.rs:46-52`) — existe **um** espaço de endereçamento.

Impacto futuro: em **8.4** entram Z80 e YM2612; o Z80 tem `pc`, `sp`, `ix`, `iy`, `af'`,
`bc'` etc. que colidem com os nomes do 68000 e não têm dono declarado. Em **M9** um backend
com CPU + PPU/APU separadas (NES, SNES, PC com paginação) precisa de: (a) mais de um espaço
de memória (VRAM/OAM/paleta não são alcançáveis por `read_memory` hoje); (b) leitura
física **e** virtual; (c) mais de 64 bits por registrador? não — mas precisa de estrutura.

Mudança concreta recomendada (barata, aditiva, sem quebrar backends existentes):

```rust
// backend-api/src/inspection.rs
pub struct MachineInspection {
    pub tick: Tick,
    pub halted: bool,
    pub state_hash: Option<StateHash>,      // ver A1-01: hash deixa de ser obrigatório
    pub processors: Vec<ProcessorView>,     // id estável ("m68k", "z80", …) + registers tipados
    pub memories: Vec<MemoryWindow>,        // (space: SpaceId, base, bytes) — 0..N janelas
    // compat: helpers que devolvem "pc" do processador primário
}
pub struct ProcessorView { pub id: ProcessorId, pub pc: u64, pub halted: bool,
                           pub registers: BTreeMap<RegisterId, u64> }
pub struct MemoryWindow { pub space: SpaceId, pub base: u64, pub bytes: Vec<u8> }
```

e `read_memory(space: SpaceId, address: u64, length: usize)` com `SpaceId` declarado no
`BackendDescriptor` (lista fechada por backend, como `Capabilities`). Alternativa mínima
(se o dono quiser evitar mudança de assinatura agora): manter `read_memory` como espaço 0 e
declarar a convenção de nomes `"<device>.<register>"` **no contrato** (não em três crates),
com um `const PRIMARY_PC: RegisterId` exportado por `backend-api` e usado por debug/trace.
Custo da versão mínima: ~1 dia, sem migração de snapshot; custo da versão estruturada:
3–5 dias, toca `backend-api`, `megadrive`, `test-backend`, `sessions` (`SessionInspection`),
`service` (JSON de `session.inspect`), `gui/view` (decoder estrito), `cli/debug`,
`trace`, `debug`. Risco: médio (contrato observável por agentes; precisa de versionamento
do JSON do service).

**A1-04 — Não existe "timebase"/quantum declarado.**

Fato: `Tick` é "backend-defined quantum" (`core/src/tick.rs:3-7`); `BackendDescriptor`
(`descriptor.rs:96-103`) não declara nada sobre tempo (nem ciclos por passo, nem
relação com frame/áudio); `StepOutcome` traz só o tick absoluto. Consequência observada: o
host codifica o conhecimento da máquina — a GUI usa `DEFAULT_FPS = 60` e
`DEFAULT_TICKS_PER_FRAME = 1` (`gui/src/lib.rs:100-107`) e o CLI expõe `--fps` com default 60
(`cli/src/gui.rs:262`). Para o Mega Drive PAL (50 Hz, já suportado por
`SchedulerConfig`) ou para um backend de granularidade "1 frame = 1 passo", o pacing fica
errado por configuração do usuário, não por contrato.

Mudança concreta: `fn timebase(&self) -> Timebase` no `Backend`/`BackendFactory`, com
`ticks_per_second: Option<u64>`, `ticks_per_frame: Option<u64>`, taxa de áudio nominal e
`quantum: Quantum { Instruction, Cycle, Frame, BackendDefined }`. Uso imediato: GUI/CLI
param de hardcodar 60 Hz; `service` pode reportar o timebase em `capabilities`; M9 ganha
pacing correto por máquina sem flags. Custo: ~2 dias (backend-api + megadrive + test-backend
+ service JSON + gui view + gui pacing + cli), risco baixo (aditivo), testes: um teste por
backend garantindo `timebase` consistente com `tick` avançado por passo.

**A1-12 — Granularidade de execução não é negociável.**

`Backend::step` avança "one backend-defined quantum" (`traits.rs:32-33`) e não há
`run_until`/`run(steps)`. Toda a semântica de execução do host é "um quantum por chamada do
host": replay de `.cfmovie` grava um registro **por passo** (`sessions/src/movie.rs:439`
e o formato v2 `V2_STEP_RECORD_LEN`), TAS edita por passo, `Player::run_to` compara hash por
passo (`sessions/src/player.rs:610`). Isso amarra o produto a um quantum = 1 instrução para
o Mega Drive: um movie de 5 000 000 passos (`MAX_STEPS`) cobre ~4 minutos de jogo neste
benchmark (87 ciclos mestre/passo). Não é um erro hoje; é um limite que M9 (backends com
quantum diferente) e M8 (captura/áudio) vão encontrar. Recomendação: declarar o quantum no
descriptor (acima) e adicionar `fn run(&mut self, input: InputFrame, steps: u64, hash: HashPolicy)
-> Result<RunOutcome, BackendError>` com default implementado sobre `step` — backends podem
sobrescrever para lotear. Custo baixo (default pronto), ganho: o host para de ditar o lote.

---

## 4. O passo de uma instrução e o hash por passo (pergunta 2) — **o achado central**

### Fato observado (código)

- `StepOutcome` carrega `state_hash` **obrigatório** (`traits.rs:12-21`), e
  `Backend::step` o devolve sempre (`traits.rs:33`).
- O caminho por passo do Mega Drive chama `self.state_hash()` em toda instrução
  (`megadrive/src/backend.rs:680-685`). `state_hash` (`:488-513`) monta o `StateView` e
  chama `state_hash_for` → `hash_state_body(state_body(view))` (`:1201-1264`): **duas**
  serializações/alocações de ~197 KB (corpo + prefixo do hash) e um SHA-256 sobre 197 058 B,
  além de uma terceira alocação de ~68 KB em `Vdp::snapshot_bytes` (`vdp/mod.rs:1081`).
- Não há no contrato nenhuma forma de pedir execução sem hash, nem de hash só em fronteira.
- Consumidores reais do hash por passo: gravação de `.cfmovie` (`movie.rs:439`), verificação
  de replay (`player.rs:610`, `movie.rs:1385`), edição de branch (`edit.rs:551`). O
  `session.run` do service **não** precisa dele (usa `inspect` no fim,
  `service/src/service.rs:461-491`), nem o tracing (usa `inspect` só para metadados,
  `trace/src/capture.rs:190`).

### Experimento (executado, release, 4 rodadas)

Comando e ambiente idênticos aos do baseline (`CARGO_HOME=/tmp/t7-cargo`,
`CARGO_TARGET_DIR=/tmp/chronofork-review-target`, `--release --locked`); o programa é o do
benchmark oficial (`interrupt_rom(3)`, NTSC, 3420 ciclos mestre/linha), estado canônico
= 197 048 B de corpo / 197 058 B de payload.

| Métrica (mediana por operação) | R1 | R2 | R3 | R4 |
|---|---|---|---|---|
| `step` (mediana / mínimo) µs | 212,7 / 101,6 | 213,5 / 103,2 | 215,4 / 102,4 | 254,5 / 137,1 |
| `hash_bytes` sobre 197 058 B | 83,2 | 84,6 | 84,2 | 93,0 |
| `snapshot()` (serializa+hash+copia) | 242,4 | 251,4 | 243,7 | 259,1 |
| `inspect()` | 211,3 | 213,6 | 216,5 | 240,2 |
| `Vdp::snapshot_bytes()` | 1,18 | 1,26 | 1,17 | 1,21 |
| `create_machine()` | 2,06 | 2,09 | 2,08 | 1,99 |
| clone de 197 048 B | 2,72 | 2,73 | 2,73 | 6,67 |
| `Writer` + cópia de 197 048 B | 2,70 | 2,73 | 2,73 | 6,62 |
| **hash / passo** | **39,1 %** | **39,6 %** | **39,1 %** | **36,5 %** |
| banda efetiva do SHA-256 | 2 369 MB/s | 2 328 MB/s | 2 342 MB/s | 2 119 MB/s |

Leitura:

- O **hash é o componente estável e dominante**: 83–93 µs, com `min ≈ mediana` (variância
  <2 %), contra um passo que varia 2× entre rodadas (101–137 µs no mínimo, 212–254 µs na
  mediana). A banda de 2,1–2,4 GB/s indica SHA-NI ativo nesta máquina.
- **As cópias e a serialização não são o gargalo**: 2,7 µs por clone/cópia de 197 KB. A
  duplicação de `StateView` (`:454-479` vs `:488-513`) justificada no comentário como "~18 %
  mais rápido" não é sustentada por este perfil — o custo está no SHA-256, não na
  construção da view.
- Portanto: **no melhor caso medido (passo 102 µs), 81 % do passo é o hash**; no caso
  mediano, 37–40 %. Removendo o hash obrigatório, o teto medido vai de ~4,7 k para
  ~9,8 k passos/s na frequência "rápida" e até ~1/19 µs ≈ 50 k passos/s se o resto do passo
  também for otimizado — mas **só o hash não resolve M8** (ver cross-area abaixo).
- `inspect()` ≈ 1,0× `step` porque recomputa o mesmo hash e ainda copia 64 KiB
  (`backend.rs:969-976`). Isso **dobra** o custo de qualquer loop que inspecione por passo.

### Cross-area (performance/emulação)

Mesmo com hash zero, o passo mínimo medido (102 µs − 83 µs de hash − ~5 µs de
serialização/cópias) deixa ~14–19 µs de CPU+VDP por instrução — duas ordens de grandeza
acima de um interpretador 68000 razoável (~50–100 ns/instrução) e incompatível com a meta
"jogável" de M8. **O hash não é a única causa e não é a maior**; o trabalho de
interpretador/VDP é da área 2. O que é meu: o contrato atual (quantum = 1 instrução, hash
obrigatório, `inspect` por passo no debug/trace, ausência de lote) **impede** que qualquer
backend amortize, então a mudança de contrato precisa entrar antes de o trabalho de
performance começar, senão ele será refeito.

### Alternativas comparadas (A1-01)

| # | Alternativa | Ganho medido/esperado | Custo | Risco | Veredito |
|---|---|---|---|---|---|
| A | **Nada** (manter hash obrigatório) | 0 | 0 | M8/M9 pagam 37–81 % do passo para sempre; debug/trace 2× | rejeitada |
| B | `StepOutcome.state_hash: Option<StateHash>` + `Backend::state_hash()`; `step` nunca hasheia, gravação/verificação de movie chamam `state_hash()` explicitamente | remove 83–93 µs do passo normal (37–40 % mediano; até 81 % do melhor caso) | ~40–60 call sites (`sessions` movie/player/edit, `service`, `cli`, testes golden) | médio: muda semântica pública; testes de hash por passo precisam migrar | **recomendada** |
| C | Manter `StepOutcome` e adicionar `step_bulk(input, steps, HashPolicy)` com default sobre `step` | idem B para `run`, sem tocar `step` | menor (~10 call sites) | baixo, mas deixa a armadilha do hash obrigatório no caminho de debug/trace | aceitável como passo 1 |
| D | Trocar SHA-256 por BLAKE3/xxh3 | ~2–6× no hash, mas continua por passo e com 197 KB de serialização | nova dependência + ADR + quebra do `core/hash.rs` documentado | alto para ganho menor que B | rejeitada isoladamente |
| E | Hash incremental (rolling) do estado | não trivial: RAM/VDP mutam em janelas arbitrárias; exigiria dirty tracking correto | alto (cada dispositivo precisa reportar dirty ranges) | alto (risco de determinismo silencioso) | rejeitada por ora; reavaliar se M9 exigir hash a cada passo |

Recomendação: **B**, ou **C → B** se o dono quiser reduzir o diff. Além disso,
`inspect()` deve separar "status barato" (tick, halted, pc) de "hash canônico": com B,
`state_hash` fica explícito e `inspect` pode devolver `state_hash: None` por padrão, o que
sozinho corta ~211 µs por passo de tracing/debug. Efeitos em testes: os golden de hash por
passo (`megadrive/tests/*`, `sessions/tests/*`) continuam válidos se chamarem
`state_hash()`; os que verificam `outcome.state_hash` mudam de forma mecânica. Nenhum
snapshot muda (o envelope já carrega hash próprio em `snapshot()`).

---

## 5. `inspect()` como caminho quente (A1-02)

Fato: `session_summary` (`manager.rs:393-408`) e `compare` (`:372-391`) chamam `inspect()`
por sessão; `session.list`/`session.compare` são operações do catálogo do service
(`catalog.rs:316,374`) e a GUI as usa em `open`/`list`. Com `max_sessions = 1024`, um
`session.list` custa até ~1024 × (hash 85 µs + 64 KiB copiados + alocações) ≈ **90–220 ms** e
aloca/libera 64 MiB no total. `session.compare` custa ~2× `inspect` ≈ 430–480 µs para uma
comparação que só precisa de `tick` + `state_hash`.

Mudança: `Backend::status(&self) -> Status { tick, halted, pc }` barato (ou reter o último
`StepOutcome`/hash no backend), `inspect` deixa de hashear por default, e `summaries`/
`compare` usam o status. Custo ~1–2 dias; risco baixo; testes: um teste por backend
garantindo `status().tick == inspect().tick` e que `status` não aloca 64 KiB.

---

## 6. Ownership, alocações e clones (fatos medidos)

**A1-05 — fork retém ~4× o estado (medido).** `fork_current` = `snapshot` + `fork_from_snapshot`
(`manager.rs:217-221`, `:184-227`); o filho guarda `initial_state` (novo `backend.snapshot()`,
197 KB) **e** o `Snapshot` clonado do pai (`adopt_snapshot`, +197 KB), e o pai retém o
registro criado por `snapshot()` (+197 KB). Medido num processo com 200 forks de estado
idêntico: RSS 159 072 KiB → 313 772 KiB ⇒ **773,5 KiB retidos por fork** (≈4 × 197 KB, como
previsto pelo código), e `fork_current` = **0,80–1,36 ms** (min/mediana). Não existe
`SessionManager::remove`: `close` só muda o estado (`manager.rs:342-346`) — com
`max_sessions = 1024` isso é um regime de ~800 MiB mantidos mesmo com sessões fechadas
(documentado em `registry.registered_count`, `service.rs:90-96`, mas sem saída).

Alternativas: (a) `initial_state: Arc<BackendSnapshot>` compartilhado com o registro do
snapshot (elimina 1 das 4 cópias); (b) `Snapshot` com `Arc<Vec<u8>>` para payloads
(elimina clones em `restore` e no fork; cada snapshot passa a custar 1 payload);
(c) `SessionManager::remove_closed()`/`reap()` com lease e lineage preservados;
(d) `fork_current` reutilizar o estado do pai em vez de re-serializar. Custo: (a)+(b) ~1–2
dias, (c) ~2 dias com cuidado de storage/linhagem, (d) ~0,5 dia. Ganho: fork de ~790 KiB
para ~200–400 KiB e de ~1 ms para ~0,5 ms; e o produto deixa de degradar com histórico de
forks. Risco: (c) é o único com risco de semântica (ids, storage, `session.list`); (a)/(b)
são mecânicos.

**A1-06 — Lock global do service (severidade alta para o produto).** `AppService` tem
`Mutex<SessionManager>` + `Mutex<Registry>` + `Mutex<LeaseTable>` (`service.rs:114-122`);
`run` pega o lock do manager para o loop inteiro (`:468-491`). Um `session.run` de 100 000
passos bloqueia **todas** as sessões por ~10–21 s (medido 102–215 µs/passo). O modelo de
ownership da sessão (`Box<dyn Backend>`, `Backend: Send`, sem compartilhamento — `traits.rs:24-27`)
**permitiria** paralelismo; o service o serializa por design. Alternativas: (a) `Arc<Mutex<Session>>`
por sessão com lock order documentada; (b) sharded lock (`Vec<Mutex<SessionManager>>` por
hash de id); (c) manter o lock e limitar `max_steps_per_request` a algo compatível com
latência (ex. 5 000) — não resolve paralelismo, só o pior caso. Recomendação: (a) com o
lease já existente como barreira de escrita; custo ~3–5 dias (revisar `Lease`, `Registry`,
`fork` que toca duas sessões, e os testes de concorrência); risco médio-alto; é a decisão de
arquitetura mais importante depois do hash. **Área de concorrência deve opinar; concordo
com o diagnóstico, não com "trocar por async".**

**A1-10 — `restore` clona o payload inteiro para driblar o borrow checker**
(`session.rs:176-188`: `let snapshot = …clone()`, depois `self.backend.restore(...)`).
Com `Arc<Vec<u8>>` no payload o clone vira refcount. Ganho pequeno (2,7 µs + 197 KB), mas é
a mesma correção de (b) acima; prioridade P3.

---

## 7. Crates, LOC, visibilidade, erros (pergunta 3)

**12 crates: justificados.** O grafo de `Cargo.toml` (verificado um a um) é exatamente o do
`AGENTS.md`: `core` ← `backend-api` ← {`debug`,`trace`}, `sessions` (core, backend-api,
trace), `script` (core, backend-api, sessions, trace), `service` (core, backend-api,
sessions), `transport` (service), `gui` (core, service), `cli` (tudo + backends concretos),
backends concretos só `core`+`backend-api`. Cada aresta é um teste de compilação da regra
"camada genérica não conhece Mega Drive" — o custo de mantê-las é 12 manifestos, o custo de
não tê-las é a regra deixar de ser verificável. **Não recomendo colapsar crates.**

Onde há concentração real:

- **`chronofork-sessions` (11 731 linhas de `src`) faz seis coisas**: ciclo de vida
  (session/manager/snapshot, ~1,4 k), storage durável (storage/*, ~3,7 k), formato `.cfmovie`
  + v2 (movie 3,7 k), player/rewind (player 1,2 k), replay/compare (0,6 k), edição de
  branch (0,8 k). Consequência observável: `SessionError` tem ~30 variantes, das quais ~17
  são de **storage** (`sessions/src/error.rs`), e o service precisa mapear todas
  (`service/src/error.rs:166-320`, 150 linhas de `match` exaustivo com sanitização de path).
  Alternativa: separar `StorageError` (baixo risco, mecânico) e, se e quando M9 chegar,
  extrair `chronofork-movie`/`chronofork-storage` como crates. **Não fazer agora** — só
  o `StorageError` tem relação custo/benefício clara (P3).
- **`chronofork-megadrive` (18 228 linhas) é grande mas é um modelo de hardware**, não um
  problema de arquitetura. O que incomoda é `backend.rs` com 3 487 linhas (1 646 de testes
  inline) acumulando fábrica, hash/serialização, restore, glue de IRQ e glue de frame; e a
  **superfície pública de 493 itens** (CPU, VDP, bus, scheduler, disasm, views) exposta
  porque os testes de integração (`tests/*.rs`, 8,4 k linhas) são crates externas. Custo:
  cada renomeação interna vira mudança de API pública; `unreachable_pub` está em `warn`
  (não `deny`), então não há pressão. Alternativa: `#[doc(hidden)]`/`pub(crate)` + mover
  testes de integração para um módulo `tests` interno quando só usam internals (P3, não
  fazer antes de M8).
- **CLI 8 991 linhas** (`main.rs` 1 421 + `debug` 1 621 + `tas` 1 273 + `gui` 865 +
  `trace` 942 + `tas_edit` 520 + `session` 522 + `script` 345 + `studio` 381 + `agent` 1 099).
  Concentra responsabilidade? Em parte: 50 funções `parse_*` manuais, `run_megadrive_demo`
  com ~200 linhas de impressão e vários comandos repetindo "parseia flags → chama camada →
  imprime blocos `key=value`". Mas a CLI é raiz de composição por contrato (só ela conhece
  transporte + backend concreto), e a maior parte do volume é **contrato de processo**
  (stdout determinístico, códigos de saída) — verificada por 15 482 linhas de testes.
  Não vejo ganho em fragmentar; vejo ganho em extrair um helper de parsing de opções
  (P3, ~200 linhas duplicadas).
- **GUI 8 629 linhas** (`view.rs` 2 012 com decoders estritos, `command.rs` 1 338,
  `model.rs` 1 240, `pacing.rs` 683, `presentation.rs` 683, `render/*` 1 229). É
  apresentação pura sobre **uma** costura (`OperationChannel`, `gui/src/channel.rs:35-40`),
  consumindo só `core`+`service`. Não há código de emulação, storage ou transporte ali
  (verificado). O tamanho vem de 8 painéis × 2 renderizadores × decoders estritos; é
  proporcional ao escopo declarado. **Único reparo arquitetural**: `DEFAULT_FPS`/
  `DEFAULT_TICKS_PER_FRAME` hardcoded (A1-04).

---

## 8. Abstrações especulativas e específicas demais (pergunta 4)

**A1-08 — `Capabilities` tem flags sem consumidor e flags constantes.** 14 campos
(`descriptor.rs:38-70`). Censo: `rendering`/`audio` são lidos pela GUI
(`gui/src/command.rs:835-844`, `gui/src/view.rs`), `debug_events` é lido pelo trace
(`trace/src/capture.rs`), o resto é impresso pelo CLI (`cli/src/main.rs:1066-1093`) ou
ignorado. `tracing`, `breakpoints` e `audio` **nunca são `true`** em nenhum backend
(`test-backend/src/lib.rs:93-103`; megadrive `backend.rs:241-262`); `execution`,
`save_states`, `state_cloning`, `deterministic_replay` são `true` em todos. Ou seja, o mapa
de capabilities hoje carrega 3 mentiras potenciais documentadas (campos que existem mas não
podem ser verdade) e 4 tautologias. Alternativa: manter as flags (custo zero, documentação
honesta) e **adicionar um teste de contrato** que exija `capabilities.tracing == false`
enquanto não houver backend que implemente; ou remover as três nunca-usadas. Preferência:
manter + teste (P3).

**A1-13 — `InputFrame` schema 2 é especulativo no hardware e real no formato.** 4 portas
digitais + 4 eixos + 64 teclas (`core/src/input.rs:41-49`), mas **nenhum backend consome
eixos ou teclas** (só `ports()[0..2]` em `megadrive/src/backend.rs:758-759`) e apenas
`ports[0]` chega ao 68000; nenhum backend tem dispositivo analógico. Já está no formato de
movie v2 (`movie.rs:512,1742-1749`), então removê-lo agora quebraria um formato versionado.
Veredito: **dívida aceitável e barata** (24 bytes por passo em memória, 26 bytes por
registro em disco), mas o comentário do módulo deveria dizer que eixos/teclas ainda não têm
consumidor, para ninguém planejar H4 em cima de uma capacidade fantasma. P3.

**A1-07 — Duplicação do `StateView` no caminho quente.** `backend.rs:454-479` (payload) e
`:488-513` (hash) constroem a mesma `StateView` duas vezes (~25 linhas cada). Se um campo
novo entrar em um e não no outro, hash e payload divergem silenciosamente (os testes golden
pegariam, mas o custo de depuração é alto). A justificativa de performance no comentário é
contraditada pela medição: a serialização custa 2,7 µs por passagem. Recomendação: uma única
construção de `StateView`, duas funções de consumo. P3, ~30 linhas.

**A1-11 — Seis loops de execução sem driver comum.** `trace/src/capture.rs:208-261`,
`debug/src/debugger.rs:439-500`, `sessions/src/movie.rs` (gravação/replay),
`sessions/src/player.rs:491-651`, `service/src/service.rs:461-491`, e vários loops no CLI
(`cli/src/main.rs:391,815`). Cada um trata `halted`, limite de passos, `inspect` e erro de
forma ligeiramente diferente (ex.: trace inspeciona **antes** do passo e de novo **depois**;
debugger inspeciona no topo; service não inspeciona no loop). Um `HostDriver` que recebe
`FnMut(&mut dyn Backend) -> Control` unificaria a semântica de parada/halted/limite e
permitiria tirar o `inspect` por passo em dois deles de uma vez. Custo ~3–4 dias; risco
médio (mexe em todos os caminhos de execução). Recomendação: **não agora**; fazer junto com
A1-01/A1-02 (a mudança de contrato já obriga a tocar todos esses loops). P3.

**A1-09 — Duas fachadas de aplicação sobre o mesmo manager.** `chronofork-script/src/host.rs:29-180`
reimplementa `open/step/fork/checkpoint/restore/select/reset` sobre `SessionManager`, em
paralelo ao catálogo de 16 operações do service (`service/src/catalog.rs:300-450`). As duas
divergem onde importa: o service tem lease de escrita, limites por requisição, auth e
mapeamento sanitizado de erro; o script tem sandbox/limites próprios mas nenhum lease.
Hoje não há bug, mas qualquer regra de sessão nova precisa ser implementada duas vezes.
Alternativa: extrair um `SessionFacade` fino (core+backend-api+sessions) usado por service e
script. Custo ~3 dias; risco baixo-médio. P2 para M8 (script é superfície de agente).

---

## 9. O que pode ser simplificado sem perder capacidade (pergunta 5)

1. **Remover o hash obrigatório do `step`** (A1-01) — maior ganho, sem perda de capacidade:
   a verificação por passo do `.cfmovie` continua possível, só deixa de ser custo universal.
2. **Separar hash de `inspect`** e adicionar `status()` barato (A1-02) — tracing/debug
   ficam ~2× mais rápidos e `list`/`compare` deixam de hashear.
3. **`Arc<Vec<u8>>` no payload do `Snapshot` + `Arc<BackendSnapshot>` no `initial_state`**
   (A1-05/A1-10) — corta 2 das 4 cópias por fork.
4. **`SessionManager::reap()`** para sessões fechadas — evita o muro de 1024 sessões/800 MiB.
5. **`StorageError` separado** de `SessionError` (mecânico, ~150 linhas de `match` no service
   ficam mais simples) — não muda contrato externo, só a taxonomia interna.
6. **Um `Timebase` no descriptor** (A1-04) elimina o `--fps` hardcoded como conhecimento de
   máquina.
7. **Não simplificar**: os 12 crates, o envelope de snapshot, o codec único
   (`core/codec.rs` usado por 8 crates — é o exemplo de "uma fonte de verdade por formato"
   que o `AGENTS.md` exige), os dois seams opcionais com `None` default, e o dispatch
   dinâmico só na composição (`Box<dyn Backend>` por sessão, `Arc<dyn BackendFactory>`).

---

## 10. Alternativas descartadas (com motivo)

| Alternativa | Por que não |
|---|---|
| Reescrever `Backend` com generics/associated types (`Backend<Cpu, Bus>`, `impl Backend for …` sem dyn) | O dispatch dinâmico acontece **uma vez por passo** (`Box<dyn Backend>`), custo desprezível face a 100–250 µs; generics explodiriam os 6 pontos de composição (service, script, trace, debug, movie, CLI) e impediriam seleção de backend em runtime (CLI `--rom`, service configurável). Preferência estilística, não problema estrutural. |
| Unificar as 12 crates em 3–4 | A direção de dependência hoje é verificada pela compilação; colapsar reintroduz a possibilidade de o genérico depender do Mega Drive. 12 manifestos custam quase nada. |
| `Arc<Mutex<dyn Backend>>` compartilhado entre sessões para "economizar" máquina | Viola o invariante explícito de `traits.rs:24-27` (uma máquina mutável por instância) e destrói o modelo de fork determinístico; o custo real (790 KiB/fork) se resolve com `Arc<Vec<u8>>`, não com compartilhamento de máquina. |
| Trocar o service por um actor/event-loop por sessão agora | Resolveria A1-06, mas quebra o contrato síncrono do service e os testes de processo (HTTP/MCP determinísticos) e traz um executor — dependência proibida sem ADR. Lock por sessão resolve 90 % por 10 % do custo. |
| Remover `backend_version` do envelope (bastaria `config_hash`) | M9 precisa recusar snapshots de versões diferentes da mesma família (a assinatura de `validate_envelope` existe exatamente para isso); remover amplia o risco de reinterpretar bytes. |
| Hash mais rápido (BLAKE3/xxh3) como solução de performance | Continua hasheando 197 KB por passo; ganho menor que B, custo de dependência + ADR + mudança do `core/hash.rs` documentado. Como otimização **opcional depois** de B, ainda é defensável para `state_hash` explícito. |
| Remover os seams opcionais (`debug_access`, `media_mut`) por "abstração especulativa" | São usados por `trace`/`debug` e `megadrive` (rendering) hoje, com default `None` que não custa nada ao test-backend. São o oposto de especulação. |

---

## 11. Achados classificados

Severidade = impacto se não corrigido. Prioridade = ordem de implementação recomendada.
Confiança = força da evidência (alta = código lido + medido/executado).

### P0 — severidade

**A1-01 — Hash canônico completo obrigatório em cada passo, sem caminho de lote**
· *Severidade P0 · Prioridade P1 · Confiança alta (medido)*
· Evidência: `traits.rs:12-21,33`; `megadrive/src/backend.rs:680-685,488-513,1201-1264`;
`inspect()` refazendo hash em `backend.rs:969-971`; medições da §4 (hash 83–93 µs; passo
213–254 µs; 37–40 % mediano, 81 % do melhor caso).
· Componentes: `backend-api`, `megadrive`, `sessions` (movie/player/edit), `service`, testes golden.
· Condições: qualquer backend que cumpra `step`; qualquer `session.run`.
· Impacto atual: throughput teto de 3,9–4,7 k passos/s neste benchmark; tracing/debug 2×
mais caros; `session.list`/`compare` proporcionais ao número de sessões.
· Impacto futuro: em 8.5 o estado cresce (Z80 + YM2612 + PSG) e o hash por passo cresce
junto, sem teto; em M9 todo backend herda o custo ou mente no contrato.
· Probabilidade de manifestar: certa (já manifesta).
· Alternativas: ver §4 tabela (A–E).
· Recomendação: B (`Option<StateHash>` + `Backend::state_hash()`), ou C→B.
· Custo/risco: 40–60 call sites, ~3–5 dias, risco médio (contrato público, testes golden).
· Efeito em testes/contratos: `StepOutcome` muda; snapshots não mudam; `.cfmovie` não muda
(a gravação passa a chamar `state_hash()` explicitamente).
· Milestone: **antes de 8.5 / junto de 8.4**; indispensável antes do trabalho de performance.

### P1 — severidade

**A1-02 — `inspect()` é caminho quente: hash completo + 64 KiB copiados; é o único leitor de
`tick`/`pc`**
· *P1 · P1 · alta (código + medido: `inspect` 211–240 µs ≈ `step`)*
· Evidência: `inspection.rs:12-26`; `megadrive/src/backend.rs:969-976` (`memory: ram.to_vec()`);
`trace/src/capture.rs:256` (inspect por passo); `debug/src/debugger.rs:440`;
`sessions/src/manager.rs:372-408` (compare/summaries).
· Impacto: tracing ~2× mais lento que stepping; `session.list` com N sessões ≈ N × 85 µs de
hash + N × 64 KiB.
· Recomendação: `Backend::status()` barato; `inspect` deixa de hashear por default (combinar
com A1-01). Custo ~2 dias. Milestone: 8.4/8.5.

**A1-03 — `MachineInspection` plano e sem espaço de endereçamento; "pc" é string mágica**
· *P1 · P2 · alta*
· Evidência: §3 (`inspection.rs:12-26`, `backend.rs:883-976`, `trace/src/capture.rs:28`,
`debug/src/debugger.rs:24`, `cli/src/main.rs:1308`).
· Impacto atual: funciona para 1 CPU / 1 espaço; o Mega Drive já empilha 5 dispositivos num
mapa plano e codifica serial em `u64`.
· Impacto futuro: 8.4 (Z80) colide nomes; M9 precisa de espaços múltiplos e CPU virtual/física.
· Condições: qualquer backend com >1 processador ou memória de dispositivo.
· Recomendação: `processors: Vec<ProcessorView>` + `memories: Vec<MemoryWindow>` +
`read_memory(space, …)`; ou, mínimo, convenção `"<device>.<register>"` **no contrato** com
constante exportada. Custo 1–5 dias conforme opção; risco médio (JSON do service, decoder
estrito da GUI). Milestone: **M9 (mínimo: 8.4)**.

**A1-04 — Sem `Timebase`/quantum declarado no descriptor; host hardcoda 60 fps**
· *P1 · P2 · alta*
· Evidência: `core/src/tick.rs:3-7`; `descriptor.rs:96-103`; `gui/src/lib.rs:100-107`;
`cli/src/gui.rs:262`; `SchedulerConfig` PAL existe.
· Impacto atual: pacing de 50 Hz exige flag manual; `ticks_per_frame` é conhecimento de
máquina no host.
· Impacto futuro: M9 com máquinas de 59,7/50/60 Hz e backends de quantum "frame".
· Recomendação: `Timebase` no `Backend`/descriptor + uso pela GUI/CLI. Custo ~2 dias, risco baixo.
· Milestone: antes de M9; útil já em 8.4 (PAL).

**A1-05 — Fork retém ~790 KiB (4× o estado) e o manager não remove sessões**
· *P1 (produto) · P1 · alta (medido: 773,5 KiB/fork; fork 0,80–1,36 ms)*
· Evidência: `manager.rs:184-227,217-221,342-346`; `service/src/limits.rs` (`max_sessions = 1024`);
RSS medido.
· Impacto: 1024 sessões ≈ 800 MiB; forks fechados continuam ocupando registro e memória.
· Recomendação: `Arc<Vec<u8>>` no payload, `Arc<BackendSnapshot>` no `initial_state`,
`reap()` com lease; custo ~3 dias, risco baixo-médio. Milestone: 8.x/antes de M9.

**A1-06 — `Mutex<SessionManager>` global serializa sessões independentes**
· *P1 · P1 · alta (código; cross-area com concorrência)*
· Evidência: `service/src/service.rs:114-122,461-491`; `limits.rs:37`.
· Impacto: um `run` de 100 k passos bloqueia todas as sessões por 10–21 s; nenhuma
paralelidade apesar de `Backend: Send` e máquinas independentes.
· Recomendação: lock por sessão (com o lease como barreira de escrita) ou shard.
Custo 3–5 dias, risco médio-alto. Milestone: antes de M9 (ou junto do trabalho de concorrência).

### P2 — severidade

**A1-08 — `Capabilities`: 3 flags que nunca são `true`, 4 tautologias** · *P2 · P3 · alta*
· `descriptor.rs:38-70`; `test-backend/src/lib.rs:93-103`; uso em `gui/src/command.rs:835-844`.
· Recomendação: teste de contrato por flag não implementada + documentar; ou remover
`tracing`/`breakpoints`. Custo ~2 h. Milestone: 8.x.

**A1-09 — `SessionError` mistura 17 variantes de storage; `sessions` acumula 6 subsistemas**
· *P2 · P3 · alta* · `sessions/src/error.rs`; `service/src/error.rs:166-320`.
· Recomendação: `StorageError` separado (mecânico). Split de crates só se/quando doer.
Custo ~1 dia. Milestone: pós-M8.

**A1-12 — Superfície pública de `megadrive`: 493 itens `pub`** · *P2 · P3 · alta*
· `crates/chronofork-megadrive/src/lib.rs:39-120`; testes de integração são crates externas.
· Recomendação: `#[doc(hidden)]` para internals + `pub(crate)` onde os testes permitirem;
mover testes que só usam internals para módulos internos. Custo ~2 dias, risco baixo.
Milestone: pós-M8.

**A1-13 — Fachadas duplicadas service vs script** · *P2 · P2 · média*
· `script/src/host.rs:29-180` vs `service/src/catalog.rs:300-450`.
· Recomendação: `SessionFacade` comum. Custo ~3 dias. Milestone: M8 (script é superfície de agente).

### P3 — severidade

**A1-07 — `StateView` duplicado no caminho quente** · *P3 · P3 · alta* ·
`backend.rs:454-479` vs `:488-513`. Custo ~30 linhas, risco baixo. A justificativa de perf
no comentário não se sustenta (serialização = 2,7 µs).

**A1-10 — `restore` clona o payload inteiro** · *P3 · P3 · alta* · `session.rs:176-188`.
Resolvido junto com A1-05(b).

**A1-11 — Seis loops de execução sem driver comum** · *P3 · P3 · alta* · §8.

**A1-14 — 50 funções `parse_*` manuais no CLI (~200 linhas de padrão repetido)** ·
*P3 · P3 · média* · `cli/src/*.rs`. Extrair helper de opções; não fragmentar a CLI.

---

## 12. O que não consegui verificar

1. **Onde vão os 14–125 µs que sobram do passo sem o hash.** Não perfilei o interpretador
   68000 nem `Vdp::advance_master` (é da área de performance/emulação). Afirmo apenas a
   decomposição hash/serialização/cópias e que o resto *não* é hash.
2. **Custo real em 8.4** (Z80 + YM2612 + PSG): o checkout pai está em `agent/8.4-z80-audio`
   com trabalho não commitado e é intocável; não li nem executei. Todo número deste relatório
   é do baseline `b8d63a1` (estado 197 048 B). O hash por passo **cresce** com o estado, então
   os 37–40 % são um piso, não um teto.
3. **Custo de `debug_access`/watchpoints habilitados por passo** (não medi; o
   `observe_scheduler` por passo é O(2) e `debug_begin_step` só grava o tick, então não
   espero custo material, mas não experimentei).
4. **Se algum consumidor externo (fora do repo) depende de `StepOutcome.state_hash`** —
   não há como saber; a mudança B é breaking para clientes de biblioteca (não para o wire do
   service).
5. **Custo de migrar os testes golden** se A1-01/B for adotado: estimei 40–60 call sites por
   busca textual, mas não tentei a migração.
6. **Comportamento sob 1024 sessões reais** (memória e latência de `list`): extrapolei de
   200 forks medidos; não executei o cenário de 1024.

---

## 13. Perguntas para o capitão / crítico

1. **A1-01 é mudança de contrato público**: o dono precisa decidir entre B (limpo, quebra
   biblioteca) e C (aditivo, deixa a armadilha). Minha recomendação é B, mas C é aceitável se
   a prioridade for não tocar `StepOutcome` antes de M8 fechar.
2. **A1-06 (lock global) precisa de decisão de concorrência**: minha posição é lock por
   sessão com o lease existente, **não** async. Se a área de concorrência discordar, o
   achado de arquitetura continua válido (o ownership atual serializa o que é independente).
3. **Prioridade relativa**: o hash (37–40 % do passo) é grande, mas o interpretador é duas
   ordens de grandeza. Peço que a síntese **não** apresente "remover o hash" como a solução de
   performance de M8 — é uma fração necessária, não suficiente.
4. **Corrigir o baseline**: "~123 k LOC" → 66,4 k `src` + 48,9 k `tests`; e
   "`create_machine` clona o Cartridge" é verdade mas custa 2,0–2,5 µs (não é achado).
