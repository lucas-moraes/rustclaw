# RustClaw — Documentação de Features

RustClaw é um **coding agent harness** (estilo OpenCode / Claude Code) escrito em Rust.
O core é o módulo `src/harness/`; o restante são módulos de suporte (`config.rs`, `error.rs`).

---

## 1. Loop do agente (session/processor.rs)

O coração do sistema é `SessionProcessor::run_turn`, que executa um turno do usuário:

- **Native tool calling**: stream da resposta do LLM com tool calls nativas
  (`ProviderEvent::ToolCallStart/Delta/End`), sem parsing de texto.
- **Execução paralela de tools**: múltiplas tool calls na mesma mensagem do
  assistente rodam concorrentemente via `JoinSet`.
- **Streaming incremental**: `TextDelta` e `ReasoningDelta` são emitidos em tempo
  real para a UI (CLI ou TUI).
- **Doom-loop detection**: a mesma tool call repetida 3x gera um aviso ao modelo
  ("System note: you just repeated the same tool call…"); 5x para o turno.
- **Proteções contra travamento (watchdogs)**:
  - `STREAM_TIMEOUT_SECS = 300` — se o stream do provider não emitir evento por
    5 min, o turno é abortado.
  - **Watchdog de turno** (`turn_timeout_secs`, default **600s = 10 min**): limite
    global de wall-clock por turno, checado no topo de cada iteração e via
    `tokio::select!` no loop de stream. Configurável via `/settings turn_timeout <secs>`
    (mínimo 30s) e persistido no `config.json`.
  - `max_iterations` (default 50): limite de iterações LLM→tools por turno.
- **Cancelamento responsivo**: Esc/Ctrl+C abortam o turno em até 50 ms, mesmo
  com provider travado (poll de abort com `biased` no `select!`).
- **Compaction automática**: quando o contexto excede `max_context_tokens`,
  mensagens antigas são resumidas pelo próprio LLM (com timeout de 120s e
  fallback para placeholder) e o histórico é persistido já compactado.

## 2. Tools nativas (tool/)

Registradas em `runtime::build_default_registry`:

| Tool | Função |
|---|---|
| `bash` | Executa comandos shell com denylist, timeout próprio (`timeout_secs`, default 120s, máx 600s) e truncamento de saída |
| `read` | Leitura de arquivos (janela offset/limit) |
| `write` | Escrita/criação de arquivos |
| `edit` | Edição por substituição exata de string (old/new) |
| `glob` | Busca de arquivos por padrão |
| `grep` | Busca de conteúdo por regex |
| `git_status` / `git_diff` / `git_log` | Wrappers somente-leitura de git |
| `fetch_webpage` | Download de página → Markdown limpo (via `htmd`) |
| `web_search` | Busca na web |
| `todo_write` / `todo_read` | Lista de tarefas da sessão |
| `question` | Faz pergunta de múltipla escolha ao usuário (modal no TUI) |
| `task` | Dispara subagente (child session isolada) via `TaskRunner` |
| `remember` | Memória persistente do projeto (fatos em SQLite) |

Infra de tools:

- **`ToolRegistry`** (builder): specs JSON Schema por agente (allowlist) e
  execução com checagem defensiva de allowlist.
- **`ToolContext`**: cwd (`PathBufGuard` — toda resolução de path passa por ele),
  `AbortSignal`, `PermissionEngine`, askers (permissão + pergunta ao usuário),
  todos, task runner e project memory.
- **`truncate_output`**: truncamento char-boundary-safe com aviso.
- **`unified_diff`**: diff unificado de edits/writes exibido na UI.

## 3. Agents (agent/)

Builtins em `agent/builtin.rs`:

- **`build`** — agente padrão; único com permissão para tools mutáveis (write/edit).
- **`plan`** — modo planejamento (somente leitura).
- **`explore`** — exploração/pesquisa no código.
- **`general`** — propósito geral.

Cada `AgentSpec` tem: allowlist de tools, system prompt próprio, model override,
temperature calibrada por modo (`turn_temperature`) e overrides de permissão.

## 4. Subagentes (tool/task + runtime::TaskRunner)

- A tool `task` cria uma **child session isolada** com agente escolhido
  (`build`/`plan`/`explore`/`general`), próprio histórico e próprio contexto.
- O resultado do subagente volta como resultado da tool call na sessão pai.
- **Batch paralelo**: passando `tasks: [{description, prompt, agent}, ...]` o tool
  dispara N subagentes concorrentes (`JoinSet` + semáforo, máx. 4 simultâneos),
  com resultados agregados por task (budget 4000 chars/task, 8000 total).
