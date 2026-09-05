# RustClaw — Undo, Git tools e Permissões persistentes

> **Problema:** três features de alto valor do SUGGESTIONS.md (Tier 1) ainda não estão
> completas:
> 1. **Undo/revert do último turno** (`/undo`) — o modal de prompt já tem hint de "undo",
>    mas falta o comando e a reconstrução do transcript.
> 2. **Git-aware tools** (`git_status`, `git_diff`, `git_log`) — hoje git passa pelo `bash`
>    genérico; o model gasta tokens e erra parsing.
> 3. **Permissões persistentes por projeto** — o "always allow" é só por run; falta persistir
>    em `rustclaw.json` + comando `/permissions`.
>
> **Escopo:** 3 fases independentes que evoluem o sistema existente em vez de criar paralelos:
> undo (U0) → git tools (G0) → permissões persistentes (P0).

---

## Visão geral das features

| ID | Feature | Fase | Status |
|----|---------|------|--------|
| U0 | Comando `/undo` (reverter último turno) | 1 | ⬜ |
| U1 | Reconstrução do transcript no TUI após undo | 1 | ⬜ |
| G0 | Tool `git_status` | 2 | ⬜ |
| G1 | Tool `git_diff` | 2 | ⬜ |
| G2 | Tool `git_log` | 2 | ⬜ |
| G3 | Registro + allowlists + permissões das git tools | 2 | ⬜ |
| P0 | Campo `permission` no `ProjectConfig` (rustclaw.json) | 3 | ⬜ |
| P1 | Carregamento das permissões persistentes no runtime | 3 | ⬜ |
| P2 | Persistência do "always allow" → regra permanente | 3 | ⬜ |
| P3 | Comando `/permissions` (list/set/rm) | 3 | ⬜ |
| V0 | Verificação final + commit | 4 | ⬜ |

**Legenda:** ⬜ pendente · 🟡 em progresso · ✅ feito · ❌ cancelado

---

## U0 — Comando `/undo` (reverter último turno)

**Objetivo:** permitir reverter a última mensagem do user + respostas/tools associadas após o
turno terminar (complementa o Esc, que cancela *durante* o turn). Reutiliza o fluxo já existente
de `revert_to_prompt` / `delete_messages_from`.

### Feature: Comando `/undo` no dispatcher compartilhado

- [ ] Em `src/harness/ui/commands/mod.rs`, adicionar braço `"/undo"` no `match cmd`:
  - [ ] Encontrar o **último** `Message` com `role == "user"` em `session.messages`
  - [ ] Se não houver, retornar feedback `"nothing to undo"`
  - [ ] Chamar `runtime.store.delete_messages_from(&session.id, &session.cwd, &msg_id)`
  - [ ] Truncar `session.messages` até o índice da mensagem user (excluindo-a)
  - [ ] Chamar `runtime.store.save_session(session)` para persistir
  - [ ] Retornar `CommandOutcome::Continue(vec!["session reverted to before last prompt"])`
- [ ] Adicionar `/undo` à lista de comandos do `/help`
- [ ] Adicionar teste unitário: `/undo` remove a última mensagem user + tudo depois dela

### Feature: Feedback e edge cases

- [ ] Mensagem clara quando não há turno para reverter
- [ ] Não quebrar quando a última mensagem é do assistant (sem user após) — tratar como "nothing to undo"
- [ ] Adicionar teste: `/undo` em sessão vazia retorna "nothing to undo"

### Definition of done U0

- [ ] `cargo test` verde (incl. testes de `/undo`)
- [ ] `cargo check` verde
- [ ] `cargo clippy --bin rustclaw` sem novos warnings
- [ ] `/undo` reverte o último turno no CLI e no TUI

---

## U1 — Reconstrução do transcript no TUI após undo

**Objetivo:** hoje `revert_to_prompt` trunca `session.messages` mas **não** atualiza `app.lines`,
deixando o transcript com as mensagens removidas ainda visíveis. Corrigir isso e integrar o
`/undo` ao TUI.

### Feature: Helper de rebuild do transcript

