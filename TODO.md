# RustClaw — Dívida Técnica & Features (3ª rodada: quebrar `app.rs`)

> **Problema:** o `app.rs` (3.167 linhas) concentra 7 responsabilidades sem relação
> entre si: estado (48 campos `pub`), eventos, skills, contabilidade de custo,
> event loop, dispatch de teclas e undo/revert. É o maior arquivo do projeto e o
> mais difícil de navegar/refatorar. As rodadas anteriores (D1–D10, E1–E10) pagaram
> os juros de config, retry, segurança e observabilidade; esta rodada **A1–A10**
> quebra o monólito da UI em módulos coesos.
>
> **Escopo:** 3 tiers. Tier 1 = setup + cortes seguros (state, usage, undo).
> Tier 2 = cortes grandes (events, skills, pickers, keys, runner). Tier 3 = testes,
> docs e verificação final.
>
> **Decisões-chave:**
> - Cada feature tem checklist + Definition of Done (`cargo test` + `cargo clippy`
>   + `cargo fmt --check` verdes).
> - Refactors são **incrementais**: um bloco coeso por vez, mantendo os 296 testes
>   verdes a cada passo.
> - Nenhuma mudança de comportamento visível ao usuário (só estrutura interna).
> - `git mv` para preservar history/blame; mover blocos inteiros sem reformatar.
> - `mod.rs` re-exporta tudo com `pub use` — os 9 imports de `draw/` não mudam.

---

## Visão geral das features

| ID | Feature | Tier | Status |
|----|---------|------|--------|
| A0 | Setup: `git mv app.rs → app/mod.rs` + criar este TODO | 1 | ⬜ |
| A1 | Extrair `app/state.rs` (App + Modal + 4 picker states) | 1 | ⬜ |
| A2 | Extrair `app/usage.rs` (contabilidade de custo) | 1 | ⬜ |
| A3 | Extrair `app/undo.rs` (undo/revert + helpers) | 1 | ⬜ |
| A4 | Extrair `app/skills.rs` (toggles + pickers + comando) | 2 | ⬜ |
| A5 | Extrair `app/events.rs` (apply_event + transcript rebuild) | 2 | ⬜ |
| A6 | Extrair `app/keys.rs` (handle_key + handle_modal_key) | 2 | ⬜ |
| A7 | Extrair `app/pickers.rs` (key handlers dos modais) | 2 | ⬜ |
| A8 | Extrair `app/runner.rs` (run_tui + submit_input + TerminalGuard) | 2 | ⬜ |
| A9 | Extrair `app/tests.rs` (input_tests + code_block_tests) | 3 | ⬜ |
| A10 | Sync docs (AGENTS.md, ARCHITECTURE.md) + verificação final + commit | 3 | ⬜ |

**Legenda:** ⬜ pendente · 🟡 em progresso · ✅ feito · ❌ cancelado

---

## Estrutura alvo

```
tui/app/
├── mod.rs        (~120 ln)  — pub use re-exports, MODES, doc do módulo
├── state.rs      (~380 ln)  — App struct + Modal + 4 picker states + tests
├── events.rs     (~290 ln)  — apply_event, transcript rebuild, streaming
├── skills.rs     (~200 ln)  — toggles, pickers, handle_skills_command
├── usage.rs      (~90 ln)   — record_usage, session_cost, usage_report
├── undo.rs       (~120 ln)  — undo_last_turn, revert_to_prompt, mark_for
├── keys.rs       (~530 ln)  — handle_key, handle_modal_key, paste_clipboard
├── pickers.rs    (~460 ln)  — 3 picker key handlers + settings/persist helpers
├── runner.rs     (~570 ln)  — run_tui, submit_input, TerminalGuard
└── tests.rs      (~290 ln)  — input_tests, code_block_tests
```

**Total:** ~3.150 linhas distribuídas em 10 arquivos, máximo ~570 ln por arquivo.

---

## Tier 1 — Setup + cortes seguros

### Feature A0: Setup do módulo `app/`

**Onde:** `src/harness/ui/tui/app.rs` → `src/harness/ui/tui/app/mod.rs`

**Problema:** o arquivo é um monólito; precisa virar um diretório de módulos antes
de qualquer extração.

- [ ] `git mv src/harness/ui/tui/app.rs src/harness/ui/tui/app/mod.rs`
      (preserva blame/history)
- [ ] Criar este `TODO.md` (3ª rodada, convenção A1–A10)
- [ ] `cargo test` baseline verde (296 testes) antes de qualquer corte
- [ ] Confirmar que `tui/mod.rs` (`pub mod app;`) continua compilando com o diretório

