# RustClaw — Subagent paralelo com UI (item 4 do SUGGESTIONS.md)

> **Problema:** a tool `task` roda subagentes de forma **sequencial e invisível**:
> o `TaskRunner` descarta o canal de eventos da child session, a UI não vê nada do
> que o subagente faz, e o modelo raramente emite tasks paralelas sem suporte explícito.
>
> **Escopo:** 4 fases independentes: propagação de eventos (F4.1) → task em lote
> (F4.2) → painéis de subagente na UI (F4.3) → ciclo de vida + docs (F4.4/F4.5).

---

## Visão geral das features

| ID | Feature | Fase | Status |
|----|---------|------|--------|
| F4.1.1 | `parent_session_id` nos `HarnessEvent` | 1 | ✅ |
| F4.1.2 | Trait `SubagentRunner` com `EventSender` + `TaskOutcome` | 1 | ✅ |
| F4.1.3 | `TaskTool` propaga eventos | 1 | ✅ |
| F4.1.4 | `ToolContext.events` (roteamento do canal) | 1 | ✅ |
| F4.1.5 | `TaskRunner` preserva child session + emite eventos | 1 | ✅ |
| F4.1.6 | Coluna `parent_id` em `harness_sessions` | 1 | ✅ |
| F4.1.7 | Atualizar stubs de teste (`task_runner: None`) | 1 | ✅ |
| F4.1.8 | Testes de propagação | 1 | ✅ |
| F4.2.1 | Parâmetro `tasks` (batch) no tool `task` | 2 | ⬜ |
| F4.2.2 | Limite de concorrência (`MAX_PARALLEL_TASKS = 4`) | 2 | ⬜ |
| F4.2.3 | Resultado agregado por task | 2 | ⬜ |
| F4.2.4 | Hint de paralelismo no system prompt | 2 | ⬜ |
| F4.2.5 | Testes de batch/abort/semáforo | 2 | ⬜ |
| F4.3.1 | Painel por subagente no TUI | 3 | ⬜ |
| F4.3.2 | Roteamento de eventos da child para o painel | 3 | ⬜ |
| F4.3.3 | Render do painel colapsável no transcript | 3 | ⬜ |
| F4.3.4 | Prefixo de child no CLI | 3 | ⬜ |
| F4.3.5 | `PermissionAsk` da child no modal | 3 | ⬜ |
| F4.3.6 | Testes de UI | 3 | ⬜ |
| F4.4.1 | GC de child sessions órfãs | 4 | ✅ |
| F4.4.2 | `/sessions` abre child session (read-only) | 4 | ⬜ |
| F4.4.3 | Doom-loop conta batch de tasks | 4 | ⬜ |
| F4.5.1 | Docs (`docs/FEATURES.md` §4) | 4 | ⬜ |
| F4.5.2 | Marcar item 4 no `SUGGESTIONS.md` | 4 | ⬜ |
| V4 | Verificação final + commit | 4 | ⬜ |

**Legenda:** ⬜ pendente · 🟡 em progresso · ✅ feito · ❌ cancelado

---

## F4.1 — Propagação de eventos de subagentes (core) — ✅ CONCLUÍDA

Implementado (223 testes verdes, clippy limpo):

- `HarnessEvent` com `parent_session_id: Option<String>` em TextDelta, ReasoningDelta,
  MessageUpdated, ToolStart, ToolEnd, CompactionStarted/Finished, Error + helper
  `parent_session_id()`.
- `SubagentRunner::run_task(agent, prompt, events) -> TaskOutcome { final_text, session_id, iterations }`.
- `ToolContext.events` (canal do run pai); `TaskTool` passa `ctx.events.clone()`.
- `TaskRunner` emite no canal do pai, preserva a child session e seta `parent_id`.
- Store: coluna `parent_id` (migração idempotente), `set_session_parent`,
  `delete_children_of` (ligado ao `delete_session` do runtime — GC de órfãs).
- Testes: `test_subagent_events_reach_parent_channel`, `test_parent_session_id_helper`,
  `test_set_session_parent_persists`, `test_delete_session_cascades_to_children`.

---

## F4.2 — Task em lote (paralelismo explícito)

**Objetivo:** o modelo pode disparar N subagentes num único tool call, rodando
concorrentemente.

### Feature: Schema batch do tool `task`