- [ ] Em `src/harness/ui/tui/app.rs`, criar `fn rebuild_transcript_from_session(app: &mut App)`:
  - [ ] Limpar `app.lines` (e `streaming`, `tool_status`, `active_tools`)
  - [ ] Reconstruir a partir de `app.session.messages`:
    - [ ] `role == "user"` → `LineKind::User` (texto do primeiro `Part::Text`)
    - [ ] `role == "assistant"` → `LineKind::Assistant` (texto) + `LineKind::ToolOk`/`ToolError`
          para cada `Part::Tool` terminal
  - [ ] Resetar `scroll`/`stick_bottom` para o fim
- [ ] Chamar o helper no `revert_to_prompt` após truncar `session.messages`
- [ ] Adicionar teste: após undo, `app.lines` reflete a sessão truncada

### Feature: Integração do `/undo` no TUI

- [ ] Em `submit_input` (app.rs), interceptar `text == "/undo"` antes do fallback genérico:
  - [ ] Se `app.running`, avisar `[busy] cannot undo while a turn is running`
  - [ ] Senão, executar a lógica de undo (reusar `revert_to_prompt` com o índice do último user)
  - [ ] Chamar `rebuild_transcript_from_session` e `add_system("session reverted")`
- [ ] Garantir que o `/undo` do TUI não passe pelo `commands::handle` genérico (que não tem
      acesso ao transcript)

### Definition of done U1

- [ ] `cargo test` verde (incl. teste de rebuild)
- [ ] `cargo check` verde
- [ ] `cargo clippy --bin rustclaw` sem novos warnings
- [ ] Após `/undo` no TUI, o transcript mostra apenas as mensagens restantes

---

## G0 — Tool `git_status`

**Objetivo:** tool nativa que reporta status + branch + dirty files de forma estruturada e
truncada, sem depender do model parsear `git status` via shell.

### Feature: Módulo `src/harness/tool/git.rs`

- [ ] Criar `src/harness/tool/git.rs` com helper comum:
  - [ ] `async fn run_git(ctx: &ToolContext, args: &[&str]) -> Result<String, String>` que roda
        `git <args>` via `tokio::process::Command` com `current_dir(ctx.cwd.path())`
  - [ ] Capturar stdout+stderr e retornar output truncado (reusar `truncate::truncate_lines`)
  - [ ] Tratar exit code != 0 como erro com mensagem amigável
- [ ] Implementar `GitStatusTool`:
  - [ ] `name()` → `"git_status"`
  - [ ] `description()` → status + branch + dirty files
  - [ ] `parameters()` → JSON Schema (sem args obrigatórios; opcional `porcelain: bool`)
  - [ ] `execute()` → roda `git status --short --branch`, chama `ctx.check_permission` antes
- [ ] Adicionar teste: `git_status` em repo temporário (`tempfile` + `git init`) retorna branch

### Definition of done G0

- [ ] `cargo test` verde (incl. teste de git_status)
- [ ] `cargo check` verde
- [ ] `cargo clippy --bin rustclaw` sem novos warnings
- [ ] `git_status` retorna status estruturado e truncado

---

## G1 — Tool `git_diff`

**Objetivo:** tool nativa que mostra diff staged/unstaged com filtro de path, truncado.

### Feature: Implementação de `GitDiffTool`

- [ ] Em `src/harness/tool/git.rs`, implementar `GitDiffTool`:
  - [ ] `name()` → `"git_diff"`
  - [ ] `description()` → diff staged/unstaged com path filter
  - [ ] `parameters()` → JSON Schema com `staged: bool` (default false) e `path: Option<String>`
  - [ ] `execute()`:
    - [ ] `staged=true` → `git diff --cached [path]`
    - [ ] `staged=false` → `git diff [path]`
    - [ ] Chamar `ctx.check_permission` antes
    - [ ] Truncar output (reusar `truncate::truncate_lines`)
- [ ] Adicionar teste: `git_diff` em repo com mudança unstaged retorna o diff

### Definition of done G1

- [ ] `cargo test` verde (incl. teste de git_diff)
- [ ] `cargo check` verde
- [ ] `cargo clippy --bin rustclaw` sem novos warnings
- [ ] `git_diff` retorna diff truncado com filtro de path

---

## G2 — Tool `git_log`

**Objetivo:** tool nativa que mostra log curto (`--oneline`) com limite de entradas.

### Feature: Implementação de `GitLogTool`