- **Eventos propagados**: a child session emite no canal de eventos do pai com
  `parent_session_id` setado — o CLI prefixa as linhas com `[sub#n]` e o TUI
  mostra um painel compacto por subagente (`⏳ explore — 3 tools` → `✓ … summary`).
- **Persistência**: child sessions ficam no store com `parent_id` setado
  (coluna `harness_sessions.parent_id`); aparecem com `↳` em `/sessions` e são
  garbage-collected quando a sessão pai é deletada.

## 5. Permissões (permission/)

- **`PermissionEngine`**: decide `Allow`/`Ask`/`Deny` por tool + path.
  Fora do CWD → `Ask`.
- Regras configuráveis (`Rule::from_json`), aplicáveis via projeto
  (`apply_project_config`) e em runtime (`set_rule`/`remove_rule`).
- **"Always allow"**: resposta `a` no modal persiste a regra (via callback
  `set_persist`).
- Tools mutáveis pedem confirmação no CLI (y/n/always) e modal no TUI.
- `/permissions` lista e gerencia regras.

## 6. Providers (provider/)

- **Trait `Provider`**: `stream(LlmRequest)` e `complete(LlmRequest)`, com
  `ProviderEvent` unificado (adaptadores convertem `Message`/`Part`).
- **Adaptadores**:
  - `openai.rs` — `/chat/completions` + tool_calls (SSE).
  - `anthropic.rs` — `/messages` + tool_use (SSE).
  - `opencode_go.rs` — roteia minimax → `/messages`, senão → `/chat/completions`.
- **Catálogo builtin** (`catalog.rs`): deepinfra, xai (model ids com ponto:
  grok-4.5, grok-4.6…), opencode-go, openrouter, moonshot, huggingface,
  villamarket, anthropic, etc.
- **Provedores do usuário** (`user_store.rs` → `providers.json`): criados via
  `/provider add|rm|list` ou `/models add`; mesclados com o catálogo (override
  por nome).
- **HTTP client compartilhado** com `connect_timeout` 30s e `read_timeout` 120s
  (time entre chunks, não total — streams longos válidos não são cortados).

## 7. Sessões e persistência (session/)

- **`SessionStore`** (SQLite, `rusqlite`): DB em
  `<data_local_dir>/rustclaw/harness.db` (no macOS:
  `~/Library/Application Support/rustclaw/`).
  - Tabelas `harness_sessions` / `session_messages` (migração idempotente no open).
  - `create_session`, `save_message`, `load_session`, `list_sessions`,
    `set_session_title`, `delete_messages_from`, `delete_session`, `save_session`.
- **Modelo de mensagem**: `Message` com `Part`s (Text, Reasoning, Tool) —
  reasoning separado do texto; tool parts com status (pending/running/success/error).
- **Resumo/continuidade**: `load_last_session` retoma a última sessão do projeto.

## 8. Skills = memória da sessão (skill/)

Modelo de memória em três camadas: **prompt** (pedido atual) + **session**
(histórico) + **memory** (skills).

- **Discovery** (`loader.rs`): procura `SKILL.md` em
  `<cwd>/.agents/skills`, `<cwd>/.opencode/skills`, `~/.agents/skills`,
  `~/.config/opencode/skills` e `$RUSTCLAW_SKILLS_DIR`.
- **`SkillCatalog`**: parse de specs; skills escolhidas na criação da session
  (SkillPicker) ou via `/skills`.
- **`inject.rs`**: skills marcadas no turno (checkbox) entram no system prompt
  (`# Session skills`).
- Persistidas em `harness_sessions.skills_json`.

## 9. Memória de projeto (project/)

- **`ProjectProfiler`** (`profiler.rs`): analisa o projeto (linguagens, estrutura)
  e gera resumo de contexto (`render_summary`).
- **`ProjectMemoryStore`** (`memory.rs`): memória persistente por projeto em
  tabela `<table>_facts` (uma linha por fato) com metadados
  `kind/confidence/hit_count/last_used/archived`.
  - `append_fact`, `list_fact_rows`, `bump_usage`, `set_archived`, dedup,
    `archive_stale`, `compact`, `migrate_summary_facts`.
  - **`render_memory`**: seleciona top-N fatos por score (recência + hit_count +
    match lexical) com orçamento `MAX_MEMORY_CHARS = 2048`; injetado no contexto
    via `runtime::project_context_for`.
