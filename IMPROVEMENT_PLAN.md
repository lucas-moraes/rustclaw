# RustClaw - Plano de Melhorias

> Análise completa do projeto em Apr 2026
> ~20.040 linhas em 67 arquivos fonte (antes da limpeza)

---

## 0. Limpeza Realizada (Fase 1 — Concluída)

### Arquivos removidos (código morto — ~1.663 linhas eliminadas)

| Arquivo | Linhas | Motivo |
|---------|--------|--------|
| `src/ab_testing.rs` | 328 | Nunca compilado. `FeatureFlags` duplicava `features.rs`. |
| `src/wither.rs` | 313 | Nunca compilado. Tipos duplicavam `app_state.rs`. |
| `src/lazy_loader.rs` | 232 | Nunca compilado. `clone_tool()` panica com `todo!()`. |
| `src/context_compactor.rs` | 305 | Nunca compilado. Nenhuma importação. |
| `src/time_travel.rs` | 241 | Nunca compilado. `undo()`/`redo()` eram no-ops. |
| `src/bridge.rs` | 162 | Declarado mas nunca importado. Nenhum modo "bridge". |
| `src/prefetch.rs` | 82 | Declarado mas `start_background_prefetch()` nunca chamado. |

### Outras limpezas

- Removidos `mod bridge;` e `mod prefetch;` de `main.rs`
- Removidos diretórios vazios `src/agent/` e `src/cli/`
- Removida dependência `futures` do `Cargo.toml` (não usada)
- Removida variável descartada `let _ = std::env::var("TOKEN")...` em `main.rs`
- Removido código comentado de `agent.rs` (3 blocos)
- Removida variável `forced_tool_use` não usada (2 ocorrências)
- Removido bloco morto de workspace trust skip em `execute_tool()`

### Bugs corrigidos

- **SQL bug**: `update_memory_access()` em `memory/store.rs` tinha parâmetros errados no WHERE — simplificado para `importance * 0.95`
- **`is_blocked()`**: Retorno mudado de `Result<bool, String>` para `Result<(), String>` — `Ok(true) => unreachable!()` removido

---

## 1. Bugs (Referência — Ver CPs)

> Bugs 2.1–2.4 foram corrigidos no CP-1. Bug 2.5 (file_write path validation) corrigido no CP-3.

### 1.1 `file_write.rs` — validação de caminho ✅ CORRIGIDO (CP-3)

A ferramenta agora bloqueia paths de sistema (/etc, /usr, /bin, etc.) e verifica `workspace_trust`.

---

## 2. Problemas de Segurança

| Severidade | Problema | Status | Detalhe |
|------------|----------|--------|---------|
| **ALTA** | `file_write` escreve em qualquer caminho | ✅ Corrigido (CP-3) | `validate_path()` bloqueia paths de sistema |
| **ALTA** | `workspace_trust` nunca consultado | ✅ Corrigido (CP-3) | Integrado em `execute_tool()` |
| **MÉDIA** | `shell.rs` — `split_whitespace()` quebra aspas | ✅ Corrigido (CP-4) | Usando `shell_words::split()` |
| **MÉDIA** | `shell.rs:84` — `canonicalize()` bypass | ✅ Corrigido (CP-3) | Retorna restrito em vez de path bruto |
| **MÉDIA** | `cli.rs` — `unsafe` com `libc` | ⬜ Pendente | CP-12 |
| **BAIXA** | `sanitize_markdown()` remove tags HTML | ⬜ Pendente | Baixa prioridade |

---

## 3. Problemas de Performance

### 3.1 Regex compilados em cada chamada ✅ CORRIGIDO (CP-5)

Todos os regex em hot paths (`agent.rs` e `sanitizer.rs`) agora usam `OnceLock<Regex>` para compilação única.

- `parse_response()`: 7 padrões → `OnceLock`
- `sanitize_model_response()`, `extract_final_answer()`, `parse_action_input_json()`: 3 usos de `<system-reminder>` → 1 `OnceLock`
- `review_re`, `suggestion_re`, `plan_step_re`, heredoc patterns, json recovery: todos `OnceLock`
- `sanitizer.rs`: HTML, ANSI, cookie, auth headers: todos `OnceLock`

