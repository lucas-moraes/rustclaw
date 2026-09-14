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

**Métricas atuais:** ~37.600 linhas, 113 arquivos `.rs`, 539 testes, CI + release.

As lacunas abaixo são o que separa o projeto de "equivalente" — não de "bom".

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

**Estado atual:** `BashTool` executa direto no host. Há permissões
(allow/ask/deny) e guard de path, mas **nenhum isolamento de processo**.

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

**Estado atual:** **não existe**. Zero embeddings, zero índice vetorial. A busca
é `grep` (regex), `glob` (nomes) e `ast_search` (Tree-Sitter, sintática).

**Lacuna:** a maior lacuna funcional. Em repo grande, o agente gasta muitos
turnos "procurando" porque não acha código por *significado*. Claude Code e
Codex indexam o repositório.

**Proposta:**
- Índice local persistente (por projeto, em SQLite — já é dependência):
  - **Chunking** por símbolo (reusar o parser do `ast_search`) em vez de por linha.
  - **Embeddings** via provider configurável (API) ou modelo local leve
    (`fastembed-rs` / `candle`) para não depender de rede.
  - **Busca híbrida**: BM25 (já há scoring BM25 na memória de projeto) + vetorial.
- Nova tool `semantic_search(query, k)` retornando trechos com path:linha.
- Indexação incremental: reindexar só arquivos alterados (hash/mtime).
- Comando `/index` para rebuild manual e status.

**Esforço:** ~1–2 semanas. **Risco:** médio (custo de embeddings, tamanho do
índice, cold start).

---

### 5. Renderização web com JavaScript

**Estado atual:** `fetch_webpage` faz HTTP puro + `scraper` + `htmd`. Sites SPA
voltam vazios.

**Lacuna:** muitos docs modernos são SPA. Não é crítico para coding agent, mas é
um buraco visível.

**Proposta:**
- Opção `render: "auto" | "http" | "browser"` na tool.
- `browser` via `chromiumoxide` (headless Chrome) — **feature flag opcional**,
  nunca no binário default (adiciona ~150 MB e dependência de Chrome).
- Detecção de "conteúdo vazio" → sugerir retry com `render: "browser"`.
- Alternativa mais leve: avaliar `spider` (crate Rust, MIT) para crawling em
  profundidade, caso surja necessidade de um tool `crawl`.

**Esforço:** ~3–5 dias (com feature flag). **Risco:** médio (distribuição).

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

---

## P2 — Polimento e ecossistema

### 7. Maturidade de provider

3 adaptadores + custom. Claude Code/Codex são monolíticos e afinados para 1
modelo. O RustClaw é mais flexível, menos otimizado. Ação: manter, mas adicionar
testes de contrato por provider (paridade de eventos, tool calling, caching).

### 8. Integrações de ecossistema

Marketplace de plugins, extensão de IDE (VS Code/JetBrains), GitHub Action
nativo, apps web/mobile. **Não competir** — é ecossistema de empresa, não de
harness solo. Priorizar apenas se houver demanda real de usuários.

### 9. Qualidade de código / dívida

- Cobertura de testes por módulo (hoje 539 testes, mas distribuição desigual —
  `ui/` tem 13k linhas e provavelmente menos cobertura relativa).
- `cargo clippy -- -D warnings` já no CI ✅.
- Documentar invariantes do loop agêntico (o que pode/não pode acontecer entre
  iterações).

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
- **Não** adicionar headless Chrome ao binário default — manter como feature
  flag opcional.
