#!/bin/bash
# setup_permissions.sh
# Script to setup correct permissions for RustClaw

echo "🔧 Configurando permissões para RustClaw..."

# Create data directory
mkdir -p data
echo "✅ Diretório data/ criado"

# Create memory database file if not exists
if [ ! -f "data/memory_cli.db" ]; then
    touch data/memory_cli.db
    echo "✅ Arquivo data/memory_cli.db criado"
fi

# Set permissions
chmod 755 data
chmod 644 data/memory_cli.db

echo "✅ Permissões configuradas"

# Verify
echo ""
echo "📁 Verificando estrutura:"
ls -la data/

echo ""
echo "✅ Setup concluído!"
echo "🚀 Execute: cargo run"
