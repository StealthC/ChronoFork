# Crítica final independente — ChronoFork

**Veredito: precisa de revisão, não rejeitar.** O diagnóstico estrutural é sólido; a síntese ainda contém erros quantitativos e recomendações cujo alcance excede a evidência. Corrigir os itens C1–C7 antes de apresentar decisões ao proprietário. Nenhuma reescrita de arquitetura é justificada.

**Escopo e método:** baseline `b8d63a1`, worktree de revisão, 2026-11-20. Li o [baseline](baseline.md), os sete [relatórios de área](areas/01-rust-systems.md) ([A2](areas/02-emulator-engineering.md), [A3](areas/03-performance.md), [A4](areas/04-concurrency.md), [A5](areas/05-determinism-quality.md), [A6](areas/06-interfaces-security.md), [A7](areas/07-product-planning.md)) e a [síntese](SYNTHESIS-DRAFT.md). Fiz exatamente cinco spot-checks abaixo, sem reexecutar benchmarks nem suíte, sem acessar o trabalho 8.4 para investigação e sem modificar produção ou relatórios. Confiança alta para correções aritméticas/código observado; média para prioridades e projeções.

## 1. Correções obrigatórias

### C1 — Decomposição correta, mas números e denominadores ainda misturados (P1)

**Experimento documental + cálculo independente, alta confiança.** A divergência obrigatória não é uma contradição física:

| Medida | Numerador / denominador correto | Resultado |
|---|---|---:|
| A2-F4 | SHA puro 84,7 µs / passo completo 224 µs | 37,81% |
| A3, tabela §3 | SHA puro 86,9 µs / passo completo 209,8 µs | 41,42% |
| A3, verificação isolada | corpo + SHA = 91,0 µs / (91,0 + emulação 0,173 µs) | 99,81% |
| A3, caminho completo estimado | (91,0 + churn 116,6) / 209,8 | 98,95% |

Portanto **~40% é apenas SHA; ~99% é o caminho completo**, incluindo alocações/page faults. O “99,9% do passo” de A3 é impreciso: exclui churn/residual do denominador e, com os valores impressos, arredonda para 99,8%. Tampouco é literalmente “verificação/emulação”: essa razão seria ~526×; a porcentagem divide por **verificação + emulação**. O ~502× publicado não se reproduz com 91,0/0,173; usar valores consistentes ou apenas ordem de grandeza.

