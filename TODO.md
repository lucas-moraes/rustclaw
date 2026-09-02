# RustClaw TUI — ratatui + crossterm

> Entregar a **TUI completa** sobre o harness existente (SessionRuntime / processor / tools).
> Stack: **ratatui + crossterm**. CLI print-only vira fallback opcional.
> Branch: `feat/opencode` / `feat/tui`

---

## Visão geral das features

| ID | Feature | Fase | Status |
|----|---------|------|--------|
| T0 | Deps + scaffold `ui/tui/` + entry main | 0 | ✅ |
| T1 | Abort handle + Ctrl+C cancela run | 1 | ✅ |
| T2 | Transcript event-driven + scroll + cores + status | 2 | ✅ |
| T3 | Input buffer + histórico + slash commands | 3 | ✅ |
| T4 | Permission / question modals (async oneshot) | 4 | ✅ |
| T5 | Diff view (edit/write metadata + painel) | 5 | ✅ |
| T6 | Polish terminal lifecycle + help + mouse scroll | 6 | ✅ |
| T7 | Testes + docs + TODO/README/ARCHITECTURE | 7 | ✅ |

**Legenda:** ⬜ pendente · 🟡 em progresso · ✅ feito · ❌ cancelado

---

## Objetivo do produto

Substituir o streaming CLI (`print!` + rustyline) por TUI com:

- layout em painéis (header / transcript / status / input)
- transcript scrollável + cores
- input (multiline)
- permission modal + question modal
- Ctrl+C cancela o run ativo
- status bar rica
- diff view para `edit` / `write`

**Non-negotiable:** mesmo `SessionRuntime` / processor / tools — só a surface UI muda.

### Layout alvo

```text
┌─ header: agent · model · cwd · session id ─────────────────────┐
│ transcript (scroll)                                             │
│  user / assistant / tool cards / diffs / errors                 │
├─ status: running? · iterations · tools · mem · agent ───────────┤
│ › input (multiline via Alt+Enter or trailing \)                 │
└─ help: Ctrl+C cancel · Esc close modal · /help ─────────────────┘
```

Modal overlay (permission/question): centro da tela.

---

## T0 — Deps + scaffold + entry

**Objetivo:** TUI compila e sobe alternate screen (mesmo que vazia).

### Feature: Dependências

- [x] Adicionar `ratatui` no `Cargo.toml`
- [x] Adicionar `crossterm` no `Cargo.toml`
- [x] (Opcional) `unicode-width` se necessário para cursor/input
- [x] `cargo check` verde com deps novas

### Feature: Scaffold de módulos

- [x] Criar `src/harness/ui/tui/mod.rs` com `pub async fn run(...)`
- [x] Criar `src/harness/ui/tui/app.rs` — `App` state + `apply_event`
- [x] Criar `src/harness/ui/tui/draw.rs` — `draw(frame, &app)`
- [x] Criar `src/harness/ui/tui/input.rs` — key handling
- [x] Criar `src/harness/ui/tui/askers.rs` — askers TUI (stub ok)
- [x] Atualizar `src/harness/ui/mod.rs` → `pub mod tui;` (+ `cli` mantido)

### Feature: Entry point

- [x] `main.rs` chama TUI por default (`harness::ui::tui::run`)
- [x] Fallback CLI: `RUSTCLAW_UI=cli` **ou** flag `--ui cli`
- [x] Auto-fallback: se `!stdout.is_terminal()` → CLI (piped/CI)
- [x] Smoke: `cargo run` entra em alternate screen e sai com `q` / Ctrl+C idle

### Definition of done T0

- [x] `cargo check` / `cargo test` passam
- [x] TUI sobe e restaura terminal ao sair (mesmo layout mínimo)
- [x] CLI ainda acessível via env/flag

---

## T1 — Abort handle + Ctrl+C

**Objetivo:** cancelar run ativo sem matar o processo.

### Feature: Abort exposto no runtime

