# RustClaw

Coding agent harness em Rust, no estilo OpenCode / Claude Code. Loop de agente com **native tool calling**, sessões persistidas em SQLite, **skills** (memória da sessão), TUI animada, permissões human-in-the-loop, agents builtin e subagentes.

## ✨ Features

- 🤖 **Native tool calling** — tools descritas via JSON Schema; sem parsing ReAct em texto livre
- ⚡ **Execução paralela** — múltiplos tool calls independentes rodam concorrentemente
- 💾 **Sessões SQLite** — messages/parts persistidos; listar, resumir, deletar
- 🧠 **Skills (memória da sessão)** — escolha skills por sessão, injetadas no prompt por turno (checkbox)
- 🔐 **Permissions HITL** — allow / ask / deny por tool e path (y/n/always no CLI)
- 🎭 **Agents** — `build`, `plan`, `explore`, `general` + subagente via tool `task`
- 🛠️ **Tools de coding** — bash, read, write, edit, glob, grep, todo, question, task
- 🎨 **TUI Cyberclaw (ratatui + crossterm)** — tema colorido (4 temas trocáveis), splash animado, transcript em bubbles, status bar com tokens/contexto, command palette, permission/question modals, diff colorido. CLI streaming como fallback (`RUSTCLAW_UI=cli` ou non-TTY).
- 🔁 **Compaction** — resume de contexto em overflow
- 🐛 **Doom-loop detection** — para quando o agente repete a mesma tool call

## Supported Providers

| Provider | Base URL (default) |
|----------|--------------------|
| **DeepInfra** | `https://api.deepinfra.com/v1/openai` |
| **opencode-go** | `https://opencode.ai/zen/go/v1` |
| OpenRouter | `https://openrouter.ai/api/v1` |
| Moonshot | `https://api.moonshot.ai/v1` |
| VillaMarket | `https://api.minimax.villamarket.ai/v1` |
| HuggingFace | `https://router.huggingface.co/v1` |

Qualquer provider OpenAI-compatível funciona via `BASE_URL` + `Authorization: Bearer`.

## 🚀 Instalação

macOS (Apple Silicon) e Linux x64, via release pré-compilada:

```bash
curl -fsSL https://raw.githubusercontent.com/lucas-moraes/rustclaw/main/scripts/install.sh | bash
```

O instalador baixa a última release, valida o SHA-256, copia para `~/.local/bin/rustclaw`
e ajusta o PATH se necessário. Versão específica:

```bash
RUSTCLAW_VERSION=v0.2.0 curl -fsSL .../install.sh | bash
```

Instalação a partir do código-fonte (desenvolvimento):

```bash
git clone git@github.com:lucas-moraes/rustclaw.git && cd rustclaw
./scripts/link-local.sh   # cargo build --release + copia para ~/.local/bin
# ou: cargo install --path .
```

## ⚙️ Configuração (sem `.env`)

Toda a configuração vive em arquivos — nenhuma variável de ambiente obrigatória:

| Arquivo | Escopo | Conteúdo |
|---------|--------|----------|
| `~/.local/share/rustclaw/auth.json` | global | API key por provider (chmod 600) |
| `~/.local/share/rustclaw/config.json` | global | provider/model, `max_iterations`, `max_context_tokens`, tema |
| `rustclaw.json` (raiz do projeto) | por projeto | provider/model/base_url (grava pelo `/models`) |

Na **primeira execução**, a TUI abre um wizard: escolha **provider → modelo** (`/models`)
e cole o **token** (`/auth <provider>`) — nada é editado manualmente. Comandos de config:

| Comando | O que faz |
|---------|-----------|
| `/models` | picker de provider/modelo (grava `rustclaw.json` do projeto) |
| `/auth <provider>` | salva o token no `auth.json` (input mascarado) |
| `/settings` | ver/editar `max_iterations`, `max_context_tokens`, tema |
| `/provider <nome>` | troca de provider (default model do catálogo) |
| `/model <nome>` | troca de modelo |

Precedência: **catálogo builtin → `config.json` → `rustclaw.json` → token do `auth.json`**.

## 🤖 Uso

