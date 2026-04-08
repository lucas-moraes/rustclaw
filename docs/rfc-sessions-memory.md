# RFC: Melhorias no Sistema de Sessions e Memory

## Status

Proposto - Em discussão

## Sumário

Melhorar a arquitetura de sessions e memory para permitir:
- Persistência de conhecimento cross-session
- Hierarquia de sessões para tasks relacionadas
- snapshots mais inteligentes e eficientes
- Lifecycle management para dados antigos

---

## Motivação

O sistema atual:
- Sessions são isoladas → conhecimento não é reutilizado entre sessões
- Checkpoints são snapshots inteiros → ocupa espaço, difícil de analisar
- Todas memórias vivem no mesmo SQLite → degrada performance com tempo
- Sem diferenciação entre memória importante e descartável

---

## Checkpoints de Implementação

### Feature 1: Hierarquia de Sessões ✅

- [x] **1.1** Adicionar campo `parent_id: Option<Uuid>` em `CheckpointStore` (migration)
- [x] **1.2** Criar enum `SessionType { Project, Subtask, Research, Chat }`
- [x] **1.3** Criar método `get_ancestors(session_id)` para buscar chain de pais
- [x] **1.4** Modificar `list_session_summaries()` para suportar `parent_id` filter
- [x] **1.5** Implementar `get_full_context(session_id)` - merge contexto pai + filho
- [x] **1.6** Atualizar CLI `/sessions` para mostrar hierarchy com indentação
- [x] **1.7** Testar resume de subtask inclui contexto do parent

### Feature 2: Cross-Session Memory ✅

- [x] **2.1** Criar migration para adicionar colunas em `memories`:
  - `scope TEXT` (session/project/global)
  - `importance REAL DEFAULT 0.5`
  - `access_count INTEGER DEFAULT 0`
  - `last_accessed TIMESTAMP`
- [x] **2.2** Criar enum `MemoryScope` com variant paths
- [x] **2.3** Implementar `calculate_importance(entry)` based on access_count + age
- [x] **2.4** Criar `MemoryStore::get_global_memories(project_filter)`
- [x] **2.5** Criar `MemoryStore::get_project_memories(repo_path)`
- [x] **2.6** Modificar `format_memories_for_prompt()` para incluir memórias globais
- [x] **2.7** Adicionar config `GLOBAL_MEMORY_TTL_DAYS` (default: never)
- [x] **2.8** Testar cross-session: memória criada em sessao A visível em sessao B

### Feature 3: Event Sourcing para Checkpoints

- [x] **3.1** Criar enum `SessionEvent` com variantes (ToolExecuted, PhaseChanged, etc)
- [x] **3.2** Criar tabela `session_events`:
  ```sql
  CREATE TABLE session_events (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    event_data JSON NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
  );
  CREATE INDEX idx_session_events ON session_events(session_id, created_at);
  ```
- [x] **3.3** Criar `SessionEventStore` com métodos:
  - `append_event(session_id, event)`
  - `get_events(session_id, from_ts, to_ts)`
  - `get_session_timeline(session_id)` → analytics
- [x] **3.4** Migrar `save_checkpoint` para gerar eventos automaticamente
- [x] **3.5** Implementar `get_session_timeline(session_id)` para analytics
- [x] **3.6** Adicionar compression para `event_data` JSON grande
- [x] **3.7** Deprecar snapshots full em favor de events (manter backward compat)

### Feature 4: Auto-Snapshot Inteligente

- [x] **4.1** Criar enum `SnapshotTrigger` com variantes
- [x] **4.2** Criar `SnapshotPolicy` struct com config:
  - `trigger_on_build_success: bool`
  - `trigger_on_build_fail: bool`
  - `trigger_on_phase_change: bool`
  - `periodic_interval: Option<u32>` (N mensagens)
- [x] **4.3** Hook no `BuildDetector` para detectar successful/failed builds
- [x] **4.4** Criar `SnapshotManager` que decide quando fazer snapshot
- [x] **4.5** Implementar debounce: não snapshotted mais que 1x por minuto
- [x] **4.6** Adicionar config ao `config.rs`:
  ```rust
  SNAPSHOT_ON_SUCCESS=true
  SNAPSHOT_ON_FAILURE=false
  SNAPSHOT_PERIODIC_MESSAGES=50
  ```
- [x] **4.7** Testar: build succeed → checkpoint criado automaticamente

### Feature 5: TTL e Archive Policy

- [x] **5.1** Criar struct `LifecyclePolicy` com defaults
- [x] **5.2** Criar `cleanup_job()` que roda em background:
  - Delete checkpoints WHERE created_at < now() - checkpoint_ttl
  - Downgrade memories importance WHERE last_accessed < now() - session_memory_ttl
  - Mark sessions as archived WHERE last_accessed < now() - archive_after
- [x] **5.3** Criar `archive_session(session_id, location)`:
  - Serializa session + memories para JSON
  - Move para arquivo/archive
  - Remove do SQLite
- [x] **5.4** Criar `restore_session(archive_path)` para recovery
- [x] **5.5** Hook no startup: `LifecycleManager::new().run_cleanup()`
- [x] **5.6** Criar comando `/archive` ou via config para trigger manual
- [x] **5.7** Testar: criar dado antigo, rodar cleanup, verificar TTL applied

### Feature 6: Session Fingerprinting

- [ ] **6.1** Criar struct `SessionFingerprint` com detectors:
  - `has_git()`
  - `has_package_manager()` (Cargo.toml, package.json, etc)
  - `detect_language()` → Rust, JS, Python, etc
  - `detect_repo_url()` → from git remote