A [síntese:103–119](SYNTHESIS-DRAFT.md#L103-L119) acerta o conceito, mas chama 91 µs de SHA puro, embora A3 inclua 4,02 µs de corpo nesses 91. Também soma **91 + 116,6 + 9 = 216,6 µs**, ou **103,24%** do passo de 209,8. Avisar “parcelas com sobreposição” não transforma isso em decomposição quantitativa válida. Substituir pela tabela interna de A3: **(a) SHA 86,9; (b) corpo/cópias ~4,02; (c) churn ~116,6; (d) emulação ~0,173; residual ~2 µs**. Não somar medianas de harnesses diferentes como parcelas disjuntas. O churn é uma diferença experimental com thresholds glibc, não perfil exclusivo de page faults nem prova de ganho idêntico em buffers de produção.

**Erro de fator dez:** [síntese:125–129](SYNTHESIS-DRAFT.md#L125-L129): 9 µs/passo = **111 mil passos/s**, ~**10,8 fps** a 10.256 passos/frame; não 10–11 mil/~1 fps. Remover só SHA do baseline deixa, por subtração ilustrativa, 209,8 − 86,9 = **122,9 µs**, ~8,1 mil passos/s/~0,79 fps, pois churn continua. Com alocador estabilizado, 93,2 − 86,9 = ~6,3 µs (~159 mil/s), ainda longe de 60 fps nesse workload. Essas subtrações não são medições de variantes.

**Correção exigida:** manter quatro componentes separados e três intervenções distintas: trocar algoritmo SHA; evitar serialização/alocações; deixar de solicitar o caminho inteiro em passos que não precisam de verificação. O patch experimental faz a terceira: retorna antes de construir o corpo. Seu ganho não pode ser atribuído à primeira.

### C2 — Eficiência de concorrência calculada incorretamente (P1)

**Cálculo independente, alta confiança.** A [tabela A4:207–214](areas/04-concurrency.md#L207-L214) define eficiência como throughput(N)/(N × throughput(1)). Seus próprios throughputs dão **1,00 / 0,446 / 0,227 / 0,114**, não 1,00/0,13/0,07/0,03, copiados na síntese. Corrigir todos os resumos finais. O diagnóstico permanece: throughput agregado quase constante e CPU/wall ~1 demonstram serialização. Não é necessário exagerar o número para justificar lock por sessão.

### C3 — Relatar falha de boot com precisão, sem trocar uma caricatura por outra (P1)

**Spot-check de evidência bruta, alta confiança.** O [log E4:37–49](areas/02-evidence/E4-rom-boot.txt#L37-L49) registra Kid falhando no **primeiro passo solicitado**, após zero instruções completadas, ao validar o intervalo da moldura de exceção. Não encontrei suporte para afirmar que Kid executou várias instruções antes do push: não adotar essa correção sem novo traço. Chakan transfere controle ao handler fatal na tentativa inicial de `TST.L` e continua executando o laço do handler; não deixa de executar instruções.

Redação segura: **“Os dois fixtures não progridem no bootstrap: Chakan cai no handler fatal no primeiro passo; Kid aborta o primeiro passo ao preparar a moldura de exceção.”** O endereço reportado é o fim do intervalo validado, **não evidência de escrita parcial**. Isso não muda as correções de bus/I/O. Também não prova validade externa do dump: A2 refuta a alegação de SSP implausível, não autentica todos os bytes da ROM. Substituir “ROM válida” por “SSP legítimo; integridade externa não verificada”.

### C4 — Roadmap ambíguo e gates fracos (P1)

**Julgamento, confiança média.** D1 recomenda 8.5→8.4, mas sua mitigação manda começar pelo Z80/bus de 8.4. A revisão só avaliou o baseline; não sabe o custo restante nem o estado verificado do trabalho em andamento. Não fundamenta interrupção/descarte de 8.4, tampouco o prazo “uma semana”.

**Recomendação:** corrigir os bloqueadores locais e instalar gates agora; preservar o trabalho existente. Antes de inverter fases inteiras, o responsável por 8.4 informa o que está pronto e o próximo bloqueio real. A decisão do proprietário é **adiar áudio/realocar esforço**, não escolher o próximo registrador. Remover a previsão universal “qualquer jogo morre no init” e marcar o próximo obstáculo como hipótese.

G-BOOT-0 é só ausência de erro: um `BRA *` passa. G-BOOT-1 pode passar em outro laço e exigir que todos os registros VDP 00–07 sejam não zero rejeita inicializações legítimas. Usar marcos observáveis específicos por fixture (trace/PCs de inicialização, transições esperadas de VDP e avanço além do handler), mais um sintético redistribuível obrigatório. Comercial ausente deve ser **não executado**, nunca aprovação. Frame com hash e áudio não silencioso também não provam gameplay nem precisão.

Resolver ainda a duplicação D2 “agora” versus 8.6b “hash fora do step”; D3 “8.6c” versus D4 “agora” para o mesmo lock/cancelamento; e UC-5, que depende de modos/captura 8.7 e não pode bloquear uma consolidação anterior à sua implementação.

## 2. Recomendações que precisam de limites

### C5 — Ganho sintético não é capacidade futura demonstrada (P1)

Os **319 fps** são aritmética válida: 10.256 × 0,173 µs + 1,267 ms de render + 0,091 ms de hash ≈ 3,132 ms/frame. **Não são frame de jogo corrigido**, muito menos previsão com Z80/áudio/scanline/contenda. Retirar “suficiente com folga” e distinguir projeção de meta. O 0,84 fps “ROM real” de A7 mede Chakan preso no handler: não é perfil representativo do jogo.

Preservar o alvo de orçamento por frame, mas não fazer **≥5 M passos/s** gate universal: passos têm custos e ciclos diferentes; o host é compartilhado. Medir workloads pinados, regressão relativa controlada, frames/ciclos por segundo e memória, antes/depois, com hashes iguais nas fronteiras. Gate absoluto em CI genérico requer runner calibrado.

**Otimização prematura:** render opt-in não é pré-requisito para 60 fps na projeção apresentada, que **já inclui render por frame**. 1,267 ms é ~7,6% do orçamento de 16,67 ms. Separar render descartado é razoável para headless; não condicionar a correção principal à introdução de política nova. Hoje renderiza clone congelado na fronteira; lazy deve preservar esse estado, não renderizar o estado futuro no `take`. Com raster futuro, mudar política de observação nunca pode mudar latches, frames, áudio ou estado canônico. Hash incremental, CoW, dedup, driver universal e fachada comum continuam adiados; ausência de duplicação sintática não basta para justificar essas abstrações.

### C6 — API mínima não fecha sozinha a jornada nem o resultado desconhecido (P1)

O diagnóstico M6-FU1 é forte. Contudo “produto agêntico não existe” apaga o valor real de CLI/script/experiment: dizer **“jornada interativa de RE sobre MCP incompleta”**. Os 5,2 milhões de tokens são extrapolação bytes/4 de um padrão deliberadamente granular, não consumo medido de LLM nem limite inferior de análise.

A lista de “9 ops” agrupa vários verbos e não contém a persistência/restauração durável de **sessão viva ramificada**, bloqueio central de A7. Incluir esse caso na aceitação de consolidação antes de congelar nomes/quantidade. `frame.capture` com teto 96 KiB base64 não comporta RGBA bruto 320×224 (286.720 B antes de base64); definir formato/compressão/paginação e limite real, sem promessa inviável.

Lock por sessão resolve serialização entre máquinas, **não** cancelamento, fairness, quota ou pool esgotado. `list` que inspeciona cada sessão ainda pode esperar uma delas: aceitar metadados baratos/paginação ou declarar semântica, não prometer simplesmente “nenhuma operação bloqueia list”. Deadline/cancelamento não elimina *unknown outcome*: a resposta pode perder-se após mutação. Definir prefixo aplicado (`performed`, tick/hash final), identificador consultável ou precondição de tick para retry seguro; não exigir exatamente-once sem protocolo. Tempo de host escolhe onde parar, não a evolução interna: reproduzir exige registrar **k passos efetivamente aplicados e inputs**, não repetir o deadline e esperar o mesmo k.

Segurança: concordo com Origin P2/P3 sob Bearer + loopback, sem exploração autenticada demonstrada. Isso não rebaixa slowloris **pré-auth**, cuja indisponibilidade foi demonstrada. “Sem exploração demonstrada” não pode apagar esse experimento. Não expor script antes de deadline dentro dos builtins; manter esse gate explícito mesmo se a API inicial não incluir script.

### C7 — Decisões do proprietário devem ser menores e reais (P2)

D3/D4 duplicam decisão; D5 mistura governança, CI, áudio e licença. Separar:

- **Proprietário:** público e escopo v1 (MCP interativo, usuário único), prioridade de consolidação versus janela/M9, orçamento de qualidade/compatibilidade, tolerância a quebra pública de API e política de incorporação/licença de áudio. Apresentar consequências e recomendação.
- **Equipe:** `Mutex` por sessão, scratch/streaming, enum de política, tamanho inicial de batches, constante PC, número/nome de tools e organização de scripts. Documentar decisões técnicas e pedir aprovação de ADR quando exigido; não terceirizar a investigação especializada.

`ymfm` é **candidato**, não adoção pronta. A2 não fixa commit e sua licença foi citada por consumidores; a síntese reconhece faltar avaliação de build/precisão/estado. Adoção exige licença primária por componente, notices, estratégia de snapshot determinístico, portabilidade e compatibilidade com a política de unsafe/FFI/dependências. LGPL não é automaticamente proibida por existir um núcleo MIT; distribuição/relink têm obrigações que precisam de análise. Processo separado também não absolve licença. Não recomendar pacote fechado “ymfm + governança” ao proprietário.

## 3. O que manter e como aceitar a revisão

**Manter:** Rust próprio e crates atuais; backend opaco/versionado; estado mutável exclusivo por máquina; separação core/serviço/transporte/apresentação; emulação sem relógio de host; hash canônico disponível e formatos preservados na otimização; verificação por passo onde TAS/replay a exigirem; rejeição explícita de DMA incorreto até implementação; gates de valor e diferencial reproduzível; nenhuma migração genérica para async/atores/core externo. Isso preserva a identidade de sessões determinísticas, forkáveis e inspecionáveis.

“Determinismo” não depende de calcular hash: hash detecta divergência, não impede. Mover a verificação para fronteiras reduz resolução diagnóstica; preservar log de inputs e replay para localizar a primeira divergência. Melhorar bootstrap/hardware **muda** comportamento canônico: separar esses patches de otimizações behavior-preserving; não exigir antigos goldens byte-idênticos após corrigir semântica. Versionar/regenerar evidência justificadamente.

A5 sustenta cobertura diferencial limitada, **não zero cobertura total de exceções**. Corrigir essa generalização em S-06. Não substituir testes entre execuções apenas por golden: regressão e não determinismo são propriedades diferentes. “Anti-slop lite” deve remover duplicação sem instituir contagens monotônicas de testes ou limites rígidos de dez linhas como proxies de qualidade; documentação de handoff precisa conter riscos suficientes.

**Aceitação:** corrigir contas/denominadores e redações factuais; explicitar projeções; reconciliar sequência sem descartar 8.4; trazer persistência de sessão e resultado desconhecido aos gates; separar escolhas estratégicas de implementação. Essas revisões bastam: não há evidência que peça recomeçar o projeto.

## 4. Registro dos cinco spot-checks independentes

1. **Ablação de performance:** [patch:3–12](benchmarks/patches/review-instrumentation.patch#L3-L12): `REVIEW_HASH=0` retorna antes de serializar; mede remoção do caminho inteiro, não só SHA.
2. **Aritmética executada:** Python 3, exit 0, recalculou razões, soma 216,6/209,8, recíprocos de 9/122,9/6,3 µs, eficiências A4 e frame projetado; resultados registrados em C1/C2/C5. Sem nova medição de desempenho.
3. **Boot:** [E4:37–81](areas/02-evidence/E4-rom-boot.txt#L37-L81): Kid falha no primeiro passo, Chakan permanece no handler. Não reexecutei ROMs nem validei dumps externamente.
4. **Render:** [backend:664–736](../../crates/chronofork-megadrive/src/backend.rs#L664-L736): captura automática na fronteira e composição em clone; lazy precisa preservar o boundary.
5. **Serviço:** [service:394–491](../../crates/chronofork-service/src/service.rs#L394-L491): `list` inspeciona cada sessão; `run` segura manager durante loop e inspect final. Alterar granularidade do lock não define sozinho leitura não bloqueante nem cancelamento.

**Limites:** sem nova execução do diferencial/oráculo, sem hardware real, sem medição pós-correção e sem exame do progresso privado de 8.4. Lacunas registradas, não preenchidas por suposição. Somente este parecer foi criado; nada implementado, commitado, publicado ou integrado.