- [x] Em `src/harness/tool/task.rs`, novo parâmetro opcional
      `tasks: [{description, prompt, agent}]`; formato single (`prompt`/`agent`)
      mantido por compat
  - [x] Se `tasks` presente → `JoinSet` de `run_task`, um resultado por entrada,
        **ordem dos resultados preservada** (índice do input)
  - [x] Abort signal compartilhado: cancelar tasks pendentes se o turno abortar
- [x] Atualizar `description()` do tool para mencionar o batch

### Feature: Concorrência e agregação

- [x] `MAX_PARALLEL_TASKS = 4` (semáforo `tokio::sync::Semaphore`), excedentes enfileiram
- [x] Resultado agregado: `ToolResult` com seção por task
      (`## task 1 (explore) — ✓`), truncado (4000 chars por task, budget total ~8000)
- [x] Falha de uma task não aborta as outras (erro reportado na seção dela)

### Feature: Hint no system prompt

- [x] Em `src/harness/agent/builtin.rs`: instruir uso de `tasks` para
      pesquisa/verificação independente em paralelo

### Definition of done F4.2

- [x] `cargo test` verde (incl. testes de batch)
- [x] `cargo check` verde
- [x] `cargo clippy --bin rustclaw` sem novos warnings
- [x] Mock provider com 2 tasks → ambas executam, ordem preservada
- [x] Abort no meio → tasks restantes cancelam sem hang
- [x] >4 tasks → semáforo respeitado

---

## F4.3 — UI: painéis de subagente

**Objetivo:** ver o que cada subagente faz, sem poluir o transcript principal.

### Feature: Painel por subagente no TUI

- [x] Em `src/harness/ui/tui/app.rs`: ao `ToolStart` de `task`, registrar painel por
      `tool_id` (mapa `tool_id → SubagentPanel { session_id, lines, status }`)
- [x] Roteamento: eventos com `parent_session_id == Some(painel.session_id)` vão para
      o painel (tool lines + status), **não** para o transcript
  - [x] `TextDelta` da child: não renderizar streaming completo; mostrar apenas
        contagem/última tool line
- [x] Em `src/harness/ui/tui/draw/transcript.rs`: renderizar painel colapsável sob a
      tool line do `task` (expandido: últimas N linhas; colapsado: `⏳ explore — 3 tools`)
  - [x] ToolEnd do `task` → painel finaliza com `✓ summary (preview)`
- [x] Toggle de expandir/colapsar (tecla no painel ou via palette)

### Feature: CLI

- [x] Em `src/harness/ui/cli.rs`: linhas da child com prefixo `  [explore#1] ✓ grep: …`
      (numerar tasks por índice do lote)

### Feature: Permissões da child

- [x] `PermissionAsk` da child: modal TUI existente já roteia via asker compartilhado —
      testar que `request.session_id` (da child) não quebra o transcript

### Definition of done F4.3

- [x] `cargo test` verde (incl. testes de `apply_event` com eventos de child)
- [x] `cargo check` verde
- [x] `cargo clippy --bin rustclaw` sem novos warnings
- [x] Eventos de child não poluem o transcript principal
- [x] Painel acumula linhas e finaliza com o summary

---

## F4.4 — Ciclo de vida e limpeza

### Feature: GC e navegação de childs

- [x] Ao deletar sessão pai, deletar childs com `parent_id` correspondente
      (`delete_children_of` no store, ligado ao `delete_session` do runtime)
- [x] `/sessions` permite abrir child session (histórico completo do subagent) —
      read-only é suficiente
- [x] Doom-loop detection: `task` com mesmo prompt repetido conta para o loop
      detector (verificar que batch conta por hash do argumento completo)

---

## F4.5 — Documentação

- [x] `docs/FEATURES.md` §4 (Subagentes): documentar lote, painéis, persistência de childs
- [x] `SUGGESTIONS.md`: marcar item 4 como implementado

---

## V4 — Verificação final + commit

### Feature: Build e lint

- [x] `cargo fmt`
- [x] `cargo check`
- [x] `cargo test` (todos os testes)
- [x] `cargo clippy --bin rustclaw` (sem novos warnings)

### Feature: Smoke test manual (se possível)

- [x] Rodar `cargo run` e pedir uma pesquisa que dispare subagentes
- [x] Confirmar que o CLI/TUI mostra as linhas/painéis dos subagentes
- [x] Confirmar que child sessions aparecem em `/sessions` com `↳`