- [ ] **6.2** Implementar `detect_context_change(input, cwd)`:
  - Compara fingerprint atual com anterior
  - Returna `ContextChange { new_project, new_language, is_continuing }`
- [ ] **6.3** Hook no `Agent::process_input()` para detectar mudanças
- [ ] **6.4** Se `context_change.is_new_project` → sugerir criar nova sessão
- [ ] **6.5** Persistir fingerprint na session_summary
- [ ] **6.6** Auto-create session com tipo correto (Project vs Chat)
- [ ] **6.7** Testar: entrar em dir com .git → detectado como project mode

---

## Proposta

### 1. Hierarquia de Sessões

```
Session (projeto)
├── Session (subtask - implement feature X)
│   └── Session (subtask aninhada)
└── Session (subtask - fix bug Y)
```

**Benefício:** Permite criar "sprint" ou "task" dentro de um projeto. Resumir uma subtask resume ela + contexto do pai.

**Estrutura:**
```rust
struct SessionNode {
    id: Uuid,
    parent_id: Option<Uuid>,  // None = root
    session_type: SessionType, // Project, Subtask, Research, Chat
    created_at: DateTime,
    archived: bool,
}
```

---

### 2. Cross-Session Memory (Global Knowledge)

Separar memórias em categorias:

| Tipo | Escopo | TTL | Exemplo |
|------|--------|-----|---------|
| `session` | Sessão atual | 7 dias | "Usuário está trabalhando no parser" |
| `project` | Projeto (por repo) | 30 dias | "Projeto usa rusqlite, tokio" |
| `global` |永久 | None | "Rust best practices", libraries comuns |

```rust
enum MemoryScope {
    Session(String),   // session_id específico
    Project(String),   // repo/path
    Global,            // universal
}

struct MemoryEntry {
    // ... existente
    scope: MemoryScope,
    importance: f32,        // 0.0-1.0, auto-calculated
    access_count: u32,
    last_accessed: DateTime,
}
```

---

### 3. Event Sourcing para Checkpoints

Em vez de salvar estado completo, salvar eventos:

```rust
enum SessionEvent {
    ToolExecuted { tool: String, input: Value, output: Value, timestamp: DateTime },
    PhaseChanged { from: Phase, to: Phase, reason: String },
    FileModified { path: PathBuf, change_type: ChangeType },
    MessageAdded { role: Role, content: String },
    BranchCreated { name: String, reason: String },
}
```

**Benefício:**
- Replay parcial (não precisa restaurar tudo)
- Análise posterior: "quanto tempo gasto no cargo build?"
- Delta compression → menos storage

---

### 4. Auto-Snapshot Inteligente

Snapshots só em milestones, não a cada iteração:

```rust
enum SnapshotTrigger {
    AfterSuccessfulBuild,
    AfterFailedBuild,
    BeforeMajorRefactor,
    OnUserRequest,
    OnPhaseTransition,
    Periodic(u32),  // a cada N mensagens
}
```

---

### 5. TTL e Archive Policy

```rust
struct LifecyclePolicy {
    session_memory_ttl: Duration,     // 7 dias
    checkpoint_ttl: Duration,         // 14 dias
    project_memory_ttl: Duration,     // 30 dias
    
    archive_after: Duration,          // 90 dias sem acesso
    archive_to: ArchiveLocation,     // S3, file, etc
}
```

**Estratégia de Archive:**
1. Sessions > 90 dias sem acesso → mover para archive
2. Checkpoints > 14 dias → delete, manter só session summary
3. Memórias com `access_count < 2` e velhas → downgrade importance

---

### 6. Session Fingerprinting

Detectar contexto automaticamente:

```rust
struct SessionContext {
    is_project_mode: bool,    // detect via .git, Cargo.toml, etc
    repo_url: Option<String>,
    language: Option<String>,
    recent_commands: Vec<String>,
}

fn detect_session_type(input: &str, cwd: &Path) -> SessionContext;
```

Criar sessão nova automaticamente quando:
- Usuário entra em diretório diferente com `.git`
- Comando indica projeto (`cargo build`, `npm`, etc)
- Contexto muda significativamente (pattern matching em conversation)

---

## Implementação Sugerida

### Fase 1: Non-breaking
- [ ] Adicionar `MemoryScope` ao schema (migration)
- [ ] Criar tabela `session_events` para novos checkpoints
- [ ] Implementar TTL cleanup job

### Fase 2: Breaking changes
- [ ] Adicionar `parent_id` em sessions
- [ ] Mover para event sourcing para checkpoints
- [ ] Auto-snapshot triggers

### Fase 3: Polish
- [ ] Session fingerprinting
- [ ] Archive system
- [ ] UI para gerenciar lifecycle

---

## Considerações

- **Migration:** Precisa migrar dados existentes
- **Performance:** Queries mais complexas com JOINs de hierarchy
- **Storage:** Event sourcing pode aumentar volume, compensar com compression

---

## Alternativas Consideradas

1. **Tudo em memória:** Rápido mas não persistente
2. **Uma tabela só:** Simples mas inflexível
3. **Event Store separado:** Mais correto mas overkill para este projeto

---

## Perguntas Abertas

1. Como lidar com sessões de projetos diferentes que usam as mesmas memórias?
2. Quando fazer merge de sessões (ex: duas sessões de research sobre mesmo tema)?
3. Qual granularity para event sourcing? Todo tool call ou só "importantes"?
