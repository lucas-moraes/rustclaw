# RustClaw — Dívida Técnica & Features (2ª rodada)

> **Problema:** a 1ª rodada (TODO.md, D1–D10) pagou os "juros" de config, temperatura,
> dedup de compaction, cwd vs project_root e higiene/docs. Esta 2ª rodada foca em
> **robustez do loop do agente** (retry de provider), **segurança do `bash`**,
> **observabilidade** (tracing + persistência não-silenciosa) e **quebra do monólito
> `app.rs`** (D4 que ficou pendente), além de features de qualidade do agente.
>
> **Escopo:** 3 tiers de prioridade. Tier 1 = robustez/segurança (retry, bash, tracing).
> Tier 2 = quebrar `app.rs` + features de qualidade. Tier 3 = polish/UX.
>
> **Decisões-chave:**
> - Cada feature tem checklist + Definition of Done (`cargo test` + `cargo clippy`
>   + `cargo fmt --check` verdes).
> - Refactors são **incrementais**: um bloco coeso por vez, mantendo os 276 testes
>   verdes a cada passo.
> - Nenhuma mudança de comportamento visível ao usuário (só estrutura interna),
>   exceto onde a feature explicitamente adiciona comportamento novo (retry, sandbox).

---

## Visão geral das features

| ID | Feature | Tier | Status |
|----|---------|------|--------|
| E1 | Retry/backoff em erros transitórios do provider | 1 | ✅ |
| E2 | Segurança do `bash` (detecção por tokens + sudo Ask) | 1 | ✅ |
| E3 | Logging `tracing` + helper de persistência não-silenciosa | 1 | ✅ |
| E4 | Quebrar `app.rs` (3529 linhas) | 2 | ✅ |
| E5 | Tool `diagnostics` (cargo check --message-format=json) | 2 | ⬜ |
| E6 | Budget de output tokens por turno | 2 | ⬜ |
| E7 | Testes de integração do loop completo (MockProvider) | 2 | ⬜ |
| E8 | Painel "thinking" (reasoning) no TUI | 3 | ⬜ |
| E9 | Custo estimado na sidebar (estimativa $ por provider/model) | 3 | ✅ |
| E10 | Remover `expect()`/`unwrap()` de produção | 3 | ✅ |

**Legenda:** ⬜ pendente · 🟡 em progresso · ✅ feito · ❌ cancelado

---

## Tier 1 — Robustez e segurança (pagar juros de confiabilidade)

### Feature E1: Retry/backoff em erros transitórios do provider

**Onde:** `src/harness/session/processor.rs` (`run_turn`, linha ~136 `self.provider.stream(&req).await?`),
`src/harness/provider/openai.rs` (linhas 377-381), `src/harness/provider/anthropic.rs` (linhas 335-354),
`src/harness/session/compaction.rs` (linha 174 `provider.complete`).

**Problema:** qualquer erro HTTP transitório (429 rate limit, 500/503, timeout de rede)
aborta o turno inteiro com `?`. Só o MCP tem `reconnect+retry` (`mcp/mod.rs:207`). Um
rate-limit de 2s derruba um turno que já gastou tokens e iterações. A compaction
(`complete()`) também falha sem retry no meio do turno.

- [x] Definir um `RetryPolicy` (max_attempts, base_delay, max_delay, jitter) em
      `provider/mod.rs` ou novo `provider/retry.rs`
- [x] Classificar erros: retryável (429, 500, 502, 503, 504, timeout de rede) vs
      não-retryável (400, 401, 404, 422 — erro de request/model)
- [x] Respeitar `Retry-After` header quando presente (429/503)
- [x] Backoff exponencial com jitter (ex.: `base * 2^n + rand`)
- [x] Aplicar o retry no `run_turn` ao redor de `provider.stream()` (não dentro do
      provider, para manter o provider puro)
- [x] Aplicar o retry na compaction (`compaction.rs:174`) ao redor de `provider.complete()`
- [x] Emitir `HarnessEvent::Error`/warn no `tracing` quando um retry for acionado
      (visível ao usuário: "retrying in 2s (429)")
- [x] Não retryar em abort do usuário (`ctx.abort.is_aborted()`)
- [x] Testes: mock provider que falha 2x com 429 e depois responde; verificar que o
      turno completa; verificar que erro 400 não é retryado
- [x] `cargo test` + `cargo clippy` + `cargo fmt --check` verdes

