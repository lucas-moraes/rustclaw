# Roadmap — Melhorias para equiparar o RustClaw a Claude Code / Codex

> Documento de trabalho. Cada item traz **estado atual**, **lacuna**, **proposta**
> e **esforço estimado**. Ordenado por retorno sobre esforço (ROI), não por
> dificuldade.

## Contexto

O RustClaw já implementa o núcleo de um coding agent moderno:

- Loop agêntico com native tool calling (streaming + execução paralela via `JoinSet`)
- 18 tools (read/write/edit/glob/grep/ast_search/bash/git/diagnostics/task/...)
- Subagents (`task`) com 5 perfis: build, plan, explore, general, chat_free
- MCP client (stdio/HTTP, health-check, reconnect)
- Compactação de contexto, detecção de doom loop, hooks pre/post-tool
- Permissões allow/ask/deny com guard de path fora do workspace
- Prompt caching, retry com backoff, budget/custo por token
- TUI própria (ratatui) + CLI fallback
- Memória de projeto persistente (SQLite, scoring recência+uso+BM25)
- Multi-provider (Anthropic / OpenAI / opencode-go / custom)

**Métricas atuais:** ~42.800 linhas, 128 arquivos `.rs`, 635 testes, CI + release.

As lacunas abaixo são o que separa o projeto de "equivalente" — não de "bom".

## Status de execução

| # | Item | Estado |
|---|------|--------|
| 1 | Subagents aninhados | ✅ concluído |
| 2 | Compactação proativa / incremental | ✅ concluído (2a trigger 70%, 2b ledger durável, 2c cadeia de resumos) |
| 3 | Evals / observabilidade | ✅ concluído (métricas por sessão + `/stats` + suite de evals offline + gate no CI) |
| 4 | Busca semântica | ✅ concluído (chunking por símbolo + índice SQLite/FTS5 + busca híbrida + tool `semantic_search` + `/index`) |
| 5 | Sandbox de execução | ✅ concluído (Landlock via `pre_exec` no Linux + `sandbox = "off"\|"landlock"` em `rustclaw.json`; no-op em outros SOs) |
| 6 | Render JS | ✅ concluído (headless Chrome via `chromiumoxide` no binário default + `render: auto\|http\|browser` + `rustclaw doctor`) |
| 7 | Provider/ecossistema | ✅ testes de contrato implementados (C1–C8); manutenção contínua |

> Nota: a numeração desta tabela segue a **ordem de execução** (ver seção
> "Ordem sugerida de execução"). As seções P0/P1/P2 abaixo usam numeração
> própria por prioridade — os títulos são a referência canônica.

---

## P0 — Alto impacto, esforço baixo/médio

### 1. Subagents aninhados (composição)

**Estado atual:** `TaskTool` (`src/harness/tool/task.rs`) spawna subagents via
`SubagentRunner::run_task(agent, prompt, events)`. O `ToolContext` não carrega
nenhuma noção de profundidade, e o `TaskRunner` (`src/harness/runtime/task_runner.rs`)
não limita recursão. Um subagent **não pode** spawnar outro.

**Lacuna:** Claude Code permite composição de subagents (dividir-e-conquistar
real). Aqui é um nível só — tarefas grandes não podem ser decompostas
recursivamente pelo próprio agente.

**Proposta:**
- Adicionar `depth: usize` ao `ToolContext` (default `0`).
- Propagar `depth + 1` ao construir o contexto do subagent no `TaskRunner`.
- Definir `MAX_SUBAGENT_DEPTH` (sugestão: `3`) e fazer o `TaskTool` retornar erro
  claro quando `depth >= MAX_SUBAGENT_DEPTH`.
- Expor a profundidade no painel de subagents da TUI (`ui/tui/subagent.rs`) para
  o usuário ver a árvore.

**Esforço:** ~1 dia. **Risco:** baixo (mudança aditiva).

---

### 2. Compactação proativa / incremental

**Estado atual:** `compaction.rs` é reativa — `should_compact_and_execute` só
dispara quando `approx_tokens(messages) > max_context_tokens`. Ao compactar,
descarta tudo antes de `keep_recent_messages` e substitui por **um** resumo.

**Lacuna:** em sessões longas, perde-se granularidade de uma vez. Claude Code
gerencia contexto continuamente (resumos incrementais, contexto em camadas),
evitando o "degrau" de qualidade após cada compactação.

**Proposta:**
- Compactação **incremental**: em vez de resumir `[0..cut]` de uma vez, resumir
  em blocos e manter uma cadeia de resumos (resumo do resumo), preservando
  decisões-chave e caminhos de arquivo tocados.
- Disparo **proativo**: compactar em ~70% do budget (não em 100%), para nunca
  chegar ao limite no meio de um turno.
- Preservar um "ledger" estruturado (arquivos editados, comandos rodados,
  decisões) que sobrevive à compactação — hoje isso se perde no resumo textual.

**Esforço:** ~3–5 dias. **Risco:** médio (afeta qualidade de todas as sessões
longas; exige testes de regressão com sessões sintéticas).

---

