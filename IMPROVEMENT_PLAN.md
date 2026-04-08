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

> Bugs 2.1–2.4 foram corrigidos no CP-1.

### 1.1 `file_write.rs` — sem validação de caminho (Ver CP-3)

A ferramenta cria diretórios e escreve arquivos em qualquer caminho do filesystem sem verificar `workspace_trust`. Isso é uma vulnerabilidade de segurança.

> **Ação:** Integrar validação com `workspace_trust` antes de qualquer operação de escrita. (CP-3)

---

## 2. Problemas de Segurança

| Severidade | Problema | Local | Solução |
|------------|----------|-------|---------|
| **ALTA** | `file_write` escreve em qualquer caminho sem validação | `src/tools/file_write.rs` | Integrar `workspace_trust` |
| **ALTA** | `workspace_trust` nunca é consultado antes de executar ferramentas | `src/agent.rs:118` | Adicionar check antes de `execute_tool()` |
| **MÉDIA** | `shell.rs:293` — `split_whitespace()` quebra argumentos com aspas | `src/tools/shell.rs` | Usar crate `shell-words` |
| **MÉDIA** | `shell.rs:84` — fallback de `canonicalize()` bypassa segurança | `src/tools/shell.rs` | Retornar erro em vez de usar path bruto |
| **MÉDIA** | `cli.rs:411-573` — `unsafe` com `libc` para terminal raw | `src/cli.rs` | Migrar para `crossterm` |
| **BAIXA** | `sanitize_markdown()` remove todas as tags HTML | `src/security/sanitizer.rs` | Revisar lista de permitidos |

---

## 3. Problemas de Performance

### 4.1 Regex compilados em cada chamada (ALTA)

`parse_response()` cria ~9 regex por invocação (chamada em cada iteração do ReAct loop).

```rust
// Atual (lento):
fn parse_response(text: &str) -> ... {
    let re1 = Regex::new(r"...").unwrap();
    let re2 = Regex::new(r"...").unwrap();
    // ...
}

// Corrigir com OnceLock:
static RE_ACTION: OnceLock<Regex> = OnceLock::new();
fn parse_response(text: &str) -> ... {
    let re1 = RE_ACTION.get_or_init(|| Regex::new(r"...").unwrap());
}
```

**Arquivos afetados:** `src/agent.rs` (linhas 1746-1747, 2185-2186, 2370, 2378, 2387-2392, 2773, 2784, 2880-2881), `src/security/sanitizer.rs:178-179`

### 4.2 `search_similar_memories` — scan linear (MÉDIA)

Carrega TODAS as memórias em memória e faz scan linear.

> **Solução:** Adicionar índice ANN (approximate nearest neighbor) no SQLite ou usar tabela FTS5.

### 4.3 Embedding sem cache (MÉDIA)

Cada operação de memória faz uma chamada HTTP para gerar embeddings.

> **Solução:** Cache em memória com `HashMap<String, Vec<f32>>` + LRU eviction.

### 4.4 `canonicalize()` repetido (BAIXA)

`workspace_trust.rs` chama `canonicalize()` (I/O de filesystem) repetidamente.

> **Solução:** Cache com `HashMap<PathBuf, PathBuf>`.

---

## 4. Arquitetura — Decompor God Objects

### 5.1 `agent.rs` (3.114 linhas)

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

### 5.2 `memory/checkpoint.rs` (2.348 linhas)

Dividir em:

```
src/memory/checkpoint/
├── mod.rs           # Re-exports
├── store.rs         # Operações de banco
├── types.rs         # Structs e enums
└── migration.rs     # Schema e migrações
```

### 5.3 Unificar gerenciamento de estado

O projeto tem 4 padrões de estado:
1. `AppState` + `Store<T>` — ativo, manter como padrão
2. `TimeTravelState` — morto, será removido
3. `FeatureFlags` (`features.rs`) — ativo
4. `OnceLock<OutputManager>` / `OnceLock<TmuxManager>` globais em `agent.rs`

> **Ação:** Migrar globais `OnceLock` para dentro de `AppState` ou `Store`.

---

## 5. Tratamento de Erros

