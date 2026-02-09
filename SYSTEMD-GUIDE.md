# Systemd Setup Guide for RustClaw

## 📋 Arquivos Necessários

1. `rustclaw` - Seu binário compilado
2. `rustclaw.service` - Arquivo de serviço systemd
3. `setup-systemd.sh` - Script de instalação automática

## 🚀 Instalação no Raspberry Pi

### Opção 1: Script Automático (Recomendado)

```bash
# 1. Copiar arquivos para o Raspberry Pi
scp rustclaw rustclaw.service setup-systemd.sh pi@raspberrypi.local:~/

# 2. Conectar ao Raspberry Pi
ssh pi@raspberrypi.local

# 3. Executar script de setup
chmod +x setup-systemd.sh
sudo ./setup-systemd.sh
```

### Opção 2: Instalação Manual

```bash
# 1. Criar diretórios
sudo mkdir -p /etc/rustclaw /var/lib/rustclaw /var/log/rustclaw
sudo chown -R pi:pi /var/lib/rustclaw /var/log/rustclaw

# 2. Copiar service file
sudo cp rustclaw.service /etc/systemd/system/

# 3. Criar arquivo de variáveis
sudo nano /etc/rustclaw/.env
```

Conteúdo do `/etc/rustclaw/.env`:
```bash
HF_TOKEN=seu_token_aqui
TAVILY_API_KEY=sua_chave_aqui
OPENAI_API_KEY=sua_chave_aqui
TELEGRAM_TOKEN=seu_token_bot_aqui
TELEGRAM_CHAT_ID=seu_chat_id_aqui
```

```bash
# 4. Configurar permissões
sudo chown root:root /etc/rustclaw/.env
sudo chmod 600 /etc/rustclaw/.env

# 5. Ativar serviço
sudo systemctl daemon-reload
sudo systemctl enable rustclaw
```

## 🎮 Comandos de Controle

```bash
# Iniciar serviço
sudo systemctl start rustclaw

# Parar serviço
sudo systemctl stop rustclaw

# Reiniciar serviço
sudo systemctl restart rustclaw

# Ver status
sudo systemctl status rustclaw

# Ver logs em tempo real
sudo tail -f /var/log/rustclaw/rustclaw.log

# Ver logs de erro
sudo tail -f /var/log/rustclaw/rustclaw-error.log

# Ver todos os logs via systemd
sudo journalctl -u rustclaw -f
```

## 🔧 Configuração do Serviço

### Iniciar automaticamente no boot
```bash
sudo systemctl enable rustclaw
```

### Desativar início automático
```bash
sudo systemctl disable rustclaw
```

### Ver se está habilitado
```bash
sudo systemctl is-enabled rustclaw
```

## 📁 Estrutura de Arquivos

```
/etc/
├── rustclaw/
│   └── .env              # Variáveis de ambiente
└── systemd/
    └── system/
        └── rustclaw.service   # Config do serviço

/var/
├── lib/rustclaw/         # Dados do aplicativo
└── log/rustclaw/
    ├── rustclaw.log      # Logs normais
    └── rustclaw-error.log # Logs de erro

/home/pi/
└── rustclaw              # Binário
```

## 🔄 Atualizando o Binário

```bash
# 1. Parar o serviço
sudo systemctl stop rustclaw

# 2. Copiar novo binário
scp target/aarch64-unknown-linux-gnu/release/rustclaw pi@raspberrypi.local:~/rustclaw-new

# 3. No Raspberry Pi
ssh pi@raspberrypi.local
mv ~/rustclaw-new ~/rustclaw
chmod +x ~/rustclaw

# 4. Iniciar serviço
sudo systemctl start rustclaw

# 5. Verificar se funcionou
sudo systemctl status rustclaw
```

## 🐛 Troubleshooting

### Serviço não inicia
```bash
# Verificar erro
sudo systemctl status rustclaw

# Ver logs detalhados
sudo journalctl -u rustclaw --no-pager | tail -50
```

### Permissão negada
```bash
# Verificar permissões
ls -la /home/pi/rustclaw
ls -la /etc/rustclaw/.env
ls -la /var/log/rustclaw/

# Corrigir
sudo chown pi:pi /home/pi/rustclaw
sudo chmod +x /home/pi/rustclaw
sudo chown -R pi:pi /var/log/rustclaw
```

### Variáveis de ambiente não carregam
```bash
# Verificar arquivo
sudo cat /etc/rustclaw/.env

# Verificar se serviço está usando
sudo systemctl show rustclaw --property=EnvironmentFile
```

### Binário não encontrado
```bashn# Verificar caminho
which rustclaw
ls -la /home/pi/rustclaw

# Se estiver em outro lugar, editar service file
sudo nano /etc/systemd/system/rustclaw.service
# Alterar: ExecStart=/caminho/correto/rustclaw
sudo systemctl daemon-reload
sudo systemctl restart rustclaw
```

## 📝 Configurações Avançadas

### Reiniciar em caso de falha
Já está configurado no service file:
```ini
Restart=always
RestartSec=10
```

### Limitar recursos (opcional)
Adicionar ao `[Service]`:
```ini
MemoryMax=300M
CPUQuota=50%
```

### Múltiplas instâncias (ex: CLI + Telegram)
Criar `/etc/systemd/system/rustclaw-cli.service`:
```ini
ExecStart=/home/pi/rustclaw --mode cli
```

## 🎯 Comandos Úteis

```bash
# Ver todos os serviços ativos
sudo systemctl list-units --type=service --state=active

# Ver uso de recursos
sudo systemctl show rustclaw --property=MemoryCurrent,CPUUsageNSec

# Limpar logs antigos
sudo journalctl --vacuum-time=7d

# Backup das memórias
sudo tar -czf backup-$(date +%Y%m%d).tar.gz /var/lib/rustclaw/
```

## 📊 Status do Sistema

```bash
# Ver se está rodando
sudo systemctl is-active rustclaw

# Ver últimas mensagens
sudo tail -20 /var/log/rustclaw/rustclaw.log

# Ver erro mais recente
sudo tail -5 /var/log/rustclaw/rustclaw-error.log
```

## ✅ Checklist Pós-Instalação

- [ ] Binário copiado para `/home/pi/rustclaw`
- [ ] Variáveis configuradas em `/etc/rustclaw/.env`
- [ ] Serviço copiado para `/etc/systemd/system/`
- [ ] Permissões corretas (pi:pi)
- [ ] Serviço habilitado (`sudo systemctl enable rustclaw`)
- [ ] Serviço iniciado (`sudo systemctl start rustclaw`)
- [ ] Status mostra "active (running)"
- [ ] Logs aparecem em `/var/log/rustclaw/`

---

**Pronto!** O RustClaw agora inicia automaticamente no boot do Raspberry Pi! 🎉
