# RustClaw — Avaliação do Projeto & Roadmap

> **Data:** 2026-09-04 · **Base:** `main` pós-commit `0fe621c` (scroll no session manager)
> **Complementa:** `TODO.md` (D1–D10, concluídos), `TODO_2.md` (E1–E10), `SUGGESTIONS.md`

---

## 📊 Diagnóstico geral

### Métricas coletadas

| Métrica | Valor |
|---|---|
| Total de linhas Rust (`src/`) | 25.581 |
| Testes | 297 (296 passando + 1 ignored) |
| Arquivo maior | `ui/tui/app.rs` — 3.167 linhas |
| Tools registradas | 16 |
| Providers builtin | 8 (+ user-defined via `providers.json`) |
| Slash commands | ~21 |
| CI | Só build em tag (`release.yml`) |

### Pontos fortes

- **Arquitetura modular clara**: tools / providers / sessions / agents / mcp bem separados
- **Features de nível comercial**: MCP (stdio + streamable HTTP), skills, memória
  persistente per-projeto, subagentes paralelos com painéis no TUI, permissões persistentes
- **Cobertura de testes sólida** para o porte do projeto (297 testes)
- **CI de release** multi-plataforma (macOS ARM + Linux x64)
- **Docs mantidas**: `docs/ARCHITECTURE.md`, `docs/FEATURES.md`, TODO/SUGGESTIONS versionados

### Fraquezas

- `app.rs` com **3.167 linhas** — god struct `App` com 48 campos `pub`; E4 marcado ✅
  no TODO_2, mas o arquivo cresceu de volta
- `processor.rs` com **931 linhas e apenas 2 testes** — o coração do sistema é o
  módulo menos testado
- Arquivos grandes: `memory.rs` (1.364), `store.rs` (1.128), `commands/mod.rs` (831)
- `Mutex<Connection>` (std) chamado de `async fn` **sem `spawn_blocking`** —
  bloqueia worker tokio (`ProjectMemoryStore`, tool `remember`)
- ~88 unwraps/expects em `memory.rs` (grande parte em testes, mas ainda assim)
- CI só builda em tag — **não roda testes nem clippy** em push/PR
- **Sem prompt caching** (Anthropic `cache_control` / OpenAI automatic) —
  custo desnecessário em sessões longas

---

## 🔥 Prioridade 1 — Robustez do core (pagar juros)

### 1.1. Testes de integração do loop completo (E7, pendente)

`processor.rs` é o módulo mais crítico e menos testado. Criar um `MockProvider`
que emite sequências de `ProviderEvent` (tool calls, timeouts, doom-loops) e
testar `run_turn` de ponta a ponta: paralelismo, abort, watchdogs, compaction.

**É o investimento de maior retorno.**

### 1.2. Quebrar `app.rs` de verdade

Corte sugerido (seguindo o padrão já existente em `draw/`):

- `app/state.rs` — struct `App` + transcript/scroll/selection (campos `pub(crate)`)
- `tui/modals/{skill,model,auth,resume}.rs` — state+keys de cada modal
  (hoje espalhados em 3 lugares: state em app.rs, keys em app.rs, draw em `draw/`)
- `app/loop.rs` — `run_tui` (~330 linhas) + drenagem de canais
- `app/keys.rs` — dispatch de teclas (`handle_key` tem ~320 linhas)
- Mover `handle_skills/settings_command` → `ui/commands/`
- Mover undo/revert → service no runtime (UI não deveria falar com `SessionStore`)

### 1.3. CI de qualidade em cada push/PR

O `release.yml` só builda em tags. Adicionar workflow `ci.yml`:
`cargo test` + `cargo clippy -- -D warnings` + `cargo fmt --check` em push/PR.
Barato e evita regressões.

### 1.4. SQLite async-safe

`ProjectMemoryStore` e `SessionStore` usam `Mutex<Connection>` (std) chamado de
async fns sem `spawn_blocking`. Opções:

- `tokio::task::spawn_blocking` nos call sites, **ou**
- pool via `deadpool-sqlite` / `tokio-rusqlite`

Risco real de contensão e bloqueio do runtime.

### 1.5. Prompt caching (custo $ direto)

Nenhum `cache_control`/`cached_tokens` no código. Anthropic (ephemeral cache) e
OpenAI (caching automático) suportam. Em sessões longas com system prompt grande
(AGENTS.md + skills + memory), a economia é de **50–90% dos input tokens**.

A tabela de preços já existe (E9) — falta ler `cache_read_input_tokens` do usage
e exibir no `/usage`.

---

## 🚀 Prioridade 2 — Features de alto valor

### 2.1. Tool `diagnostics` (E5, pendente)

`cargo check --message-format=json` → erros estruturados por arquivo/linha.
O agente para de "adivinhar" erros de compilação. Já especificada no TODO_2.

### 2.2. Checkpoints de arquivos + `/diff` + `/restore` (SUGGESTIONS #1)

Snapshot antes de `write`/`edit`; `/restore <path>` reverte. Sem isso, o usuário
não tem rollback de arquivos (o `/undo` só reverte mensagens do DB).
Encaixa em `ToolContext` + processor.

### 2.3. Hooks de projeto (SUGGESTIONS #5)

`.agents/hooks.json` ou seção em `rustclaw.json`:
`pre_tool` / `post_tool` / `on_turn_end`.
Ex.: auto-`cargo fmt` após edit em `*.rs`; bloquear `bash` que bata em produção.

### 2.4. Export de sessão (`/export md|json`)

Transcript + tool results → Markdown/JSON. Barato (a session store já tem tudo
serializado) e ótimo para bug reports/post-mortems.

### 2.5. Plan → Build handoff

`/agent plan` produz plano + todos → `/apply-plan` injeta o plano no context e
troca para `build`. Agents já existem; falta o bridge de UX.

---

## ✨ Prioridade 3 — Polish / UX

| Feature | Descrição |
|---|---|
| **Fork de sessão** | `/fork` — branch de conversa a partir da mensagem N |
| **Transcript search** | Ctrl+F no histórico renderizado + jump |
| **Multimodal** | `Part::Image` (path/clipboard) → Anthropic/OpenAI vision |
| **Background bash** | `bash --background` → job id, `/jobs`, notificação |
| **Painel thinking** | E8 — reasoning tokens em painel colapsável |
| **Custom agents** | `.agents/agents/*.md` com tools/model/prompt |
| **Replay de eventos** | Gravar `HarnessEvent` como JSONL + `/replay` |
| **Budget diário** | Warn ao bater limite $/dia (base: tabela E9) |

---

## 🧭 Roadmap sugerido

```
Sprint 1 (robustez):   E7 testes do loop → CI de push/PR → SQLite spawn_blocking
Sprint 2 (qualidade):  diagnostics (E5) → checkpoints + /restore
Sprint 3 (produto):     prompt caching → hooks → /export → plan→build
Sprint 4 (polish):      fork, search, multimodal, thinking panel, budget
```

### O que NÃO priorizar

- Voice / speech
- Editor embutido full (já tem TUI + tools de edit)
- Multi-user / cloud sync
- Plugin marketplace genérico (MCP resolve melhor)

---

## 📈 Veredicto

O RustClaw está em **bom estado** — arquitetura sã, 297 testes, features de nível
comercial (MCP, subagentes paralelos, memória persistente, permissões). As lacunas
são as clássicas de projeto que cresceu rápido:

1. **Testes no core loop** (E7) — módulo mais crítico, menos testado
2. **`app.rs` monolítico** — precisa ser quebrado de novo
3. **CI sem testes** — só build em tag
4. **Prompt caching** — dinheiro parado na mesa

Pagar esses juros primeiro multiplicará o retorno de qualquer feature nova.