| Problema | Local | Solução |
|----------|-------|---------|
| ~134 `unwrap()` em código de produção | Múltiplos arquivos | Substituir por `.expect("contexto")` ou propagar com `?` |
| `create_http_client()` usa `.expect()` | `src/agent.rs:39` | Retornar `Result` e tratar gracefulmente |
| Erros de ferramentas viram `Ok(err_msg)` | `src/agent.rs:2489-2492` | Logar erro adequadamente |
| `RwLock` com `.unwrap()` | `src/app_store.rs:24,35,47,51,57` | Usar `.expect()` com contexto |
| `embeddings.rs` — `.expect()` sem API key | `src/memory/embeddings.rs:116` | Retornar `Result` e fallback graceful |

### Código comentado (removido no CP-1)

---

## 6. Dependências

| Ação | Dependência | Motivo |
|------|-------------|--------|
| **Avaliar** | `chaser-oxide` | Usado em `browser/mod.rs` — avaliar se é necessário |
| **Remover** ✅ | `futures` | Removido no CP-1 |
| **Substituir** | `atty` → `is-terminal` | `atty` não é mais mantido (Ver CP-4) |
| **Adicionar** | `shell-words` | Para parsing seguro de comandos shell (Ver CP-4) |
| **Adicionar** | `crossterm` | Para substituir `unsafe` libc no CLI (Ver CP-12) |

---

## 7. Testes

Cobertura atual estimada: **~15-20%** dos módulos.

### Módulos sem nenhum teste (prioridade alta):

| Módulo | Linhas | Criticidade |
|--------|--------|-------------|
| `src/agent.rs` | 3.114 | Core do sistema, zero testes |
| `src/tools/shell.rs` | 350 | Segurança crítica |
| `src/tools/file_write.rs` | 85 | Escrita arbitrária no filesystem |
| `src/tools/file_read.rs` | ~100 | Leitura de arquivos |
| `src/tools/file_edit.rs` | ~150 | Edição de arquivos |
| `src/memory/store.rs` | 595 | Persistência crítica |
| `src/memory/embeddings.rs` | 118 | Embedding service |
| `src/config.rs` | 238 | Configuração |
| `src/cli.rs` | 764 | Interface principal |

### Módulos com testes:

- `src/tools/mod.rs` — testes de integração
- `src/app_state.rs` — testes unitários
- `src/workspace_trust.rs` — testes unitários
- `src/memory/search.rs` — testes unitários
- `src/memory/checkpoint.rs` — testes in-file
- `src/security/*` — testes abrangentes

### Meta de cobertura por checkpoint:

1. **CP-10** — Testes unitários para `shell.rs` (validação de comandos), `file_write.rs` (validação de caminhos), `memory/store.rs` (CRUD)
2. **CP-11** — Testes de integração para o ReAct loop do agente + testes de segurança
3. **CP-13** — Testes de segurança (injection, path traversal, sanitização)

---

## 8. Documentação

| Ação | Detalhe |
|------|---------|
| Adicionar `//!` docs em cada módulo | Descrever propósito e responsabilidades |
| Adicionar doc comments em métodos públicos | Especialmente `Agent`, `MemoryStore`, `ToolRegistry` |
| Criar `ARCHITECTURE.md` | Diagrama de módulos, fluxo de dados, sistema de trust |
| i18n ou constantes para strings | Strings hardcoded misturam Português/Inglês — definir padrão ou extrair para constantes |

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

### CP-3 — Segurança Crítica (EM PROGRESSO)

- [x] Integrar `workspace_trust` no fluxo de ferramentas em `agent.rs::execute_tool()`
- [x] Adicionar validação de caminho em `file_write.rs` — bloqueia paths de sistema (/etc, /usr, /bin, etc.)
- [x] Adicionar validação de caminho em `file_read.rs` — bloqueia arquivos sensíveis (/etc/shadow, .ssh, etc.)
- [x] Adicionar validação de caminho em `file_edit.rs` — bloqueia paths de sistema
- [x] Corrigir fallback de `canonicalize()` em `shell.rs:84` — agora retorna `true` (restrito) em vez de usar path bruto
- [x] Expandir lista de comandos bloqueados em `shell.rs` — `DANGEROUS_COMMANDS` e `SYSTEM_COMMANDS`
- [x] Escrever testes unitários para `shell.rs::is_blocked()` (testes shell_blocks_system_commands)
- [x] Escrever testes para `file_write.rs` e `file_read.rs` validação de paths (testes file_write_rejects, file_read_rejects, file_edit_rejects, file_write_allows)

