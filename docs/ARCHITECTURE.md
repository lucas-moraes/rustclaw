# RustClaw Architecture

> Coding agent harness (OpenCode / Claude Code style).
> Loop com native tool calling, sessões SQLite, skills (memória da sessão), permissions HITL.

## Visão geral

O RustClaw é um harness de agente de coding. O modelo recebe as tools **nativamente**
(JSON Schema via API de tool calling) — não há parsing de texto estilo ReAct. O runtime
orquestra o loop, executa tools (em paralelo quando possível), persiste tudo em SQLite
e oferece uma CLI de streaming.

```
User prompt
    │
    ▼
┌─────────────────────── SessionRuntime (runtime.rs) ───────────────────────┐
│  append user msg → build system prompt (+ AGENTS.md + enabled skills)     │
│  → SessionProcessor.run_turn (session/processor.rs)                        │
└───────────────┬───────────────────────────────────────────────────────────┘
                │
    ┌───────────▼───────────────┐        ┌──────────────────────────────┐
    │   Provider (provider/)    │        │   ToolRegistry (tool/)       │
    │   stream LlmRequest +     │        │   specs → JSON Schema        │
    │   tools; emite events     │        │   execute(args, ctx)         │
    └───────────┬───────────────┘        └──────────────┬───────────────┘
                │                                       │
                ▼                                       ▼
   ProviderEvent (TextDelta, ToolCallStart/End, End)   ToolContext
                │                                       │  • permission check
                ▼                                       │  • asker/user_asker
   Loop: tool_calls? ──yes──▶ JoinSet (paralelo) ──▶ results append       │
        │ no                                            • todos / task_runner
        ▼                                               • skills catalog
   final answer ──▶ persist session                                         │
                                                                          │
   Surfaces: ui/cli.rs (streaming CLI) ── event bus (event.rs) ───────────┘
```

## Componentes

### `harness/session` — tipos e loop

| Arquivo | Papel |
|---------|-------|
| `mod.rs` | `Session`, `Message`, `Part` (Text/Reasoning/Tool), `TodoItem` — fonte da verdade |
| `store.rs` | Persistência SQLite (`harness_sessions`, `session_messages`, `skills_json`) |
| `processor.rs` | Loop central: stream → tool calls → execução paralela → repetir |
| `compaction.rs` | Resume de contexto em overflow (via LLM) |

O `Part::Tool(ToolPart)` guarda o input e o resultado (status/output/error) da chamada.
Os adapters de provider sintetizam mensagens de resultado (OpenAI `role:"tool"` /
Anthropic `tool_result`) a partir desses parts.

### `harness/provider` — adapters LLM

| Arquivo | Papel |
|---------|-------|
| `mod.rs` | Trait `Provider`, `ProviderEvent`, `LlmRequest`, SSE parser |
| `openai.rs` | `/chat/completions` + `tool_calls` (streaming SSE e non-stream) |
| `anthropic.rs` | `/messages` + content blocks `tool_use` (streaming SSE e non-stream) |
| `opencode_go.rs` | Roteia por modelo: minimax→`/messages`, senão→`/chat/completions`; `X-API-Key` |

Cada adapter converte `Message`/`Part` para o wire format e devolve um stream unificado
de `ProviderEvent` (`TextDelta`, `ReasoningDelta`, `ToolCallStart/Delta/End`, `End`).

### `harness/tool` — tools com schema

| Arquivo | Tool |
|---------|------|
| `mod.rs` | Trait `Tool { name, description, parameters(JSON Schema), execute }` + `ToolResult` |
| `registry.rs` | `ToolRegistry` (builder) + `ToolSpec` exportado aos providers |
| `context.rs` | `ToolContext` (cwd, abort, permission, askers, todos, task_runner) |
| `bash.rs` `read.rs` `write.rs` `edit.rs` `glob.rs` `grep.rs` | Tools de coding |
| `todo.rs` `question.rs` `task.rs` | Todos, pergunta, subagente |
| `truncate.rs` | Truncate compartilhado de output |

### `harness/permission` — allow/ask/deny