### 3.2 `search_similar_memories` — scan linear ⬜ Pendente

> **Solução:** Adicionar índice FTS5 no SQLite (CP-14).

### 3.3 Embedding sem cache ⬜ Pendente

> **Solução:** Cache em memória com `HashMap<String, Vec<f32>>` + LRU eviction.

### 3.4 `canonicalize()` repetido ⬜ Pendente

> **Solução:** Cache com `HashMap<PathBuf, PathBuf>`.

---

## 4. Arquitetura — Decompor God Objects

### 4.1 `agent.rs` (3.500+ linhas) ⬜ Pendente (CP-7)

Dividir em:
```
src/agent/
├── mod.rs               # Re-exports e Agent struct
├── llm_client.rs         # Chamadas HTTP para LLM
├── response_parser.rs    # Parse de respostas (regex, extração de ações)
├── plan_executor.rs      # Execução de planos e steps
├── development.rs         # Modo structured development
├── session.rs             # Gerenciamento de sessões
├── build_validator.rs     # Validação de builds e compilação
└── output.rs              # Formatação de output, cores
```

### 4.2 `memory/checkpoint.rs` (2.348 linhas) ⬜ Pendente (CP-8)

### 4.3 Unificar gerenciamento de estado ⬜ Pendente (CP-9)

O projeto tem 4 padrões de estado:
1. `AppState` + `Store<T>` — ativo, manter como padrão
2. `TimeTravelState` — removido no CP-1
3. `FeatureFlags` (`features.rs`) — ativo
4. `OnceLock<OutputManager>` / `OnceLock<TmuxManager>` globais em `agent.rs` → migrar para dentro de `AppState`

---

## 5. Tratamento de Erros

| Problema | Local | Status |
|----------|-------|--------|
| ~134 `unwrap()` em código de produção | Múltiplos arquivos | ⬜ Pendente (CP-6) |
| `create_http_client()` usa `.expect()` | `src/agent.rs:39` | ⬜ Pendente (CP-6) |
| Erros de ferramentas viram `Ok(err_msg)` | `src/agent.rs::execute_tool()` | ⬜ Pendente (CP-6) |
| `RwLock` com `.unwrap()` | `src/app_store.rs` | ⬜ Pendente (CP-6) |
| `embeddings.rs` — `.expect()` sem API key | `src/memory/embeddings.rs` | ⬜ Pendente (CP-6) |

### Código comentado ✅ Removido (CP-1)

---

## 6. Dependências

| Ação | Dependência | Status |
|------|-------------|--------|
| **Avaliar** | `chaser-oxide` | ⬜ Pendente |
| **Remover** ✅ | `futures` | Removido no CP-1 |
| **Substituir** ✅ | `atty` → `is-terminal` | Concluído no CP-4 |
| **Adicionar** ✅ | `shell-words` | Concluído no CP-4 |
| **Adicionar** | `crossterm` | ⬜ Pendente (CP-12) |
| **Remover** | `atty` | ✅ Removido no CP-4 |

---

## 7. Testes

Cobertura atual: **72 testes passando** (5 novos de segurança adicionados no CP-3).

### Módulos sem nenhum teste (prioridade alta):

| Módulo | Linhas | Criticidade | Status |
|--------|--------|-------------|--------|
| `src/agent.rs` | 3.500+ | Core do sistema | ⬜ Pendente (CP-10) |
| `src/tools/shell.rs` | 350 | Segurança | ✅ 1 teste (CP-3) |
| `src/tools/file_write.rs` | 85 | Escrita | ✅ 2 testes (CP-3) |
| `src/tools/file_read.rs` | ~100 | Leitura | ✅ 1 teste (CP-3) |
| `src/tools/file_edit.rs` | ~100 | Edição | ✅ 1 teste (CP-3) |
| `src/memory/store.rs` | 595 | Persistência | ⬜ Pendente (CP-10) |
| `src/memory/embeddings.rs` | 118 | Embedding | ⬜ Pendente |
| `src/config.rs` | 238 | Configuração | ⬜ Pendente |
| `src/cli.rs` | 764 | Interface | ⬜ Pendente |

