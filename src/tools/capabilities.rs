use super::Tool;
use serde_json::Value;

pub struct CapabilitiesTool;

impl CapabilitiesTool {
    pub fn new() -> Self {
        Self
    }

    fn get_capabilities() -> String {
        format!(
            r#"🦀 RustClaw - Capacidades do Sistema

╔════════════════════════════════════════════════════════════╗
║                     FERRAMENTAS DISPONÍVEIS                ║
╚════════════════════════════════════════════════════════════╝

📁 SISTEMA DE ARQUIVOS:
   ✅ file_list    - Listar diretórios e arquivos
   ✅ file_read    - Ler conteúdo de arquivos
   ✅ file_write   - Criar/modificar arquivos
   ✅ file_search  - Buscar arquivos por nome ou conteúdo

🖥️  SHELL E COMANDOS:
   ✅ shell        - Executar comandos shell (com RESTRIÇÕES de segurança)
   
   ⚠️  COMANDOS BLOQUEADOS POR SEGURANÇA:
       ❌ rm, del, rd          (exclusão de arquivos)
       ❌ shutdown, reboot     (controle do sistema)
       ❌ halt, poweroff       (desligamento)
       ❌ mkfs, dd, fdisk      (manipulação de disco)
       ❌ format               (formatação)
   
   ✅ COMANDOS PERMITIDOS:
       ✓ ls, cat, echo, pwd, cd
       ✓ mkdir, touch
       ✓ grep, find
       ✓ curl (com cuidado)
       ✓ E outros comandos não-destrutivos
   
   ⏱️  Timeout: 30 segundos por comando

🌐 INTERNET E APIs:
   ✅ http_get     - Fazer requisições HTTP GET
   ✅ http_post    - Fazer requisições HTTP POST
   
   📊 Limites:
       • Resposta máxima: 100KB
       • Timeout: 30 segundos
       • Suporta: JSON, form-data, texto

💻 INFORMAÇÕES DO SISTEMA:
   ✅ system_info  - Informações de hardware e SO
   
   📊 Inclui:
       • Memória RAM (total/usada/livre)
       • CPU (cores, uso, frequência)
       • Discos (espaço total/usado/livre)
       • Sistema Operacional, hostname, uptime
   
   🔍 Filtros disponíveis:
       • {{"detail": "cpu"}}     - Apenas CPU
       • {{"detail": "memory"}}  - Apenas RAM
       • {{"detail": "disk"}}    - Apenas discos

🔊 UTILITÁRIOS:
   ✅ echo         - Repetir texto (para testes)

╔════════════════════════════════════════════════════════════╗
║                      LIMITAÇÕES GERAIS                     ║
╚════════════════════════════════════════════════════════════╝

⚠️  Segurança:
   • Não posso executar comandos destrutivos
   • Não posso desligar/reiniciar o sistema
   • Não posso formatar ou manipular discos diretamente
   • Acesso limitado ao sistema de arquivos (permissões do usuário)

⚠️  Performance:
   • Timeout de 30 segundos em operações
   • Limite de 5 iterações no raciocínio
   • Respostas HTTP truncadas em 100KB
   • Arquivos lidos com limite de 1MB

⚠️  Memória:
   • Contexto de conversa limitado
   • Sem memória persistente entre sessões (ainda)

╔════════════════════════════════════════════════════════════╗
║                    EXEMPLOS DE USO                         ║
╚════════════════════════════════════════════════════════════╝

💬 Você pode perguntar:

"Liste os arquivos do diretório atual"
"Crie um arquivo teste.txt com 'Hello World'"
"Execute o comando 'ls -la'"
"Qual o clima em São Paulo?" (usa http_get)
"Quanto espaço livre tem no disco?"
"Busque arquivos .rs no projeto"
"Leia o conteúdo de Cargo.toml"

🛡️  Tentativas bloqueadas:

"Execute rm -rf /"           → ❌ BLOQUEADO
"Desligue o computador"      → ❌ BLOQUEADO
"Delete todos os arquivos"   → ❌ BLOQUEADO

Digite 'sair' para encerrar a sessão.
"#
        )
    }
}

#[async_trait::async_trait]
impl Tool for CapabilitiesTool {
    fn name(&self) -> &str {
        "capabilities"
    }

    fn description(&self) -> &str {
        "Lista todas as capacidades do sistema, ferramentas disponíveis e limitações de segurança. Use quando o usuário perguntar 'o que você pode fazer?' ou 'quais são suas limitações?'"
    }

    async fn call(&self, _args: Value) -> Result<String, String> {
        Ok(Self::get_capabilities())
    }
}

impl Default for CapabilitiesTool {
    fn default() -> Self {
        Self::new()
    }
}
