#!/bin/bash
# Setup script for RustClaw systemd service
# Run this on your Raspberry Pi

set -e

echo "=== RustClaw Systemd Setup ==="
echo ""

# Check if running as root
if [ "$EUID" -ne 0 ]; then 
   echo "Please run as root (use sudo)"
   exit 1
fi

echo "Creating directories..."
mkdir -p /etc/rustclaw
mkdir -p /var/lib/rustclaw
mkdir -p /var/log/rustclaw

echo "Setting permissions..."
chown -R pi:pi /var/lib/rustclaw
chown -R pi:pi /var/log/rustclaw
chmod 755 /var/lib/rustclaw
chmod 755 /var/log/rustclaw

echo "Copying service file..."
cp rustclaw.service /etc/systemd/system/

echo "Creating environment file..."
if [ ! -f /etc/rustclaw/.env ]; then
    cat > /etc/rustclaw/.env << 'EOF'
# RustClaw Environment Variables
# Edit these values with your actual API keys

# Hugging Face API Token (required for LLM)
HF_TOKEN=your_huggingface_token_here

# Tavily API Key (required for web search)
TAVILY_API_KEY=your_tavily_key_here

# OpenAI API Key (optional, for embeddings)
OPENAI_API_KEY=your_openai_key_here

# Telegram Bot Token (required for Telegram mode)
TELEGRAM_TOKEN=your_telegram_bot_token_here

# Telegram Chat ID (optional, for restricting access)
TELEGRAM_CHAT_ID=your_chat_id_here
EOF
    echo ""
    echo "⚠️  IMPORTANT: Edit /etc/rustclaw/.env with your actual API keys!"
    echo "   Run: sudo nano /etc/rustclaw/.env"
    echo ""
else
    echo "Environment file already exists at /etc/rustclaw/.env"
fi

echo "Setting permissions for .env..."
chown root:root /etc/rustclaw/.env
chmod 600 /etc/rustclaw/.env

echo "Reloading systemd..."
systemctl daemon-reload

echo "Enabling service..."
systemctl enable rustclaw.service

echo ""
echo "=== Setup Complete ==="
echo ""
echo "Next steps:"
echo "1. Edit /etc/rustclaw/.env with your API keys"
echo "2. Start the service: sudo systemctl start rustclaw"
echo "3. Check status: sudo systemctl status rustclaw"
echo "4. View logs: sudo tail -f /var/log/rustclaw/rustclaw.log"
echo ""