### Meta de cobertura por checkpoint:

1. **CP-10** — Testes unitários para `memory/store.rs` (CRUD), `config.rs`, mais testes para shell
2. **CP-11** — Testes de segurança (injection, path traversal, sanitização) e integração ReAct loop

---

## 8. Documentação

| Ação | Status |
|------|--------|
| Adicionar `//!` docs em cada módulo | ⬜ Pendente (CP-13) |
| Adicionar doc comments em métodos públicos | ⬜ Pendente (CP-13) |
| Criar `ARCHITECTURE.md` | ⬜ Pendente (CP-13) |
| Extrair strings hardcoded (mistura PT/EN) | ⬜ Pendente (CP-13) |

---

## 10. Ordem de Execução

### CP-1 — Limpeza e Bugs Críticos ✅ CONCLUÍDO

- [x] Remover `src/ab_testing.rs`, `src/wither.rs`, `src/lazy_loader.rs`, `src/context_compactor.rs`, `src/time_travel.rs`
- [x] Remover `src/bridge.rs`, `src/prefetch.rs`
- [x] Remover `mod bridge;` e `mod prefetch;` de `src/main.rs`
- [x] Remover diretórios vazios `src/agent/`, `src/cli/`
- [x] Remover `futures` de `Cargo.toml`
- [x] Corrigir bug SQL em `src/memory/store.rs:337-346`
- [x] Corrigir `is_blocked()` em `src/tools/shell.rs`
- [x] Remover código comentado de `src/agent.rs` (3 blocos + `forced_tool_use`)
- [x] Remover linha morta `let _ = std::env::var("TOKEN")...` em `main.rs`

**Verificação:** `cargo check` passa com 0 erros.

---

### CP-2 — Lint e Formatação ✅ CONCLUÍDO

- [x] Executar `cargo fmt`
- [x] Executar `cargo clippy --fix` (corrigiu ~100 warnings automaticamente)
- [x] Remover imports não usados (`OutputSink`, `crate::tools::Tool`, e ~30 outros via clippy fix)
- [x] Remover `unsafe` aninhados desnecessários em `cli.rs` (6 blocos)
- [x] Verificar: `cargo check` com 122 warnings restantes (majoritariamente dead code — CP-9)

**Verificação:** `cargo check` passa. `cargo test` passa com 67 testes.

---

### CP-3 — Segurança Crítica ✅ CONCLUÍDO

- [x] Integrar `workspace_trust` no fluxo de ferramentas em `agent.rs::execute_tool()`
- [x] Adicionar validação de caminho em `file_write.rs` — bloqueia paths de sistema (/etc, /usr, /bin, etc.)
- [x] Adicionar validação de caminho em `file_read.rs` — bloqueia arquivos sensíveis (/etc/shadow, .ssh, etc.)
- [x] Adicionar validação de caminho em `file_edit.rs` — bloqueia paths de sistema
- [x] Corrigir fallback de `canonicalize()` em `shell.rs:84` — agora retorna `true` (restrito) em vez de usar path bruto
- [x] Expandir lista de comandos bloqueados em `shell.rs` — `DANGEROUS_COMMANDS` e `SYSTEM_COMMANDS`
- [x] Escrever testes unitários para `shell.rs::is_blocked()` (teste shell_blocks_system_commands)
- [x] Escrever testes para `file_write.rs` e `file_read.rs` validação de paths (5 testes de segurança)

**Verificação:** `cargo test` passa com 72 testes (5 novos de segurança). `file_write` rejeita paths de sistema. `file_read` bloqueia arquivos sensíveis. `shell` bloqueia comandos do sistema.

---

### CP-4 — Dependências e Depreciações ✅ CONCLUÍDO

- [x] Substituir `atty` por `is-terminal` em `cli.rs`
- [x] Adicionar crate `shell-words` ao `Cargo.toml`
- [x] Usar `shell_words::split()` em `shell.rs` em vez de `split_whitespace()` para parsing correto de aspas
- [x] Remover `atty` de `Cargo.toml`

**Verificação:** `cargo test` passa com 72 testes. Shell parsing agora lida com argumentos entre aspas.

