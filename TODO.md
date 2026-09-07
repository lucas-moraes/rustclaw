# RustClaw — Dívida Técnica (refactor)

> **Problema:** o RustClaw cresceu rápido (MCP, skills, memória, subagentes, TUI)
> e acumulou dívidas de manutenção: camadas de config sobrepostas, duplicação de
> lógica, arquivos grandes e documentação desatualizada. Antes de adicionar novas
> features (checkpoints, export, plan→build), vale pagar os "juros".
>
> **Escopo:** 3 tiers de prioridade. Tier 1 = pagar juros (config, temperatura,
> dedup, cwd). Tier 2 = quebrar arquivos grandes. Tier 3 = higiene/docs.
>
> **Decisões-chave:**
> - Cada feature tem checklist + Definition of Done (`cargo test` + `cargo clippy`
>   + `cargo fmt --check` verdes).
> - Refactors são **incrementais**: um bloco coeso por vez, mantendo os 271 testes
>   verdes a cada passo.
> - Nenhuma mudança de comportamento visível ao usuário (só estrutura interna).

---

## Visão geral das features

| ID | Feature | Tier | Status |
|----|---------|------|--------|
| D1 | Consolidar camadas de config (3 → 1) | 1 | ✅ |
| D2 | `default_temperature` conhecer `chat-free` | 1 | ✅ |
| D3 | Dedup do `maybe_compact` (runtime + processor) | 1 | ✅ |
| D4 | Quebrar `app.rs` (3745 linhas) | 2 | ✅ |
| D5 | `cwd` vs `project_root` no runtime | 1 | ✅ |
| D6 | Unificar defaults de `HarnessConfig`/`Config` | 1 | ✅ |
| D7 | Limpar warnings de clippy pré-existentes | 3 | ✅ |
| D8 | Comentário `TODO F3 spec` obsoleto | 3 | ✅ |
| D9 | Sincronizar README com o estado atual | 3 | ✅ |
| D10 | Atualizar SUGGESTIONS.md (MCP já feito) | 3 | ✅ |

**Legenda:** ⬜ pendente · 🟡 em progresso · ✅ feito · ❌ cancelado

---

## Tier 1 — Pagar juros (reduzir complexidade de manutenção)

### Feature D1: Consolidar camadas de config (3 → 1)

**Onde:** `src/config.rs`, `src/harness/runtime.rs` (`HarnessConfig`, `from_legacy`,
`from_legacy_in`), `src/harness/project/config_file.rs`.

**Problema:** três representações do mesmo conceito:
- `Config` (resolvido, `api_key: Option<String>`)
- `HarnessConfig` (runtime, `api_key: String`)
- `GlobalSettings` + `ProjectConfig` (persistência)

`from_legacy`/`from_legacy_in` existem só para traduzir `Config → HarnessConfig`.

- [x] Definir um único `RuntimeConfig` resolvido (com `api_key: String`, já que o
      token é obrigatório no runtime)
- [x] Eliminar `HarnessConfig` e `from_legacy`/`from_legacy_in`
- [x] `Config::resolve` passa a produzir o `RuntimeConfig` diretamente
- [x] Atualizar todos os call sites (`main.rs`, `runtime.rs`, testes, smoke test)
- [x] Manter `GlobalSettings`/`ProjectConfig` como camada de persistência (não mudar)
- [x] Testes: `config.rs` já cobre a resolução — manter todos verdes

### Feature D2: `default_temperature` conhecer `chat-free`

**Onde:** `src/harness/agent/mod.rs` (linhas 34-42), `src/harness/agent/builtin.rs`.

**Problema:** o `match` em `default_temperature` tem `build/plan/explore/general` e
cai em `_ => 0.0`. O `chat-free` funciona só porque carrega `temperature: Some(0.8)`
explícito. Dois lugares precisam ser mantidos em sincronia manualmente
(`default_temperature` + `find_builtin`) a cada agente novo.

- [x] Mover a temperatura para dentro de cada `builtin::*()` (que já setam
      `temperature: Some(...)`)
