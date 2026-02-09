# RustClaw - Raspberry Pi Edition

Versão otimizada do RustClaw para Raspberry Pi 3 Model B com 1GB RAM.

## Diferenças da Versão Desktop

| Feature | Desktop | Raspberry Pi |
|---------|---------|--------------|
| **Embeddings** | fastembed (local) | API OpenAI/Cohere |
| **Browser** | Playwright (Chromium) | Removido |
| **Agendamento** | tokio-cron-scheduler | cron do Linux |
| **Busca Web** | Tavily API | Tavily API |
| **Memória RAM** | ~500-800MB | ~150-250MB |
| **Tamanho** | ~50-100MB | ~15-25MB |

## Requisitos

- Raspberry Pi 3 Model B (ou superior)
- 1GB RAM (compartilhada com GPU)
- 20GB armazenamento (SD Card)
- Raspberry Pi OS Lite (64-bit recomendado)
- Swap de 1GB configurado

## Instalação

### 1. Preparar o Raspberry Pi

```bash
# Atualizar sistema
sudo apt update && sudo apt upgrade -y

# Instalar dependências
sudo apt install -y sqlite3 libsqlite3-dev pkg-config

# Configurar swap de 1GB
sudo dphys-swapfile swapoff
sudo nano /etc/dphys-swapfile
# Alterar: CONF_SWAPSIZE=1024
sudo dphys-swapfile setup
sudo dphys-swapfile swapon
```

### 2. Instalar Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

### 3. Configurar Variáveis de Ambiente

```bash
# Criar arquivo .env
nano ~/.env

# Adicionar:
HF_TOKEN=seu_token_huggingface
TAVILY_API_KEY=sua_chave_tavily
OPENAI_API_KEY=sua_chave_openai
TELEGRAM_TOKEN=seu_token_telegram
TELEGRAM_CHAT_ID=seu_chat_id
```

### 4. Compilar

#### Opção A: Compilar no Raspberry Pi (lento, ~2-3 horas)

```bash
cd rustclaw
cargo build --release --jobs 1
```

#### Opção B: Cross-compilar no PC (rápido, ~5 minutos)

No seu PC (macOS/Linux):

```bash
# Instalar cross
cargo install cross --git https://github.com/cross-rs/cross

# Build
cross build --target aarch64-unknown-linux-gnu --release

# Copiar para Raspberry Pi
scp target/aarch64-unknown-linux-gnu/release/rustclaw pi@raspberrypi.local:~/
```

### 5. Executar

```bash
# No Raspberry Pi
chmod +x rustclaw
./rustclaw --mode telegram
```

## Configuração de Agendamento (cron)

Como o scheduler foi removido, use o cron do Linux:

```bash
# Editar crontab
sudo crontab -e

# Adicionar heartbeat diário às 8h:
0 8 * * * /home/pi/rustclaw --mode telegram --task heartbeat

# Verificar logs
grep CRON /var/log/syslog
```

## Otimizações Aplicadas

### Cargo.toml
- `opt-level = "z"` - Otimização para tamanho
- `lto = true` - Link Time Optimization
- `panic = "abort"` - Menor código de erro
- `strip = true` - Remover símbolos de debug

### Dependências
- Tokio com features mínimas
- Reqwest sem features desnecessárias
- SQLite sem bundle (usa sistema)
- fastembed removido (usa API externa)

### Código
- Limites de memória reduzidos
- Histórico de conversa: 10 mensagens (era 20)
- Max tokens: 256 (era 500)
- Timeout: 30s (era 60s)

## Troubleshooting

### "Out of memory" durante compilação
```bash
# Aumentar swap temporariamente
sudo dphys-swapfile swapoff
sudo nano /etc/dphys-swapfile  # CONF_SWAPSIZE=2048
sudo dphys-swapfile setup
sudo dphys-swapfile swapon

# Compilar com single thread
cargo build --release --jobs 1
```

### RAM insuficiente em runtime
```bash
# Verificar uso de memória
free -h
ps aux --sort=-%mem | head

# Matar processos pesados
sudo killall chromium-browser  # Se houver
```

### Problemas com embeddings
```bash
# Verificar se OPENAI_API_KEY está configurada
echo $OPENAI_API_KEY

# Testar API
curl https://api.openai.com/v1/embeddings \
  -H "Authorization: Bearer $OPENAI_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"input": "test", "model": "text-embedding-3-small"}'
```

## Uso de Recursos Esperado

| Recurso | Uso |
|---------|-----|
| RAM | 150-250MB |
| CPU | 5-15% (idle), 50-80% (processando) |
| Disco | ~20MB (binário) + dados SQLite |
| Swap | 100-500MB (depende da carga) |

## Comandos Disponíveis

### CLI
```
> sair                    # Sair do programa
> clear-memory            # Limpar memórias
> clear-all              # Limpar memórias e tarefas
> status                 # Ver status do sistema
```

### Telegram
```
/start      - Iniciar o bot
/status     - Status do sistema
/tasks      - Listar tarefas agendadas
/clear_memory - Limpar memórias
/internet <query> - Pesquisar na internet
/help       - Ajuda
```

## Manutenção

### Backup
```bash
# Backup do banco de dados
cp data/memory_cli.db backup/$(date +%Y%m%d)_memory.db

# Backup via scp
scp pi@raspberrypi.local:~/data/memory_cli.db ./backup/
```

### Logs
```bash
# Ver logs
tail -f /var/log/rustclaw.log

# Ou se usando systemd
journalctl -u rustclaw -f
```

## Licença

MIT License - Veja LICENSE para detalhes.

## Contribuindo

Esta é uma versão otimizada específica para Raspberry Pi. 
Para versão completa (desktop), veja a branch main.