---

### CP-5 — Performance — Regex e Cache ✅ CONCLUÍDO

- [x] Pré-compilar todos os regex de `parse_response()` em `agent.rs` com `OnceLock<Regex>` (7 padrões)
- [x] Pré-compilar `sanitize_model_response()` e `extract_final_answer()` regex (system-reminder, 3 usos → 1 OnceLock)
- [x] Pré-compilar `review_re` e `suggestion_re` em `agent.rs`
- [x] Pré-compilar `plan_step_re` em `agent.rs`
- [x] Pré-compilar heredoc/EOF/json regex em `parse_heredoc_input()` e `recover_action_input()`
- [x] Pré-compilar regex em `security/sanitizer.rs` (HTML, ANSI, cookie, auth) com `OnceLock`
- [ ] Adicionar cache de embeddings em memória
- [ ] Cachear `canonicalize()` em `workspace_trust.rs`

**Verificação:** Todos os regex em hot paths agora usam `OnceLock<Regex>`. `cargo test` passa com 72 testes.

---

### CP-6 — Tratamento de Erros ✅ CONCLUÍDO

- [x] Substituir `.unwrap()` em `app_store.rs` por `.expect("lock poisoned")` com contexto
- [x] Substituir `.unwrap()` em `features.rs` por `.expect("feature flags lock poisoned")` com contexto
- [x] Converter `create_http_client()` em `agent.rs` de `.expect()` para `Result` propagável
- [x] Remover regex `_done_re` não usado em `agent.rs`
- [x] Converter `Default` de `EmbeddingService` para fallback graceful em vez de panic
- [ ] Rotular todos os `unwrap()` restantes com issue tracker ou converter para `expect()` (opcional)

**Verificação:** `cargo test` passa com 72 testes. `create_http_client()` agora propaga erros. `EmbeddingService::default()` não panica mais.

---

### CP-7 — Arquitetura — Decompor `agent.rs` ⬜ Pendente

- [ ] Criar `src/agent/mod.rs` com `Agent` struct e re-exports
- [ ] Extrair `src/agent/llm_client.rs` — funções `call_llm()`, `create_http_client()`
- [ ] Extrair `src/agent/response_parser.rs` — `parse_response()`, `sanitize_model_response()`, todos os regex
- [ ] Extrair `src/agent/plan_executor.rs` — `execute_plan_steps()`, lógica de planos
- [ ] Extrair `src/agent/development.rs` — `run_structured_development()`, `DevelopmentCheckpoint` helpers
- [ ] Extrair `src/agent/session.rs` — `session_save()`, `session_load()`, gerenciamento de sessões
- [ ] Extrair `src/agent/build_validator.rs` — `validate_build()`, detecção de erros de compilação
- [ ] Extrair `src/agent/output.rs` — funções `output_write_*`, `OutputManager`, `OutputSink`
- [ ] Atualizar imports em todos os arquivos que referenciam `crate::agent::*`
- [ ] Verificar: `cargo check` e `cargo test` passam

**Verificação:** `agent.rs` original reduzido para < 200 linhas (apenas struct + constructor + métodos de orquestração). Todos os testes passam.

---

### CP-8 — Arquitetura — Decompor `checkpoint.rs` ⬜ Pendente

- [ ] Criar `src/memory/checkpoint/mod.rs` com re-exports
- [ ] Extrair `src/memory/checkpoint/types.rs` — todos os structs e enums (`DevelopmentCheckpoint`, `SessionSummary`, etc.)
- [ ] Extrair `src/memory/checkpoint/store.rs` — `CheckpointStore`, operações de banco
- [ ] Extrair `src/memory/checkpoint/events.rs` — `SessionEventStore`, `SessionEvent`, compressão
- [ ] Extrair `src/memory/checkpoint/lifecycle.rs` — `LifecycleManager`, `SnapshotManager`, políticas
- [ ] Extrair `src/memory/checkpoint/migration.rs` — schema init e migrações
- [ ] Atualizar imports em todos os arquivos que referenciam `crate::memory::checkpoint::*`
- [ ] Verificar: `cargo check` e `cargo test` passam