### Feature E2: Segurança do `bash` (detecção por tokens + sudo Ask)

**Onde:** `src/harness/tool/bash.rs` (linhas 9-52, `check_denylist`).

**Problema:** o denylist bloqueia por substring (`"rm -rf /"`, `"mkfs"`, `"dd if="`),
o que é trivial de contornar (`rm -rf --no-preserve-root /`, `sudo dd if=...`). Não há
detecção por tokens, nem exigência de `Ask` para comandos privilegiados.

- [x] Reescrever `check_denylist` para detectar por **tokens** (split por whitespace)
      em vez de substring: ex. `rm` + flag `-rf`/`-r` + caminho absoluto/`/`
- [x] Cobrir variantes: `rm -rf --no-preserve-root /`, `rm -fr`, `rm -r /`
- [x] Manter os bloqueios de comandos de sistema (`shutdown`, `reboot`, `mkfs`, `dd if=`)
- [x] Adicionar detecção de `sudo`/`su` → exigir `Ask` (não bloquear, mas escalar permissão)
- [x] Adicionar detecção de redirecionamento destrutivo (`> /dev/sda`, `> /etc/...`)
- [ ] (Opcional, Linux) suporte a sandbox real via `bubblewrap`/`nix` configurável
- [x] Testes: `rm -rf --no-preserve-root /` bloqueado; `rm -rf ./target` permitido;
      `sudo apt install` → Ask; `ls -la` permitido
- [x] `cargo test` + `cargo clippy` + `cargo fmt --check` verdes

### Feature E3: Logging `tracing` + helper de persistência não-silenciosa

**Onde:** `src/harness/session/processor.rs` (padrão `let _ = self.events.send(...)` e
`let _ = self.store.save_message(...)` em ~linhas 269, 306, 361, 389, 415),
`src/main.rs` (setup do subscriber).

**Problema:** o crate `tracing` está no Cargo.toml mas quase não é usado. Persistência
e eventos usam `let _ =` que engole erros em silêncio — se o store falhar, o histórico
diverge do que o agente vê sem nenhum sinal.

- [x] Configurar um subscriber `tracing` no `main.rs` (formato `pretty` para dev,
      `json` opcional via env `RUSTCLAW_LOG`)
- [x] Criar helper `persist()`/`emit()` no processor que logue `warn!` em falha de
      `save_message`/`events.send` (em vez de `let _ =`)
- [x] Substituir os `let _ = self.store.save_message(...)` e `let _ = self.events.send(...)`
      do processor pelo helper
- [x] Adicionar `tracing::info!`/`debug!` nos pontos-chave do loop: início de turno,
      tool call executada, retry acionado, compaction, auto-continue, doom-loop
- [x] Não mudar comportamento visível (só logging)
- [x] Testes: verificar que o helper não quebra o fluxo normal; `cargo test` verde
- [x] `cargo test` + `cargo clippy` + `cargo fmt --check` verdes

---

## Tier 2 — Quebrar monólito + qualidade do agente

### Feature E4: Quebrar `app.rs` (3529 linhas)

**Onde:** `src/harness/ui/tui/app.rs`.

**Problema:** um único arquivo com ~3500 linhas misturando estado da App, `SubagentPanel`,
`ToolBatch`, `AutoComplete`, `SkillPickerState`, splash, partículas e editor de input.
É o maior arquivo do projeto e o mais difícil de navegar/refatorar. (D4 do TODO.md
ficou pendente.)

- [x] Extrair o **editor de input** (já há testes isolados) para módulo próprio
- [x] Extrair o **gerenciamento de subagentes** (`SubagentPanel`) para módulo próprio
- [x] Extrair o **`ToolBatch`/`ActiveTool`** (estado de tools em execução) para módulo próprio
- [x] Extrair o **`AutoComplete`** para módulo próprio
- [x] Extrair helpers de render/estado coesos (ex.: `last_code_block`, soft-wrap)
- [x] Fazer **incremental**: um bloco coeso por vez, mantendo os testes verdes
- [x] Não reduzir linhas por reduzir — separar responsabilidades
- [x] Testes: todos os testes de `app.rs` (editor, wrap, subagentes) continuam verdes
- [x] `cargo test` + `cargo clippy` + `cargo fmt --check` verdes

### Feature E5: Tool `diagnostics` (cargo check --message-format=json)

**Onde:** novo `src/harness/tool/diagnostics.rs`; registrar em `runtime.rs`
(`build_default_registry`), `agent/builtin.rs` (allowlist) e `permission/mod.rs` (Allow).