- [x] `default_temperature` vira apenas o fallback para agentes custom/desconhecidos
- [x] Remover o `match` por nome (ou reduzir a `_ => 0.0`)
- [x] Testes: `test_default_temperature_per_mode` e `test_turn_temperature_override_wins`
      continuam verdes; adicionar `chat-free` ao caso de teste

### Feature D3: Dedup do `maybe_compact`

**Onde:** `src/harness/runtime.rs` (linhas 385-446), `src/harness/session/processor.rs`
(linhas 644-681), `src/harness/session/compaction.rs`.

**Problema:** o mesmo algoritmo de compaction (com constantes e o cálculo
`before - new.len() + 1`) existe **duas vezes**, quase idêntico, com constantes
potencialmente divergentes (`COMPACTION_KEEP_RECENT: 6` no processor vs
`KEEP_RECENT: 6` no runtime).

- [x] Extrair um único `compaction::compact_if_needed(session, provider, config, events)`
      chamado pelos dois
- [x] Unificar as constantes (`KEEP_RECENT`, `MIN_MESSAGES`) num único lugar
- [x] Manter o cálculo `summarized = before - new.len() + 1` num único ponto
- [x] Manter a persistência imediata pós-compaction (não perder histórico em abort)
- [x] Testes: `compaction.rs` já cobre o núcleo; adicionar teste do wrapper que
      persiste + emite eventos

### Feature D5: `cwd` vs `project_root` no runtime

**Onde:** `src/harness/runtime.rs` (9 ocorrências de `current_dir()` + 1 no
`TaskRunner::runtime_current_cwd`).

**Problema:** vários métodos chamam `std::env::current_dir()` internamente em vez de
usar o `project_root` que o runtime já guarda. Isso cria inconsistência potencial
(ex.: `TaskRunner` usa `current_dir()` em vez do cwd da sessão pai) e dificulta
testar com cwd diferente.

- [x] Varrer e substituir `current_dir()` por `self.project_root` nos métodos do runtime
- [x] `TaskRunner` usa o cwd da sessão pai (não `current_dir()` global)
- [x] `create_session`/`load_session`/`list_sessions`/`delete_session`/`set_session_title`
      usam `self.project_root`
- [x] Testes: adicionar teste que cria runtime com `project_root` explícito e verifica
      que as operações de sessão usam esse root (não o cwd do processo)

### Feature D6: Unificar defaults de `HarnessConfig`/`Config`

**Onde:** `src/harness/runtime.rs` (linhas 36-49), `src/config.rs` (linhas 106-118).

**Problema:** `HarnessConfig::default()` tem `max_iterations: 100`,
`max_context_tokens: 100_000`, `turn_timeout_secs: 1800`; `Config::defaults()` tem
`50`, `100_000`, `600`. Dois conjuntos de números mágicos para a mesma coisa — um
turno pode rodar com limites diferentes do que o `/settings` mostra.

- [x] Unificar os defaults num único lugar (resolvido junto com D1)
- [x] O runtime sempre usa os valores resolvidos (nunca os defaults "de fábrica")
- [x] Testes: verificar que `/settings` e o runtime concordam nos limites

---

## Tier 2 — Quebrar arquivos grandes

### Feature D4: Quebrar `app.rs` (3745 linhas)

**Onde:** `src/harness/ui/tui/app.rs`.

**Problema:** um único arquivo com ~3700 linhas de estado + lógica de eventos +
editor de input + subagentes + testes. É o maior arquivo do projeto e o mais difícil
de navegar/refatorar.

- [x] Extrair o **editor de input** (já há testes isolados disso) para módulo próprio
- [x] Extrair o **gerenciamento de subagentes** (painel/accordion) para módulo próprio
- [x] Extrair helpers de render/estado coesos (ex.: `last_code_block`, soft-wrap)
- [x] Fazer **incremental**: um bloco coeso por vez, mantendo os testes verdes
- [x] Não reduzir linhas por reduzir — separar responsabilidades
- [x] Testes: todos os testes de `app.rs` (editor, wrap, subagentes) continuam verdes

---

## Tier 3 — Higiene e documentação

### Feature D7: Limpar warnings de clippy pré-existentes