- [ ] Em `src/harness/tool/git.rs`, implementar `GitLogTool`:
  - [ ] `name()` → `"git_log"`
  - [ ] `description()` → log curto com limite
  - [ ] `parameters()` → JSON Schema com `n: integer` (default 20, max 100)
  - [ ] `execute()`:
    - [ ] Roda `git log --oneline -n <n>`
    - [ ] Chamar `ctx.check_permission` antes
    - [ ] Truncar output
- [ ] Adicionar teste: `git_log` em repo com commits retorna o log

### Definition of done G2

- [ ] `cargo test` verde (incl. teste de git_log)
- [ ] `cargo check` verde
- [ ] `cargo clippy --bin rustclaw` sem novos warnings
- [ ] `git_log` retorna log curto truncado

---

## G3 — Registro + allowlists + permissões das git tools

**Objetivo:** registrar as 3 git tools no registry, adicioná-las às allowlists dos agents
read-only e às permissões default (Allow), seguindo o padrão do projeto.

### Feature: Registro no registry

- [ ] Em `src/harness/tool/mod.rs`, adicionar `pub mod git;`
- [ ] Em `src/harness/runtime.rs::build_default_registry` (linha ~568):
  - [ ] Importar `GitStatusTool`, `GitDiffTool`, `GitLogTool`
  - [ ] Registrar as 3 no builder

### Feature: Allowlists dos agents

- [ ] Em `src/harness/agent/builtin.rs`, adicionar `"git_status"`, `"git_diff"`, `"git_log"`
      ao `READONLY_TOOLS` (são read-only; `plan`/`explore`/`general` passam a usá-los)
- [ ] `build` já tem acesso total (allowlist vazia) — sem mudança

### Feature: Permissões default

- [ ] Em `src/harness/permission/mod.rs::with_defaults`, adicionar `"git_status"`,
      `"git_diff"`, `"git_log"` ao grupo `Rule::Allow` (não pedem confirmação)
- [ ] Adicionar teste: `check("git_status", None, cwd)` retorna `Allow`

### Definition of done G3

- [ ] `cargo test` verde (incl. teste de permissão)
- [ ] `cargo check` verde
- [ ] `cargo clippy --bin rustclaw` sem novos warnings
- [ ] As 3 git tools aparecem no registry, nas allowlists e como `Allow` por default

---

## P0 — Campo `permission` no `ProjectConfig` (rustclaw.json)

**Objetivo:** persistir regras de permissão por projeto em `rustclaw.json` no formato
`"permission": { "bash": "allow", "edit": "ask", "bash.rm": "deny" }`.

### Feature: Campo `permission` no struct

- [ ] Em `src/harness/project/config_file.rs`, adicionar ao `ProjectConfig`:
  - [ ] `#[serde(default, skip_serializing_if = "Option::is_none")] pub permission: Option<PermissionConfig>`
- [ ] Ajustar derives: `ProjectConfig` deriva `PartialEq, Eq`; adicionar `PartialEq, Eq` a
      `PermissionConfig` em `src/harness/permission/mod.rs` (e `Rule` já tem)
- [ ] Atualizar `is_empty()` para incluir `permission.is_none()`
- [ ] Adicionar teste: roundtrip de `ProjectConfig` com `permission` presente

### Feature: Serialização do formato

- [ ] Garantir que `PermissionConfig` serializa como `{ "tools": {...}, "default": ... }`
      (ou o formato compacto sugerido no SUGGESTIONS.md)
- [ ] Adicionar teste: `serde_json::to_string` do `ProjectConfig` com permission gera o JSON esperado

### Definition of done P0

- [ ] `cargo test` verde (incl. testes de roundtrip/serialização)
- [ ] `cargo check` verde
- [ ] `cargo clippy --bin rustclaw` sem novos warnings
- [ ] `rustclaw.json` aceita e persiste o bloco `permission`

---

## P1 — Carregamento das permissões persistentes no runtime

**Objetivo:** ao construir o `SessionRuntime`, carregar as regras de `rustclaw.json` e aplicá-las
sobre os defaults, sem quebrar o comportamento atual.

### Feature: Merge de defaults + overrides

- [ ] Em `src/harness/permission/mod.rs`, adicionar método `PermissionConfig::merge(&self, base: &PermissionConfig) -> PermissionConfig`:
  - [ ] Começar de `base` (defaults)
  - [ ] Aplicar `tools` do projeto por cima (override por tool)
  - [ ] Aplicar `default` do projeto se presente