**Problema:** o loop depende do agente "adivinhar" erros de compile. Uma tool que roda
`cargo check --message-format=json` (ou `eslint`/`tsc`) e devolve erros estruturados por
arquivo/linha reduziria muito iterações desperdiçadas.

- [ ] Criar `DiagnosticsTool` que roda `cargo check --message-format=json` no cwd
- [ ] Parsear a saída JSON e devolver erros/warnings estruturados (arquivo, linha, coluna, mensagem)
- [ ] Suportar linguagem/ferramenta configurável (default `cargo check`; extensível)
- [ ] Truncar saída (reusar `truncate::truncate_output`)
- [ ] `read_only() = true`
- [ ] Registrar em `build_default_registry` + allowlist do agente + `PermissionEngine` (Allow)
- [ ] Testes: mock de `cargo check` (ou fixture JSON) → parse correto; `cargo test` verde
- [ ] `cargo test` + `cargo clippy` + `cargo fmt --check` verdes

### Feature E6: Budget de output tokens por turno

**Onde:** `src/harness/session/processor.rs` (`run_turn`, acumula `total_usage`),
`src/harness/runtime.rs` (`HarnessConfig`/`RuntimeConfig`).

**Problema:** hoje só há `max_iterations` e `max_context_tokens`. Um orçamento de
*output* acumulado (ex.: parar se o turno gastar >X output tokens) evita surpresas de
custo em loops longos.

- [ ] Adicionar `max_output_tokens` ao config (default alto, ex. 0 = ilimitado)
- [ ] No `run_turn`, acumular `total_usage.output_tokens` e parar quando exceder o budget
- [ ] Emitir `HarnessEvent::Error`/mensagem final explicando o limite atingido
- [ ] Expor no `/settings` (junto de `max_iterations`/`max_context_tokens`)
- [ ] Testes: mock provider que emite muitos tokens → turno para no budget
- [ ] `cargo test` + `cargo clippy` + `cargo fmt --check` verdes

### Feature E7: Testes de integração do loop completo (MockProvider)

**Onde:** `src/harness/session/processor.rs` (já há mocks em linhas 679, 778).

**Problema:** os 276 testes são majoritariamente unitários (parsers, tools, config).
Faltam testes que rodem `run_turn` com um `MockProvider` fake cobrindo os fluxos
críticos do loop.

- [ ] Teste: doom-loop → parada após N repetições
- [ ] Teste: watchdog de turno → auto-continue (e parada após `DEFAULT_MAX_CONTINUATIONS`)
- [ ] Teste: abort do usuário no meio de tool-call → turno para e marca tools como Error
- [ ] Teste: tool call paralela (múltiplas calls numa mensagem) → execução concorrente
- [ ] Teste: retry de provider (E1) → turno completa após falha transitória
- [ ] Teste: compaction disparada em overflow → resumo + persistência
- [ ] `cargo test` + `cargo clippy` + `cargo fmt --check` verdes

---

## Tier 3 — Polish de UX / higiene

### Feature E8: Painel "thinking" (reasoning) no TUI

**Onde:** `src/harness/ui/tui/app.rs`, `src/harness/ui/tui/draw/transcript.rs`,
`src/harness/session/processor.rs` (linha 199, `ReasoningDelta`).

**Problema:** o `ProviderEvent::ReasoningDelta` já é emitido e persistido, mas o TUI
trata de forma opaca. Um painel "thinking" colapsável (estilo DeepSeek/ChatGPT)
melhoraria a percepção de progresso em agentes com reasoning.

- [ ] Renderizar o reasoning como bloco colapsável no transcript
- [ ] Atalho de teclado para expandir/colapsar (ex.: `Tab` ou `r`)
- [ ] Não contar o reasoning como texto final (já é `Part::Reasoning` separado)
- [ ] Testes: render do reasoning colapsado/expandido
- [ ] `cargo test` + `cargo clippy` + `cargo fmt --check` verdes

### Feature E9: Custo estimado na sidebar (estimativa $ por provider/model)

**Onde:** `src/harness/ui/tui/draw/sidebar.rs`, `src/harness/provider/catalog.rs`,
`src/harness/ui/tui/app.rs` (`session_usage`/`last_usage`).

**Problema:** o `Usage` já acumula tokens por sessão, mas não há estimativa de custo.
Uma tabela $/1M por provider/model permitiria mostrar o gasto estimado.