**Verificação:** `checkpoint.rs` original não existe mais (dividido em 5-6 arquivos). Todos os testes passam.

---

### CP-9 — Unificar Estado e Remover Código Morto Restante ⬜ Pendente

- [ ] Migrar `OnceLock<OutputManager>` e `OnceLock<TmuxManager>` globais de `agent.rs` para dentro de `AppState`
- [ ] Remover ou marcar `features.rs` como `#[allow(dead_code)]` se não for usado — decidir se integra ou remove
- [ ] Remover ou marcar `auth.rs` como `#[allow(dead_code)]` — decidir se integra ou remove
- [ ] Remover `app_store.rs` se `Store<AppState>` não for usado — verificar usos reais
- [ ] Remover structs não usados em `memory/checkpoint.rs`: `SessionContext`, `SessionEvent`, `EventSummary`, `SnapshotPolicy`, etc.
- [ ] Remover funções não usadas em `security/`: `get_defense_prompt`, `Sanitizer::tool_output`, `mask_sensitive_data`, etc.
- [ ] Remover `HookManager`, `McpClient` e structs associados em `skills/` se não forem usados
- [ ] Verificar: `cargo check` com < 10 warnings (reduzidos de 171)

**Verificação:** `cargo check` com número significativamente reduzido de warnings. Nenhuma struct/função morta visível.

---

### CP-10 — Testes — Ferramentas e Memória ⬜ Pendente

- [ ] Testes unitários para `shell.rs`: comandos bloqueados, paths restritos, heredoc, redirect seguro, parsing
- [ ] Testes unitários para `file_write.rs`: escrita dentro do workspace, rejeitar path traversal (`../`), rejeitar paths absolutos fora
- [ ] Testes unitários para `file_read.rs`: leitura dentro do workspace, rejeitar paths fora
- [ ] Testes unitários para `file_edit.rs`: edição dentro do workspace
- [ ] Testes unitários para `memory/store.rs`: CRUD, search, importância, cleanup
- [ ] Testes unitários para `config.rs`: carregar de env, defaults, validação
- [ ] Verificar: `cargo test` passa com nova cobertura

**Verificação:** `cargo test` executa testes novos em `shell`, `file_write`, `file_read`, `file_edit`, `store`, `config`.

---

### CP-11 — Testes — Segurança e Integração ⬜ Pendente

- [ ] Testes de segurança para `security/injection_detector.rs`: prompt injection, JSON breakout, command injection
- [ ] Testes de segurança para `security/sanitizer.rs`: sanitização de output, mascaramento de dados sensíveis
- [ ] Testes de segurança para path traversal: `../../../etc/passwd`, symlinks, paths absolutos
- [ ] Teste de integração para ReAct loop: simular chamada LLM, verificar parsing de ação, execução de ferramenta
- [ ] Teste de integração para checkpoint: criar, salvar, carregar, retomar
- [ ] Verificar: `cargo test` passa

**Verificação:** Testes de segurança cobrem os vetores de ataque conhecidos. Teste de integração do ReAct loop passa.

---

### CP-12 — CLI — Migrar Unsafe para Crossterm ⬜ Pendente

- [ ] Adicionar `crossterm` ao `Cargo.toml`
- [ ] Refatorar `cli.rs:406-573` para usar `crossterm` em vez de `libc::termios` + `libc::read`
- [ ] Remover blocos `#[cfg(unix)]` e `#[cfg(not(unix))]` duplicados — `crossterm` é cross-platform
- [ ] Refatorar `run()` function (>500 linhas) em funções menores
- [ ] Remover dependência `libc` se não for mais necessária
- [ ] Verificar: CLI funciona em macOS e Linux

**Verificação:** `cargo test` passa. CLI interativo funciona sem `unsafe`. `libc` removido de `Cargo.toml`.

---

### CP-13 — Documentação ⬜ Pendente