**Onde:** `src/harness/ui/tui/app.rs` (linhas ~2505, 2507) — funções usadas só em testes.

- [x] Marcar funções usadas só em testes com `#[cfg(test)]` ou `#[allow(dead_code)]`
      com justificativa
- [x] `cargo clippy` 100% limpo (sem warnings)

### Feature D8: Comentário `TODO F3 spec` obsoleto

**Onde:** `src/harness/permission/mod.rs` (linha 78).

**Problema:** o TODO.md agora é sobre dívida técnica; o comentário "matching the
TODO F3 spec" está desatualizado (F3 era do plano MCP, já concluído).

- [x] Atualizar ou remover o comentário obsoleto

### Feature D9: Sincronizar README com o estado atual

**Onde:** `README.md`.

**Problema:** o README ainda lista só `build, plan, explore, general` (linhas 12 e
130), não menciona `chat-free`. Também não lista `git_status/git_diff/git_log` nas
tools de coding nem o `remember`/`/memory`.

- [x] Adicionar `chat-free` à lista de agents (linha 12 e 130)
- [x] Listar `git_status/git_diff/git_log` nas tools de coding
- [x] Mencionar `remember` tool e comandos `/memory`
- [x] Revisar a seção de atalhos/comandos para refletir o estado atual

### Feature D10: Atualizar SUGGESTIONS.md (MCP já feito)

**Onde:** `SUGGESTIONS.md`.

**Problema:** o item 6 (MCP client) está **feito**, mas ainda listado como pendente
no Tier 2.

- [x] Riscar/mover o item 6 (MCP) para a seção "já implementado" (como foi feito com
      `/undo` e git tools no topo do arquivo)
- [x] Revisar o roadmap para refletir a realidade

---

## Ordem de execução recomendada

```text
D3 (dedup compaction) → D2 (temperatura) → D1+D6 (config) → D5 (cwd)
→ D7–D10 (higiene/docs) → D4 (app.rs, por último e incremental)
```

Cada feature: `cargo test` + `cargo clippy` + `cargo fmt --check` verdes.

---

## Riscos e mitigações

| Risco | Mitigação |
|-------|-----------|
| Refactor de config quebra resolução | `config.rs` já tem testes de resolução (catalog→global→projeto); manter todos verdes |
| Dedup do compaction muda comportamento | `compaction.rs` já cobre o núcleo; adicionar teste do wrapper (persist + eventos) |
| Quebrar `app.rs` introduz regressão de UI | Extração incremental, um bloco por vez, testes de editor/wrap/subagentes verdes |
| `current_dir()` vs `project_root` diverge | Teste com `project_root` explícito ≠ cwd do processo |
| Documentação fica desatualizada de novo | Sincronizar README/SUGGESTIONS na mesma PR das mudanças |

---

## Arquivos principais a tocar

| Path | Mudança |
|------|---------|
| `src/config.rs` | Consolidar `Config`/`HarnessConfig` num `RuntimeConfig` (D1, D6) |
| `src/harness/runtime.rs` | Eliminar `from_legacy*`; usar `project_root`; dedup compaction (D1, D3, D5, D6) |
| `src/harness/session/processor.rs` | Usar `compaction::compact_if_needed` (D3) |
| `src/harness/session/compaction.rs` | Novo wrapper `compact_if_needed` + constantes unificadas (D3) |
| `src/harness/agent/mod.rs` | `default_temperature` sem `match` por nome (D2) |
| `src/harness/agent/builtin.rs` | Temperaturas explícitas por agente (D2) |
| `src/harness/ui/tui/app.rs` | Extrair editor/subagentes/helpers (D4); limpar clippy (D7) |
| `src/harness/permission/mod.rs` | Comentário obsoleto (D8) |
| `README.md` | Sincronizar agents/tools/memory (D9) |
| `SUGGESTIONS.md` | Mover MCP para "já implementado" (D10) |

---

## Notas de progresso

| Data | Nota |
|------|------|
| 2026-09-04 | TODO.md substituído: plano de MCP (F1–F4, concluído) removido; novo plano de dívida técnica D1–D10 com checklists e tiers. |