- [ ] Adicionar teste: merge preserva defaults e aplica overrides

### Feature: Carregamento no runtime

- [ ] Em `src/harness/runtime.rs::SessionRuntime::new` (linha ~117):
  - [ ] Carregar `ProjectConfig::load(&cwd)`
  - [ ] Se `permission` presente, construir `PermissionEngine::from_config(&merged)` em vez de
        `PermissionEngine::default()`
  - [ ] Expor o `PermissionConfig` carregado num campo do runtime (ex. `permission_config`) para
        o `/permissions` listar
- [ ] Adicionar teste: runtime com `rustclaw.json` com permission aplica as regras

### Definition of done P1

- [ ] `cargo test` verde (incl. testes de merge e carregamento)
- [ ] `cargo check` verde
- [ ] `cargo clippy --bin rustclaw` sem novos warnings
- [ ] Regras de `rustclaw.json` são aplicadas no runtime, preservando defaults

---

## P2 — Persistência do "always allow" → regra permanente

**Objetivo:** quando o usuário escolhe "always" num prompt de permissão, persistir a regra em
`rustclaw.json` para valer em runs futuras (não só na run atual).

### Feature: Métodos de persistência no `PermissionEngine`

- [ ] Em `src/harness/permission/mod.rs`, adicionar:
  - [ ] `pub fn set_rule(&mut self, tool: &str, rule: Rule)` — atualiza `self.rules`
  - [ ] `pub fn rules(&self) -> &HashMap<String, Rule>` — para listar
  - [ ] `pub fn remove_rule(&mut self, tool: &str)` — remove a regra (volta ao default)
- [ ] Adicionar teste: `set_rule`/`remove_rule` atualizam o engine

### Feature: Persistência no TUI

- [ ] Em `src/harness/ui/tui/app.rs` (tecla `a` no modal Permission, linha ~2456):
  - [ ] Além de `set_always_allow`, persistir a regra `Allow` no `rustclaw.json` via
        `ProjectConfig` (carregar, atualizar `permission.tools[tool] = Allow`, salvar)
- [ ] Adicionar teste: decisão "always" persiste a regra no arquivo

### Feature: Persistência no CLI

- [ ] Em `src/harness/ui/cli.rs` (opção `a`/`always`, linha ~44):
  - [ ] Além de `set_always_allow`, persistir a regra `Allow` no `rustclaw.json`
- [ ] Adicionar teste: decisão "always" no CLI persiste a regra

### Definition of done P2

- [ ] `cargo test` verde (incl. testes de persistência)
- [ ] `cargo check` verde
- [ ] `cargo clippy --bin rustclaw` sem novos warnings
- [ ] "always allow" persiste em `rustclaw.json` e vale em runs futuras

---

## P3 — Comando `/permissions` (list/set/rm)

**Objetivo:** gerenciar as permissões persistentes via slash command, compartilhado TUI+CLI.

### Feature: Comando `/permissions` no dispatcher

- [ ] Em `src/harness/ui/commands/mod.rs`, adicionar braço `"/permissions"`:
  - [ ] `/permissions` — lista as regras atuais (tool → allow/ask/deny) + default
  - [ ] `/permissions set <tool> <allow|ask|deny>` — atualiza a regra e persiste em `rustclaw.json`
  - [ ] `/permissions rm <tool>` — remove a regra (volta ao default) e persiste
  - [ ] Validar valores de regra; erro claro em input inválido
- [ ] Adicionar `/permissions` à lista do `/help`
- [ ] Adicionar teste: `/permissions set bash allow` persiste e `/permissions` lista

### Feature: Feedback e edge cases

- [ ] Mensagem de sucesso apontando o arquivo (`rustclaw.json`)
- [ ] Erro claro se tool desconhecida ou regra inválida
- [ ] Adicionar teste: `/permissions set` com regra inválida retorna erro

### Definition of done P3

- [ ] `cargo test` verde (incl. testes de `/permissions`)
- [ ] `cargo check` verde
- [ ] `cargo clippy --bin rustclaw` sem novos warnings
- [ ] `/permissions` lista, define e remove regras persistentes

---

## V0 — Verificação final + commit

