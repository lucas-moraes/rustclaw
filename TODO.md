# RustClaw — Suporte a MCP (Model Context Protocol)

> **Problema:** o RustClaw só enxerga as tools builtin. Não há como plugar
> servidores MCP (filesystem, github, postgres, etc.) que o ecossistema já
> oferece — cada integração exigiria uma tool nativa nova.
>
> **Escopo:** 4 fases: config/parsing (F1) → cliente stdio + tools no registry
> (F2) → permissões/agentes/UX (F3) → robustez + transporte HTTP (F4).
>
> **Decisões-chave:**
> - Crate [`rmcp`](https://crates.io/crates/rmcp) (cliente MCP oficial, async/tokio),
>   features mínimas (`client`, `transport-child-process`).
> - F1–F3: só transporte **stdio** (subprocesso `command/args/env`). F4: HTTP.
> - Nomes de tool: `mcp_<server>_<tool>` (ex.: `mcp_github_create_issue`).
> - Config: `~/.local/share/rustclaw/mcp.json` (global) + `rustclaw.json["mcp"]`
>   (projeto, sobrescreve por nome). Formato padrão `mcpServers` (Claude/Cursor).
> - Permissões: MCP tools caem no fallback `Ask` (default) — zero mudança no
>   engine; `/permissions set mcp_<server>_<tool> allow` já funciona.

---

## Visão geral das features

| ID | Feature | Fase | Status |
|----|---------|------|--------|
| F1.1 | `McpServerConfig` + parse do formato `mcpServers` | 1 | ✅ |
| F1.2 | Load/merge global (`mcp.json`) + projeto (`rustclaw.json`) | 1 | ✅ |
| F1.3 | `/mcp list` placeholder (servers configurados) | 1 | ✅ |
| F2.1 | Dependência `rmcp` + módulo `harness/mcp/` | 2 | ✅ |
| F2.2 | `McpClient`: spawn stdio + handshake + `tools/list` | 2 | ✅ |
| F2.3 | `McpTool`: impl `Tool` → `tools/call` | 2 | ✅ |
| F2.4 | `McpManager::connect_all` (paralelo, falha isolada) | 2 | ✅ |
| F2.5 | Registro das MCP tools no `ToolRegistry` do runtime | 2 | ✅ |
| F3.1 | Allowlist por agente (readonly p/ plan/explore) | 3 | ✅ |
| F3.2 | `/mcp list\|status\|restart` | 3 | ✅ |
| F3.3 | TUI: help + status de servers | 3 | ✅ |
| F4.1 | Reconnect com backoff ao morrer o subprocesso | 4 | ✅ |
| F4.2 | Health check periódico (`ping`) | 4 | ✅ |
| F4.3 | Transporte streamable HTTP + `Authorization` | 4 | ✅ |
| F4.4 | Env expansion (`${VAR}`) em `env`/`args` | 4 | ✅ |
| F4.5 | Docs (`docs/FEATURES.md` §MCP) | 4 | ✅ |
| V | Verificação final + commit | 4 | ✅ |

**Legenda:** ⬜ pendente · 🟡 em progresso · ✅ feito · ❌ cancelado

---

## F1 — Config + parsing (sem cliente ainda)

**Objetivo:** ler e validar a config de MCP servers, sem conectar nada.

### Feature F1.1: `McpServerConfig` + parse

- [x] Criar `src/harness/mcp/mod.rs` (vazio por ora) e `src/harness/mcp/config.rs`
- [x] `McpServerConfig { command: String, args: Vec<String>, env: HashMap<String,String>, enabled: bool (default true), timeout_secs: u64 (default 60) }`
      com `serde::{Deserialize, Serialize}` + `#[serde(default)]`
- [x] `McpConfig { servers: HashMap<String, McpServerConfig> }` com parse do
      envelope `{"mcpServers": {...}}` (formato Claude/Cursor)
- [x] Rejeitar entrada sem `command` (F4 adiciona `url` como alternativa)
- [x] Testes: parse do formato padrão; `enabled=false`; entrada inválida (sem command)

### Feature F1.2: Load/merge global + projeto

- [x] `McpConfig::load_global()` lê `~/.local/share/rustclaw/mcp.json`
      (via `dirs::data_local_dir()` — no macOS é `~/Library/Application Support/rustclaw/`)
- [x] `McpConfig::load_project(root)` lê `rustclaw.json["mcp"]["mcpServers"]`
- [x] `McpConfig::merge(global, project)`: projeto sobrescreve server de mesmo nome
- [x] Arquivo inexistente → config vazia (não é erro); JSON inválido → erro com path
- [x] Testes: merge com override por nome; arquivo ausente; JSON inválido

### Feature F1.3: `/mcp list` placeholder

- [x] Em `src/harness/ui/commands/mod.rs`: comando `/mcp list` mostra servers
      configurados (nome, command, enabled) — sem status de conexão ainda
- [x] `/mcp` sem args → usage
- [x] Adicionar `mcp` à palette do TUI (`src/harness/ui/tui/palette.rs`)

### Definition of done F1

- [x] `cargo test` verde (incl. testes novos de config)
- [x] `cargo check` verde
- [x] `/mcp list` mostra servers de um `mcp.json` de exemplo

---

## F2 — Cliente stdio + tools no registry (MVP utilizável)

**Objetivo:** conectar nos servers configurados e expor as tools deles ao modelo.

### Feature F2.1: Dependência + módulo

- [x] `Cargo.toml`: `rmcp` com features mínimas (`client`, `transport-child-process`);
      medir impacto no `cargo build` (se explodir, reavaliar)
- [x] Declarar `pub mod mcp;` em `src/harness/mod.rs`

### Feature F2.2: `McpClient` (stdio)

- [x] `src/harness/mcp/client.rs`: wrapper sobre `rmcp`
- [x] `McpClient::connect(name, cfg) -> Result<Self>`: spawn do subprocesso
      (`tokio::process::Command`, `kill_on_drop(true)`), handshake `initialize`,
      `tools/list` — tudo com timeout de conexão de 10s
- [x] `McpClient::call_tool(name, args) -> Result<String>`: `tools/call` com
      timeout `timeout_secs` do config; serializa content (text/image/resource)
      em texto único
- [x] `McpClient::tools() -> Vec<McpToolSpec>` (nome, descrição, inputSchema,
      `readOnlyHint` das annotations)
- [x] Testes com um server fake (script shell que fala JSON-RPC no stdio) ou
      mock do transporte

### Feature F2.3: `McpTool` (impl `Tool`)

- [x] `src/harness/mcp/tool.rs`: `McpTool { server: String, spec: McpToolSpec, client: Arc<McpClient> }`
- [x] `name()` → `mcp_<server>_<tool>` (sanitizar: lowercase, `[a-z0-9_]`)
- [x] `description()` → descrição do server truncada em 200 chars
- [x] `parameters()` → `inputSchema` do server (pass-through)
- [x] `execute()` → `client.call_tool`, respeitando `ctx.abort`
- [x] Colisão de nome com builtin → sufixo `_2` + warn no log
- [x] Testes: nome sanitizado; execute delega e serializa; abort cancela

### Feature F2.4: `McpManager::connect_all`

- [x] `src/harness/mcp/mod.rs`: `McpManager { clients: HashMap<String, Arc<McpClient>>, tools: Vec<Arc<McpTool>> }`
- [x] `connect_all(config) -> Self`: spawns em paralelo (`JoinSet`), um por server
      `enabled`; falha de um server → log warn + server marcado `failed`, **não**
      derruba os outros nem o startup
- [x] Cap de 50 tools por server (excesso → warn + trunca)
- [x] `status()` → snapshot nome → `Connected | Failed(String) | Disabled`

### Feature F2.5: Registro no runtime

- [x] `SessionRuntime` ganha `mcp: Option<Arc<McpManager>>>`
- [x] Em `runtime.rs` (perto de `build_default_registry`): após montar o registry,
      registrar cada `McpTool` do manager
- [x] Config carregada no boot do runtime (global + projeto, merge F1.2)
- [x] Sem servers configurados → `mcp: None`, zero overhead

### Definition of done F2

- [x] `cargo test` verde (incl. testes de client/tool/manager)
- [x] `cargo check` + `cargo clippy --bin rustclaw` limpos
- [x] Config com `npx -y @modelcontextprotocol/server-filesystem` (ou server fake
      local) → tools `mcp_filesystem_*` aparecem no registry e executam
- [x] Server que trava no handshake → `failed` em 10s, startup não bloqueia

---

## F3 — Permissões, agentes e UX

**Objetivo:** MCP tools se comportam como cidadãs de primeira classe.

### Feature F3.1: Allowlist por agente

- [x] `build`/`general`: MCP tools entram automaticamente (allowlist vazia = tudo)
- [x] `plan`/`explore`: MCP tools com `readOnlyHint: true` entram na allowlist
      readonly; mutáveis ficam fora
- [x] Implementar via hook no registry: `specs(allowlist)` aceita tools `mcp_*`
      readonly quando o agente é readonly
- [x] Testes: plan mode vê `mcp_x_read` mas não `mcp_x_write`

### Feature F3.2: `/mcp list|status|restart`

- [x] `/mcp list` → servers + tools expostas (contagem)
- [x] `/mcp status` → estado de conexão por server (do `McpManager::status()`)
- [x] `/mcp restart <name>` → derruba e reconecta um server, re-registra tools
- [x] Permissões: confirmar que `/permissions set mcp_<server>_<tool> allow`
      persiste e é respeitado (fallback `Ask` já funciona)

### Feature F3.3: TUI

- [x] Linha de MCP no `/help` (comandos `/mcp`)
- [x] Render de tool lines `mcp_*` como qualquer tool (já deve funcionar via
      `ToolStart`/`ToolEnd` — validar)
- [x] Modal de permissão `Ask` para MCP tool mostra server + tool + args

### Definition of done F3

- [x] `cargo test` verde
- [x] `cargo check` + `cargo clippy --bin rustclaw` limpos
- [x] Plan mode usa tools readonly de MCP; mutáveis pedem `Ask` no modal
- [x] `/mcp status` reflete servers conectados/falhos

---

## F4 — Robustez + transporte HTTP

**Objetivo:** operação confiável no dia a dia + servers remotos.

### Feature F4.1: Reconnect

- [x] `call_tool` detecta subprocesso morto (erro de transporte) → tenta respawn
      1x com backoff (1s) antes de falhar
- [x] Server marcado `failed` após 2 falhas consecutivas de respawn
- [x] Teste: matar o processo do server → próxima chamada reconecta

### Feature F4.2: Health check

- [x] Task de fundo no `McpManager`: `ping` a cada 60s por server
- [x] Ping falho → marca `failed` (visível no `/mcp status`); próxima chamada
      dispara reconnect (F4.1)
- [x] Task cancelada no `Drop` do manager

### Feature F4.3: Transporte streamable HTTP

- [x] `McpServerConfig` aceita `url: String` como alternativa a `command`
      (exatamente um dos dois obrigatório)
- [x] Header `Authorization: Bearer <token>` opcional (campo `headers` no config)
- [x] `McpClient::connect` escolhe transporte por `command` vs `url`
- [x] Testes: parse de config com `url`; validação command-xor-url

### Feature F4.4: Env expansion

- [x] `${VAR}` expandido em `env` e `args` a partir do ambiente do processo
- [x] Var indefinida → erro de config com nome da var
- [x] Testes: expansão; var ausente

### Feature F4.5: Docs

- [x] `docs/FEATURES.md`: seção MCP (config, formato `mcpServers`, naming
      `mcp_<server>_<tool>`, permissões, `/mcp`)
- [x] `AGENTS.md`: mencionar `src/harness/mcp/` na estrutura

### Definition of done F4

- [x] `cargo test` verde
- [x] `cargo check` + `cargo clippy --bin rustclaw` limpos
- [x] Matar o processo do server → próxima chamada reconecta
- [x] Server remoto via HTTP funciona (ou teste de integração com mock)

---

## V — Verificação final + commit

### Feature: Build e lint

- [x] `cargo fmt`
- [x] `cargo check`
- [x] `cargo test` (todos os testes)
- [x] `cargo clippy --bin rustclaw` (sem novos warnings)

### Feature: Smoke test manual (se possível)

- [x] `cargo run` com um `mcp.json` apontando para um server real (ex.: filesystem)
- [x] Pedir ao modelo algo que use uma tool MCP → confirmar execução + permissão
- [x] `/mcp status` mostra o server conectado

### Feature: Commit

- [x] `git add -A`
- [x] Commit com mensagem descritiva, ex:
      `feat: MCP client support (stdio) with config, registry and /mcp commands`
- [x] Corpo do commit listando as fases (F1–F4)

---

## Ordem de execução

```text
F1 config + /mcp list (PR 1) → F2 cliente stdio + registry (PR 2, MVP)
 → F3 agentes + UX (PR 3) → F4 robustez + HTTP (PR 4) → V verificação + commit
```

Cada fase: `cargo test` + `cargo check` (+ `cargo clippy` no final).
Sugestão: F1+F2 juntas entregam o MVP utilizável.

---

## Riscos e mitigações

| Risco | Mitigação |
|-------|-----------|
| Server trava no handshake | Timeout de conexão 10s; server fica `failed` e não bloqueia o startup (F2.4) |
| `tools/list` gigante incha o system prompt | Descrições truncadas (200 chars) + cap de 50 tools/server (F2.3/F2.4) |
| Nome colide com builtin | Prefixo `mcp_` obrigatório; conflito interno → sufixo `_2` (F2.3) |
| Subprocesso vaza ao sair | `kill_on_drop` + `Drop` no manager; abort da sessão cancela calls em voo (F2.2) |
| `rmcp` puxa deps pesadas | Features mínimas (`client`, `transport-child-process`); medir `cargo build` (F2.1) |
| Server morre no meio da sessão | Reconnect com backoff (F4.1) + health check (F4.2) |

---

## Arquivos principais a tocar

| Path | Mudança |
|------|---------|
| `Cargo.toml` | Dependência `rmcp` (features mínimas) |
| `src/harness/mod.rs` | `pub mod mcp;` |
| `src/harness/mcp/mod.rs` | `McpManager` (connect_all, status, restart, health) |
| `src/harness/mcp/config.rs` | `McpConfig`/`McpServerConfig` + load/merge global+projeto |
| `src/harness/mcp/client.rs` | `McpClient` (spawn, handshake, list/call, timeout, reconnect) |
| `src/harness/mcp/tool.rs` | `McpTool` impl `Tool` |
| `src/harness/runtime.rs` | Campo `mcp` + registro das MCP tools no registry |
| `src/harness/agent/builtin.rs` | Allowlist readonly p/ plan/explore (tools `mcp_*` com `readOnlyHint`) |
| `src/harness/ui/commands/mod.rs` | `/mcp list\|status\|restart` |
| `src/harness/ui/tui/palette.rs` | Entrada `mcp` na palette |
| `docs/FEATURES.md` | Seção MCP |
| `AGENTS.md` | `src/harness/mcp/` na estrutura |

---

## Notas de progresso

| Data | Nota |
|------|------|
| 2026-09-04 | TODO.md substituído: plano de suporte a MCP convertido em features F1–F4 + V com checklists detalhados. |
| 2026-09-04 | **F1–F4 + V concluídas** (256 testes, clippy/fmt limpos): config `mcpServers` (global+projeto, merge, `${VAR}` expansion), `McpClient` stdio + streamable HTTP (rmcp 3.2), `McpTool` (`mcp_<server>_<tool>`, `readOnlyHint`), `McpManager` (connect_all paralelo, falha isolada, reconnect 1x, health check 60s), registro no runtime (`init_mcp`), allowlist readonly p/ plan/explore (marcador `mcp_readonly`), `/mcp list\|status\|restart` + palette, docs (FEATURES.md §16, AGENTS.md). |
