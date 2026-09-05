# SUGGESTIONS.md — Novas Features para o RustClaw

> Análise do estado atual do harness + sugestões de features priorizadas por impacto.
> Gerado a partir de uma revisão do código em `src/harness/`.

---

## Estado atual (o que o RustClaw já tem)

O núcleo "Claude Code / OpenCode" está coberto:

| Área | Estado atual |
|---|---|
| **Agents** | `build` / `plan` / `explore` / `general` |
| **Tools** | read, write, edit, bash, glob, grep, web_search, fetch_webpage, todo, question, task, remember |
| **Providers** | OpenAI-compat, Anthropic, OpenCode Go + catálogo user-defined |
| **Sessão** | SQLite por projeto, resume, rename/title, compact auto |
| **Memória** | skills (prompt) + facts de projeto (`remember`) + profiler de stack |
| **Permissões** | allow/ask/deny + always-allow + path fora do CWD |
| **TUI** | streaming, sidebar, pickers, seleção/copy, cancel Esc, temas |
| **Subagents** | `task` spawna child session isolada |

---

## 🔥 Tier 1 — Alto valor, encaixa no código atual

### 1. Undo / revert do último turno (`/undo`)
Já existe hint no modal (`"undo this prompt + replies"`), mas a feature real não está completa.
Reverter a última mensagem do user + respostas/tools associadas (já tem `delete_messages_from` / save com orphan cleanup).

**Por quê:** Esc cancela *durante* o turn; undo recupera *depois*. Fluxo natural pós-cancelamento/erro.

### 2. Checkpoint de arquivos + `/diff` / `/restore`
Antes de `write`/`edit`, snapshot do arquivo (ou git stash-like por sessão).
Comandos:
- `/diff` — ver mudanças da sessão
- `/restore <path>` — voltar arquivo
- opcional: auto-checkpoint no início do turn

**Por quê:** o agente já edita; sem rollback o usuário confia menos em turns longos.

### 3. Git-aware tools (`git_status`, `git_diff`, `git_log`)
Hoje git passa por `bash` genérico. Tools dedicadas com output truncado/estruturado:
- status + branch + dirty files
- diff staged/unstaged com path filter
- log curto

**Por quê:** o model gasta tokens e erra parsing de `git` via shell; tools nativas melhoram planos e PRs.

### 4. Export de sessão (`/export md|json`)
Exportar transcript + tool results para Markdown/JSON (share, bug report, post-mortem).

**Por quê:** barato, útil, e a session store já tem tudo serializado.

### 5. Permissões persistentes por projeto
`always allow` hoje é por run. Persistir em `rustclaw.json` / config:
```json
"permission": { "bash": "allow", "edit": "ask", "bash.rm": "deny" }
```
UI: `/permissions` list/set + "remember this decision".

**Por quê:** o engine já existe; falta só persistência + UX.

---

## 🚀 Tier 2 — Diferenciais de produto

### 6. Plan mode → Build mode com handoff
Fluxo explícito:
1. `/agent plan` produz plano + todos
2. `/apply-plan` ou botão "Implement" troca para `build` com o plano injetado no system/context

**Por quê:** agents `plan`/`build` já existem; falta o *bridge* de UX.

### 7. Subagent paralelo com UI
`task` já spawna subagent, mas o TUI trata como tool opaca. Melhorias:
- painel/accordion "subagent explore #2 running…"
- streaming resumido do filho
- cancelar subagent individual
- N tasks em paralelo com budget

### 8. Hooks de projeto (estilo Claude Code)
Arquivo `.agents/hooks.json` ou seção em `rustclaw.json`:
- `pre_tool` / `post_tool` / `on_turn_end`
- ex.: rodar `cargo fmt` depois de `edit` em `*.rs`
- bloquear `bash` que bata em produção

Encaixa no `ToolContext` + processor loop.

### 9. MCP client (Model Context Protocol)
Conectar servers MCP como tools dinâmicas (GitHub, DB, browser, Sentry…).
Registry vira: builtins + MCP tools discovered.

**Por quê:** é o padrão de extensibilidade que OpenCode/Claude Code estão usando; skills cobrem prompt, MCP cobre *ações*.

### 10. Memória com retrieval decente
Hoje `render_memory` é lexical + recência. Evoluir para:
- embedding local (fastembed / sqlite-vec) **ou**
- FTS5 no SQLite dos facts
- `/memory search <q>`
- auto-promote de facts muito usados → skill

O store de facts já tem `hit_count`/`confidence` — só falta retrieval.

---

## ✨ Tier 3 — Polish de UX / power-user

### 11. Fork de sessão (`/fork`)
Duplicar sessão a partir da mensagem N (branch de conversa). Bom para "tentar outro approach".

### 12. Transcript search (`/` ou Ctrl+F)
Busca no histórico renderizado + jump; seleção de texto já existe, falta find.

### 13. Image / multimodal input
Colar screenshot no prompt (path ou clipboard) → `Part::Image` nos providers que suportam (Anthropic/OpenAI vision).

### 14. Cost & budget
`/usage` já mostra tokens. Somar:
- custo estimado por provider/model (tabela $/1M)
- budget diário/sessão com warn
- breakdown por tool/subagent

### 15. Custom agents no projeto
`.agents/agents/*.md` ou JSON:
```yaml
name: rust-reviewer
tools: [read, grep, glob]
model: claude-sonnet
prompt: ...
```
Hoje só builtins em `agent/builtin.rs`.

### 16. Background bash / long jobs
`bash` com `background: true` → job id, `/jobs`, notificação ao terminar (builds longos sem travar o turn).

### 17. LSP-lite diagnostics tool
Tool `diagnostics` que roda `cargo check --message-format=json` (ou eslint) e devolve erros estruturados por arquivo/linha.
O agent para de "adivinhar" compile errors.

### 18. Session sharing / replay
Gravar eventos (`HarnessEvent`) como JSONL e `/replay` para debug do harness em si — ouro para desenvolvimento do RustClaw.

---

## 🧭 Roadmap pragmático

Se o objetivo é **produto usável no dia a dia**, nesta ordem:

```
1. /undo + restore de mensagens          (1–2 dias)
2. file checkpoints + /diff + /restore   (2–3 dias)
3. permissions persistentes              (1 dia)
4. git tools estruturadas                (1–2 dias)
5. /export md                            (meio dia)
6. plan→build handoff                    (1 dia)
7. hooks de projeto                      (2–3 dias)
8. MCP client                            (1 semana+)
9. memory FTS/embeddings                 (2–4 dias)
10. custom agents + background jobs      (depois)
```

### O que **não** priorizar agora
- Voice / speech
- Editor embutido full (já tem TUI + tools de edit)
- Multi-user/cloud sync
- Plugin marketplace genérico (MCP resolve melhor)

---

## Critério de escolha

| Se você quer… | Faça primeiro… |
|---|---|
| **Confiança ao editar código** | checkpoints + undo + permissions persistentes |
| **Melhor qualidade do agent** | git tools + diagnostics + plan→build |
| **Extensibilidade** | hooks + MCP + custom agents |
| **Diferencial de UX no TUI** | fork, search, subagent panel, cost |