**Objetivo:** garantir que tudo compila, testa e está documentado antes do commit.

### Feature: Build e lint

- [ ] `cargo fmt`
- [ ] `cargo check`
- [ ] `cargo test` (todos os testes)
- [ ] `cargo clippy --bin rustclaw` (sem novos warnings)

### Feature: Smoke test manual (se possível)

- [ ] Rodar `cargo run` e usar `/undo` para reverter um turno
- [ ] Verificar que o transcript do TUI reflete a sessão truncada após `/undo`
- [ ] Usar as tools `git_status`/`git_diff`/`git_log` num repo e confirmar output estruturado
- [ ] Rodar `/permissions set bash allow` e confirmar persistência em `rustclaw.json`
- [ ] Escolher "always" num prompt de permissão e confirmar que vale em runs futuras

### Feature: Commit

- [ ] `git add -A`
- [ ] Commit com mensagem descritiva, ex:
      `feat: /undo, git tools (status/diff/log) and persistent project permissions`
- [ ] Corpo do commit listando as fases (U0–U1, G0–G3, P0–P3)

### Definition of done V0

- [ ] Todos os testes verdes
- [ ] Clippy sem novos warnings
- [ ] Commit criado

---

## Ordem de execução

```text
U0 /undo (dispatcher) → U1 rebuild do transcript no TUI
 → G0 git_status → G1 git_diff → G2 git_log → G3 registro+allowlists+permissões
 → P0 campo permission no ProjectConfig → P1 carregamento no runtime
 → P2 persistência do always allow → P3 /permissions
 → V0 verificação + commit
```

Cada fase: `cargo test` + `cargo check` (+ `cargo clippy` no final).

---

## Riscos e mitigações

| Risco | Mitigação | Status |
|-------|-----------|--------|
| `/undo` deixa o transcript com mensagens obsoletas | Helper `rebuild_transcript_from_session` reconstrói `app.lines` a partir da sessão truncada | ⬜ |
| Git tools duplicam o `bash` genérico | Tools nativas com output estruturado/truncado; `bash` continua para casos gerais | ⬜ |
| Git tools quebram fora de repo git | `run_git` trata exit code != 0 com mensagem amigável ("not a git repository") | ⬜ |
| Merge de permissões quebra defaults | `PermissionConfig::merge` começa de `with_defaults()` e aplica overrides por cima | ⬜ |
| `PermissionConfig` sem `PartialEq/Eq` quebra derives do `ProjectConfig` | Adicionar `PartialEq, Eq` a `PermissionConfig` | ⬜ |
| Persistência do "always allow" sobrescreve regras manuais | `set_rule` atualiza só a tool escolhida; `rm` restaura o default | ⬜ |
| Git tools pedem confirmação desnecessária | Adicionadas ao grupo `Rule::Allow` (read-only) | ⬜ |

---

## Arquivos principais a tocar

| Path | Mudança |
|------|---------|
| `src/harness/ui/commands/mod.rs` | `/undo` + `/permissions` (list/set/rm) |
| `src/harness/ui/tui/app.rs` | `rebuild_transcript_from_session` + integração `/undo` + persistência "always" |
| `src/harness/ui/cli.rs` | persistência "always" no CLI |
| `src/harness/tool/git.rs` (novo) | `GitStatusTool`, `GitDiffTool`, `GitLogTool` + helper `run_git` |
| `src/harness/tool/mod.rs` | `pub mod git;` |
| `src/harness/runtime.rs` | registro das git tools + carregamento de permissões persistentes |
| `src/harness/agent/builtin.rs` | git tools no `READONLY_TOOLS` |
| `src/harness/permission/mod.rs` | `PartialEq/Eq` em `PermissionConfig` + `merge` + `set_rule`/`rules`/`remove_rule` + git tools em `with_defaults` |
| `src/harness/project/config_file.rs` | campo `permission: Option<PermissionConfig>` + `is_empty` |

---

## Notas de progresso

| Data | Nota |
|------|------|
| 2026-09-04 | TODO.md recriado: plano dos itens 1 (undo), 3 (git tools) e 5 (permissões persistentes) do SUGGESTIONS.md convertido em features U0–U1, G0–G3, P0–P3 + V0 com checklists detalhados. Conteúdo anterior (M0–M5, já concluído) substituído. |
