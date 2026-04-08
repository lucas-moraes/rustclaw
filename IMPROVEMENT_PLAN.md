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

## 1. Arquivos Mortos — Remover

Os seguintes arquivos **não estão declarados em `main.rs`** e **não são importados** por nenhum outro módulo. São código morto totalizando ~1.419 linhas.

| Arquivo | Linhas | Motivo |
|---------|--------|--------|
| `src/ab_testing.rs` | 328 | Nunca compilado. `FeatureFlags` duplica `features.rs`. `ABTestingEngine` não é thread-safe. |
| `src/wither.rs` | 313 | Nunca compilado. `TaskState`, `Notification`, `AppSettings` duplicam `app_state.rs`. |
| `src/lazy_loader.rs` | 232 | Nunca compilado. `clone_tool()` panica com `todo!()`. `LazyToolWrapper` hardcoded para `EchoTool`. |
| `src/context_compactor.rs` | 305 | Nunca compilado. Nenhuma importação no projeto. |
| `src/time_travel.rs` | 241 | Nunca compilado. `undo()`/`redo()`/`go_to()` são no-ops (modificam `&self`). |

### Arquivos declarados mas nunca usados

| Arquivo | Linhas | Motivo |
|---------|--------|--------|
| `src/bridge.rs` | 162 | Declarado em `main.rs` mas nunca importado. Nenhum modo "bridge" existe. |
| `src/prefetch.rs` | 82 | Declarado em `main.rs` mas `start_background_prefetch()` nunca é chamado. |

### Diretórios vazios

| Diretório | Motivo |
|-----------|--------|
| `src/agent/` | Diretório vazio, vestigial |
| `src/cli/` | Diretório vazio, vestigial |

### Ações de limpeza

```bash
# Remover arquivos mortos
rm src/ab_testing.rs src/wither.rs src/lazy_loader.rs src/context_compactor.rs src/time_travel.rs

# Remover arquivos declarados mas não usados
rm src/bridge.rs src/prefetch.rs

# Remover diretórios vazios
rmdir src/agent/ src/cli/
```

Remover de `src/main.rs`:
```rust
// Remover estas linhas:
mod bridge;
mod prefetch;
```

Remover de `Cargo.toml`:
```toml
# Dependências não usadas:
chaser-oxide = "0.1"    # nunhum arquivo importa esta crate
futures = "0.3"          # uso mínimo, substituível por tokio
```

---

## 2. Bugs Críticos

### 2.1 `time_travel.rs` — undo/redo são no-ops (REMOVER)

Os métodos `undo()`, `redo()` e `go_to()` recebem `&self` mas tentam modificar `current_index`. Como não há mutabilidade interior (`Cell<isize>`), as modificações são descartadas.

> **Decisão:** Remover o arquivo inteiro. A funcionalidade nunca foi integrada.

### 2.2 `lazy_loader.rs` — sistema inteiramente quebrado (REMOVER)

- `clone_tool()` panica com `todo!()`
- `LazyToolWrapper` sempre cria `EchoTool` independentemente do tipo

> **Decisão:** Remover. Se lazy loading for necessário no futuro, reimplementar do zero.

### 2.3 Bug SQL em `memory/store.rs:337-346`

`update_memory_access()` usa `?1` na subquery de importância mas `?2` no WHERE externo. O parâmetro `?1` na subquery deveria ser `?2` para referenciar o `importance` correto.

```rust
// Atual (bugado):
let sql = "UPDATE memories SET importance = (
    SELECT AVG(importance) * 0.95 FROM memories WHERE session_id = ?1
), last_accessed = ?2 WHERE id = ?3";

// Corrigir para:
let sql = "UPDATE memories SET importance = importance * 0.95, last_accessed = ?1 WHERE id = ?2";
```

### 2.4 `shell.rs:262` — `is_blocked()` nunca retorna `Ok(true)`

A lógica de bloqueio de comandos perigosos (linha 73) sempre retorna `Ok(false)`, tornando `Ok(true) => unreachable!()` na linha 262 morto. O bloqueio de comandos está **desativado de fato**.

```rust
// Linha 73: is_command_dangerous() retorna false para tudo
fn is_command_dangerous(cmd: &str) -> bool {
    // lista não cobre comandos perigosos comuns
}
```

> **Ação:** Revisar e implementar lista real de comandos bloqueados. Remover `unreachable!()`.