### 3. Sandbox de execução para `bash`

**Estado atual:** ✅ **Implementado** — `src/harness/tool/sandbox.rs` aplica
Landlock no processo filho via `pre_exec` (antes do `exec`), configurável com
`sandbox = "off" | "landlock"` em `rustclaw.json`. Regras: leitura de `/usr`,
`/bin`, `/etc`, `/dev` e toolchains do `PATH`; leitura+escrita do cwd e `/tmp`.
Best-effort: kernels sem Landlock rodam sem restrição (status reportado).
macOS/Windows: no-op.

**Lacuna:** é a diferença de **segurança** mais séria. Codex e Claude Code
oferecem execução isolada (container/seccomp/landlock). Sem isso, não dá para
deixar o agente rodando sem supervisão.

**Proposta (Linux primeiro):**
- `landlock` (restrição de filesystem) + `seccomp` (syscalls) via crates
  `landlock` e `seccompiler`, aplicados no processo filho antes do `exec`.
- Modo configurável: `sandbox = "off" | "landlock" | "container"`.
- Fallback gracioso: se o kernel não suportar, avisar e cair para `off` com
  permissão explícita.
- macOS: `sandbox-exec` (deprecated mas funcional) ou documentar como não
  suportado.

**Esforço:** ~1–2 semanas. **Risco:** alto (específico de plataforma; precisa de
testes em kernels diferentes).

---

## P1 — Alto impacto, esforço médio/alto

### 4. Busca semântica de código / indexação

**Estado atual:** ✅ **implementado**. Módulo `src/harness/index/`:
- `chunk.rs` — chunking por símbolo (reusa o parser Tree-Sitter do `ast_search`)
  com fallback por linhas para arquivos não-`.rs`.
- `embed.rs` — trait `Embedder` + `ApiEmbedder` (endpoint OpenAI-compatível
  configurável) + `NullEmbedder` (degradação graciosa para BM25-only).
- `store.rs` — índice SQLite por projeto (tabela `project_<hash>_chunks` + FTS5
  BM25 + coluna `embedding` BLOB), indexação incremental por hash SHA-256.
- `search.rs` — busca híbrida (BM25 + cosseno, pesos 0.5/0.5).
- `indexer.rs` — walk do repo (ignora `target`/`.git`/`node_modules`, limite de
  tamanho/arquivos) + reindexação incremental.
- Tool `semantic_search(query, k)` (read-only) e comando `/index [status|search]`.

**Decisão de arquitetura:** embeddings **somente via API** (provider
configurável em `rustclaw.json` → `embeddings`, ou env `RUSTCLAW_EMBED_*`).
Evitou-se `fastembed-rs`/`candle` (deps pesadas, cold start, ~150 MB). Sem
backend configurado, a busca continua funcionando em BM25 puro — nunca quebra.

**Verificação:** 28 testes do módulo `index` + 4 da tool + 5 do comando; smoke
test `#[ignore]` indexa o próprio repo (142 arquivos → 2867 chunks) e acha
`doom_loop` por consulta em linguagem natural.

**Esforço:** ~1–2 semanas. **Risco:** médio (custo de embeddings, tamanho do
índice, cold start).

---

### 5. Renderização web com JavaScript

**Estado atual:** ✅ **implementado**. `fetch_webpage` aceita
`render: "auto" | "http" | "browser"`:
- `http` (padrão) — HTTP puro + `htmd`, como antes.
- `browser` — renderiza com headless Chrome via `chromiumoxide` (executa JS),
  espera o DOM assentar e converte o HTML renderizado.
- `auto` — tenta HTTP e, se o Markdown vier vazio (heurística `looks_empty`),
  refaz com o browser; se o Chrome faltar, mantém o resultado HTTP.

**Decisão de arquitetura:** `chromiumoxide` é **dependência do binário default**
(recurso de primeira classe, não feature flag). Custo aceito: ~150 MB no binário
e dependência de um Chrome/Chromium instalado na máquina do usuário.

**Verificação de dependências do SO:** módulo `harness/deps.rs` sonda a presença
de Chrome/Chromium (caminhos absolutos conhecidos de macOS/Linux **primeiro**,
depois o PATH) e valida que o candidato é **executável** — evita falsos positivos
como o shim quebrado do Homebrew. Exposto como `rustclaw doctor` (subcomando,
sai != 0 se faltar algo) e `/doctor` (slash command). No boot da TUI,
dependências ausentes geram aviso não-fatal — o harness roda e a feature degrada
para o fallback (`render: "http"`).

**Verificação:** 3 testes do módulo `browser` (heurística de vazio) + 2 novos da
tool (`render` inválido, enum no schema) + 8 do `deps`; smoke test `#[ignore]`
renderiza `example.com` em headless Chrome de verdade.

**Esforço:** ~3–5 dias. **Risco:** médio (distribuição).

---

### 6. Observabilidade e evals

**Estado atual:** `tracing` + `/record` + `/replay`. Não há medição de qualidade
ao longo do tempo.

**Lacuna:** sem evals, não dá para saber se uma mudança no prompt/loop melhorou
ou piorou o agente. Claude Code/Codex têm pipelines internos de avaliação.