`PermissionEngine::check(tool, path, cwd)` resolve a decisão. Defaults:
- leitura (`read`/`glob`/`grep`/`todo_read`) → **allow**
- mutação (`write`/`edit`/`bash`/`task`) → **ask**
- path fora do CWD → escalado para **ask**

O `check_permission` é chamado dentro de cada task de tool antes de executar.

### `harness/agent` — agents

`AgentSpec { name, description, tools, system_prompt, model?, temperature?, ... }`.
Builtins: `build` (todas), `plan` (read-only), `explore` (read-only), `general` (subset).
O `task` tool dispara subagent via `TaskRunner` (child session isolada).

### `harness/skill` — memória da sessão (skills)

Modelo **prompt / session / memory(skills)**:
- **prompt** — pedido atual do turno (user message)
- **session** — histórico do trabalho (messages SQLite)
- **memory** — skills da sessão (0..N), escolhidas no SkillPicker ou `/skills`

Skills descobertas em `.agents/skills`/`.opencode/skills`/`~/.agents/skills`
(`SKILL.md` com frontmatter `name`/`description`). As skills marcadas no turno
entram no system prompt sob `# Session skills`. Persistidas em
`harness_sessions.skills_json` (mesmo DB das sessões).

### `harness/ui` — superfícies

`ui/tui/` (default): TUI ratatui + crossterm com:
- `app/` — estado e loop, fatiado em módulos coesos:
  - `state.rs` — `App`, `Modal`, picker states
  - `events.rs` — `apply_event`, rebuild do transcript
  - `skills.rs` / `usage.rs` / `undo.rs` — concerns isolados
  - `keys.rs` / `pickers.rs` — dispatch de teclado e modais
  - `runner.rs` — `run_tui`, `TerminalGuard`
- `draw/` — widgets (header/transcript/status/input, modals, help overlay)
- `askers.rs` — `TuiAsker`/`TuiUserAsker` via canais oneshot (sem bloquear stdin)
- `input.rs` — key bindings

`ui/cli.rs`: streaming CLI print-only, fallback para `RUSTCLAW_UI=cli` ou non-TTY.
`ui/commands.rs`: slash commands compartilhados entre TUI e CLI.

O `SessionRuntime.prompt` aceita um `AbortSignal` (Ctrl+C cancela o run); o processor
checa o abort entre iterações e no stream. Tools mutáveis (`edit`/`write`) devolvem
`metadata.diff` (via `tool/diff.rs`) exibido como diff na TUI.

## Fluxo do processor (`run_turn`)

1. Loop até `max_iterations`:
2. Compactar contexto se `approx_tokens > max_context_tokens`
3. `provider.stream(LlmRequest)` com tools do agent
4. Consumir `ProviderEvent`s → acumular parts de texto/raciocínio/tool calls
5. Criar mensagem do assistant; persistir
6. Se há tool calls → `execute_tool_calls` (permission check + JoinSet paralelo)
7. Doom-loop detection (mesma tool call repetida 3x warn / 5x stop)
8. Se o modelo responder sem tools → resposta final; senão continua

## Storage

Single SQLite DB (`harness.db`) com tabelas:
- `harness_sessions` — id, agent, cwd, todos_json, skills_json (memória da sessão)
- `session_messages` — role, parts_json, ord

## Rodando

```bash
cargo run                 # CLI harness
cargo test                # testes unitários
cargo test --bin rustclaw smoke_native_tool_calling -- --ignored --nocapture  # smoke live
```

## Invariantes do loop agêntico

Garantias que valem **entre iterações** do `run_turn` (e entre turnos). Ao
mudar o processor, verifique cada uma delas — a maioria tem teste de
regressão.

### Iterações e limites

- **I1 — Orçamento duplo de iterações.** Cada segmento de turno respeita
  `max_iterations`; o turno inteiro (todas as continuações) respeita
  `max_total_iterations` (default `max_iterations * 3`, ver
  `default_total_iterations`). Continuações são limitadas por
  `DEFAULT_MAX_CONTINUATIONS = 10`; ao exceder, o turno para com motivo
  explícito — nunca silenciosamente.