- [ ] Adicionar `//!` doc comments em cada módulo (`agent`, `memory`, `tools`, `security`, `skills`, `cli`)
- [ ] Adicionar `///` doc comments em métodos públicos de `Agent`, `MemoryStore`, `ToolRegistry`, `CheckpointStore`
- [ ] Criar `ARCHITECTURE.md` com diagrama de módulos, fluxo de dados, sistema de trust
- [ ] Extrair strings hardcoded (mistura pt/en) para constantes ou arquivo de i18n
- [ ] Atualizar `AGENTS.md` com comandos atuais e estrutura de módulos pós-refatoração

**Verificação:** `cargo doc --no-deps` gera documentação sem warnings. `ARCHITECTURE.md` reflete a estrutura real do código.

---

### CP-14 — Memory — Busca Escalável ⬜ Pendente

- [ ] Implementar índice FTS5 no SQLite para `search_similar_memories` em `memory/store.rs`
- [ ] Benchmark: buscar entre 1000, 10000 e 100000 memórias
- [ ] Adicionar migração de schema para criar tabela FTS5
- [ ] Fallback para scan linear se FTS5 não estiver disponível
- [ ] Verificar: `cargo test` passa. Busca é O(log n) com FTS5.

**Verificação:** Benchmark mostra busca < 10ms com 10.000+ memórias.

---

### Resumo de Checkpoints

| Checkpoint | Descrição | Status | Estimativa |
|------------|-----------|--------|------------|
| **CP-1** | Limpeza e Bugs Críticos | ✅ Concluído | — |
| **CP-2** | Lint e Formatação | ✅ Concluído | — |
| **CP-3** | Segurança Crítica | ✅ Concluído | — |
| **CP-4** | Dependências e Depreciações | ✅ Concluído | — |
| **CP-5** | Performance — Regex e Cache | ✅ Concluído | — |
| **CP-6** | Tratamento de Erros | ✅ Concluído | — |
| **CP-7** | Decompor `agent.rs` | ⬜ Pendente | 3-5 dias |
| **CP-8** | Decompor `checkpoint.rs` | ⬜ Pendente | 2-3 dias |
| **CP-9** | Unificar Estado e Remover Morto | ⬜ Pendente | 2-3 dias |
| **CP-10** | Testes — Ferramentas e Memória | ⬜ Pendente | 2-3 dias |
| **CP-11** | Testes — Segurança e Integração | ⬜ Pendente | 2-3 dias |
| **CP-12** | CLI — Migrar para Crossterm | ⬜ Pendente | 2-3 dias |
| **CP-13** | Documentação | ⬜ Pendente | 2-3 dias |
| **CP-14** | Memory — Busca Escalável | ⬜ Pendente | 2-3 dias |

---

## Referência Rápida — Problemas por Arquivo

| Arquivo | Linhas | Rating | Problemas Principais | Checkpoint |
|---------|--------|--------|----------------------|------------|
| `agent.rs` | 3.500+ | 2/5 | God object (melhorou com OnceLock), unwrap, duplicação | CP-5 ✅, CP-6, CP-7 |
| `memory/checkpoint.rs` | 2.348 | 2/5 | Arquivo massivo, deve ser dividido | CP-8 |
| `cli.rs` | 764 | 3/5 | Unsafe libc, display duplicado | CP-12 |
| `tools/shell.rs` | 350 | 3/5 | ✅ Path validation, ✅ shell-words | CP-3 ✅, CP-4 ✅ |
| `tools/file_write.rs` | 105 | 3/5 | ✅ Path validation de sistema | CP-3 ✅ |
| `tools/file_read.rs` | 100 | 3/5 | ✅ Sensible file blocking | CP-3 ✅ |
| `memory/store.rs` | 595 | 3/5 | ✅ SQL fix, ALTER TABLE silencioso | CP-1 ✅, CP-14 |
| `memory/embeddings.rs` | 118 | 3/5 | Fallback ingênuo, panic sem API key | CP-6 |
| `config.rs` | 238 | 4/5 | Limpo e bem estruturado | — |
| `security/*` | ~1.300 | 4/5 | Módulo bem projetado com testes | — |
| `workspace_trust.rs` | 381 | 4/5 | ✅ Integrado em execute_tool | CP-3 ✅ |
| `tools/mod.rs` | 494 | 4/5 | ✅ 5 testes de segurança adicionados | CP-3 ✅ |