**DoD:** `cargo test` + `cargo clippy` + `cargo fmt --check` verdes; blame preservado.

---

### Feature A1: Extrair `app/state.rs`

**Onde:** linhas 32–373 de `app.rs` (struct `App` + `Modal` + 4 picker states + impls)

**Problema:** o estado da UI (48 campos `pub`) e os 5 tipos de overlay/modal vivem
juntos com toda a lógica. Os `draw/` importam `app::App`, `app::Modal`,
`app::ModelPickerState`, `app::AuthPromptState` — precisam continuar funcionando.

- [ ] Mover `pub struct App` (linhas 32–102) para `state.rs`
- [ ] Mover `pub enum Modal` (105) para `state.rs`
- [ ] Mover `SkillPickerState` + impl (123–186) para `state.rs`
- [ ] Mover `ModelPickerState` + `AddProviderForm` + impls (188–283) para `state.rs`
- [ ] Mover `AuthPromptState` + impl (285–308) para `state.rs`
- [ ] Mover `ResumePickerState` + impl (310–373) para `state.rs`
- [ ] Mover `resume_picker_tests` (3105–3167) para `state.rs`
- [ ] `mod.rs` re-exporta: `pub use state::{App, Modal, SkillPickerState, ...}`
- [ ] Atualizar os 9 imports de `draw/` se necessário (idealmente zero mudança via re-export)
- [ ] `App::new`/`inline_for_tests` continuam em `state.rs` (ou ficam no `mod.rs`)

**DoD:** `cargo test` verde; `draw/` compila sem mudança de call site.

---

### Feature A2: Extrair `app/usage.rs`

**Onde:** linhas 1111–1180 de `app.rs` (impl App: contabilidade de custo)

**Problema:** a contabilidade de tokens/custo é um concern isolado, sem dependências
externas — o corte mais seguro para validar o padrão de extração.

- [ ] Mover `record_usage` (1111) para `usage.rs`
- [ ] Mover `reset_usage` (1117) para `usage.rs`
- [ ] Mover `context_tokens` (1123) para `usage.rs`
- [ ] Mover `max_context_tokens` (1127) para `usage.rs`
- [ ] Mover `session_cost` (1132) para `usage.rs`
- [ ] Mover `last_cost` (1142) para `usage.rs`
- [ ] Mover `usage_report` (1152) para `usage.rs`
- [ ] `impl App` parcial em `usage.rs` (Rust permite múltiplos `impl App` em módulos)
- [ ] `mod.rs` re-exporta o que for necessário

**DoD:** `cargo test` verde; `sidebar.rs` (`app.session_cost()`) continua funcionando.

---

### Feature A3: Extrair `app/undo.rs`

**Onde:** linhas 2507–2600 de `app.rs` (undo/revert + helpers)

**Problema:** `undo_last_turn`/`revert_to_prompt` falam direto com `SessionStore`
da UI — um acoplamento que deve ser documentado como dívida para um futuro service
layer, mas que hoje pode ser isolado num módulo próprio.

- [ ] Mover `user_prompt_text` (2507) para `undo.rs`
- [ ] Mover `mark_for` (2522) para `undo.rs`
- [ ] Mover `undo_last_turn` (2532) para `undo.rs`
- [ ] Mover `revert_to_prompt` (2563) para `undo.rs`
- [ ] Adicionar comentário `// TODO(service-layer): UI não deveria falar com SessionStore`
      documentando a dívida
- [ ] `mod.rs` re-exporta as funções usadas por `keys.rs`/`runner.rs`

**DoD:** `cargo test` verde; comportamento de `/undo` inalterado.

---

## Tier 2 — Cortes grandes

### Feature A4: Extrair `app/skills.rs`

**Onde:** linhas 930–1110 de `app.rs` (toggles + pickers + comando)

**Problema:** a lógica de skills (toggles por turno, picker, comando `/skills`) é um
concern coeso de ~200 linhas misturado com o resto.

- [ ] Mover `sync_prompt_toggles` (930) para `skills.rs`
- [ ] Mover `toggle_prompt_skill` (944) para `skills.rs`
- [ ] Mover `cycle_focus` (953) para `skills.rs`
- [ ] Mover `enabled_skill_ids` (958) para `skills.rs`
- [ ] Mover `apply_skill_picker` (976) para `skills.rs`
- [ ] Mover `open_skill_picker` (987) para `skills.rs`
- [ ] Mover `handle_skills_command` (998–1110, 111 ln) para `skills.rs`
- [ ] `mod.rs` re-exporta o que `keys.rs`/`runner.rs` precisam