- **I2 — Deadline de turno.** Cada iteração verifica o wall-clock
  (`turn_timeout_secs`) **antes** de chamar o provider; estourar o prazo
  encerra o turno com `stop_reason`, não com erro.
- **I3 — Compactação antes de cada chamada.** `maybe_compact` roda no topo de
  cada iteração (R1: só quando o contexto cresceu desde a última). O trigger
  proativo é 70% do budget (`DEFAULT_TRIGGER_RATIO = 0.7`); `force` usa 100%.
  Após compactar, `invalidate_messages_cache()` é obrigatório — a próxima
  iteração deve ler as mensagens compactadas, não as em cache.

### Estado e persistência

- **I4 — Mensagens persistem em ordem.** Toda mensagem (assistant, tool
  result) é gravada no SQLite com `ord = MAX(ord)+1` **antes** da próxima
  iteração; um crash nunca deixa o modelo ver mensagens que não estão no
  store. A reconstrução (`rebuild_transcript_from_session`) lê por `ord`.
- **I5 — Ledger sobrevive à compactação.** O `ContextLedger` vive em
  `Session.ledger` (coluna `ledger_json`), não nas mensagens; o resumo de
  compactação é sempre precedido do bloco
  `[Session ledger — durable facts, survives compaction]`. A cadeia de
  resumos (`summary_chain`, máx `MAX_SUMMARY_CHAIN = 8`, oldest-first) é
  foldada no próximo resumo.
- **I6 — Tool results casam com tool calls.** Cada `ToolCallEnd` gera
  exatamente um `Part::Tool` na mensagem do assistant, com `ToolStatus`
  (success/error/aborted) sempre preenchido — um tool abortado produz status
  `aborted`, nunca é omitido. Os adaptadores de provider sintetizam os
  resultados a partir desses parts (`tool_result_message` no OpenAI;
  blocos `tool_result` no Anthropic), então um part faltando quebraria a
  conversa com o modelo.

### Execução de tools

- **I7 — Paralelismo com aplicação em ordem de conclusão.** `execute_tool_calls`
  roda as tools num `JoinSet` e aplica os resultados **na ordem de conclusão**
  (cada resultado atualiza o `Part::Tool` correspondente por `tool_id`).
  Permissão (allow/ask/deny) é checada antes do spawn; `catch_unwind` isola
  panics de tools. R6: arquivos alvo de write/edit têm snapshot pré-iteração
  para rollback se o batch inteiro falhar.
- **I8 — Abort corta o batch.** Se o sinal de abort disparar no meio do
  batch, as tools restantes recebem `Err("aborted")` e o turno termina; o
  loop principal também checa o abort no topo de cada iteração (I2/I3 valem
  mesmo abortado — nada é executado depois do abort).

### Loops e repetição

- **I9 — Doom loop.** A mesma assinatura de tool call repetida
  `DOOM_LOOP_WARN = 3` vezes emite aviso; em `DOOM_LOOP_STOP = 5` o turno
  para (janela de `DOOM_LOOP_WINDOW = 5`). O mesmo vale para texto repetido
  (`TEXT_LOOP_WARN/STOP`). A detecção roda **depois** de aplicar os tool
  results e **antes** da próxima iteração.
- **I10 — Continuação explícita.** Auto-continuação (ex.: truncamento de
  tool args, limite de tokens) incrementa `continuations` e registra o
  motivo; o transcript sempre mostra por que o turno continuou.

### Eventos e UI

- **I11 — Eventos ordenados por turno.** `RunStarted` precede todos os
  eventos do turno; `RunFinished` é o último. Eventos de subagente carregam
  `parent_session_id` e são roteados para painéis (`apply_subagent_event`),
  nunca para o transcript do pai; a finalização do painel aninhado roda
  antes da associação de `child_session_id`.
- **I12 — Transcript reflete o store.** Após undo/revert/fork,
  `rebuild_transcript_from_session` é chamado — a UI nunca exibe mensagens
  que não existem mais no store.