**Decisão (2026-09-07):** implementado como **exibição na sidebar** (seção "Cost"
abaixo de CONTEXT), em vez de comando `/cost`.

- [x] Adicionar tabela de preço $/1M (input/output) por provider/model no catálogo
- [x] Seção "Cost" na sidebar abaixo de CONTEXT: total da sessão (bold) + último turno
- [x] Cálculo via `catalog::estimate_cost(provider, model, in, out)` × `session_usage`
- [x] Testes: `test_price_per_million_known_model`, `test_estimate_cost_and_format`
- [x] `cargo test` + `cargo clippy` + `cargo fmt --check` verdes

### Feature E10: Remover `expect()`/`unwrap()` de produção

**Onde:** `src/config.rs` (linhas 121, 146 `expect("opencode-go must exist in the catalog")`),
`src/harness/runtime.rs` (linhas 746, 769 `.expect("runtime")`/`.expect("prompt")`).

**Problema:** panics em produção se o catálogo mudar. O AGENTS.md já pede `anyhow!`
com contexto, mas há exceções.

- [x] Trocar `expect("opencode-go must exist in the catalog")` por `anyhow!` com contexto
- [x] Trocar `.expect("runtime")`/`.expect("prompt")` por `?`/`anyhow!`
- [x] Varrer o resto de `src/` (fora de `#[cfg(test)]`) por `unwrap()`/`expect()`/`panic!`
- [x] Testes: `cargo test` verde (resolução de config continua funcionando)
- [x] `cargo test` + `cargo clippy` + `cargo fmt --check` verdes

---

## Ordem de execução recomendada

```text
E1 (retry provider) → E2 (segurança bash) → E3 (tracing/persistência)
→ E10 (expect/unwrap) → E7 (testes de integração do loop)
→ E5 (diagnostics) → E6 (budget output) → E4 (app.rs, por último e incremental)
→ E8 (painel thinking) → E9 (/cost)
```

Cada feature: `cargo test` + `cargo clippy` + `cargo fmt --check` verdes.

---

## Riscos e mitigações

| Risco | Mitigação |
|-------|-----------|
| Retry (E1) mascara erro real ou duplica side-effect | Retry só em erros transitórios (429/5xx/timeout); nunca em 4xx de request; abort do usuário cancela retry |
| Sandbox do bash (E2) bloqueia comando legítimo | Detecção por tokens com allowlist de casos comuns; `sudo` vira Ask (não bloqueio) |
| `tracing` (E3) muda comportamento | Só logging; helper de persistência mantém o fluxo, apenas loga `warn!` em falha |
| Quebrar `app.rs` (E4) introduz regressão de UI | Extração incremental, um bloco por vez, testes de editor/wrap/subagentes verdes |
| `diagnostics` (E5) roda `cargo check` lento | Timeout + truncate; `read_only`; rodar sob demanda (não a cada turno) |
| Budget de output (E6) interrompe turno legítimo | Default alto/ilimitado; mensagem clara ao atingir; configurável no `/settings` |

---

## Arquivos principais a tocar

| Path | Mudança |
|------|---------|
| `src/harness/session/processor.rs` | Retry (E1), budget output (E6), tracing (E3), testes integração (E7) |
| `src/harness/provider/openai.rs` / `anthropic.rs` | Classificar erros retryáveis (E1) |
| `src/harness/session/compaction.rs` | Retry no `complete()` (E1) |
| `src/harness/tool/bash.rs` | Detecção por tokens + sudo Ask (E2) |
| `src/main.rs` | Subscriber `tracing` (E3) |
| `src/harness/ui/tui/app.rs` | ~~Quebrar monólito (E4)~~ ✅ → `editor.rs`/`subagent.rs`/`codeblock.rs`/`transcript.rs`; painel thinking (E8) |
| `src/harness/tool/diagnostics.rs` (novo) | Tool diagnostics (E5) |
| `src/harness/runtime.rs` | Registrar diagnostics (E5), config budget (E6), remover expect (E10) |
| `src/harness/agent/builtin.rs` | Allowlist diagnostics (E5) |
| `src/harness/permission/mod.rs` | Allow diagnostics (E5) |
| `src/harness/ui/commands/mod.rs` | Comando `/cost` (E9) |
| `src/harness/provider/catalog.rs` | Tabela de preços (E9) |
| `src/config.rs` | Remover expect (E10) |

---

## Notas de progresso