**Verificação:** `cargo test` passa com 72 testes (5 novos de segurança). `file_write` rejeita paths de sistema. `file_read` bloqueia arquivos sensíveis. `shell` bloqueia comandos do sistema.

### CP-3 — Segurança Crítica

- [ ] Integrar `workspace_trust` no fluxo de ferramentas em `agent.rs` — verificar trust antes de `execute_tool()` para `file_write`, `file_read`, `file_edit`, `shell`
- [ ] Adicionar validação de caminho em `file_write.rs` — rejeitar paths fora do workspace
- [ ] Adicionar validação de caminho em `file_read.rs` — rejeitar paths fora do workspace
- [ ] Adicionar validação de caminho em `file_edit.rs` — rejeitar paths fora do workspace
- [ ] Corrigir fallback de `canonicalize()` em `shell.rs:84` — retornar erro em vez de path bruto
- [ ] Expandir lista de comandos bloqueados em `shell.rs` — `DANGEROUS_COMMANDS` e `SYSTEM_COMMANDS`
- [ ] Escrever testes unitários para `shell.rs::is_blocked()` — cobrir comandos perigosos, paths restritos, heredoc, redirect

**Verificação:** `cargo test` passa. `file_write` rejeita paths fora do workspace. `shell` bloqueia comandos perigosos.

---

### CP-4 — Dependências e Depreciações

- [ ] Substituir `atty` por `is-terminal` em `cli.rs`
- [ ] Adicionar crate `shell-words` ao `Cargo.toml`
- [ ] Usar `shell-words::split()` em `shell.rs` em vez de `split_whitespace()`
- [ ] Avaliar se `chaser-oxide` é realmente necessário (usado em `browser/mod.rs`) — se não, remover
- [ ] Avaliar se `keyring` e `libc` são necessários — se `auth.rs` for removido, podem sair
- [ ] Verificar: `cargo check` passa

**Verificação:** `cargo check` sem warnings de depreciação. `shell.rs` faz parsing correto de argumentos com aspas.

---

### CP-5 — Performance — Regex e Cache

- [ ] Extrair todos os regex de `parse_response()` em `agent.rs` para `OnceLock<Regex>` estáticos
- [ ] Extrair regex de `sanitize_model_response()` em `agent.rs` para `OnceLock<Regex>`
- [ ] Extrair regex de `security/sanitizer.rs` para `OnceLock<Regex>`
- [ ] Adicionar cache de embeddings em `memory/embeddings.rs` — `HashMap<String, Vec<f32>>` com LRU
- [ ] Cachear `canonicalize()` em `workspace_trust.rs` — `HashMap<PathBuf, PathBuf>`
- [ ] Benchmark antes/depois: tempo de uma iteração do ReAct loop com regex compilados vs dinâmicos

**Verificação:** Benchmark mostra redução mensurável em alocações por iteração. `cargo test` passa.

---

### CP-6 — Tratamento de Erros

- [ ] Substituir `.unwrap()` em `app_store.rs` por `.expect("contexto")`
- [ ] Substituir `.expect("Failed to create HTTP client")` em `config.rs` por `Result` propagável
- [ ] Substituir `.unwrap()` em `RwLock` de `features.rs` por `.expect()` com contexto
- [ ] Converter erro de tool em `agent.rs::execute_tool()` — logar como erro em vez de mascarar em `Ok(err_msg)`
- [ ] Converter `.expect()` em `memory/embeddings.rs:116` para `Result` com fallback graceful
- [ ] Rotular todos os `unwrap()` restantes com issue tracker ou converter para `expect()`
- [ ] Verificar: `cargo check` passa

**Verificação:** `grep -r "\.unwrap()" src/` mostra apenas testes ou casos explicitamente seguros.

---

### CP-7 — Arquitetura — Decompor `agent.rs`

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

### CP-8 — Arquitetura — Decompor `checkpoint.rs`

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

### CP-9 — Unificar Estado e Remover Código Morto Restante

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

### CP-10 — Testes — Ferramentas e Memória

- [ ] Testes unitários para `shell.rs`: comandos bloqueados, paths restritos, heredoc, redirect seguro, parsing
- [ ] Testes unitários para `file_write.rs`: escrita dentro do workspace, rejeitar path traversal (`../`), rejeitar paths absolutos fora
- [ ] Testes unitários para `file_read.rs`: leitura dentro do workspace, rejeitar paths fora
- [ ] Testes unitários para `file_edit.rs`: edição dentro do workspace
- [ ] Testes unitários para `memory/store.rs`: CRUD, search, importância, cleanup
- [ ] Testes unitários para `config.rs`: carregar de env, defaults, validação
- [ ] Verificar: `cargo test` passa com nova cobertura