- [x] `SessionRuntime::prompt` aceita `abort: AbortSignal` (caller owns)
  ```rust
  pub async fn prompt(
      &self,
      session: &mut Session,
      events: &EventSender,
      user_text: &str,
      abort: AbortSignal,
  ) -> Result<PromptResult>
  ```
- [x] Remover criação “escondida” de `AbortSignal` só dentro de `prompt` (ou manter default + overload)
- [x] Smoke test / callers atualizados para passar `AbortSignal::new()`
- [x] Processor continua checando `ctx.abort.is_aborted()` entre iterações

### Feature: Cancel mais responsivo (opcional mas recomendado)

- [x] Checar abort no loop de stream LLM (entre `ProviderEvent`s)
- [x] Checar abort entre spawns de tools / no `JoinSet` wait
- [x] bash/write/edit já checam no início — manter

### Feature: Ctrl+C na TUI

- [x] Durante `Running`: Ctrl+C → `abort.abort()` + status “cancelling…”
- [x] Mensagem no transcript: run aborted by user
- [x] Idle: Ctrl+C → quit (ou double-Ctrl+C / confirm) — definir UX:
  - [x] **Recomendado:** 1º Ctrl+C idle = quit limpo; durante run = só cancel
- [x] Não deixar terminal raw se panic no meio do cancel (ver T6)

### Definition of done T1

- [x] Ctrl+C mid-turn para o processor e libera a UI
- [x] Turno seguinte funciona após cancel
- [x] Teste unitário ou smoke documentado do abort path

---

## T2 — Transcript + scroll + cores + status

**Objetivo:** eventos do harness viram UI tipada e legível.

### Feature: Modelo de transcript

- [x] `enum LineKind { User, Assistant, Reasoning, Tool, System, Error, Diff }`
- [x] `struct TranscriptLine { kind, text, meta? }`
- [x] Buffer streaming do assistant (TextDelta concatena na última linha Assistant)
- [x] `App::apply_event(HarnessEvent)` puro (testável sem terminal)

### Feature: Consumo de eventos no loop TUI

- [x] **Não** usar task `print_events` + stdout na TUI
- [x] Drain `EventReceiver` a cada tick do loop (non-blocking / try_recv)
- [x] Mapear:
  - [x] `UserMessage` / prompt local → linha User
  - [x] `TextDelta` → Assistant streaming
  - [x] `ReasoningDelta` → Reasoning (dim/italic se suportado)
  - [x] `ToolStart` → card “● name args…”
  - [x] `ToolEnd` → “✓/✗ title” (+ preview)
  - [x] `Error` → vermelho
  - [x] `CompactionStarted/Finished` → system/dim
  - [x] `RunStarted/Finished` → status

### Feature: Scroll

- [x] `scroll_offset` no App
- [x] PgUp / PgDn (e opcional ↑/↓ com modifier)
- [x] Auto-scroll só se usuário já está no fundo (“stick to bottom”)
- [x] Ao novo conteúdo no fundo, manter stick; se scrollou pra cima, não pular

### Feature: Cores / estilo

- [x] User: cyan/blue
- [x] Assistant: default/white
- [x] Tool running: yellow
- [x] Tool ok: green · error: red
- [x] System/compaction: dark gray
- [x] Error: red bold
- [x] Header/status: inverted ou borda ratatui

### Feature: Status bar

- [x] agent · model
- [x] estado: `Idle` | `Running` | `Waiting permission` | `Waiting question` | `Cancelling`
- [x] última tool (nome)
- [x] iterations do último turn (após `PromptResult`)
- [x] memory count (se memory enabled)
- [x] cwd truncado
- [x] Atualizar a cada evento + fim do turn

### Feature: Header

- [x] agent · model · cwd · session id (curto)
- [x] Título “RustClaw”

### Definition of done T2

- [x] Prompt simples mostra stream de texto + tool cards coloridos
- [x] Scroll PgUp/PgDn funciona; stick-to-bottom correto
- [x] Status reflete Idle/Running
- [x] Testes unitários de `apply_event` (crescimento transcript + stickiness)

---

## T3 — Input + histórico + slash commands