| Data | Nota |
|------|------|
| 2026-09-06 | TODO_2.md criado: 2ª rodada de dívida técnica + features (E1–E10) com checklists, tiers, ordem de execução e riscos. Foco em robustez (retry), segurança (bash), observabilidade (tracing) e qualidade do agente (diagnostics, budget, testes de integração). |
| 2026-09-07 | Implementado comando `/allow-all-permissions` (fora da lista E1–E10): concede ao harness liberdade total para alterar qualquer arquivo do projeto. `PermissionEngine::allow_all()` + `SessionRuntime::allow_all_permissions()` (persiste em rustclaw.json) + handler em `ui/commands/mod.rs`. Paths fora do projeto continuam exigindo aprovação. 2 testes novos (278 no total). |
| 2026-09-07 | **E4 concluído** — `app.rs` quebrado de 3529 → 3066 linhas. Módulos extraídos: `editor.rs` (input/cursor/história), `subagent.rs` (`SubagentPanel` + roteamento de eventos), `codeblock.rs` (`last_code_block`/copy/save), `transcript.rs` (`LineKind`/`TranscriptLine`/`ActiveTool`/`ToolBatch`/`tool_arg_label`/`preview`). `AutoComplete` já vivia em `palette.rs`. `app.rs` re-exporta os tipos via `pub use` para manter a API estável. `cargo test` (278) + `cargo clippy` + `cargo fmt --check` verdes. |
| 2026-09-07 | **E1 concluído** — retry/backoff de provider em `provider/retry.rs` (novo, 395 linhas). `RetryPolicy` (max_attempts, base_delay, max_delay, jitter), classificação de erros retryáveis (429/5xx/timeout) vs permanentes (4xx), respeito a `Retry-After`, backoff exponencial com jitter, aplicado em `run_turn` (stream) e `compaction` (complete), sem retry em abort do usuário. 7 testes novos. |
| 2026-09-07 | **E2 concluído** — `bash.rs` reescrito com detecção por **tokens** (tokenize com quotes/escapes) em vez de substring. Bloqueia `rm` destrutivo (`-rf`/`-r` + caminho absoluto/`/`, incl. `--no-preserve-root`), comandos de sistema (`shutdown`/`reboot`/`mkfs`/`dd if=`), redirecionamento destrutivo (`> /dev/sd*`, `> /etc/...`). `sudo`/`su` → escalam para `Ask` (não bloqueiam). 5 testes novos. |
| 2026-09-07 | **E3 concluído** — subscriber `tracing` em `main.rs` (`RUSTCLAW_LOG=json` → JSON, senão pretty; nível configurável). Helpers `persist()`/`emit()` no processor logam `warn!` em falha de `save_message`/`events.send` (substituindo `let _ =`). `tracing::info!`/`debug!` nos pontos-chave do loop (início de turno, tool call, retry, compaction, auto-continue, doom-loop). Deps: `tracing-subscriber` + `rand`. |
| 2026-09-07 | **E10 concluído** — removidos `expect()`/`unwrap()`/`panic!` de produção. `config.rs` (`expect("opencode-go must exist in the catalog")` → `anyhow!` com contexto), `runtime.rs` (`.expect("runtime")`/`.expect("prompt")` → `?`), `provider/mod.rs` + `web_search.rs` + `ui/tui/input.rs` (unwrap → `?`/`unwrap_or_else`), e ~36 `Mutex::lock().unwrap()` → `unwrap_or_else(|e| e.into_inner())` em `permission/mod.rs`, `project/memory.rs`, `runtime.rs`, `session/store.rs`, `tool/task.rs`. Varredura de `src/` confirma que só restam `unwrap()`/`expect()` em `#[cfg(test)]`. `cargo test` (288) + `cargo clippy` + `cargo fmt --check` verdes. |
| 2026-09-07 | **E9 concluído** — custo estimado exibido na **sidebar** (seção "Cost" abaixo de CONTEXT), conforme decisão do usuário (sem comando `/cost`). Tabela de preços $/1M (input/output) por provider/model + fallback por provider em `catalog.rs` (`price_per_million`, `estimate_cost`, `format_cost`). Método `App::session_cost()` calcula via `session_usage` × preço do modelo atual. `sidebar.rs::cost_block` mostra total da sessão (bold) + último turno (dim, quando há uso). 2 testes novos (290 no total). `cargo test` + `cargo clippy` + `cargo fmt --check` verdes. |
