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

```bash
git clone <repo> rustclaw
cd rustclaw
cp .env.example config/.env
# edite config/.env (TOKEN, PROVIDER, MODEL, BASE_URL)
```

## ⚙️ Configuração (`config/.env`)

```bash
TOKEN=your_api_key_here
PROVIDER=deepinfra          # opencode-go | deepinfra | openrouter | moonshot | ...
MODEL=deepseek-ai/DeepSeek-V4-Flash-0731
BASE_URL=https://api.deepinfra.com/v1/openai

MAX_TOKENS=16000
MAX_ITERATIONS=50
MAX_CONTEXT_TOKENS=100000
```

| Var | Default | Descrição |
|-----|---------|-----------|
| `TOKEN` | — (obrigatório) | API key do provider |
| `PROVIDER` | `opencode-go` | Provider usado |
| `MODEL` | por provider | Modelo |
| `BASE_URL` | por provider | Endpoint OpenAI/Anthropic-compatível |
| `MAX_TOKENS` | 16000 | Tokens máximos por resposta |
| `MAX_ITERATIONS` | 50 | Iterações máximas por turno |
| `MAX_CONTEXT_TOKENS` | 100000 | Limite de contexto antes de compactar |
| `RUSTCLAW_THEME` | `cyberclaw` | Tema da TUI: `cyberclaw`·`aurora`·`ember`·`mono` |
| `RUSTCLAW_SKILLS` | — | Skills da session via CLI (`id1,id2`) |
| `RUSTCLAW_SKILLS_DIR` | — | Diretório extra de `SKILL.md` |
| `RUSTCLAW_UI` | `auto` | Forçar `tui` ou `cli` |

## 🤖 Uso

```bash
cargo run                # TUI Cyberclaw (requer terminal); fallback para CLI streaming
```

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
| `/sessions` | Listar sessões |
| `/resume <id>` | Resumir sessão |
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
├── config.rs        # Configuração por env
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

Um smoke test ao vivo (`--ignored`) valida native tool calling contra um provider real usando a key de `config/.env`:

```bash
cargo test --bin rustclaw smoke_native_tool_calling -- --ignored --nocapture
```

## 📄 Licença

MIT
