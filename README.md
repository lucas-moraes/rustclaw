# RustClaw - Raspberry Pi Edition

Agente AI em Rust otimizado para Raspberry Pi 3 Model B com 1GB RAM. Interface via Telegram ou CLI, com memória persistente em SQLite.

## ✨ Características

- 🤖 **Agente AI** com arquitetura ReAct
- 💾 **Memória persistente** via SQLite
- 🔍 **Busca na internet** via Tavily API
- 💬 **Interface** via Telegram Bot ou CLI
- 🧠 **Embeddings** via OpenAI API (com fallback offline)
- ⚡ **Otimizado** para baixo consumo de RAM (~150-250MB)

## 📋 Requisitos

### Hardware
- Raspberry Pi 3 Model B (ou superior)
- 1GB RAM (compartilhada com GPU)
- 20GB+ armazenamento (SD Card)
- Conexão internet

### Sistema
- Raspberry Pi OS Lite (64-bit recomendado)
- Swap de 1GB configurado
- Acesso SSH (para setup remoto)

### API Keys Necessárias
- [Hugging Face](https://huggingface.co/settings/tokens) - Para LLM
- [Tavily](https://app.tavily.com) - Para busca na web
- [OpenAI](https://platform.openai.com/api-keys) - Para embeddings (opcional, tem fallback)
- [Telegram Bot](https://t.me/botfather) - Para bot do Telegram

---

## 🚀 Instalação

### Opção 1: Cross-Compile no PC (Recomendado - 5 minutos)

Mais rápido! Compile no seu computador e transfira para o Raspberry Pi.

#### No PC (macOS/Linux):

```bash
# 1. Entrar no diretório do projeto
cd rustclaw

# 2. Instalar cross (se não tiver)
cargo install cross --git https://github.com/cross-rs/cross

# 3. Build para ARM64
cross build --target aarch64-unknown-linux-gnu --release

# 4. Verificar binário foi criado
ls -lh target/aarch64-unknown-linux-gnu/release/rustclaw
```

#### Transferir para Raspberry Pi:

```bash
# Copiar binário para o Raspberry Pi
scp target/aarch64-unknown-linux-gnu/release/rustclaw pi@raspberrypi.local:~/

# Ou copiar para o SD card diretamente
```

#### No Raspberry Pi:

```bash
# Tornar executável
chmod +x ~/rustclaw

# Testar
./rustclaw --help
```

---

### Opção 2: Build Nativo no Raspberry Pi (2-3 horas)

Compile diretamente no Raspberry Pi (mais lento, mas não precisa de PC).

#### 1. Preparar o Sistema

```bash
# Atualizar sistema
sudo apt update && sudo apt upgrade -y

# Instalar dependências
sudo apt install -y sqlite3 libsqlite3-dev pkg-config libssl-dev

# Configurar swap de 1GB (ESSENCIAL!)
sudo dphys-swapfile swapoff
sudo nano /etc/dphys-swapfile
# Alterar: CONF_SWAPSIZE=1024
sudo dphys-swapfile setup
sudo dphys-swapfile swapon
```

#### 2. Instalar Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

#### 3. Clonar e Compilar

```bash
# Copiar o projeto para o Raspberry Pi
# (via git clone, scp, ou pendrive)

# Entrar no diretório
cd rustclaw

# Compilar (use --jobs 1 para economizar RAM)
cargo build --release --jobs 1

# O binário estará em:
# target/release/rustclaw
```

---

## ⚙️ Configuração

### 1. Criar Arquivo de Variáveis

```bash
# Criar diretório de dados
mkdir -p ~/data

# Criar arquivo .env
nano ~/.env
```

Adicione suas API keys:

```bash
# Hugging Face API Token (obrigatório)
HF_TOKEN=seu_token_hf_aqui

# Tavily API Key (obrigatório para busca)
TAVILY_API_KEY=sua_chave_tavily_aqui

# OpenAI API Key (opcional, para embeddings)
# Se não fornecido, usa fallback offline
OPENAI_API_KEY=sua_chave_openai_aqui

# Telegram Bot Token (obrigatório para modo telegram)
TELEGRAM_TOKEN=seu_token_bot_aqui

# Telegram Chat ID (opcional, restringe acesso)
# Deixe em branco para permitir qualquer chat
TELEGRAM_CHAT_ID=seu_chat_id_aqui
```

### 2. Carregar Variáveis

```bash
# Carregar variáveis
source ~/.env

# Ou adicionar ao .bashrc para carregar automaticamente
echo 'source ~/.env' >> ~/.bashrc
```

---

## 🤖 Executar

### Modo CLI (Terminal)

```bash
./rustclaw --mode cli

# Você verá:
# > 
# Digite mensagens ou comandos:
# - sair: Encerrar
# - clear-memory: Limpar memórias
# - clear-all: Limpar memórias e tarefas
# - status: Ver status
```

### Modo Telegram

```bash
./rustclaw --mode telegram

# O bot ficará rodando e responderá mensagens no Telegram
```

**Comandos disponíveis no Telegram:**
- `/start` - Iniciar o bot
- `/status` - Status do sistema
- `/tasks` - Listar tarefas
- `/clear_memory` - Limpar memórias
- `/internet <consulta>` - Buscar na web
- `/help` - Ajuda

---

## ⚡ Configurar Systemd (Iniciar Automaticamente)

Para o RustClaw iniciar automaticamente no boot:

### 1. Copiar Arquivos de Configuração

```bash
# Copiar service file
sudo cp rustclaw.service /etc/systemd/system/

# Criar diretórios
sudo mkdir -p /etc/rustclaw /var/lib/rustclaw /var/log/rustclaw
sudo chown -R pi:pi /var/lib/rustclaw /var/log/rustclaw
```

### 2. Configurar Variáveis

```bash
sudo nano /etc/rustclaw/.env
# (adicione as mesmas variáveis do ~/.env)
```

### 3. Ativar Serviço

```bash
# Recarregar systemd
sudo systemctl daemon-reload

# Habilitar início automático
sudo systemctl enable rustclaw

# Iniciar serviço
sudo systemctl start rustclaw

# Verificar status
sudo systemctl status rustclaw
```

### Comandos Úteis

```bash
# Iniciar/Parar/Reiniciar
sudo systemctl start rustclaw
sudo systemctl stop rustclaw
sudo systemctl restart rustclaw

# Ver logs
sudo tail -f /var/log/rustclaw/rustclaw.log
sudo tail -f /var/log/rustclaw/rustclaw-error.log

# Ver status completo
sudo systemctl status rustclaw
```

---

## 🛠️ Solução de Problemas

### Erro: "cannot find -lsqlite3"

```bash
sudo apt install libsqlite3-dev
```

### Erro: "cannot find -lssl"

```bash
sudo apt install libssl-dev
```

### Erro: "Out of memory" durante compilação

```bash
# Aumentar swap para 2GB temporariamente
sudo dphys-swapfile swapoff
sudo nano /etc/dphys-swapfile  # CONF_SWAPSIZE=2048
sudo dphys-swapfile setup
sudo dphys-swapfile swapon

# Compilar com thread única
cargo build --release --jobs 1
```

### Serviço não inicia

```bash
# Verificar erro
sudo systemctl status rustclaw

# Ver logs
sudo journalctl -u rustclaw --no-pager | tail -50

# Verificar permissões
ls -la /home/pi/rustclaw
ls -la /etc/rustclaw/.env
```

### Bot não responde no Telegram

1. Verifique se `TELEGRAM_TOKEN` está correto
2. Verifique se iniciou o bot com `/start`
3. Verifique logs: `sudo tail -f /var/log/rustclaw/rustclaw.log`

---

## 📊 Uso de Recursos

| Recurso | Consumo |
|---------|---------|
| **RAM** | 150-250MB |
| **CPU** | 5-15% (idle), 50-80% (processando) |
| **Disco** | ~20MB (binário) + dados SQLite |
| **Swap** | 100-500MB (depende da carga) |

---

## 🔧 Funcionalidades Disponíveis

### Ferramentas (10 total)

1. **file_list** - Listar diretórios
2. **file_read** - Ler arquivos
3. **file_write** - Escrever arquivos
4. **file_search** - Buscar arquivos
5. **shell** - Executar comandos shell (seguro)
6. **http_get** - Requisições HTTP GET
7. **http_post** - Requisições HTTP POST
8. **system_info** - Informações do sistema
9. **echo** - Teste
10. **capabilities** - Listar capacidades

### Memória
- Persistente em SQLite
- Busca semântica com embeddings
- Histórico de 10 mensagens
- Tipos: Fact, Episode, ToolResult

### Integrações
- ✅ Hugging Face (LLM)
- ✅ Tavily (busca web)
- ✅ OpenAI (embeddings, opcional)
- ✅ Telegram Bot

---

## 🔄 Atualizando

### Atualizar Binário

```bash
# 1. Parar serviço
sudo systemctl stop rustclaw

# 2. Copiar novo binário (do PC)
scp target/aarch64-unknown-linux-gnu/release/rustclaw pi@raspberrypi.local:~/rustclaw

# 3. No Raspberry Pi, dar permissão
chmod +x ~/rustclaw

# 4. Iniciar serviço
sudo systemctl start rustclaw
```

### Backup das Memórias

```bash
# Backup
sudo tar -czf backup-$(date +%Y%m%d).tar.gz ~/data/

# Ou copiar para PC
scp pi@raspberrypi.local:~/data/memory_cli.db ./backup/
```

---

## 📝 Configuração de Agendamento (Cron)

Como o scheduler integrado foi removido, use o cron do Linux:

```bash
# Editar crontab
sudo crontab -e

# Exemplo: Heartbeat diário às 8h
0 8 * * * /usr/bin/curl -X POST http://localhost:8080/heartbeat

# Ou script personalizado
0 */6 * * * /home/pi/scripts/check-system.sh
```

---

## 🆚 Diferenças da Versão Desktop

| Feature | Desktop | Raspberry Pi |
|---------|---------|--------------|
| **Embeddings** | fastembed local | OpenAI API |
| **Browser** | Playwright | Removido |
| **Agendamento** | Integrado | Cron Linux |
| **RAM** | ~500-800MB | ~150-250MB |
| **Tamanho** | ~50-100MB | ~15-25MB |

---

## 📄 Licença

MIT License

---

## 🤝 Contribuindo

Este é um projeto otimizado específico para Raspberry Pi. Para a versão completa desktop, consulte a branch `main`.

---

## 💡 Dicas

1. **Use swap de 1GB** - Essencial para evitar "Out of memory"
2. **Prefira cross-compile** - Muito mais rápido que build nativo
3. **Monitore logs** - `sudo tail -f /var/log/rustclaw/rustclaw.log`
4. **Backup regular** - Faça backup do diretório `data/`
5. **Atualize o sistema** - `sudo apt update && sudo apt upgrade`

---

**Pronto!** Agora você tem o RustClaw rodando no Raspberry Pi 3! 🎉

Para dúvidas ou problemas, consulte o arquivo `SYSTEMD-GUIDE.md` ou verifique os logs do sistema.