- **`config_file.rs`**: `rustclaw.json` — override por projeto de provider/model.
- **`table.rs`**: sufixo de tabela por projeto (isolamento de dados por cwd).

## 10. UI

### TUI (ratatui + crossterm) — `ui/tui/`

- Header, transcript com streaming, status, input multilinha, help overlay,
  modais (permissão y/n/a, question, palette).
- **Command palette** (Ctrl+P).
- **Temas**: cyberclaw, aurora, ember, mono (Ctrl+T cicla; `/theme <name>`).
- **Seleção de texto** com drag e auto-copy (Ctrl+C copia quando há seleção).
- **Renderização de Markdown** nas respostas (`markdown.rs`).
- **Animações** (`anim.rs`) e seleção (`selection.rs`).
- **Askers** (`askers.rs`): `TuiAsker`/`TuiUserAsker` via channels oneshot —
  modais de permissão e pergunta integrados ao loop de eventos.

Key bindings (TUI):

```
Enter enviar · Shift/Alt+Enter quebra de linha · Ctrl+J quebra (macOS)
Esc cancelar run/overlay · Up/Down histórico · Ctrl+A/E início/fim de linha
Ctrl+U/W kill · Ctrl+Z reset · PgUp/PgDn scroll
Ctrl+P palette · Ctrl+T tema · Ctrl+L limpar · ? / F1 help
Tab autocomplete (/) / cycle mode · y/n/a permissão · 1..n question
```

### CLI (fallback) — `ui/cli.rs`

- Streaming de texto no terminal; `RUSTCLAW_UI=cli` força.
- `CliAsker` para confirmações y/n/always.

### Slash commands (compartilhados TUI + CLI) — `ui/commands/`

| Comando | Função |
|---|---|
| `/help` | Ajuda |
| `/settings` | Ver/alterar `iterations`, `context`, `turn_timeout` (persistido) |
| `/usage` / `/tokens` | Uso de tokens da sessão |
| `/new` | Nova sessão |
| `/sessions` | Listar/abrir sessões |
| `/agent` | Trocar agente (build/plan/explore/general) |
| `/skills` | Gerenciar skills da sessão |
| `/compact` | Compaction manual do contexto |
| `/memory` | `list\|rm\|clear\|promote <id>\|gc` da memória de projeto |
| `/models` | Picker de provider/model (com "add provider…") |
| `/model` | Troca rápida de modelo |
| `/provider` | `add\|rm\|list` provedores personalizados |
| `/auth` | Gerenciar tokens por provider (arquivo 0600) |
| `/permissions` | Regras de permissão |
| `/undo` | Desfazer mudanças do último turno |
| `/theme` | Tema |
| `/exit` / `/quit` | Sair |

## 11. Configuração (file-based, sem `.env`)

Arquivos (em `<data_local_dir>/rustclaw/` — no macOS `~/Library/Application Support/rustclaw/`):

| Arquivo | Conteúdo |
|---|---|
| `auth.json` | API token por provider (0600, via `/auth`) |
| `config.json` | provider/model, `max_iterations`, `max_context_tokens`, `turn_timeout_secs`, theme |
| `providers.json` | provedores personalizados (mesclados com o catálogo) |
| `harness.db` | sessões + memória de projeto (tabelas per-projeto) |

- `rustclaw.json` no root do projeto — override per-project de provider/model.
- **Precedência**: catálogo (builtin + user) → config.json → rustclaw.json → auth token.
- **Env vars de UX**: `RUSTCLAWUI`/`RUSTCLAW_UI` (cli|tui), `RUSTCLAW_THEME`,
  `RUSTCLAW_SKILLS_DIR`, `NO_COLOR`.
- Instalação: `scripts/install.sh` (releases) / `scripts/link-local.sh` (dev).

## 12. Event bus (event.rs)

`HarnessEvent` unifica tudo que acontece num run para qualquer UI:

`RunStarted`, `RunFinished`, `UserMessage`, `TextDelta`, `ReasoningDelta`,
`MessageUpdated`, `ToolStart`, `ToolEnd`, `PermissionAsk`, `PermissionResolved`,
`CompactionStarted`, `CompactionFinished`, `Error`.

Canal: `tokio::sync::mpsc::UnboundedSender` (`event_channel()`).

## 13. Runtime (runtime.rs)

`SessionRuntime` é a fachada do harness:

- `prompt()` — executa um turno completo (processor + eventos).
- `switch_model[_at|_with_auth]` — troca de provider/model em runtime.
- `update_settings(max_iterations, max_context_tokens, turn_timeout_secs)`.
- `create_session` / `load_session` / `load_last_session` / `list_sessions` /
  `delete_session` / `set_session_title`.