**DoD:** `cargo test` verde; `/skills` inalterado.

---

### Feature A5: Extrair `app/events.rs`

**Onde:** linhas 604–872 de `app.rs` (apply_event + transcript rebuild + streaming)

**Problema:** o processamento de eventos do harness (`apply_event`, 135 ln) e a
reconstrução do transcript são lógica de estado pura, separável do loop.

- [ ] Mover `finish_tool_batch` (604) para `events.rs`
- [ ] Mover `apply_event` (617–751, 135 ln) para `events.rs`
- [ ] Mover `flush_streaming` (752) para `events.rs`
- [ ] Mover `add_user_prompt` (760) para `events.rs`
- [ ] Mover `add_system` (764) para `events.rs`
- [ ] Mover `cancel_running_turn` (771) para `events.rs`
- [ ] Mover `scroll_by`/`clamp_scroll`/`clear_transcript` (788–818) para `events.rs`
- [ ] Mover `rebuild_transcript_from_session` (819–872, 54 ln) para `events.rs`
- [ ] `mod.rs` re-exporta o que `runner.rs`/`keys.rs` precisam

**DoD:** `cargo test` verde; streaming/transcript inalterados.

---

### Feature A6: Extrair `app/keys.rs`

**Onde:** linhas 1527–2313 de `app.rs` (handle_key + handle_modal_key + paste_clipboard)

**Problema:** `handle_key` tem **787 linhas** — a maior função do projeto. Precisa
sair do monólito; o corte é delicado porque chama `submit_input` (runner).

- [ ] Mover `handle_key` (1527–2313, 787 ln) para `keys.rs`
- [ ] Mover `handle_modal_key` (2314–2506, 190 ln) para `keys.rs`
- [ ] Mover `paste_clipboard` (1847) para `keys.rs`
- [ ] Resolver dependência com `submit_input`: `keys.rs` importa de `runner.rs`
      (uma direção só, sem ciclo)
- [ ] `mod.rs` re-exporta `handle_key` para `runner.rs`
- [ ] Verificar que todos os `use` de `crossterm::event` estão no escopo do módulo

**DoD:** `cargo test` verde; navegação por teclado inalterada.

---

### Feature A7: Extrair `app/pickers.rs`

**Onde:** linhas 1854–2313 de `app.rs` (key handlers dos modais + settings/persist)

**Problema:** os handlers de teclado dos 3 pickers (skill/model/auth) + helpers de
persistência de provider/model formam um concern coeso de ~460 linhas.

- [ ] Mover `handle_skill_picker_key` (1854) para `pickers.rs`
- [ ] Mover `handle_settings_command` (1878) para `pickers.rs`
- [ ] Mover `persist_custom_model` (1942) para `pickers.rs`
- [ ] Mover `remove_from_user_store` (1957) para `pickers.rs`
- [ ] Mover `handle_model_picker_key` (1990–2252, 263 ln) para `pickers.rs`
- [ ] Mover `handle_auth_prompt_key` (2253–2313) para `pickers.rs`
- [ ] `mod.rs` re-exporta o que `keys.rs` precisa

**DoD:** `cargo test` verde; `/models`, `/auth`, `/settings` inalterados.

---

### Feature A8: Extrair `app/runner.rs`

**Onde:** linhas 1181–1510 + 2601–2824 de `app.rs` (run_tui + submit_input + TerminalGuard)

**Problema:** o event loop (`run_tui`, 330 ln) e o `submit_input` (223 ln) são o
"motor" da TUI; `TerminalGuard` é o lifecycle do terminal. Único ponto de entrada
externo: `tui/mod.rs` chama `app::run_tui`.

- [ ] Mover `run_tui` (1181–1510, 330 ln) para `runner.rs`
- [ ] Mover `TerminalGuard` + impl Drop (1511–1526) para `runner.rs`
- [ ] Mover `submit_input` (2601–2824, 223 ln) para `runner.rs`
- [ ] `tui/mod.rs` passa a chamar `app::runner::run_tui` (ou re-export via `mod.rs`)
- [ ] `mod.rs` re-exporta `run_tui` para `tui/mod.rs`
- [ ] Resolver dependência: `runner.rs` chama `keys::handle_key` (direção única)

**DoD:** `cargo test` verde; TUI inicia e roda normalmente.

---

## Tier 3 — Testes, docs e verificação

### Feature A9: Extrair `app/tests.rs`

**Onde:** linhas 2825–3167 de `app.rs` (input_tests + code_block_tests)