```bash
rustclaw                 # TUI (qualquer diretório); CLI se não for TTY
RUSTCLAW_UI=cli rustclaw # força CLI streaming
cargo run                # no diretório do projeto (dev)

### Skills: memória da sessão

Ao abrir uma **nova sessão**, a TUI abre um **picker de skills** — escolha as skills que
compõem a memória daquela sessão (pode selecionar nenhuma e mudar depois).

Antes de **cada prompt**, as skills escolhidas aparecem como **chips checkbox** acima do input;
só as marcadas entram no prompt daquele turno.

### TUI atalhos

| Tecla | Ação |
|-------|------|
| `Ctrl+C` | cancela run ativo / sai quando idle |
| `Enter` | envia prompt |
| `Ctrl+P` | command palette (comandos/agents/temas/actions) |
| `Ctrl+T` | troca tema (cyberclaw·aurora·ember·mono) |
| `Ctrl+S` | foca os chips de skills (navega com `←/→`, marca com `Space`) |
| `Ctrl+L` | limpa o transcript local |
| `?` / `F1` | help overlay (seções com `Tab`) |
| `Esc` | limpa input / fecha overlay |
| `↑/↓` | histórico (ou navega nos chips com foco) |
| `PgUp/PgDn` ou mouse wheel | scroll transcript |
| `y`/`n`/`a` | modal de permissão (allow/deny/always) |
| `1..n` | modal de pergunta (escolher opção) |
| `Space` | marca/desmarca skill no picker e nos chips |
| `Tab` | autocomplete `/` / próxima seção do help |

Prompt simples:
```
› Liste os arquivos .rs em src/ usando glob e leia src/main.rs
```

### Slash commands

| Comando | Descrição |
|---------|-----------|
| `/new` | Nova sessão (abre o picker de skills) |
| `/sessions` | Gerenciar sessões (picker: listar/selecionar/excluir/renomear) |
| `/sessions select <id>` | Selecionar sessão por id |
| `/agent <name>` | Trocar agent (build/plan/explore/general) |
| `/compact` | Compactar contexto manualmente |
| `/skills` | Gerir skills (`list`·`add <id>`·`rm <id>`·`default <id> on\|off`·`picker`) |
| `/theme [name]` | Listar ou aplicar tema |
| `/usage` | Tokens in/out + janela de contexto da sessão |
| `/help` / `/exit` | Ajuda / sair |

### Permissões

Tools destrutivas (`bash`, `write`, `edit`, `task`) pedem confirmação:
```
[permission] write (path: /proj/x.rs) {...}
  allow? [y]es/[n]o/[a]lways:
```
`a`/`always` aprova a tool pelo resto da sessão. Paths fora do working directory são sempre escalados para `ask`.

## 🧱 Arquitetura

```
src/
├── main.rs          # Entry point
├── config.rs        # GlobalSettings (config.json) + resolução
├── error.rs         # Tipos de erro
└── harness/         # O harness em si
    ├── event.rs     # Event bus
    ├── runtime.rs   # SessionRuntime (facade)
    ├── skill/       # Skills = memória da sessão (loader + inject)
    ├── session/     # Session/Message/Part + store SQLite + processor + compaction
    ├── provider/    # OpenAI, Anthropic, opencode-go adapters (streaming + native tools)
    ├── tool/        # Trait Tool + registry + bash/read/write/edit/glob/grep/todo/question/task
    ├── permission/  # allow/ask/deny
    ├── agent/       # AgentSpec + builtin (build/plan/explore/general)
    └── ui/          # tui/ (app, draw, input, askers) + cli/ (fallback) + commands/
```

O loop central (`session/processor.rs`): envia o prompt ao provider com as tools nativas,
consome o stream, executa tool calls (paralelo via JoinSet, com permission check), e repete
até o modelo responder sem tools. O **system prompt** é montado por
`agent/build_system_prompt` (identidade + Environment + AGENTS.md + skills marcadas +
operating rules); o **histórico** da sessão vira as mensagens. Veja `docs/ARCHITECTURE.md`.

## 🧪 Testes

```bash
cargo test            # suite unitária (harness)
cargo clippy          # lint
cargo fmt --check     # formatação
```

Um smoke test ao vivo (`--ignored`) valida native tool calling contra um provider real usando o token do auth store (`~/.local/share/rustclaw/auth.json`):

```bash
cargo test --bin rustclaw smoke_native_tool_calling -- --ignored --nocapture
```

## 📄 Licença

MIT