### 2.5 `file_write.rs` — sem validação de caminho

A ferramenta cria diretórios e escreve arquivos em qualquer caminho do filesystem sem verificar `workspace_trust`. Isso é uma vulnerabilidade de segurança.

> **Ação:** Integrar validação com `workspace_trust` antes de qualquer operação de escrita.

---

## 3. Problemas de Segurança

| Severidade | Problema | Local | Solução |
|------------|----------|-------|---------|
| **ALTA** | `file_write` escreve em qualquer caminho sem validação | `src/tools/file_write.rs` | Integrar `workspace_trust` |
| **ALTA** | `workspace_trust` nunca é consultado antes de executar ferramentas | `src/agent.rs:118` | Adicionar check antes de `execute_tool()` |
| **MÉDIA** | `shell.rs:293` — `split_whitespace()` quebra argumentos com aspas | `src/tools/shell.rs` | Usar crate `shell-words` |
| **MÉDIA** | `shell.rs:84` — fallback de `canonicalize()` bypassa segurança | `src/tools/shell.rs` | Retornar erro em vez de usar path bruto |
| **MÉDIA** | `cli.rs:411-573` — `unsafe` com `libc` para terminal raw | `src/cli.rs` | Migrar para `crossterm` |
| **BAIXA** | `sanitize_markdown()` remove todas as tags HTML | `src/security/sanitizer.rs` | Revisar lista de permitidos |

---

## 4. Problemas de Performance

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

## 5. Arquitetura — Decompor God Objects

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

## 6. Tratamento de Erros

| Problema | Local | Solução |
|----------|-------|---------|
| ~134 `unwrap()` em código de produção | Múltiplos arquivos | Substituir por `.expect("contexto")` ou propagar com `?` |
| `create_http_client()` usa `.expect()` | `src/agent.rs:39` | Retornar `Result` e tratar gracefulmente |
| Erros de ferramentas viram `Ok(err_msg)` | `src/agent.rs:2489-2492` | Logar erro adequadamente |
| `RwLock` com `.unwrap()` | `src/app_store.rs:24,35,47,51,57` | Usar `.expect()` com contexto |
| `embeddings.rs` — `.expect()` sem API key | `src/memory/embeddings.rs:116` | Retornar `Result` e fallback graceful |

### Código comentado para remover

- `src/agent.rs:858-863` — bloco comentado
- `src/agent.rs:874-888` — bloco comentado
- `src/agent.rs:1116-1119` — bloco comentado
- `src/agent.rs:2453-2456` — bloco comentado
- `src/agent.rs:852` — variável `force_tool_use` atribuída mas nunca usada significativamente

---

## 7. Dependências

| Ação | Dependência | Motivo |
|------|-------------|--------|
| **Remover** | `chaser-oxide` | Nenhum arquivo fonte importa esta crate |
| **Remover** | `futures` | Uso mínimo, substituível por `tokio` |
| **Substituir** | `atty` → `is-terminal` | `atty` não é mais mantido |
| **Adicionar** | `shell-words` | Para parsing seguro de comandos shell |
| **Adicionar** | `crossterm` | Para substituir `unsafe` libc no CLI |

---

## 8. Testes

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
- `src/ab_testing.rs` — **remover** (código morto)
- `src/wither.rs` — **remover** (código morto)
- `src/context_compactor.rs` — **remover** (código morto)
- `src/time_travel.rs` — **remover** (código morto)
- `src/workspace_trust.rs` — testes unitários
- `src/memory/search.rs` — testes unitários
- `src/memory/checkpoint.rs` — testes in-file
- `src/security/*` — testes abrangentes

### Meta de cobertura por fase:

1. **Fase 1** — Testes unitários para `shell.rs` (validação de comandos), `file_write.rs` (validação de caminhos), `memory/store.rs` (CRUD)
2. **Fase 2** — Testes de integração para o ReAct loop do agente
3. **Fase 3** — Testes de segurança (injection, path traversal, sanitização)

---

## 9. Documentação

| Ação | Detalhe |
|------|---------|
| Adicionar `//!` docs em cada módulo | Descrever propósito e responsabilidades |
| Adicionar doc comments em métodos públicos | Especialmente `Agent`, `MemoryStore`, `ToolRegistry` |
| Criar `ARCHITECTURE.md` | Diagrama de módulos, fluxo de dados, sistema de trust |
| i18n ou constantes para strings | Strings hardcoded misturam Português/Inglês — definir padrão ou extrair para constantes |