**Problema:** os testes de input/soft-wrap e code-block estão no fim do monólito;
devem viver num módulo de testes dedicado.

- [ ] Mover `input_tests` (2825–3050, 225 ln) para `tests.rs`
- [ ] Mover `code_block_tests` (3051–3104, 54 ln) para `tests.rs`
- [ ] `#[cfg(test)] mod tests` com `use super::*` — `mod.rs` re-exporta os itens testados
- [ ] `resume_picker_tests` já foi movido em A1 (não duplicar)
- [ ] Confirmar que todos os itens testados estão acessíveis via re-export

**DoD:** `cargo test` verde; 296+ testes passando.

---

### Feature A10: Sync docs + verificação final + commit

**Onde:** `AGENTS.md`, `docs/ARCHITECTURE.md`, `src/harness/ui/tui/mod.rs`

**Problema:** a árvore de arquivos em `AGENTS.md` e `docs/ARCHITECTURE.md` ainda
lista `app.rs` como arquivo único; precisa refletir o novo diretório `app/`.

- [ ] Atualizar a árvore em `AGENTS.md` (seção `ui/tui/`)
- [ ] Atualizar `docs/ARCHITECTURE.md` se mencionar `app.rs`
- [ ] `cargo test` — 296+ testes verdes
- [ ] `cargo clippy` — sem warnings
- [ ] `cargo fmt --check` — ok
- [ ] Smoke test manual da TUI (`cargo run`) — navegação, modais, `/undo`, `/skills`
- [ ] Commit com mensagem descritiva (ex.: `refactor: split app.rs into app/ modules`)

**DoD:** docs sincronizadas; build/test/clippy/fmt verdes; TUI funcional; commit feito.

---

## Ordem de execução recomendada

```text
A0 (setup) → A1 (state) → A2 (usage) → A3 (undo) → A4 (skills)
→ A5 (events) → A7 (pickers) → A6 (keys) → A8 (runner) → A9 (tests) → A10 (docs+commit)
```

**A2 e A3 são os cortes mais seguros** (menos dependências) — bons para validar o
padrão antes dos cortes grandes (A6/A7/A8).

---

## Riscos e mitigações

| Risco | Mitigação |
|-------|-----------|
| Quebrar os 9 imports de `draw/` (`app::App`, `app::Modal`, etc.) | `mod.rs` re-exporta tudo com `pub use` — zero mudança nos call sites |
| `handle_key` tem dependência circular com `submit_input` (runner) | `keys.rs` importa de `runner.rs` (uma direção só) |
| Testes usam `use super::*` | `mod.rs` re-exporta itens testados; testes movidos junto com o código |
| Blame do git perdido | `git mv` + mover blocos inteiros sem reformatar |
| `impl App` espalhado em vários módulos | Rust permite múltiplos `impl App`; cada módulo declara `impl App` parcial |
| Regressão de UI | Extração incremental, um bloco por vez, testes verdes a cada passo |

---

## Arquivos principais a tocar

| Path | Mudança |
|------|---------|
| `src/harness/ui/tui/app.rs` | `git mv` → `app/mod.rs` (A0); remover blocos extraídos (A1–A9) |
| `src/harness/ui/tui/app/state.rs` | Novo — App + Modal + picker states (A1) |
| `src/harness/ui/tui/app/usage.rs` | Novo — contabilidade de custo (A2) |
| `src/harness/ui/tui/app/undo.rs` | Novo — undo/revert (A3) |
| `src/harness/ui/tui/app/skills.rs` | Novo — toggles/pickers/comando (A4) |
| `src/harness/ui/tui/app/events.rs` | Novo — apply_event/transcript (A5) |
| `src/harness/ui/tui/app/keys.rs` | Novo — handle_key/modal_key (A6) |
| `src/harness/ui/tui/app/pickers.rs` | Novo — key handlers dos modais (A7) |
| `src/harness/ui/tui/app/runner.rs` | Novo — run_tui/submit_input/TerminalGuard (A8) |
| `src/harness/ui/tui/app/tests.rs` | Novo — input/code_block tests (A9) |
| `src/harness/ui/tui/mod.rs` | Chamar `app::runner::run_tui` (A8) |
| `AGENTS.md`, `docs/ARCHITECTURE.md` | Atualizar árvore de arquivos (A10) |

---

## Notas de progresso

| Data | Nota |
|------|------|
| 2026-09-08 | TODO.md substituído: rodadas D1–D10 e E1–E10 concluídas; novo plano A1–A10 para quebrar `app.rs` (3.167 ln) em 10 módulos. |