**Verificação:** `cargo test` executa testes novos em `shell`, `file_write`, `file_read`, `file_edit`, `store`, `config`.

---

### CP-11 — Testes — Segurança e Integração

- [ ] Testes de segurança para `security/injection_detector.rs`: prompt injection, JSON breakout, command injection
- [ ] Testes de segurança para `security/sanitizer.rs`: sanitização de output, mascaramento de dados sensíveis
- [ ] Testes de segurança para path traversal: `../../../etc/passwd`, symlinks, paths absolutos
- [ ] Teste de integração para ReAct loop: simular chamada LLM, verificar parsing de ação, execução de ferramenta
- [ ] Teste de integração para checkpoint: criar, salvar, carregar, retomar
- [ ] Verificar: `cargo test` passa

**Verificação:** Testes de segurança cobrem os vetores de ataque conhecidos. Teste de integração do ReAct loop passa.

---

### CP-12 — CLI — Migrar Unsafe para Crossterm

- [ ] Adicionar `crossterm` ao `Cargo.toml`
- [ ] Refatorar `cli.rs:406-573` para usar `crossterm` em vez de `libc::termios` + `libc::read`
- [ ] Remover blocos `#[cfg(unix)]` e `#[cfg(not(unix))]` duplicados — `crossterm` é cross-platform
- [ ] Refatorar `run()` function (>500 linhas) em funções menores
- [ ] Remover dependência `libc` se não for mais necessária
- [ ] Verificar: CLI funciona em macOS e Linux

**Verificação:** `cargo test` passa. CLI interativo funciona sem `unsafe`. `libc` removido de `Cargo.toml`.

---

### CP-13 — Documentação

- [ ] Adicionar `//!` doc comments em cada módulo (`agent`, `memory`, `tools`, `security`, `skills`, `cli`)
- [ ] Adicionar `///` doc comments em métodos públicos de `Agent`, `MemoryStore`, `ToolRegistry`, `CheckpointStore`
- [ ] Criar `ARCHITECTURE.md` com diagrama de módulos, fluxo de dados, sistema de trust
- [ ] Extrair strings hardcoded (mistura PT/EN) para constantes ou arquivo de i18n
- [ ] Atualizar `AGENTS.md` com comandos atuais e estrutura de módulos refletem o código pós-refatoração

**Verificação:** `cargo doc --no-deps` gera documentação sem warnings. `ARCHITECTURE.md` reflete a estrutura real do código.

---

### CP-14 — Memory — Busca Escalável

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
| **CP-3** | Segurança Crítica | 🔄 Em progresso | 2-3 dias |
| **CP-4** | Dependências e Depreciações | ⬜ Pendente | 1 dia |
| **CP-5** | Performance — Regex e Cache | ⬜ Pendente | 2-3 dias |
| **CP-6** | Tratamento de Erros | ⬜ Pendente | 1-2 dias |
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
| `agent.rs` | 3.072 | 1/5 | God object, regex hot, unwrap, duplicação | CP-5, CP-6, CP-7 |
| `memory/checkpoint.rs` | 2.348 | 2/5 | Arquivo massivo, deve ser dividido | CP-8 |
| `cli.rs` | 764 | 3/5 | Unsafe libc, display duplicado | CP-12 |
| `tools/shell.rs` | 350 | 2/5 | Path traversal, sem testes | CP-3, CP-4 |
| `tools/file_write.rs` | 85 | 2/5 | Sem validação de caminho | CP-3 |
| `memory/store.rs` | 595 | 3/5 | ALTER TABLE silencioso | CP-10, CP-14 |
| `memory/embeddings.rs` | 118 | 3/5 | Fallback ingênuo, panic sem API key | CP-6 |
| `config.rs` | 238 | 4/5 | Limpo e bem estruturado | — |
| `security/*` | ~1.300 | 4/5 | Módulo bem projetado com testes | — |
| `workspace_trust.rs` | 381 | 4/5 | Bom design, bom testes | CP-3 |
| `tools/mod.rs` | 494 | 4/5 | Testes abrangentes | — |