**Proposta:**
- Suite de **evals** reproduzíveis: tarefas sintéticas (ex.: "corrija este bug",
  "adicione esta feature") com critérios de sucesso automáticos.
- Métricas por sessão: turnos até conclusão, tools chamadas, tokens, custo,
  taxa de sucesso, retrabalho.
- Exportar telemetria local (SQLite) + comando `/stats`.
- Rodar evals no CI (subset rápido) para detectar regressões de comportamento.

**Esforço:** ~1 semana. **Risco:** baixo.

**Implementado:**
- `session/metrics.rs`: `SessionMetrics` (turnos, iterações, tools por nome,
  erros, tokens in/out/cache, custo, wall time, compactações, subagents) +
  `ProjectMetrics`; persistido em `metrics_json` (SQLite).
- `/stats`: relatório da sessão + agregado do projeto.
- `provider/scripted.rs`: `ScriptedProvider` — replay determinístico de turnos
  (texto + tool calls) para exercitar o loop inteiro sem rede.
- `harness/eval.rs`: suite de evals offline com critérios automáticos
  (`plain_answer_no_tools`, `single_tool_then_answer`, `parallel_tool_calls`,
  `metrics_recorded`, `tool_error_is_surfaced`).
- `rustclaw evals`: roda a suite e sai com código != 0 em falha; passo dedicado
  no CI (`cargo run --quiet -- evals`).

---

## P2 — Polimento e ecossistema

### 7. Maturidade de provider

**Estado atual:** ✅ **Testes de contrato implementados** (2026-09-14,
`src/harness/provider/contract_tests.rs`, via `wiremock`): C1 paridade de
eventos de texto, C2 paridade de tool calling (id/name/args idênticos entre
adaptadores), C3 usage no `End`, C4 vocabulário de truncamento normalizado
(`max_tokens`/`length`), C5 shape do request (system + JSON Schema na wire),
C6 mapeamento de erro (429 → retryable), C7 prompt caching (breakpoints na
wire + cache usage parseado), C8 roteamento do opencode-go por modelo.
Manutenção contínua: novos adaptadores devem passar na mesma suíte.

### 8. Integrações de ecossistema

Marketplace de plugins, extensão de IDE (VS Code/JetBrains), GitHub Action
nativo, apps web/mobile. **Não competir** — é ecossistema de empresa, não de
harness solo. Priorizar apenas se houver demanda real de usuários.

### 9. Qualidade de código / dívida

**Estado atual:** ✅ **Implementado** (2026-09-14).

- Cobertura de testes por módulo: auditada (ui/ 13.9k linhas / 126 testes,
  session/ 7.7k / 130, tool/ 7.4k / 122, provider/ 3.7k / 70 — proporção
  consistente). Gaps de maior risco fechados: `palette.rs` (0 → 9 testes:
  filtro, wrap, autocomplete, agentes custom) e scroll/clamp do transcript
  (`app/tests.rs`, +5 testes). Módulos de draw (render puro de widgets) e
  handlers de tecla (I/O de terminal) permanecem sem testes unitários por
  design — a lógica testável deles vive em `state.rs`/`input.rs`/`palette.rs`,
  que têm cobertura.
- `cargo clippy -- -D warnings` já no CI ✅.
- Invariantes do loop agêntico documentados em `docs/ARCHITECTURE.md`
  (seção "Invariantes do loop agêntico", I1–I12: orçamento de iterações,
  deadline, compactação, persistência, ledger, tool results, abort,
  doom loop, continuação, eventos, transcript).

---

## Ordem sugerida de execução

| # | Item | Esforço | Impacto | Ordem |
|---|------|---------|---------|-------|
| 1 | Subagents aninhados | 1 dia | Alto | **1º** |
| 2 | Compactação proativa | 3–5 dias | Alto | **2º** |
| 3 | Evals / observabilidade | 1 semana | Médio | **3º** |
| 4 | Busca semântica | 1–2 semanas | Muito alto | **4º** |
| 5 | Sandbox de execução | 1–2 semanas | Alto (segurança) | **5º** |
| 6 | Render JS | 3–5 dias | Médio | 6º |
| 7 | Provider/ecossistema | contínuo | Baixo | 7º |

**Racional:** começar pelo item 1 (barato, ganho imediato), depois 2 e 3 (que
melhoram a qualidade de tudo que vem depois), e só então investir nas duas
grandes apostas (4 e 5). Evals antes de busca semântica porque sem medição não
dá para validar se a indexação realmente ajudou.

---

## O que NÃO fazer

- **Não** embarcar runtime Python (ex.: Crawl4AI) — incompatível com binário
  único e sobreposto ao `fetch_webpage` existente.
- **Não** perseguir paridade de ecossistema (IDE/mobile/marketplace) — custo
  desproporcional para um harness solo.
- **Sim** embarcar `chromiumoxide` (headless Chrome) no binário default — decisão
  revista: o render JS é recurso de primeira classe. Aceita-se o custo de
  ~150 MB e a dependência de Chrome instalado.