- `maybe_compact` — compaction manual ou automática.
- `set_permission_rule` / `remove_permission_rule` / `permission_rules`.
- `clone_shareable` — clona o runtime para tarefas em background.
- `build_default_registry` — registro padrão de tools.
- `TaskRunner` — implementa `SubagentRunner` para a tool `task`.

## 14. Testes e qualidade

- **219 testes** (`cargo test`), incluindo smoke test live
  (`smoke_native_tool_calling`, `--ignored`, usa token real de `auth.json`).
- Testes no mesmo arquivo sob `#[cfg(test)]`; `tempfile` para DBs/arquivos temporários.
- `cargo clippy` limpo; `cargo fmt` para formatação (100 colunas, 4 espaços).

## 15. Segurança

- `auth.json` com permissões `0600`.
- `PathBufGuard` — toda resolução de path de tool passa pelo cwd guard.
- Denylist de comandos no `bash` + timeout obrigatório.
- Allowlist de tools por agente, reforçada na execução (defense-in-depth).
- Fora do CWD → permissão `Ask`.
- Truncamento de saída de tools (evita estourar contexto).

## 16. MCP (Model Context Protocol)

O RustClaw conecta a servidores MCP externos e expõe as tools deles ao modelo
como tools nativas (`src/harness/mcp/`).

### Configuração

- Global: `~/.local/share/rustclaw/mcp.json` (no macOS:
  `~/Library/Application Support/rustclaw/mcp.json`).
- Projeto: seção `mcp` no `rustclaw.json` (sobrescreve servers de mesmo nome).
- Formato padrão `mcpServers` (compatível com Claude/Cursor):

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
      "env": {"SOME_VAR": "${SOME_VAR}"},
      "enabled": true,
      "timeout_secs": 60
    },
    "remote": {
      "url": "https://example.com/mcp",
      "headers": {"Authorization": "Bearer <token>"}
    }
  }
}
```

- Exatamente um de `command` (stdio) ou `url` (streamable HTTP) é obrigatório.
- `${VAR}` em `args`/`env` é expandido do ambiente (var indefinida = erro).

### Naming e registry

- Tools MCP entram no registry como `mcp_<server>_<tool>` (sanitizado para
  `[a-z0-9_]`), ex.: `mcp_filesystem_read_file`.
- Descrições truncadas em 200 chars; cap de 50 tools por server.
- Conexão em paralelo no boot (`McpManager::connect_all`); falha de um server
  não bloqueia os outros (timeout de conexão 10s).

### Permissões e agentes

- MCP tools caem no default `Ask`; `/permissions set mcp_<server>_<tool> allow`
  persiste em `rustclaw.json`.
- Agentes readonly (`plan`/`explore`) admitem MCP tools com annotation
  `readOnlyHint: true` (marcador `mcp_readonly` na allowlist).

### Comandos

- `/mcp list` — servers configurados + contagem de tools.
- `/mcp status` — estado de conexão por server.
- `/mcp restart <name>` — reconecta um server.

### Robustez

- Reconnect: falha de transporte → 1 respawn + retry automático.
- Health check: probe a cada 60s marca servers mortos no `/mcp status`.

## Prompt caching

- **Anthropic-style** (`anthropic`, e MiniMax via opencode-go quando ativado):
  o body ganha até 3 breakpoints `cache_control: {"type": "ephemeral"}` —
  system prompt (bp1), último item de `tools` (bp2) e último content block da
  última message (bp3). Limite da API: 4 breakpoints.
- **OpenAI-compatible**: caching automático do provider; o RustClaw apenas
  contabiliza `cached_tokens` no usage.
- **Custo cache-aware** (`/usage`): Anthropic cobra write 1.25× e read 0.1× do
  input (input_tokens exclui cache); OpenAI cobra cached a 0.5× (prompt_tokens
  inclui cached). O custo exibido no `/usage` e na sidebar usa esses fatores.
- **Kill-switch**: `"prompt_caching": false` no `config.json` global desliga os
  breakpoints em todos os providers. Override por provider: `"prompt_cache":
  true|false` no `providers.json` (default: `true` só para `anthropic`;
  MiniMax/opencode-go fica `false` pois pode rejeitar `cache_control`).
- **Onde ver**: `/usage` mostra a linha `cache · read (N% of in) · write`;
  a status bar e a sidebar mostram `↻<tokens>` quando há cache reads.
