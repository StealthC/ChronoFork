# Área 05 — Determinismo, testes diferenciais e qualidade

**Revisor:** quality (worker `t5`)
**Baseline:** `b8d63a17349e672ba6945eaba3379cec38cf9d1b` (`origin/main`), worktree
`/home/stealthc/projects/ChronoFork/.agents/worktrees/review-2026-11-20-architecture`,
branch `agent/review-2026-11-20-architecture`.
**Data:** 2026-11-20 (America/Fortaleza). **Time-box:** ~90 min.
**Ambiente:** `CARGO_HOME=/tmp/t7-cargo`, `CARGO_TARGET_DIR=/tmp/chronofork-review-target`
(compartilhado com os demais revisores).
**Labels:** **fato** = observado/executado; **hipótese**; **inferência**; **opinião**.
Cada achado traz severidade (impacto) separada de prioridade (ordem de trabalho).

Artefatos brutos desta revisão (ignorados, não versionados):
`.research/review-2026-11-20/agents/quality/` (`check_fixtures.py`, `corpus_probe.py`,
`corpus_probe.json`, `roms/`, `traces/`, `demo/`).

---

## 0. Sumário executivo

O aparato de qualidade do ChronoFork é **muito acima da média** para um projeto nesta
fase: gate local completo e verde, 1289 testes, testes de propriedade nos parsers,
testes de concorrência no serviço, matriz de rejeição de snapshots, demos
byte-determinísticos e um differential com um oráculo externo independente
(Exodus headless) que **já pagou por si**: encontrou um defeito real de decodificação
(`MOVE.L (abs).L,Dn`), um bug de `ABCD` e cinco tabelas de ciclos erradas.

O problema não é a existência de testes; é **o que as afirmações "68000 completo",
"VDP completo" e "1289 testes" realmente sustentam** e **quem consegue reproduzir a
evidência**. Três conclusões centrais:

1. **A corrida diferencial não é reproduzível a partir de um clone limpo.** Exige um
   checkout *out-of-tree* de um projeto C++ pinado, build com cmake/clang e binários
   cujo SHA-256 está registrado só em prosa. O harness **falha com erro** (exit 2) se
   faltar o oráculo, e a verificação "fixture commitada == ROM gerada" só existe
   *dentro* desse harness inalcançável (Q-01). Medido: 8 fixtures commitadas conferem
   com o gerador; as 3 ROMs legadas não estão commitadas e seus marcadores gerados
   hoje divergem em +2 dos valores registrados na documentação.
2. **A cobertura do corpus é pequena e concentrada em instruções "fáceis".** Medi a
   execução real das 11 ROMs com o binário local: **550 instruções**, **186 palavras
   de opcode distintas**, **145 delas executadas uma única vez**. Nenhuma exceção, nenhum
   handler de interrupção, nenhum `TRAP`/`RTE`/`LINK`, nenhum frame de pilha é comparado
   com o oráculo (Q-02). A frase correta é "o subconjunto documentado está coberto
   nesses 11 programas", não "68000 completo".
3. **Os controles negativos provam o comparador, não o pipeline.** Os três controles
   corrompem a *projeção* e verificam que o comparador falha; nenhum demonstra que uma
   regressão no emulador (1 ciclo a mais, flag errada) faz o harness falhar (Q-03). Um
   controle de mutação no emulador é barato e fecharia essa lacuna.

O "1289" é **verdadeiro e verificável** (`cargo test --workspace -- --list` → 1289),
mas inclui 2 testes que **pulam silenciosamente** quando os ROMs comerciais locais não
existem — ou seja, o CI (que nunca os tem) executa um número efetivo menor (Q-05).
Os gates rodados nesta revisão estão **todos verdes**: fmt 0, check 0, clippy 0, rustdoc
0 (0 warnings).

Sobre anti-slop: a política **detecta defeitos reais** (o catálogo §9 A1–A11 é prova
disso e os defeitos eram graves: leitores duplicados, ramos inalcançáveis, erros
achatados em `String`, alocação antes de validar). Mas ~40% do seu volume é
procedimento que não muda decisão, e ela duplica a lista de comandos em pelo menos
cinco lugares — violando o seu próprio princípio de fonte única. Simplificação
proposta na §6, sem perder o que funciona.

---

## 1. Gate executado (resultado real)

Comandos e resultados reais desta sessão (não reaproveitados do baseline):

