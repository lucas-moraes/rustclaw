# SUGGESTIONS.md — Features pendentes para o RustClaw

> Atualizado após revisão do código em `src/harness/`.
> **Removidos desta lista (já implementados):** `/undo` (reverte último turno,
> com testes), git tools nativas (`git_status`/`git_diff`/`git_log` em
> `tool/git.rs`), permissões persistentes por projeto (`always allow` grava em
> `rustclaw.json` via `set_persist` + `apply_project_config`).

---

## 🔥 Tier 1 — Alto valor, encaixa no código atual

### 1. Checkpoint de arquivos + `/diff` / `/restore`
Antes de `write`/`edit`, snapshot do arquivo (ou git stash-like por sessão).
Comandos:
- `/diff` — ver mudanças da sessão
- `/restore <path>` — voltar arquivo
- opcional: auto-checkpoint no início do turn

**Por quê:** o agente já edita; sem rollback o usuário confia menos em turns longos.

### 2. Export de sessão (`/export md|json`)
Exportar transcript + tool results para Markdown/JSON (share, bug report, post-mortem).

**Por quê:** barato, útil, e a session store já tem tudo serializado.

---

## 🚀 Tier 2 — Diferenciais de produto

### 3. Plan mode → Build mode com handoff
Fluxo explícito:
1. `/agent plan` produz plano + todos
2. `/apply-plan` ou botão "Implement" troca para `build` com o plano injetado no system/context

**Por quê:** agents `plan`/`build` já existem; falta o *bridge* de UX.

### 4. Subagent paralelo com UI ✅ (implementado)
`task` já spawna subagent, mas o TUI trata como tool opaca. Melhorias:
- ~~painel/accordion "subagent explore #2 running…"~~ ✅ painel compacto no TUI (`⏳ explore — 3 tools` → `✓ summary`)
- ~~streaming resumido do filho~~ ✅ eventos da child propagados com `parent_session_id`; CLI prefixa `[sub#n]`
- ~~cancelar subagent individual~~ (cancelamento via abort do turno; individual fica para depois)
- ~~N tasks em paralelo com budget~~ ✅ batch `tasks: [...]` com semáforo (4) e budget de output

### 5. Hooks de projeto (estilo Claude Code)
Arquivo `.agents/hooks.json` ou seção em `rustclaw.json`:
- `pre_tool` / `post_tool` / `on_turn_end`
- ex.: rodar `cargo fmt` depois de `edit` em `*.rs`
- bloquear `bash` que bata em produção

Encaixa no `ToolContext` + processor loop.

### 6. MCP client (Model Context Protocol)
Conectar servers MCP como tools dinâmicas (GitHub, DB, browser, Sentry…).
Registry vira: builtins + MCP tools discovered.

**Por quê:** é o padrão de extensibilidade que OpenCode/Claude Code estão usando; skills cobrem prompt, MCP cobre *ações*.

### 7. Memória com retrieval decente
Hoje `render_memory` é lexical + recência. Evoluir para:
- embedding local (fastembed / sqlite-vec) **ou**
- FTS5 no SQLite dos facts
- `/memory search <q>` (hoje só list/rm/clear/gc/promote/add)
- auto-promote de facts muito usados → skill

O store de facts já tem `hit_count`/`confidence` — só falta retrieval.

---

## ✨ Tier 3 — Polish de UX / power-user

### 8. Fork de sessão (`/fork`)
Duplicar sessão a partir da mensagem N (branch de conversa). Bom para "tentar outro approach".

### 9. Transcript search (`/` ou Ctrl+F)
Busca no histórico renderizado + jump; seleção de texto já existe, falta find.

### 10. Image / multimodal input
Colar screenshot no prompt (path ou clipboard) → `Part::Image` nos providers que suportam (Anthropic/OpenAI vision).
Hoje `Part` só tem `Text`/`Reasoning`/`Tool`.

### 11. Cost & budget
`/usage` já mostra tokens. Somar:
- custo estimado por provider/model (tabela $/1M)
- budget diário/sessão com warn
- breakdown por tool/subagent

### 12. Custom agents no projeto
`.agents/agents/*.md` ou JSON:
```yaml
name: rust-reviewer
tools: [read, grep, glob]
model: claude-sonnet
prompt: ...
```
Hoje só builtins em `agent/builtin.rs` (AGENTS.md é injetado no prompt, mas não define agentes).

### 13. Background bash / long jobs
`bash` com `background: true` → job id, `/jobs`, notificação ao terminar (builds longos sem travar o turn).

### 14. LSP-lite diagnostics tool
Tool `diagnostics` que roda `cargo check --message-format=json` (ou eslint) e devolve erros estruturados por arquivo/linha.
O agent para de "adivinhar" compile errors.

### 15. Session sharing / replay
Gravar eventos (`HarnessEvent`) como JSONL e `/replay` para debug do harness em si — ouro para desenvolvimento do RustClaw.

---

## 🧭 Roadmap pragmático

```
1. file checkpoints + /diff + /restore   (2–3 dias)
2. /export md                            (meio dia)
3. plan→build handoff                    (1 dia)
4. hooks de projeto                      (2–3 dias)
5. MCP client                            (1 semana+)
6. memory FTS/embeddings                 (2–4 dias)
7. custom agents + background jobs       (depois)
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
| **Confiança ao editar código** | checkpoints + /restore |
| **Melhor qualidade do agent** | diagnostics + plan→build |
| **Extensibilidade** | hooks + MCP + custom agents |
| **Diferencial de UX no TUI** | fork, search, subagent panel, cost |