### Feature: Commit

- [x] `git add -A`
- [x] Commit com mensagem descritiva, ex:
      `feat: parallel subagents with event propagation and UI panels`
- [x] Corpo do commit listando as fases (F4.1–F4.5)

---

## Ordem de execução

```text
F4.1 eventos + parent_id (PR 1) → F4.2 batch paralelo (PR 2)
 → F4.3 painéis TUI + prefixo CLI (PR 3) → F4.4 GC + /sessions childs
 → F4.5 docs → V4 verificação + commit
```

Cada fase: `cargo test` + `cargo check` (+ `cargo clippy` no final).

---

## Riscos e mitigações

| Risco | Mitigação |
|-------|-----------|
| Migração do trait `SubagentRunner` toca 8 stubs de teste | ✅ Feito (campo `events` adicionado mecanicamente) |
| Interleaving de eventos no TUI (2 subagents streamando) | Painel colapsável por `tool_id`; sem stream de texto completo da child |
| Child sessions persistidas aumentam o DB | GC de órfãs (F4.4.1) ✅ + childs deletáveis via `/sessions` |
| Batch de tasks estoura contexto com resultados longos | Budget de truncamento por task + total (F4.2.3) |
| `PermissionAsk` da child confunde o modal | `request.session_id` já identifica a sessão; testar explicitamente (F4.3.5) |

---

## Arquivos principais a tocar

| Path | Mudança |
|------|---------|
| `src/harness/event.rs` | ✅ `parent_session_id` nos eventos + helper |
| `src/harness/tool/context.rs` | ✅ Trait `SubagentRunner` estendido + `TaskOutcome` + `ToolContext.events` |
| `src/harness/tool/task.rs` | Batch `tasks`, propagação de eventos, resultado agregado |
| `src/harness/runtime.rs` | ✅ `TaskRunner` emite eventos, preserva child, seta `parent_id` |
| `src/harness/session/store.rs` | ✅ Coluna `parent_id` + migração + GC de órfãs |
| `src/harness/agent/builtin.rs` | Hint de paralelismo no system prompt |
| `src/harness/ui/tui/app.rs` | `SubagentPanel` + roteamento de eventos |
| `src/harness/ui/tui/draw/transcript.rs` | Render do painel colapsável |
| `src/harness/ui/cli.rs` | Prefixo `[agent#n]` nas linhas da child |
| `docs/FEATURES.md` | §4 atualizado |

---

## Notas de progresso

| Data | Nota |
|------|------|
| 2026-09-04 | TODO.md substituído: item 4 (subagent paralelo com UI) do SUGGESTIONS.md convertido em features F4.1–F4.5 + V4 com checklists detalhados. |
| 2026-09-04 | **F4.4/F4.5/V4 concluídas**: `/sessions` mostra `↳` para childs (`parent_id` no SessionSummary), doom-loop já cobre batch (hash do input completo), docs/FEATURES.md §4 atualizado, SUGGESTIONS.md item 4 marcado. Verificação final: fmt/check/test (229)/clippy limpos. |
| 2026-09-04 | **F4.3 concluída** (229 testes): TUI com `SubagentPanel` (aberto no ToolStart de `task`, roteado por `parent_session_id`, finalizado no ToolEnd com summary), render compacto no transcript (label + últimas 3 tool lines enquanto roda), CLI com prefixo `[sub#n]` e supressão de streaming da child. 2 testes novos. Nota: painel casado por child_session_id (ToolStart do task não carrega tool_id no painel — painel mais recente não finalizado recebe a child). |
| 2026-09-04 | **F4.2 concluída** (227 testes): batch `tasks: [...]` com JoinSet + semáforo (MAX_PARALLEL_TASKS=4), resultados agregados por task com budget (4000/task, 8000 total), hint de paralelismo no system prompt do build. 4 testes novos (ordem, semáforo, abort, shape single). |
| 2026-09-04 | **F4.1 concluída** (223 testes verdes): eventos com `parent_session_id`, `SubagentRunner::run_task(events) -> TaskOutcome`, `ToolContext.events`, `TaskRunner` preserva child + seta `parent_id`, coluna `parent_id` com migração idempotente, `delete_children_of` ligado ao `delete_session` (F4.4.1 adiantada). 4 testes novos. |