---

## 10. Ordem de Execução

### Fase 1 — Limpeza e Bugs Críticos (CONCLUÍDA)

- [x] Remover `src/ab_testing.rs`, `src/wither.rs`, `src/lazy_loader.rs`, `src/context_compactor.rs`, `src/time_travel.rs`
- [x] Remover `src/bridge.rs`, `src/prefetch.rs`
- [x] Remover `mod bridge;` e `mod prefetch;` de `src/main.rs`
- [x] Remover diretórios vazios `src/agent/`, `src/cli/`
- [x] Remover `futures` de `Cargo.toml`
- [x] Corrigir bug SQL em `src/memory/store.rs:337-346`
- [x] Corrigir `is_blocked()` em `src/tools/shell.rs`
- [x] Remover código comentado de `src/agent.rs` (linhas 858-863, 874-888, 1116-1119, 2453-2456)
- [x] Remover variável `force_tool_use` não usada
- [x] Remover linha morta `let _ = std::env::var("TOKEN")...` em `main.rs`
- [ ] `cargo clippy` e `cargo fmt` (171 warnings restantes)

### Fase 2 — Segurança (2-3 dias)

- [ ] Adicionar validação de caminho em `file_write.rs` e `file_read.rs`
- [ ] Integrar `workspace_trust` no fluxo de execução de ferramentas do `agent.rs`
- [ ] Corrigir fallback de `canonicalize()` em `shell.rs`
- [ ] Substituir `atty` por `is-terminal`
- [ ] Planejar substituição de `unsafe` libc por `crossterm`

### Fase 3 — Performance (2-3 dias)

- [ ] Pré-compilar regex com `OnceLock<Regex>` em `agent.rs` e `security/sanitizer.rs`
- [ ] Adicionar cache de embeddings em memória
- [ ] Otimizar `search_similar_memories` com índice SQLite ou ANN
- [ ] Cachear `canonicalize()` em `workspace_trust.rs`

### Fase 4 — Arquitetura (1-2 semanas)

- [ ] Decompor `agent.rs` em submódulos (`llm_client`, `response_parser`, `plan_executor`, `development`, `session`, `build_validator`, `output`)
- [ ] Decompor `memory/checkpoint.rs` em submódulos
- [ ] Unificar gerenciamento de estado (`AppState` + `Store<T>` como padrão único)
- [ ] Migrar globais `OnceLock` para dentro de `AppState`
- [ ] Melhorar tratamento de erros (remover `unwrap()`, usar `expect()` com contexto)

### Fase 5 — Testes e Documentação (1-2 semanas)

- [ ] Testes unitários para `shell.rs`, `file_write.rs`, `file_read.rs`
- [ ] Testes para `memory/store.rs` (CRUD + search)
- [ ] Testes de integração para ReAct loop
- [ ] Testes de segurança (injection, path traversal)
- [ ] Doc comments em módulos e métodos públicos
- [ ] Criar `ARCHITECTURE.md`
- [ ] Extrair strings hardcoded para constantes ou i18n

---

## Referência Rápida — Problemas por Arquivo

| Arquivo | Linhas | Rating | Problemas Principais |
|---------|--------|--------|---------------------|
| `agent.rs` | 3.114 | 1/5 | God object, regex hot, unwrap, dead code, duplicação |
| `memory/checkpoint.rs` | 2.348 | 2/5 | Arquivo massivo, deve ser dividido |
| `cli.rs` | 764 | 3/5 | Unsafe libc, display duplicado |
| `tools/shell.rs` | 350 | 2/5 | Bloqueio quebrado, path traversal, sem testes |
| `tools/file_write.rs` | 85 | 2/5 | Sem validação de caminho |
| `memory/store.rs` | 595 | 3/5 | Bug SQL, ALTER TABLE silencioso |
| `memory/embeddings.rs` | 118 | 3/5 | Fallback ingênuo, panic sem API key |
| `config.rs` | 238 | 4/5 | Limpo e bem estruturado |
| `security/*` | ~1.300 | 4/5 | Módulo bem projetado com testes |
| `workspace_trust.rs` | 381 | 4/5 | Bom design, bom testes |
| `tools/mod.rs` | 494 | 4/5 | Testes abrangentes |