**Objetivo:** substituir rustyline na TUI por input controlado pelo App.

### Feature: Input buffer

- [x] Buffer local + posição de cursor
- [x] Render na área inferior (`› …`)
- [x] Enter envia (se não multiline pendente)
- [x] Multiline: linha terminando em `\` **ou** Alt+Enter
- [x] Backspace / Left / Right / Home / End
- [x] Paste básico (se crossterm BracketedPaste disponível — stretch)

### Feature: Histórico

- [x] Histórico em memória (Vec) por sessão de processo
- [x] ↑ / ↓ navega histórico
- [x] Não persistir em disco na v1 TUI (ok documentar)

### Feature: Slash commands compartilhados

- [x] Extrair lógica de `handle_slash` para `ui/commands.rs` (CLI + TUI)
- [x] Comandos: `/help` `/new` `/sessions` `/resume` `/agent` `/compact` `/memory` `/exit` `/quit`
- [x] Feedback no transcript (system lines), não `println!`
- [x] `/exit` restaura terminal e encerra

### Feature: Dispatch de prompt

- [x] Enter com texto não-vazio + Idle → inicia turn
- [x] Spawn `JoinHandle` do `runtime.prompt(...)` para **não** bloquear draw loop
- [x] App guarda `running: Option<JoinHandle<…>>` + `AbortSignal` do turn
- [x] Ao completar: iterations na status, clear running, stick scroll

### Definition of done T3

- [x] Digitar prompt + Enter roda o agent na TUI
- [x] Multiline e histórico básicos funcionam
- [x] Slash commands equivalentes ao CLI
- [x] UI continua responsiva durante o run (draw loop vivo)

---

## T4 — Permission / question modals (async)

**Objetivo:** HITL sem bloquear stdin nem o event loop.

### Feature: Canais oneshot

- [x] Tipo `PendingPermission { request, reply: oneshot::Sender<PermissionReply> }`
- [x] `PermissionReply`: Allow | Deny | Always
- [x] Tipo `PendingQuestion { question, options, reply: oneshot::Sender<Option<String>> }`
- [x] App: `pending_permission: Option<…>`, `pending_question: Option<…>`

### Feature: TuiPermissionAsker

- [x] `ask()` envia pedido para a App (mpsc) e **await** oneshot
- [x] **Nunca** chamar `stdin.lines()` na TUI
- [x] Emitir `HarnessEvent::PermissionAsk` (transcript/status)
- [x] Ao resolver: emitir `PermissionResolved` + set_always_allow se Always

### Feature: TuiUserAsker

- [x] Modal com pergunta + options numeradas
- [x] Free text: focar input do modal ou reutilizar input bar
- [x] Esc / empty → `None` (user did not answer)

### Feature: Modal UI + teclas

- [x] Overlay central (Clear + Block bordered)
- [x] Permission: mostrar tool, path, args summary
- [x] Teclas: `y` / Enter = Allow · `n` / Esc = Deny · `a` = Always
- [x] Question: `1..n` escolhe option · digitar + Enter = free text
- [x] Enquanto modal aberto: não enviar prompt global; status = Waiting permission/question
- [x] Esc fecha modal com Deny / None

### Feature: Integração processor

- [x] `check_permission` continua chamando `asker.ask` (sem mudança de contrato além do asker)
- [x] Garantir que eventos Permission* chegam ao bus (asker ou runtime)

### Definition of done T4

- [x] `write`/`bash` abrem modal; `y` executa, `n` devolve erro ao model, `a` libera session
- [x] `question` tool abre modal e devolve resposta
- [x] Draw loop não trava durante wait do asker
- [x] CLI fallback mantém CliAsker antigo (sem regressão)

---

## T5 — Diff view (edit / write)

**Objetivo:** mutações de arquivo visíveis como diff no transcript/painel.

### Feature: Metadata nas tools

- [x] `edit`:
  - [x] Ler conteúdo before
  - [x] Aplicar replace
  - [x] Gerar snippet/unified diff (contexto ~3 linhas)
  - [x] `ToolResult.metadata`: `{ path, unified_diff?, before_snippet?, after_snippet?, truncated? }`
- [x] `write`:
  - [x] Se arquivo existia: before = conteúdo antigo
  - [x] after = content escrito
  - [x] diff ou preview (created N lines / first lines)
  - [x] metadata análoga
- [x] Truncar diffs grandes (ex. max 200 linhas) + flag `truncated: true`

### Feature: Diff helper

- [x] Função pura `unified_diff(before, after, path, context_lines) -> String` (sem crate extra na v1)
- [x] Testes unitários: single hunk edit, arquivo novo, truncate

### Feature: UI do diff

- [x] `ToolEnd` com metadata de diff → `LineKind::Diff` no transcript **ou** painel lateral
- [x] Cores: linhas `-` vermelho · `+` verde · hunk header dim
- [x] Tecla `d`: expand/collapse último diff mutável (se collapsado por default)
- [x] Não explodir layout: max height / scroll interno do bloco diff

### Definition of done T5

- [x] Após `edit` bem-sucedido, usuário vê diff colorido na TUI
- [x] `write` de arquivo novo mostra preview/criação clara
- [x] Diffs enormes truncam com aviso
- [x] Testes do helper de diff passam

---

## T6 — Polish: lifecycle, help, mouse

**Objetivo:** TUI “produto” — não deixa terminal quebrado; UX de teclas clara.

### Feature: Terminal lifecycle

- [x] `enable_raw_mode` + `EnterAlternateScreen` + mouse capture (se scroll mouse)
- [x] Restore no exit path normal
- [x] Restore em panic (Drop guard / `std::panic::set_hook` ou scopeguard)
- [x] `LeaveAlternateScreen` + `disable_raw_mode` + show cursor

### Feature: Help overlay

- [x] Tecla `?` ou F1: overlay com atalhos
- [x] Listar: Ctrl+C, Enter, PgUp/Dn, y/n/a, d (diff), /commands, q quit idle

### Feature: Mouse scroll (stretch recomendado)

- [x] Wheel up/down no transcript ajusta `scroll_offset`
- [x] Desabilitar se causar ruído em alguns terminais (feature-detect / flag)

### Feature: Robustez

- [x] Resize terminal (crossterm Resize event) → redraw full
- [x] Não panicar em draw se área < mínimo (mostrar “terminal too small”)
- [x] Fallback `--ui cli` / non-TTY documentado no help da TUI

### Definition of done T6

- [x] Sair sempre restaura terminal (testar quit, cancel, erro)
- [x] Help overlay legível
- [x] Resize não corrompe UI
- [x] Mouse scroll funciona **ou** documentado como não suportado

---

## T7 — Testes + docs

**Objetivo:** fechar a feature com qualidade e documentação.

### Feature: Testes

- [ ] Unit: `App::apply_event` — TextDelta concat, ToolEnd card, Error line
- [ ] Unit: stick-to-bottom vs scroll manual
- [ ] Unit: `unified_diff` helper (edit hunk, empty before, truncate)
- [ ] Unit: PermissionReply mapping (se lógica pura extraída)
- [ ] Smoke manual documentado: TUI sobe → glob/read → write pede modal → Ctrl+C cancela
- [ ] `cargo test` verde; `cargo fmt --check`; clippy sem erros novos

### Feature: Docs

- [ ] `TODO.md` desta TUI: marcar T0–T7 conforme avanço
- [x] `README.md`: seção TUI (atalhos, `--ui cli`, requirements TTY)
- [x] `docs/ARCHITECTURE.md`: diagrama surface TUI + abort + askers async
- [x] `AGENTS.md`: estrutura `ui/tui/*`, padrão de eventos, não usar rustyline na TUI
- [x] Remover nota “TUI fora de escopo v1” onde ainda existir

### Definition of done T7

- [x] Docs refletem TUI default
- [ ] Checklist T0–T6 comprovados por testes e/ou smoke
- [ ] Harness v1 + TUI considerada entregue

---

## Definition of done — TUI completa (global)

- [ ] `cargo run` abre TUI ratatui (alternate screen)
- [ ] Transcript scrollável com cores (user/assistant/tool/error/diff)
- [ ] Input + slash commands funcionam
- [ ] Permission modal y/n/a; question modal
- [ ] Ctrl+C cancela run ativo (`AbortSignal`); idle sai limpo
- [ ] edit/write mostram diff (ou preview estruturado)
- [ ] Status bar: agent/model/cwd/run state/iterations/memory
- [ ] Terminal restaurado ao sair (incl. panic path razoável)
- [ ] Testes unitários app state + diff helper
- [ ] CLI fallback para non-TTY / `--ui cli`
- [ ] TODO + README + ARCHITECTURE + AGENTS atualizados

---

## Ordem de execução

```text
T0 deps + scaffold + main switch
 → T1 abort handle + Ctrl+C
 → T2 transcript + scroll + cores + status
 → T3 input + history + slash + spawn prompt
 → T4 async permission/question modals
 → T5 edit/write diff metadata + UI
 → T6 lifecycle polish + help + mouse/resize
 → T7 tests + docs
```

Cada fase: `cargo test` + `cargo check` (+ smoke TUI manual quando UI mudar).

---

## Riscos e mitigações

| Risco | Mitigação | Status |
|-------|-----------|--------|
| rustyline + ratatui brigam pelo TTY | TUI sem rustyline; CLI opcional | ⬜ |
| Asker bloqueia event loop | oneshot + modal; zero `stdin.lines()` na TUI | ⬜ |
| Cancel mid-stream LLM | abort entre iterações já existe; checar no stream (T1) | ⬜ |
| Diff gigante | truncate + `truncated: true` (T5) | ⬜ |
| CI / piped stdin | `!is_terminal()` → CLI; `--ui cli` | ⬜ |
| Terminal stuck raw após panic | Drop guard / panic hook (T6) | ⬜ |
| Processor não emite PermissionAsk | emitir no asker TUI ou no check_permission | ⬜ |
| `prompt` esconde AbortSignal | signature com `abort:` (T1) | ⬜ |

---

## Fora de escopo desta entrega

- [ ] Mouse click em widgets complexos (além de wheel scroll)
- [ ] Multi-session side-by-side
- [ ] Tema configurável / full `opencode.json`
- [ ] Desktop app
- [ ] Reintroduzir Telegram / plugins / LSP
- [ ] Bracketed paste avançado (stretch em T3 se sobrar tempo)

---

## Arquivos principais a tocar

| Path | Mudança |
|------|---------|
| `Cargo.toml` | +ratatui, +crossterm |
| `src/main.rs` | default TUI + flag/env UI |
| `src/harness/ui/mod.rs` | mod tui |
| `src/harness/ui/tui/*` | **novo** app/draw/input/askers |
| `src/harness/ui/commands.rs` | **novo** slash compartilhado |
| `src/harness/ui/cli.rs` | fallback; usar commands compartilhados |
| `src/harness/runtime.rs` | `prompt(..., abort)` |
| `src/harness/session/processor.rs` | abort no stream (T1) |
| `src/harness/tool/edit.rs` / `write.rs` | metadata diff (T5) |
| `src/harness/event.rs` | garantir Permission* usados |
| `README.md` / `docs/ARCHITECTURE.md` / `AGENTS.md` | T7 |

---

## Notas de progresso

| Data | Nota |
|------|------|
| 2026-08-29 | Plano TUI completa (ratatui+crossterm) convertido em features T0–T7; TODO.md recriado do zero (conteúdo anterior do harness F0–F7 substituído). |
| 2026-08-29 | T0–T6 implementados: TUI (app/draw/input/askers), abort+Ctrl+C, transcript scroll+cores+status, input+histórico+slash compartilhado, modals async, diff (edit/write metadata + helper), lifecycle+mouse+help. T7 concluído: 62 testes verdes (incl. diff helper + edit diff) + smoke live CLI; docs (README/ARCHITECTURE/AGENTS) atualizados. |
