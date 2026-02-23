# RustClaw - Raspberry Pi Edition

AI Agent in Rust optimized for Raspberry Pi 3 Model B with 1GB RAM. Interface via Telegram or CLI, with persistent memory in SQLite.

## ✨ Features

- 🤖 **AI Agent** with ReAct architecture
- 💾 **Persistent memory** via SQLite
- 🔍 **Web search** via Tavily API
- 💬 **Interface** via Telegram Bot or CLI
- 🧠 **Embeddings** via OpenAI API (with offline fallback)
- ⚡ **Optimized** for low RAM consumption (~150-250MB)

## 📋 Requirements

### Hardware
- Raspberry Pi 3 Model B (or better)
- 1GB RAM (shared with GPU)
- 20GB+ storage (SD Card)
- Internet connection

### System
- Raspberry Pi OS Lite (64-bit recommended)
- 1GB swap configured
- SSH access (for remote setup)

### Required API Keys
- [Hugging Face](https://huggingface.co/settings/tokens) - For LLM
- [Tavily](https://app.tavily.com) - For web search
- [OpenAI](https://platform.openai.com/api-keys) - For embeddings (optional, has fallback)
- [Telegram Bot](https://t.me/botfather) - For Telegram bot

---

## 🚀 Installation

### Option 1: Cross-Compile on PC (Recommended - 5 minutes)

Faster! Compile on your computer and transfer to the Raspberry Pi.

#### On PC (macOS/Linux):

```bash
# 1. Enter the project directory
cd rustclaw

# 2. Install cross (if you don't have it)
cargo install cross --git https://github.com/cross-rs/cross

# 3. Build for ARM64
cross build --target aarch64-unknown-linux-gnu --release

# 4. Verify binary was created
ls -lh target/aarch64-unknown-linux-gnu/release/rustclaw
```

#### Transfer to Raspberry Pi:

```bash
# Copy binary to Raspberry Pi
scp target/aarch64-unknown-linux-gnu/release/rustclaw pi@raspberrypi.local:~/

# Or copy to SD card directly
```

#### On Raspberry Pi:

```bash
# Make executable
chmod +x ~/rustclaw

# Test
./rustclaw --help
```

---

### Option 2: Native Build on Raspberry Pi (2-3 hours)

Compile directly on Raspberry Pi (slower, but doesn't need a PC).

#### 1. Prepare the System

```bash
# Update system
sudo apt update && sudo apt upgrade -y

# Install dependencies
sudo apt install -y sqlite3 libsqlite3-dev pkg-config libssl-dev

# Configure 1GB swap (ESSENTIAL!)
sudo dphys-swapfile swapoff
sudo nano /etc/dphys-swapfile
# Change: CONF_SWAPSIZE=1024
sudo dphys-swapfile setup
sudo dphys-swapfile swapon
```

#### 2. Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

#### 3. Clone and Compile

```bash
# Copy the project to the Raspberry Pi
# (via git clone, scp, or USB drive)

# Enter the directory
cd rustclaw

# Compile (use --jobs 1 to save RAM)
cargo build --release --jobs 1

# The binary will be at:
# target/release/rustclaw
```

---

## ⚙️ Configuration

### 1. Create Environment Variables File

```bash
# Create data directory
mkdir -p ~/data

# Create .env file
nano ~/.env
```

Add your API keys:

```bash
# Hugging Face API Token (required)
HF_TOKEN=your_hf_token_here

# Tavily API Key (required for search)
TAVILY_API_KEY=your_tavily_key_here

# OpenAI API Key (optional, for embeddings)
# If not provided, uses offline fallback
OPENAI_API_KEY=your_openai_key_here

# Telegram Bot Token (required for telegram mode)
TELEGRAM_TOKEN=your_bot_token_here

# Telegram Chat ID (optional, restricts access)
# Leave empty to allow any chat
TELEGRAM_CHAT_ID=your_chat_id_here

# Max tokens for model responses (optional)
# Default: 1200
MAX_TOKENS=1200
```

### 2. Load Variables

```bash
# Load variables
source ~/.env

# Or add to .bashrc to load automatically
echo 'source ~/.env' >> ~/.bashrc
```

---

## 🤖 Running

### CLI Mode (Terminal)

```bash
./rustclaw --mode cli

# You will see:
# > 
# Type messages or commands:
# - exit: Quit
# - clear-memory: Clear memories
# - clear-all: Clear memories and tasks
# - status: Check status
```

### Telegram Mode

```bash
./rustclaw --mode telegram

# The bot will run and respond to messages on Telegram
```

**Available commands on Telegram:**
- `/start` - Start the bot
- `/status` - System status
- `/tasks` - List scheduled tasks
- `/clear_memory` - Clear memories
- `/internet <query>` - Search the web
- `/help` - Help

---

## ⚡ Configure Systemd (Start Automatically)

For RustClaw to start automatically on boot:

### 1. Copy Configuration Files

```bash
# Copy service file
sudo cp rustclaw.service /etc/systemd/system/

# Create directories
sudo mkdir -p /etc/rustclaw /var/lib/rustclaw /var/log/rustclaw
sudo chown -R pi:pi /var/lib/rustclaw /var/log/rustclaw
```

### 2. Configure Variables

```bash
sudo nano /etc/rustclaw/.env
# (add the same variables as in ~/.env)
```

### 3. Enable Service

```bash
# Reload systemd
sudo systemctl daemon-reload

# Enable auto-start
sudo systemctl enable rustclaw

# Start service
sudo systemctl start rustclaw

# Check status
sudo systemctl status rustclaw
```

### Useful Commands

```bash
# Start/Stop/Restart
sudo systemctl start rustclaw
sudo systemctl stop rustclaw
sudo systemctl restart rustclaw

# View logs
sudo tail -f /var/log/rustclaw/rustclaw.log
sudo tail -f /var/log/rustclaw/rustclaw-error.log

# View full status
sudo systemctl status rustclaw
```

---

## 🛠️ Troubleshooting

### Error: "cannot find -lsqlite3"

```bash
sudo apt install libsqlite3-dev
```

### Error: "cannot find -lssl"

```bash
sudo apt install libssl-dev
```

### Error: "Out of memory" during compilation

```bash
# Increase swap to 2GB temporarily
sudo dphys-swapfile swapoff
sudo nano /etc/dphys-swapfile  # CONF_SWAPSIZE=2048
sudo dphys-swapfile setup
sudo dphys-swapfile swapon

# Compile with single thread
cargo build --release --jobs 1
```

### Service doesn't start

```bash
# Check error
sudo systemctl status rustclaw

# View logs
sudo journalctl -u rustclaw --no-pager | tail -50

# Check permissions
ls -la /home/pi/rustclaw
ls -la /etc/rustclaw/.env
```

### Bot doesn't respond on Telegram

1. Check if `TELEGRAM_TOKEN` is correct
2. Check if you started the bot with `/start`
3. Check logs: `sudo tail -f /var/log/rustclaw/rustclaw.log`

---

## 📊 Resource Usage

| Resource | Consumption |
|---------|---------|
| **RAM** | 150-250MB |
| **CPU** | 5-15% (idle), 50-80% (processing) |
| **Disk** | ~20MB (binary) + SQLite data |
| **Swap** | 100-500MB (depends on load) |

---

## 🔧 Available Features

### Tools (10 total)

1. **file_list** - List directories
2. **file_read** - Read files
3. **file_write** - Write files
4. **file_search** - Search files
5. **shell** - Execute shell commands (safe)
6. **http_get** - HTTP GET requests
7. **http_post** - HTTP POST requests
8. **system_info** - System information
9. **echo** - Test
10. **capabilities** - List capabilities

### Memory
- Persistent in SQLite
- Semantic search with embeddings
- History of 10 messages
- Types: Fact, Episode, ToolResult

### Integrations
- ✅ Hugging Face (LLM)
- ✅ Tavily (web search)
- ✅ OpenAI (embeddings, optional)
- ✅ Telegram Bot

---

## 🔄 Updating

### Update Binary

```bash
# 1. Stop service
sudo systemctl stop rustclaw

# 2. Copy new binary (from PC)
scp target/aarch64-unknown-linux-gnu/release/rustclaw pi@raspberrypi.local:~/rustclaw

# 3. On Raspberry Pi, set permission
chmod +x ~/rustclaw

# 4. Start service
sudo systemctl start rustclaw
```

### Backup Memories

```bash
# Backup
sudo tar -czf backup-$(date +%Y%m%d).tar.gz ~/data/

# Or copy to PC
scp pi@raspberrypi.local:~/data/memory_cli.db ./backup/
```

---

## 📝 Scheduling Configuration (Cron)

Since the integrated scheduler was removed, use Linux cron:

```bash
# Edit crontab
sudo crontab -e

# Example: Daily heartbeat at 8am
0 8 * * * /usr/bin/curl -X POST http://localhost:8080/heartbeat

# Or custom script
0 */6 * * * /home/pi/scripts/check-system.sh
```

---

## 🆚 Differences from Desktop Version

| Feature | Desktop | Raspberry Pi |
|---------|---------|--------------|
| **Embeddings** | fastembed local | OpenAI API |
| **Browser** | Playwright | Removed |
| **Scheduling** | Integrated | Linux Cron |
| **RAM** | ~500-800MB | ~150-250MB |
| **Size** | ~50-100MB | ~15-25MB |

---

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

---

## 🤝 Contributing

This is a project specifically optimized for Raspberry Pi. For the full desktop version, see the `main` branch.

---

## 💡 Tips

1. **Use 1GB swap** - Essential to avoid "Out of memory"
2. **Prefer cross-compile** - Much faster than native build
3. **Monitor logs** - `sudo tail -f /var/log/rustclaw/rustclaw.log`
4. **Regular backups** - Backup the `data/` directory
5. **Update the system** - `sudo apt update && sudo apt upgrade`

---

**Ready!** You now have RustClaw running on Raspberry Pi 3! 🎉

For questions or issues, check the `SYSTEMD-GUIDE.md` file or view the system logs.