| Comando | Exit | Observação |
|---|---:|---|
| `cargo fmt --all -- --check` | 0 | limpo |
| `cargo check --workspace --locked` | 0 | `Finished dev profile` |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 | sem warnings |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked` | 0 | 0 ocorrências de `warning` no log |
| `cargo test --workspace --locked --no-fail-fast -- --list \| grep -c ': test$'` | 0 | **1289** testes listados |
| `cargo test -p chronofork-sessions --test determinism --no-run` | 0 | alvo do passo "Determinism suite" do CI existe |

**fato.** O número 1289 do baseline se reproduz por contagem independente
(`--list`), o que também confirma que **não existem doc-tests** (eles não aparecem no
`--list` e a soma coincide). Inventário: 12 crates, 11 alvos de biblioteca, **85
arquivos de teste de integração** (86 no total, um é `common/mod.rs`) →
96 binários de teste + 12 alvos de doc = 108 alvos, consistente com o baseline.

Não reexecutei `cargo test --workspace` completo: o baseline já registrou
1289/0/0 e a suíte tem testes de 166 s, 59 s e 36 s; gastar ~6 min de máquina
compartilhada para recomputar o mesmo número não mudaria nenhuma decisão. O que fiz
foi verificar **por contagem independente** e executar experimentos que o baseline não
cobre (§2, §3).

---

## 2. A estratégia diferencial com Exodus: o que ela realmente prova

### 2.1 Corpus e cobertura medida (experimento)

`scripts/oracle-diff.py` gera 11 ROMs sintéticas de 4096 bytes com um assembler
próprio e as compara com o Exodus headless pinado. Rodei as 11 ROMs **sem o oráculo**
(binário local, `--stop-pc` no marcador, `--cycles`) e contei o que é efetivamente
comparado (`.research/.../agents/quality/corpus_probe.py`):

| ROM | instruções executadas | opcodes distintos |
|---|---:|---:|
| `rom_alu` | 100 | 65 |
| `rom_mem_vdp` | 52 | 40 |
| `rom_masked` | 156 | 51 |
| `rom_ea` | 24 | 24 |
| `rom_shift` | 26 | 26 |
| `rom_bcd` | 22 | 22 |
| `rom_movem` | 22 | 22 |
| `rom_muldiv` | 26 | 26 |
| `rom_exg_tas` | 41 | 41 |
| `rom_status_reg` | 39 | 37 |
| `rom_bits_dbcc` | 42 | 42 |
| **total** | **550** | **186** |

**fato.** 550 instruções comparadas; 186 palavras de opcode distintas; **145 opcodes
aparecem uma única vez**. Duas execuções por ROM produziram **trace e state
byte-idênticos** em todos os 11 casos (comparação direta `cmp`/sha256).

Contraste: a tabela de auditoria de decodificação
(`crates/chronofork-megadrive/src/cpu/decode.rs:332-435`) fixa **102 encodings
representativos**. A matriz real do MC68000 é `instrução × tamanho × modo de EA`
(12 modos × 3 tamanhos × ~60 famílias) — ordens de grandeza acima. O corpus cobre
**~2% – 5%** do espaço de encodings válidos dependendo de como se conta. Isso não
invalida a evidência; delimita a afirmação.

### 2.2 Independência entre implementação e testes (a parte crítica)

**fato.** As 11 ROMs são geradas pelo *mesmo* harness escrito pelo autor da
implementação, com um assembler próprio (`scripts/oracle-diff.py:169-295`). O único
elemento genuinamente independente é o oráculo externo (Exodus). Isso é suficiente
para o que a evidência se propõe — mas a história do próprio repositório mostra o
risco: o defeito `MOVE.L (abs).L,Dn` só apareceu porque a ROM diferencial usou `0x2239`
enquanto a ROM de demo **e os testes unitários** usavam a forma errada `0x..38`
(`docs/research/oracle-differential-evidence.md:268-284`). É o caso canônico de
"implementação e teste compartilham o mesmo erro"; aconteceu **uma vez**, o que torna a
hipótese de recorrência realista, não teórica.

**fato (positivo).** A auditoria de decodificação **não é um gêmeo**: `classify()`
(`decode.rs:126`) é o despachante de produção usado por `M68000::step` e pela tabela de
ciclos (`cpu/timing.rs:66,473`); o teste de auditoria exercita o código real, não uma
cópia. Isso é o padrão correto (e o oposto do anti-padrão A3 do próprio catálogo).

**fato.** Fora dos 11 programas, todo o resto da semântica de instruções é verificado
por testes unitários escritos pelo autor contra a leitura do manual. Isso cobre muito
bem "o que o autor quis", mas não detecta "o que o autor leu errado" — exatamente a
classe de defeito que o oráculo pegou (`ABCD`, `ORI/ANDI/EORI` a `SR` mascarando o
imediato, 4 custos de sistema, tabela de bits placeholder).

### 2.3 Projeções e normalizações

O que é comparado (`oracle-diff.py`): sequência de endereços executados até o marcador,
custo em ciclos por instrução (delta do acumulado do oráculo), e estado final restrito
a D0–D7, A0–A7, PC, SR efetivo e 64 bytes de work RAM.

Normalizações aplicadas — todas legítimas e declaradas:

- SR: compara `IPM`/`S`/`CCR`, não a palavra crua (Exodus `0x7FE0` vs `0x2700`).
- `X` no reset: as ROMs limpam `X` explicitamente (`ADD.B D0,D0`), porque Exodus liga
  com `X=1` e ChronoFork com `X=0`. Correto e documentado (`cpu-cycle-differential-evidence.md:162-167`).
- Registradores indefinidos no reset do oráculo: cada ROM inicializa o que compara.
- Leituras de porta de dados do VDP excluídas (prefetch do oráculo difere do modelo mínimo).

**inferência.** Essas normalizações são legítimas porque cada uma remove uma diferença
de *ponto de partida* documentada, não uma diferença de *comportamento*. O risco residual
é que a lista cresça sem controle: hoje são 4; cada nova precisa de justificativa escrita.

**fato (limite aceito, mas relevante).** A comparação de endereços **não compara o
mnemônico**: o opcode do ChronoFork é verificado contra os *bytes da ROM* naquele
endereço, o que é quase tautológico (o emulador buscou a palavra). Se ele executasse
uma instrução diferente, de mesmo custo e mesmo efeito no estado final, o teste
passaria. As mitigações são as ROMs auto-verificadoras (branches que só não divergem se
as flags estiverem certas) e o estado final. Aceitável, mas deve ser dito — e é dito
(`oracle-differential-evidence.md:603-606`).

### 2.4 Controles negativos

**fato.** Três controles por ROM: endereço (corrompe um registro da projeção do
oráculo, exige exit 1 no índice exato), ciclo (soma 1 ciclo na primeira instrução
projetada), estado final (altera uma palavra de RAM/VRAM no caso do VDP).

O que eles provam: o **comparador** não é permissivo. O que eles **não** provam: que
uma regressão no **emulador** faz o harness falhar. Exemplo concreto do risco: se uma
mudança futura fizesse `MOVE.W SR,(A0)` custar 8 em vez de 6, o controle de ciclo
passaria a detectar isso apenas se alguém rodasse o harness — e não há nada que
demonstre esse encadeamento. É o achado Q-03.

### 2.5 Reprodutibilidade (o achado mais sério da área)

**fato, verificado no código e por experimento:**

- `oracle_provenance()` **aborta** (`HarnessError`, exit 2) quando o checkout do
  oráculo não está no commit pinado ou os binários não existem
  (`scripts/oracle-diff.py:1809-1823`). Não há caminho de *skip*: um contribuidor sem o
  clone de `.research/upstream/exodus-headless` **não consegue rodar nada** do diferencial
  de CPU. (O harness de VDP, ao contrário, tem `skip_report` e exit 3 —
  `scripts/oracle-vdp-diff.py:868-908`. Inconsistência entre os dois harnesses.)
- A verificação "fixture commitada == ROM gerada" está em `run_rom`
  (`oracle-diff.py:1466-1470`), ou seja **depois** da checagem do oráculo. Portanto ela
  também é inalcançável sem o oráculo, e **não existe nenhum teste Rust** que valide
  `scripts/fixtures/*.bin` contra o gerador. Se o gerador mudar e a fixture não, nada
  falha em `cargo test`.
- Medi a igualdade gerador↔fixture com um script próprio
  (`agents/quality/check_fixtures.py`): **8/8 fixtures commitadas conferem** com o
  gerador atual; **`rom_alu`, `rom_mem_vdp` e `rom_masked` não estão commitadas** e os
  marcadores gerados hoje são `0x354`, `0x2E2`, `0x2F0`, enquanto a documentação registra
  `0x352`, `0x2E0`, `0x2EE` (deslocamento de +2 causado pela inclusão do `ADD.B D0,D0`
  que limpa `X`). **fato:** a evidência publicada não é regenerável bit-a-bit pelos
  comandos publicados, e as tabelas de marcador das páginas de evidência estão
  desatualizadas.
- O próprio texto gerado pelo harness ainda diz "ten independently authored synthetic
  ROMs" (`oracle-diff.py:2374`) com 11 no corpus — drift interno do gerador de evidência.

Consequência prática: hoje a evidência diferencial funciona como **atestado histórico
de uma execução**, não como um teste que qualquer pessoa (ou CI) roda. Isso é aceitável
como estágio, mas precisa estar explícito na documentação de capacidade e precisa de um
plano de conversão para gate reproduzível (Q-01).

### 2.6 Estados intermediários e combinações incomuns

**fato.** O que se compara são **endereços por instrução** + **estado final**. Não há
comparação de registradores/flags/CUR por instrução, nem de frames intermediários. As
combinações "incomuns" cobertas são as que o autor escolheu: EA modes (`rom_ea`),
MOVEM/MOVEP, MUL/DIV, EXG/TAS/ADDX/SUBX, SR/CCR, bits/DBcc. Não cobertas pelo oráculo:
exceções e frames de pilha (`TRAP`, `RTE`, `LINK/UNLK`, `CHK`, `TRAPV`, violação de
privilégio, address error), entrada de interrupção com máscara < 7, `STOP`, `RESET`,
`MOVEP` em hardware real (só o modelo), `TAS` em memória (excluído por decisão
consciente e correta — é comportamento de barramento, não de núcleo).

**opinião (alta confiança).** A próxima ROM diferencial de maior valor não é "mais
opcodes": é **uma ROM que entra em exceção e retorna** (`TRAP #n` + handler + `RTE`,
com A7 e o frame de 6 bytes no estado final). É onde os erros de hardware são mais
prováveis (ordem de push, SR restaurado, PC, A7) e é justamente onde hoje só existem
testes unitários do próprio autor.

---

## 3. As afirmações "68000 completo", "VDP completo" e "1289 testes"

### 3.1 "1289 testes"

**fato:** verdadeiro e reproduzível (§1). Três ressalvas materiais:

1. **Pulos silenciosos.** `crates/chronofork-megadrive/tests/cartridge.rs:151-196` tem
   dois testes que fazem `eprintln!("skipping: …"); return;` quando
   `.research/roms/` está ausente. Eles **passam** sem asserção nenhuma. Isso viola o
   princípio 6 da própria estratégia de testes ("no silent skips", §1.6 de
   `docs/quality/testing-strategy.md`) e significa que o CI — que nunca tem ROMs
   comerciais — roda menos verificações do que o número sugere, sem que ninguém veja.
   Mitigação existente: o cabeçalho do arquivo documenta o skip; o teste não é
   `#[ignore]` (portanto não aparece como ignorado no relatório).
2. **Ausência de guarda de contagem.** Não há equivalente ao `TreatNoTestsAsError`:
   nenhum job falha se um alvo passar a descobrir zero testes.
3. **Contagem não é cobertura.** 1289 testes em 108 alvos incluem muitos testes
   pequenos de contrato (correto) e um alvo de demo (`megadrive_demo_*`) que custa
   166,9 s (Q-12).

### 3.2 "68000 completo"

A afirmação defensável é a de `docs/architecture/megadrive-slice.md` ("covers the
documented MC68000 instruction groups") + a lista de exclusões no mesmo arquivo. O que
**não** está sustentado:

- **fato:** nenhuma suíte externa licenciada de conformidade (não há fixtures de
  terceiros; `scripts/fixtures/` só tem ROMs sintéticas). `docs/quality/rom-fixtures.md`
  prevê o caminho, mas não foi usado.
- **fato:** nenhum fuzzing (`fuzz/` inexistente, nenhum alvo `cargo-fuzz`), nenhum teste
  exaustivo de decodificação (varrer as 65 536 palavras e exigir "classifica ou é
  `Other` tipado, nunca panic" é um teste de minutos e não existe).
- **fato:** 5 binários de `proptest` existem, mas todos em **parsers**
  (`core/tests/codec_properties.rs`, `sessions/tests/snapshot_properties.rs`,
  `megadrive/tests/snapshot_properties.rs`, `trace/tests/format.rs`, e um uso em
  `service/src/json.rs`). Nenhum teste de propriedade para CPU/decoder/VDP-compositor.
- **fato:** a matriz EA × tamanho é testada por amostragem
  (`cpu_ea.rs` 177 linhas, `cpu_move.rs` 292, etc.), não exaustivamente.
- **fato:** o differential não cobre exceções, interrupções, nem `DIVU/DIVS`
  dependentes de operando (o oráculo usa o mesmo pior caso).

### 3.3 "VDP completo"

**fato** (`docs/research/vdp-state-differential-evidence.md`,
`scripts/oracle-vdp-diff.py`): são **dois** programas (`register-vram`,
`status-counters`) comparados com o oráculo. O que é comparado: PC, S/IPM, D0/D1/D3,
work RAM, e — **só do lado ChronoFork** — VRAM/CRAM/registradores contra os valores que o
próprio programa escreveu. O oráculo **não expõe** VRAM/CRAM/registradores (o probe
retorna o latch de escrita obsoleto), então não existe comparação cruzada de memória de
vídeo.

Consequências:

- 18 valores (H/V counter, status, readback de CRAM) são classificados como
  "divergências escopadas" e **registrados**, não asseverados: só a *transição* do bit
  HBlank é comparada. **inferência:** uma regressão futura no contador V, na fase de
  power-on ou no bit FIFO pode entrar sem quebrar o harness, porque ali não há asserção.
- Frames são determinísticos e fixados por hash (`vdp_frame.rs:165,170`; H32/H40), o que
  é excelente como **regressão**, mas é auto-referente: se o renderer estiver
  sistematicamente errado (ex.: shadow/highlight invertido), o golden continua passando.
- DMA, FIFO, colisão de sprite, prioridade e janela são cobertos por testes unitários do
  autor; DMA não é comparado ao oráculo por não ser externamente observável.

**opinião.** "VDP completo" deve ser lido como "subconjunto documentado implementado,
com dois programas diferenciais e forte regressão interna". A frase é defensável **se**
as aproximações (H32 window, line-scroll, interlace ausente, VRAM byte-swapped) estiverem
na mesma página da capacidade — estão. O que falta é **asserção** sobre os 18 valores
escopados (Q-04).

---

## 4. Lacunas de categorias de teste (o que existe / o que falta)

| Categoria | Estado (fato) | Recomendação |
|---|---|---|
| Unit/integration | sólido, 85 alvos | — |
| Property-based | 5 binários, só parsers | adicionar propriedades ao decoder (exaustivo) e ao compositor VDP (invariantes de frame) |
| Fuzzing | **ausente** (`fuzz/` não existe) | 1 alvo para `Snapshot::from_bytes` + 1 para `Vdp::restore_bytes`, nightly, corpus mínimo, `cargo fuzz` com tempo limitado |
| Differential CPU | 11 ROMs, 550 instruções | ROM de exceção/RTE (Q-02), depois ROM de interrupção com máscara < 7 |
| Differential VDP | 2 programas | programa de DMA/FIFO/timing; asseverar os 18 escopados |
| Test ROMs licenciados | **nenhum** (só sintéticas) | avaliar corpus de single-step público para 680x0 — **licença precisa ser verificada antes de qualquer uso**; hoje não há `deny.toml` para apoiar essa decisão |
| Golden/regressão | forte (hashes fixos de stdout, frames, GUI, replay) | fixar o golden do `megadrive demo` (Q-12) |
| Cross-process | **forte**: CLI tests usam `Command::new`, demos 3×, GUI/transport | manter; adicionar comparação debug×release explícita (ver §5) |
| Cross-arch/SO | **só compile check** Windows/macOS | rodar a suíte de determinismo em pelo menos 1 runner não-Linux (job noturno) |
| Concorrência | **forte** no serviço (lease, conflito, leitura durante escrita) com `Gate` determinístico | estender a stepping concorrente de instâncias independentes do backend (INV-17 não tem teste explícito de 2 threads × 2 máquinas) |
| Entradas inválidas | **forte** (`rejection.rs` 557 linhas, `bounds.rs`, proptest de bytes arbitrários, `crash_recovery.rs`) | — |
| Artefatos corrompidos | **forte** (truncamento/extensão/magic tipados; corrupção de checkpoint) | — |
| Determinismo cross-profile | **parcial**: fixei empiricamente nesta revisão | tornar gate (barato, ver Q-12) |

**Experimento feito nesta revisão (cross-process + cross-profile):** executei
`megadrive demo` com o binário **debug** e com o **release**, em processos separados:
stdout **byte-idêntico**, `sha256 = fe254e5d193de273bc0d0db1b9b426ef6b069f4f96685486cb7303662d56a420`.
Tempo: 83,6 s (debug) vs **7,7 s** (release) — razão 10,8×, o que é relevante para a
estratégia de CI (§5). **fato.**

---

## 5. Estratégia de CI proposta (equilibrando cobertura, custo, duração, confiabilidade)

Estado atual (fato, `.github/workflows/ci.yml`): 7 jobs — `fmt`, `check-clippy`,
`test` (matriz em 4 grupos de crates + passo de determinismo single-thread), `docs`,
`demos` (release, 3× cada demo), `release` (build + **suíte completa em release**),
`msrv` (1.85), `portability` (check em Windows/macOS). Sem JUnit, sem `cargo deny`, sem
checagem de fixtures, sem job noturno, **nada diferencial**.

Custo medido relevante: `megadrive_demo_*` 166,91 s; `debug_observation` 59,06 s; lib
do megadrive 49,06 s; `cycle_timeline` 35,97 s; `cartridge` (CLI) 34,10 s. O CI roda a
suíte **duas vezes** (debug na matriz + release), então esses testes custam ~2× o
orçamento. O demo `megadrive` em debug custa 83,6 s **por execução**; o teste
`megadrive_demo_output_is_deterministic` o roda **duas vezes** (167 s) para comparar
run-vs-run — comparação que fica obsoleta assim que existir um golden.

Proposta em três camadas (custo crescente, confiabilidade decrescente):

**Camada 1 — PR (alvo ≤ 12 min de caminho crítico, obrigatório):**

1. `fmt` + `check` + `clippy -D warnings` (inalterado).
2. Testes de unidade/integração rápidos por crate, com `--skip megadrive_demo` (o
   libtest aceita `--skip <substring>`) para tirar os alvos pesados do caminho crítico
   do PR — eles continuam rodando na camada 2, não somem. Os 4 grupos por crate já
   existentes dão o resto do paralelismo.
3. **Golden do `megadrive demo`**: fixar o SHA-256 do stdout (medido:
   `fe254e5d…`) em um teste único em **release** (~8 s) em vez de 2× em debug (167 s).
   Isso converte um teste de 167 s em um gate de 8 s **e** ganha poder de detecção
   (passa a ser golden real, e o job `release` prova independência de perfil).
4. Job `determinism` (barato, ~20 s): 2 processos × (debug, release) do `demo` e do
   `megadrive demo`, `cmp` dos quatro pares. É a verificação D2 da estratégia de testes,
   hoje inexistente como gate.
5. Checagem de fixtures (F1/F2 do `testing-strategy.md §7.3`): script de ~40 linhas +
   `sha256sum` manifesto. Custo ~1 s; fecha a lacuna de "fixture pode divergir do
   gerador sem ninguém ver" (`rom-fixtures.md` prevê, `tests/fixtures/manifest.toml` não
   existe).
6. `cargo deny check` (G3) quando houver mais de uma dependência direta; hoje há uma
   (`sha2`) + `proptest` (dev), então o valor é preventivo — custo ~30 s, vale.
7. Guarda de contagem: falhar se um alvo descobrir 0 testes.

**Camada 2 — main/nightly (≤ 30 min, não bloqueia PR):**

8. Suíte completa em **debug** e **release** (uma vez cada) — preservando o debug para
   detectar overflow aritmético (os testes de CPU dependem disso; rodar só release
   perderia isso).
9. Determinismo **single-thread** (`--test-threads=1`) para expor vazamento de estado
   entre testes — o job do CI já existe, mantenha-o aqui em vez do PR.
10. **Diferencial**: `oracle-diff.py --skip-build` e `oracle-vdp-diff.py` numa imagem
    com o oráculo pré-compilado e cacheado (o oráculo é código C++ pinado; buildar a
    cada run custa minutos). Enquanto isso não existir: (a) documentar que a evidência
    é histórica, (b) fazer os harnesses **exit 3 (skip)** quando o oráculo faltar, como
    já faz o de VDP, e (c) no nightly, tratar `skip` como **falha** (não aceitar
    "passou sem rodar").
11. `portability` passa a **rodar** a suíte de determinismo (não só `check`) em
    `windows-latest` — um runner, não dois, para custo.

**Camada 3 — release (manual, sob demanda):** reprodutibilidade de build, MSRV,
matriz de snapshots, notices.

Confiabilidade: eliminar `thread::sleep` como sincronização onde houver alternativa
(`service.rs:1047` usa 50 ms só para ordenar um leitor; `http.rs:1332` usa 20 ms;
`m6.rs:237-247` já usa deadline de polling, que é o padrão melhor). `proptest` está sem
seed fixo — um caso gerado pode falhar de forma intermitente; recomendo **commitar
`proptest-regressions/`** (não existe hoje) e registrar a seed no log do CI.

---

## 6. Política anti-slop: o que funciona, o que é burocracia, o que simplificar

### 6.1 O que efetivamente detecta problema real

**fato.** O catálogo §9 é auto-evidência: A1 (5 leitores binários duplicados), A2 (dois
serializadores do mesmo estado — "um campo novo podia escapar do hash"), A4 (ramos
inalcançáveis), A6 (erros tipados achatados em `String` — `OutOfBounds` inutilizável),
A7 (alocação de `length` externo **antes** de validar — aborto por tamanho enorme), A10
(gate vermelho não detectado porque ninguém rodou). São defeitos reais, de severidade
alta, e as regras AS-02/AS-03/AS-12 são precisamente as que os pegam. O mecanismo
**baseline-first + igualdade de bytes** é a parte mais valiosa: transforma "acho que não
mudei nada" em `cmp`.

Também funciona: a exigência de **um único caminho de serialização alimentando payload e
hash** (AS-03) — é o que impede o bug clássico de "estado novo fora do hash", que neste
projeto significaria forks não determinísticos. E a regra "nenhum enfraquecimento"
(`allow`/`ignore`/teste apagado) com verificação por `git diff`.

### 6.2 O que é burocracia ou ruído

1. **Cinco cópias da lista de comandos de gate** (`scripts/check.sh`, `AGENTS.md`,
   `docs/quality/quality-gates.md`, `docs/quality/testing-strategy.md §8`,
   `docs/quality/anti-slop-policy.md §6.1`, além de 7 docs de aceitação). Elas **já
   divergiram**: o `quality-gates.md:1075-1076,1092-1093` afirma que o CI "runs on every
   pushed commit (all branches)" e "sets no concurrency cancellation", mas o
   `ci.yml:13-21` só dispara em `push: branches: ["main"]` e cancela runs de PR; e
   `quality-gates.md:1080` descreve `cargo test --workspace` enquanto o CI roda uma
   matriz por grupo. **fato.** Isso é exatamente o anti-padrão A9 que a própria política
   proíbe, e é mais caro de manter do que qualquer regra removida.
2. **A varredura de `allow(`** (`anti-slop-policy.md:209,244`) tem taxa de falso
   positivo de ~60%: **53 de 86** arquivos de teste começam com
   `#![allow(clippy::expect_used)]` porque o workspace define `expect_used = "deny"`
   (`Cargo.toml:56-63`) e testes precisam de `expect` com mensagem. Uma regra que
   dispara no preâmbulo padrão da suíte ensina o agente a ignorar a regra.
   **Correção:** excluir a primeira linha de alvos de teste do scan (ou reconhecer
   exatamente esse pragma) e, no lugar, proibir `allow` em `src/`.
3. **Contabilidade manual de contagem por alvo** (§3 S4, §8 item 7): para uma mudança
   de funcionalidade o "delta explicado" é sempre escrito pelo mesmo autor que o
   produziu; o valor mecânico é apenas "nenhum alvo pode perder testes". Manter o
   mecânico, eliminar a tabela manual.
4. **O documento de evidência é apresentado como *o* PASS atual**, mas descreve
   `c3e92d8` + 199 testes (`anti-slop-refactor-evidence.md:515-560`), enquanto a árvore
   roda 1289. A regra "não edite para anexar seu resultado" impede atualizá-lo, então o
   artefato envelhece e continua sendo citado. **Correção:** separar (a) registro
   datado imutável e (b) um artefato **gerado** de status atual (script → JSON +
   Markdown, publicado pelo CI), e citar o artefato gerado para "estado atual".
5. **"Um escritor por arquivo" e bookkeeping de contrato** só têm valor em execução
   multi-agente. Custo baixo, mantenha — mas não é uma regra de qualidade de código.

### 6.3 Simplificação proposta (sem perder o que funciona)

**"Anti-slop lite" — 8 regras, todas mecanicamente verificáveis em CI:**

1. Gates verdes com exit codes registrados (`scripts/check.sh` como **fonte única**;
   docs e CI chamam o script, não recopiam comandos).
2. Baseline-first apenas quando há mudança de comportamento observável (os S1–S4), com
   `cmp` de stdout e hashes — não para docs/testes puros (hoje `AGENTS.md` exige para
   "todo change", o que incentiva o teatro).
3. Nenhum `#[allow]`/`#[ignore]`/teste apagado novo fora dos preâmbulos de teste
   reconhecidos; nenhum `allow` em `src/` sem exceção registrada.
4. Um codec/serializador/validador: contagem de definições (`fn validate_envelope`,
   `struct .*Reader`) como teste de CI, não como inspeção.
5. Erros tipados preservados nas fronteiras (revisão humana; já coberto por testes).
6. Sem alocação a partir de comprimento externo antes de validar (revisão + testes de
   overflow).
7. Nenhuma afirmação de capacidade/contagem sem link para evidência **gerada**; tabela
   canônica de capacidades com link verificável (hoje não há link check).
8. Contagem de testes por alvo: somente "não pode diminuir" (mecânico).

Isso preserva AS-01/02/03/12/15 e o catálogo de anti-padrões (que é conhecimento e deve
ficar), e corta ~40% do texto procedimental que não altera decisão.

---

## 7. Achados estruturados

Tabela de priorização (severidade = impacto; prioridade = ordem de trabalho):

| ID | Título | Severidade | Confiança | Prioridade | Fase |
|---|---|---|---|---|---|
| Q-01 | Evidência diferencial não reprodutível a partir do clone | alta | alta | **P1** | 8.5/CI |
| Q-02 | Corpus 550 instruções/186 opcodes; zero exceções/interrupções comparadas | alta | alta | **P1** | 8.2/8.5 |
| Q-03 | Controles negativos provam o comparador, não o emulador | média | alta | **P1** | 8.2 |
| Q-04 | VDP "completo": 2 programas, 18 divergências registradas sem asserção | média | alta | **P2** | 8.3 |
| Q-05 | 2 testes pulam silenciosamente sem `.research/roms/`; sem guarda de contagem | média | alta | **P2** | CI |
| Q-06 | Sem fuzzing; sem teste exaustivo de decodificação (65 536 palavras) | média | alta | **P2** | 8.2/8.5 |
| Q-07 | Docs do CI divergem do workflow (drift A9 recorrente) | baixa | alta | **P2** | docs |
| Q-08 | Lista de gates duplicada em 5+ lugares + CI "espelha" mas difere | média | alta | **P2** | docs/CI |
| Q-09 | `test_support` é API pública sem feature gate, com caminho `$HOME` hardcoded | baixa | alta | **P3** | 8.1 |
| Q-10 | Fixtures/marcadores documentados divergem do gerador (+2); texto do harness diz "ten" | baixa | alta | **P3** | docs |
| Q-11 | `proptest` sem seed/regressions; `sleep` como sincronização em testes | baixa | média | **P3** | CI |
| Q-12 | Teste de demo determinístico custa 167 s comparando run-vs-run em vez de golden | média | alta | **P2** | CI/perf |
| Q-13 | Política anti-slop: sobreposição, varredura ruidosa, evidência datada como "atual" | média | alta | **P2** | processo |

### Q-01 — Evidência diferencial não reprodutível a partir do clone
**Descrição:** rodar `python3 scripts/oracle-diff.py` exige checkout out-of-tree em
commit pinado e binários C++ compilados; sem eles o harness aborta (exit 2) e não há
skip. A verificação fixture↔gerador está dentro desse caminho inalcançável e não há
teste Rust equivalente.
**Evidência:** `oracle-diff.py:1809-1823` (aborta), `:1466-1470` (fixture só dentro de
`run_rom`), `:2366-2380` (limitações); experimento `check_fixtures.py`: 8/8 MATCH,
3 MISSING (`rom_alu`, `rom_mem_vdp`, `rom_masked`).
**Componentes:** `scripts/oracle-diff.py`, `scripts/fixtures/`, `docs/research/*`,
CI.
**Condições:** contribuidor ou CI sem `.research/upstream/exodus-headless`.
**Impacto atual:** evidência não verificável por terceiros; regressão nas fixtures passa
despercebida. **Impacto futuro:** o claim de "pinned oracle differential" vira folclore;
aumenta o risco de o corpus divergir da implementação sem sinal.
**Probabilidade:** alta (é o estado atual). **Severidade:** alta. **Confiança:** alta.
**Alternativas:** (a) não fazer nada — manter como atestado histórico declarado; (b)
commitar as 3 ROMs legadas e adicionar teste Rust que reexecuta o gerador e compara
fixtures (barato, resolve metade do problema sem o oráculo); (c) imagem de CI com o
oráculo pré-buildado e cacheado; (d) implementar um segundo oráculo nativo (Musashi/
MAME), decisão de licença e custo muito maiores.
**Recomendação:** (b) + (c). Agora: commitar as 3 ROMs + teste de fixture em Rust +
exit 3 (skip) no harness com "skip é falha" no nightly. Depois: imagem de CI.
**Custo/risco:** baixo (dezenas de linhas + 12 KiB de fixtures); risco: fixtures
"sagradas" sem gerador executado por todos — mitigado pelo teste Rust.
**Efeitos em testes/contratos:** +1 teste de fixture (barato); nenhum contrato de API
muda. **Prioridade:** P1. **Fase:** 8.5 / CI.

### Q-02 — Cobertura do differential e ausência de exceções/interrupções
**Descrição:** 550 instruções, 186 opcodes, 145 executados 1×; nenhuma exceção, nenhum
`RTE`, nenhum frame de pilha comparado ao oráculo.
**Evidência:** experimento `corpus_probe.py` (tabela §2.1); `ROMS` em
`oracle-diff.py:860-872`; builders não emitem `TRAP/RTE/LINK/CHK/STOP`.
**Componentes:** `scripts/oracle-diff.py`, `docs/research/cpu-cycle-differential-evidence.md`.
**Condições:** qualquer regressão em exceções, `RTE`, privilégio, A7.
**Impacto atual:** a classe de defeito mais provável (frames/manipulação de pilha) só tem
testes do autor. **Impacto futuro:** afirmação "68000 completo" fica exposta; um bug de
frame quebra jogos reais mais cedo do que qualquer outra coisa.
**Probabilidade:** alta. **Severidade:** alta. **Confiança:** alta.
**Alternativas:** (a) nada; (b) nova ROM de exceção (`TRAP #n` + handler + `RTE`, A7 e
frame no estado final comparado); (c) nova ROM de interrupção com máscara < 7; (d)
corpus single-step externo (licença a verificar).
**Recomendação:** (b) agora — é o maior ganho por unidade de esforço; (c) depois; (d) só
com licença verificada e `deny.toml` em vigor.
**Custo/risco:** ~1 dia (mais um builder no harness + marcador + estado final); risco
baixo (se divergir, é achado, não falha de teste). **Efeitos:** nova evidência; nenhum
contrato. **Prioridade:** P1. **Fase:** 8.2/8.5.

### Q-03 — Controle negativo de mutação ausente
**Descrição:** os controles corrompem a projeção; nenhum demonstra que uma regressão do
emulador é detectada.
**Evidência:** `oracle-diff.py:1768-1807` (`negative_control` roda o comparador sobre
projeções corrompidas), `:2240-2266` (controle de ciclo soma 1 na projeção).
**Alternativas:** (a) nada; (b) build de mutação pontual (ex.: +1 ciclo em `MOVE.W SR,<ea>`
ou flag `V` invertida em `ADD.B`) e exigir que o harness falhe; executado manualmente a
cada release ou como script de "meta-teste".
**Recomendação:** (b), como script de meta-teste documentado (`--mutation-control`), não
como gate de PR. **Custo:** ~2 h. **Efeitos:** nenhum no produto; aumenta a confiança no
aparato. **Prioridade:** P1 (barato e decisivo para a credibilidade da evidência).

### Q-04 — VDP: divergências escopadas sem asserção
**Descrição:** 18 valores (status/H/V/CRAM readback) são registrados, não asseverados;
VRAM/CRAM/registradores comparados só do lado ChronoFork.
**Evidência:** `docs/research/vdp-state-differential-evidence.md:100-127`;
`scripts/oracle-vdp-diff.py` (comparação de RAM/registradores, não de VRAM do oráculo).
**Alternativas:** (a) nada; (b) asseverar invariantes nos 18 valores (ex.: bit HBlank
alterna exatamente nos limites de linha; V counter avança 1 por linha, módulo 262; CRAM
readback = palavra escrita − autoincremento conhecido); (c) programa de DMA/FIFO/timing.
**Recomendação:** (b) + (c). **Custo:** ~1 dia. **Efeitos:** o harness passa a falhar em
regressão de contador; hoje não falha. **Prioridade:** P2.

### Q-05 — Pulos silenciosos e ausência de guarda de contagem
**Descrição:** dois testes de cartucho imprimem "skipping" e retornam sucesso.
**Evidência:** `crates/chronofork-megadrive/tests/cartridge.rs:151-157,170-177`; ausência
de equivalente a `TreatNoTestsAsError` no CI; `docs/quality/testing-strategy.md:38-39`
(princípio 6 "No silent skips").
**Alternativas:** (a) nada; (b) `#[ignore = "requires local .research/roms fixture"]` +
job opcional com `--ignored` e env `CHRONOFORK_ROM_DIR` — o relatório passa a mostrar
"ignored" em vez de "passed"; (c) falhar quando `CI=true` e a fixture faltar.
**Recomendação:** (b) + guarda de contagem de testes por alvo no CI. Custo ~1 h.
**Prioridade:** P2.

### Q-06 — Sem fuzzing e sem varredura exaustiva de decodificação
**Descrição:** `fuzz/` não existe; nenhum teste varre as 65 536 palavras de opcode.
**Evidência:** `ls fuzz` → ausente; `grep cargo-fuzz docs` → só a estratégia.
**Alternativas:** (a) nada; (b) teste de varredura (todas as palavras → `classify` ou
`Other`; `M68000::step` com ROM sintética nunca panic e devolve erro tipado) — minutos;
(c) `cargo fuzz` em `Snapshot::from_bytes`/`Vdp::restore_bytes`, nightly.
**Recomendação:** (b) no PR; (c) no nightly. **Custo:** (b) ~2 h, (c) ~meio dia.
**Prioridade:** P2. **Efeitos:** pode revelar lacunas de decode imediatamente.

### Q-07 — Drift entre `quality-gates.md` e `ci.yml`
**Descrição:** doc afirma gatilho em todas as branches e ausência de cancelamento;
o workflow diz o contrário.
**Evidência:** `docs/quality/quality-gates.md:1075-1076,1080,1092-1093` vs
`.github/workflows/ci.yml:13-21,77-134`.
**Recomendação:** gerar a seção de CI a partir do workflow (script simples) ou reduzir a
prosa a um link. **Custo:** ~1 h. **Prioridade:** P2 (é o anti-padrão A9 da própria
política).

### Q-08 — Duplicação da lista de comandos de gate
**Descrição:** a mesma lista aparece em `scripts/check.sh`, `AGENTS.md`,
`quality-gates.md`, `testing-strategy.md §8`, `anti-slop-policy.md §6.1` e 7 docs de
aceitação; já divergiu (CI usa `--locked` e matriz por grupo; `check.sh` usa
`cargo check --workspace` sem `--all-targets`).
**Evidência:** `grep -l 'cargo clippy --workspace --all-targets'` → 16 arquivos.
**Alternativas:** (a) nada; (b) `scripts/check.sh` como fonte única e CI chamando o
script (com `--locked` como variável de ambiente); docs apenas linkam.
**Recomendação:** (b). **Custo:** ~2 h. **Prioridade:** P2.

### Q-09 — `test_support` público e caminho `$HOME` hardcoded
**Descrição:** módulo público sem feature gate, incluindo um localizador que embute
`$HOME/projects/ChronoFork/.research/roms` no crate publicado.
**Evidência:** `crates/chronofork-megadrive/src/lib.rs:77-149`.
**Recomendação:** `#[cfg(any(test, feature = "test-support"))]`, e remover o candidato
`$HOME/projects/ChronoFork` (usar só `CHRONOFORK_ROM_DIR` e caminho relativo ao
manifest). **Custo:** ~1 h. **Prioridade:** P3.

### Q-10 — Drift de fixtures/marcadores e texto do harness
**Descrição:** marcadores documentados `0x352/0x2E0/0x2EE` vs gerados
`0x354/0x2E2/0x2F0`; o harness escreve "ten" ROMs com 11 no corpus.
**Evidência:** experimento `check_fixtures.py`; `oracle-diff.py:2374`.
**Recomendação:** gerar as tabelas de evidência do JSON e adicionar assert de contagem do
corpus. **Custo:** ~2 h. **Prioridade:** P3.

### Q-11 — `proptest` sem seed/regressions; `sleep` como sincronização
**Evidência:** nenhum `proptest-regressions/`; `service.rs:1047` (50 ms),
`http.rs:1332` (20 ms); `m6.rs:237-247` (polling com deadline, padrão melhor).
**Recomendação:** commitar regressions, registrar seed no log, trocar sleeps por gates
onde já existe `Gate`. **Custo:** ~2 h. **Prioridade:** P3.

### Q-12 — O teste determinístico de 167 s mede a coisa errada
**Descrição:** `megadrive_demo_output_is_deterministic` roda o demo 2× em debug e compara
run-vs-run (167 s); não há golden.
**Evidência:** `crates/chronofork-cli/tests/demo.rs:133-147`; medição: debug 83,6 s vs
release 7,7 s; stdout idêntico entre perfis
(`sha256 fe254e5d193de273bc0d0db1b9b426ef6b069f4f96685486cb7303662d56a420`).
**Alternativas:** (a) nada; (b) fixar golden (padrão já usado no `demo` sintético,
`demo.rs:61-64`) e rodar 1× em release; (c) manter 2× debug.
**Recomendação:** (b) — mesmo poder de detecção, 8 s em vez de 167 s, e ganha cobertura
cross-profile real via job `release`. **Custo:** ~30 min. **Prioridade:** P2.

### Q-13 — Crítica à política anti-slop (detalhe na §6)
**Recomendação:** aplicar as 8 regras da §6.3; corrigir a varredura de `allow`; separar
evidência datada de status atual gerado; reduzir a lista de comandos a `scripts/check.sh`.
**Prioridade:** P2 (processo), sem impacto direto em runtime.

---

## 8. Limitações desta revisão

- **Não reexecutei o diferencial com Exodus.** O oráculo não está neste worktree
  (`.research/oracle/build/` é do checkout pai, fora do meu escopo de escrita e o build
  C++ não está disponível aqui). Tudo que afirmo sobre o harness vem de leitura do código
  e de experimento com o binário local; os números de cobertura (§2.1) são medições
  minhas, não do harness.
- **Não reexecutei a suíte completa** (1289) para poupar ~6 min de máquina compartilhada;
  verifiquei a contagem por `--list` e executei os gates baratos. O baseline já registrou
  1289/0.
- **Não avaliei desempenho** (área de outro revisor); uso os tempos apenas como insumo de
  custo de CI.
- **Não verifiquei Windows/macOS em runtime** (nenhum runner aqui). O job `portability` é
  só `cargo check`.
- **Não julguei licenças** de eventuais corpora externos; qualquer adoção precisa de
  `deny.toml` + revisão de proveniência antes.
- A recomendação de corpus single-step público para 680x0 é **hipótese** minha sobre
  existência/utilidade; a licença não foi verificada e não pode ser assumida.

## 9. Perguntas cruzadas para o capitão / crítico

1. **Produto:** o dono aceita que "68000 completo" signifique "conjunto documentado,
   avaliado em 550 instruções sintéticas" — ou exige cobertura de exceções antes de
   marcar 8.2 como concluída? (Isso muda o DoD de 8.5/8.6.)
2. **Custo de CI:** há orçamento para um runner Windows de testes (não só `check`)? Sem
   isso, a afirmação de determinismo cross-platform fica limitada a "compila".
3. **Oráculo:** autoriza o capitão a investir numa imagem com o oráculo pré-buildado
   (ou a commitar as 3 ROMs legadas + teste de fixture) como parte desta revisão?
4. **Política:** posso propor a "anti-slop lite" da §6.3 como patch de documentação ao
   dono? Envolve `AGENTS.md`, `docs/quality/` e o workflow — fora do meu escopo de
   escrita aqui.
