# RustClaw - Guia Completo do Sistema

## Índice

1. [Visão Geral](#visão-geral)
2. [Inicialização do Sistema](#inicialização-do-sistema)
3. [Loop Principal do Agente (ReAct)](#loop-principal-do-agente-react)
4. [Sistema de Ferramentas](#sistema-de-ferramentas)
5. [Integração com LLM (Moonshot API)](#integração-com-llm-moonshot-api)
6. [Sistema de Memória](#sistema-de-memória)
7. [Sistema de Segurança](#sistema-de-segurança)
8. [Sistema de Skills](#sistema-de-skills)
9. [Fluxo Completo de uma Requisição](#fluxo-completo-de-uma-requisição)
10. [Como Estender o Sistema](#como-estender-o-sistema)

---

## Visão Geral

RustClaw é um agente de IA CLI que usa o padrão **ReAct** (Reasoning + Acting) para executar tarefas complexas. O agente:
- Recebe comandos do usuário
- Raciocina sobre como resolver a tarefa
- Executa ferramentas disponíveis
- Aprende com o histórico de conversação
- Mantém memória de longo prazo

### Arquitetura de Alto Nível

```
┌─────────────────────────────────────────────────────────────┐
│                        USUÁRIO                               │
└────────────────────┬────────────────────────────────────────┘
                     │ Input (CLI)
                     ▼
┌─────────────────────────────────────────────────────────────┐
│                   RUSTCLAW AGENT                             │
│  ┌───────────────────────────────────────────────────────┐  │
│  │  1. CLI Interface (src/cli.rs)                        │  │
│  │     - Captura input do usuário                        │  │
│  │     - Exibe respostas formatadas                      │  │
│  └────────────────┬──────────────────────────────────────┘  │
│                   ▼                                          │
│  ┌───────────────────────────────────────────────────────┐  │
│  │  2. Agent Core (src/agent.rs)                         │  │
│  │     - ReAct Loop (20 iterações max)                   │  │
│  │     - Gerencia estado da conversação                  │  │
│  │     - Coordena todos os subsistemas                   │  │
│  └───┬────────────────────────────────────────────┬──────┘  │
│      │                                             │          │
│      ▼                                             ▼          │
│  ┌──────────────────────┐           ┌──────────────────────┐│
│  │ 3. LLM Integration   │           │ 4. Tool System       ││
│  │   (Moonshot API)     │           │   (18 ferramentas)   ││
│  │  - call_llm()        │           │  - execute_tool()    ││
│  │  - Prompt building   │           │  - Validation        ││
│  └──────────────────────┘           └──────────────────────┘│
│      │                                             │          │
│      ▼                                             ▼          │
│  ┌──────────────────────┐           ┌──────────────────────┐│
│  │ 5. Memory System     │           │ 6. Security System   ││
│  │  - SQLite storage    │           │  - Input validation  ││
│  │  - Embeddings        │           │  - Output filtering  ││
│  │  - Semantic search   │           │  - Path sanitization ││
│  └──────────────────────┘           └──────────────────────┘│
│                                                               │
└─────────────────────────────────────────────────────────────┘
```

---

## Inicialização do Sistema

### 1. Entry Point (`src/main.rs`)

```rust
// Fluxo de inicialização:
fn main() -> Result<()> {
    // 1. Carrega configuração (.env + variáveis de ambiente)
    let config = Config::load()?;
    
    // 2. Inicializa sistema de memória (SQLite)
    let memory = MemoryStore::new(&config.memory_db_path)?;
    
    // 3. Cria instância do agente
    let agent = Agent::new(config, memory)?;
    
    // 4. Inicia interface CLI
    Cli::new(agent).run()?;
}
```

### 2. Carregamento de Configuração (`src/config.rs`)

```rust
pub struct Config {
    // API Configuration
    pub api_key: String,           // MOONSHOT_API_KEY
    pub base_url: String,          // https://api.moonshot.ai/v1
    pub model: String,             // kimi-k2-thinking
    
    // Generation Parameters
    pub temperature: f32,          // 0.7
    pub max_tokens: u32,          // 4000
    pub top_p: f32,               // 0.9
    
    // System Paths
    pub memory_db_path: String,   // config/memory_cli.db
    pub skills_dir: String,       // skills/
    
    // Embedding Configuration
    pub embedding_api_key: String, // OpenAI API key
    pub embedding_model: String,   // text-embedding-3-small
}
```

**Processo de Load:**
1. Procura `.env` em `config/.env` e diretório atual
2. Lê variáveis de ambiente do sistema
3. Valida que campos obrigatórios existem
4. Define valores padrão para opcionais
5. Retorna `Config` ou erro se faltar alguma key

### 3. Inicialização do Agent (`src/agent.rs`)

```rust
impl Agent {
    pub fn new(config: Config, memory: MemoryStore) -> Result<Self> {
        // 1. Registra todas as ferramentas disponíveis
        let tools = Self::register_tools();
        
        // 2. Carrega skills do diretório skills/
        let skills = SkillManager::load_all(&config.skills_dir)?;
        
        // 3. Inicializa detector de builds/testes
        let build_detector = BuildDetector::new();
        
        // 4. Cria histórico vazio de conversação
        let conversation_history = Vec::new();
        
        Ok(Self {
            config,
            memory,
            tools,
            skills,
            build_detector,
            conversation_history,
        })
    }
}
```

---

## Loop Principal do Agente (ReAct)

O coração do RustClaw é o **ReAct Loop** - um padrão que alterna entre **Raciocínio** e **Ação**.

### Padrão ReAct

```
┌────────────────────────────────────────────────────┐
│                 ReAct Pattern                      │
├────────────────────────────────────────────────────┤
│                                                    │
│  1. THOUGHT (Pensamento)                          │
│     ↓ O agente raciocina sobre o que fazer       │
│     "Preciso buscar informações sobre X"          │
│                                                    │
│  2. ACTION (Ação)                                 │
│     ↓ Decide qual ferramenta usar                 │
│     Tool: read_file                               │
│                                                    │
│  3. ACTION INPUT (Entrada da Ação)                │
│     ↓ Parâmetros para a ferramenta                │
│     { "path": "/path/to/file.txt" }               │
│                                                    │
│  4. OBSERVATION (Observação)                      │
│     ↓ Resultado da execução                       │
│     "File contents: Hello World"                  │
│                                                    │
│  ┌─────────────────────────────────────┐          │
│  │ Loop continua até:                  │          │
│  │ - Agente responde Final Answer      │          │
│  │ - Máximo de 20 iterações            │          │
│  │ - Erro crítico                      │          │
│  └─────────────────────────────────────┘          │
│                                                    │
│  5. FINAL ANSWER (Resposta Final)                 │
│     ↓ Resultado para o usuário                    │
│     "Tarefa concluída com sucesso!"               │
│                                                    │
└────────────────────────────────────────────────────┘
```

### Implementação do Loop (`src/agent.rs:run()`)

```rust
pub async fn run(&mut self, user_input: &str) -> Result<String> {
    const MAX_ITERATIONS: usize = 20;
    
    // 1. Adiciona mensagem do usuário ao histórico
    self.conversation_history.push(Message {
        role: "user".to_string(),
        content: user_input.to_string(),
    });
    
    // 2. Busca memórias relevantes (semantic search)
    let relevant_memories = self.memory
        .search_similar(&user_input, 5)
        .await?;
    
    // 3. Inicia loop ReAct
    for iteration in 0..MAX_ITERATIONS {
        // 3a. Chama LLM para obter próxima ação
        let llm_response = self.call_llm().await?;
        
        // 3b. Adiciona resposta ao histórico
        self.conversation_history.push(Message {
            role: "assistant".to_string(),
            content: llm_response.clone(),
        });
        
        // 3c. Parse da resposta (extrai Thought, Action, Action Input)
        let parsed = self.parse_response(&llm_response)?;
        
        // 3d. Verifica se é resposta final
        if parsed.is_final_answer {
            // Salva na memória de longo prazo
            self.memory.store(user_input, &parsed.content).await?;
            return Ok(parsed.content);
        }
        
        // 3e. Executa ferramenta se especificada
        if let Some(tool_name) = parsed.tool_name {
            let tool_result = self.execute_tool(
                &tool_name,
                &parsed.tool_input
            ).await?;
            
            // 3f. Adiciona observação ao histórico
            self.conversation_history.push(Message {
                role: "user".to_string(),
                content: format!("Observation: {}", tool_result),
            });
        }
    }
    
    // Se chegou aqui, excedeu max iterations
    Err(Error::MaxIterationsExceeded)
}
```

### Exemplo Prático do Loop

**Input do Usuário:**
```
"Leia o arquivo README.md e me diga quantas linhas ele tem"
```

**Iteração 1:**
```
Thought: Preciso ler o conteúdo do arquivo README.md
Action: read_file
Action Input: {"path": "README.md"}
```

**Sistema executa `read_file`**

**Iteração 2:**
```
Observation: [conteúdo do arquivo com 50 linhas]

Thought: Já tenho o conteúdo, agora preciso contar as linhas
Action: NONE
Final Answer: O arquivo README.md tem 50 linhas.
```

**Retorna para o usuário:** "O arquivo README.md tem 50 linhas."

---

## Sistema de Ferramentas

O RustClaw tem **18 ferramentas** organizadas em categorias.

### Arquitetura de Ferramentas

```
src/tools/
├── mod.rs              # Tool registry
├── file_ops.rs         # read_file, write_file, list_files, search_files
├── shell.rs            # execute_command
├── http.rs             # http_request
├── browser.rs          # browse_web
├── memory_tools.rs     # store_memory, search_memory
├── skill_tools.rs      # load_skill, list_skills
└── reminder_tools.rs   # set_reminder, list_reminders
```

### 1. Definição de Ferramenta

Cada ferramenta implementa o trait `Tool`:

```rust
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    
    async fn execute(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> Result<String>;
}
```

**Exemplo: ReadFile Tool**

```rust
pub struct ReadFileTool;

impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    
    fn description(&self) -> &str {
        "Reads the contents of a file from the filesystem"
    }
    
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read"
                }
            },
            "required": ["path"]
        })
    }
    
    async fn execute(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> Result<String> {
        // 1. Valida parâmetros
        let path = params["path"]
            .as_str()
            .ok_or(Error::InvalidToolParams)?;
        
        // 2. Sanitiza path (segurança)
        let safe_path = context.security.sanitize_path(path)?;
        
        // 3. Executa operação
        let content = tokio::fs::read_to_string(&safe_path).await?;
        
        // 4. Retorna resultado
        Ok(content)
    }
}
```

### 2. Registro de Ferramentas

Todas as ferramentas são registradas no início:

```rust
impl Agent {
    fn register_tools() -> HashMap<String, Box<dyn Tool>> {
        let mut tools = HashMap::new();
        
        // File Operations
        tools.insert("read_file", Box::new(ReadFileTool));
        tools.insert("write_file", Box::new(WriteFileTool));
        tools.insert("list_files", Box::new(ListFilesTool));
        tools.insert("search_files", Box::new(SearchFilesTool));
        
        // Shell
        tools.insert("execute_command", Box::new(ExecuteCommandTool));
        
        // HTTP
        tools.insert("http_request", Box::new(HttpRequestTool));
        
        // Browser
        tools.insert("browse_web", Box::new(BrowseWebTool));
        
        // Memory
        tools.insert("store_memory", Box::new(StoreMemoryTool));
        tools.insert("search_memory", Box::new(SearchMemoryTool));
        
        // Skills
        tools.insert("load_skill", Box::new(LoadSkillTool));
        tools.insert("list_skills", Box::new(ListSkillsTool));
        
        // Reminders
        tools.insert("set_reminder", Box::new(SetReminderTool));
        tools.insert("list_reminders", Box::new(ListRemindersTool));
        
        tools
    }
}
```

### 3. Execução de Ferramenta

```rust
async fn execute_tool(
    &mut self,
    tool_name: &str,
    params: &str,
) -> Result<String> {
    // 1. Busca ferramenta pelo nome
    let tool = self.tools
        .get(tool_name)
        .ok_or(Error::ToolNotFound)?;
    
    // 2. Parse dos parâmetros JSON
    let params_json: serde_json::Value = 
        serde_json::from_str(params)?;
    
    // 3. Cria contexto de execução
    let context = ToolContext {
        security: &self.security,
        memory: &self.memory,
        config: &self.config,
        working_dir: std::env::current_dir()?,
    };
    
    // 4. Validação de segurança PRÉ-execução
    self.security.validate_tool_execution(
        tool_name,
        &params_json,
    )?;
    
    // 5. Executa ferramenta
    let result = tool.execute(params_json, &context).await?;
    
    // 6. Validação de segurança PÓS-execução
    let safe_result = self.security.sanitize_output(&result)?;
    
    // 7. Detecta se foi build/teste (para auto-loop)
    self.build_detector.check_output(&safe_result);
    
    // 8. Retorna resultado
    Ok(safe_result)
}
```

### 4. Schema de Ferramentas para LLM

O agente passa a lista de ferramentas para o LLM no prompt:

```json
{
  "tools": [
    {
      "name": "read_file",
      "description": "Reads the contents of a file from the filesystem",
      "parameters": {
        "type": "object",
        "properties": {
          "path": {
            "type": "string",
            "description": "Path to the file to read"
          }
        },
        "required": ["path"]
      }
    },
    {
      "name": "write_file",
      "description": "Writes content to a file",
      "parameters": {
        "type": "object",
        "properties": {
          "path": {"type": "string"},
          "content": {"type": "string"}
        },
        "required": ["path", "content"]
      }
    }
    // ... outras 16 ferramentas
  ]
}
```

---

## Integração com LLM (Moonshot API)

### Arquitetura da Chamada

```
┌──────────────────────────────────────────────────────┐
│            Agent (src/agent.rs)                      │
│                                                      │
│  1. build_prompt()                                  │
│     ↓ Constrói prompt com contexto                  │
│     - System message (instruções + tools)           │
│     - Conversation history                          │
│     - Relevant memories                             │
│     - Active skills                                 │
│                                                      │
│  2. call_llm()                                      │
│     ↓ Faz requisição HTTP                           │
│                                                      │
└──────────────────┬───────────────────────────────────┘
                   │
                   ▼ HTTPS POST
┌──────────────────────────────────────────────────────┐
│    Moonshot API (api.moonshot.ai/v1/chat/completions)│
│                                                      │
│  Model: kimi-k2-thinking                            │
│  - Reasoning capabilities                           │
│  - Context: 128k tokens                             │
│  - Output: reasoning_content + content              │
│                                                      │
└──────────────────┬───────────────────────────────────┘
                   │
                   ▼ JSON Response
┌──────────────────────────────────────────────────────┐
│            Agent (parse response)                    │
│                                                      │
│  3. extract_content()                               │
│     ↓ Extrai content OU reasoning_content           │
│                                                      │
│  4. filter_empty_messages()                         │
│     ↓ Remove mensagens vazias                       │
│                                                      │
│  5. parse_response()                                │
│     ↓ Extrai Thought/Action/Action Input            │
│                                                      │
└──────────────────────────────────────────────────────┘
```

### 1. Construção do Prompt

```rust
fn build_prompt(&self) -> Vec<Message> {
    let mut messages = Vec::new();
    
    // 1. System Message (instruções principais)
    messages.push(Message {
        role: "system".to_string(),
        content: self.build_system_message(),
    });
    
    // 2. Skill Personalities (se carregadas)
    if let Some(skill) = self.active_skill() {
        messages.push(Message {
            role: "system".to_string(),
            content: format!("Active Skill: {}\n{}", 
                skill.name, skill.instructions),
        });
    }
    
    // 3. Relevant Memories (busca semântica)
    if !self.relevant_memories.is_empty() {
        let memory_context = self.format_memories();
        messages.push(Message {
            role: "system".to_string(),
            content: format!("Relevant Context:\n{}", memory_context),
        });
    }
    
    // 4. Conversation History
    messages.extend(self.conversation_history.clone());
    
    messages
}
```

### 2. System Message

```rust
fn build_system_message(&self) -> String {
    format!(r#"
You are RustClaw, an advanced AI agent capable of executing tasks using tools.

## Your Capabilities

You have access to these tools:
{}

## Response Format

You MUST respond using this exact format:

Thought: [Your reasoning about what to do]
Action: [Tool name to use, or NONE if ready to answer]
Action Input: [JSON parameters for the tool]

OR if you have the final answer:

Thought: [Your reasoning]
Final Answer: [Your complete response to the user]

## Rules

1. Always think step-by-step before acting
2. Use tools when you need information or to perform actions
3. Validate tool outputs before using them
4. If a tool fails, try an alternative approach
5. Give Final Answer only when you have complete information
6. Never make up information - use tools to verify
7. Be concise but thorough

## Examples

Example 1 - Reading a file:
Thought: I need to read the contents of config.rs to answer the question
Action: read_file
Action Input: {{"path": "src/config.rs"}}

Example 2 - Final answer:
Thought: I have all the information needed to answer
Final Answer: The configuration file contains 55 lines and defines the Config struct with API settings.
"#, self.format_tools_schema())
}
```

### 3. Chamada à API

```rust
async fn call_llm(&mut self) -> Result<String> {
    // 1. Prepara mensagens
    let messages = self.build_prompt();
    
    // 2. Filtra mensagens vazias (Moonshot rejeita)
    let filtered_messages: Vec<_> = messages
        .into_iter()
        .filter(|m| !m.content.trim().is_empty())
        .collect();
    
    // 3. Constrói request body
    let request_body = json!({
        "model": self.config.model,
        "messages": filtered_messages,
        "temperature": self.config.temperature,
        "max_tokens": self.config.max_tokens,
        "top_p": self.config.top_p,
    });
    
    // 4. Faz requisição HTTP
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{}/chat/completions", self.config.base_url))
        .header("Authorization", format!("Bearer {}", self.config.api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await?;
    
    // 5. Verifica status
    if !response.status().is_success() {
        let error_text = response.text().await?;
        return Err(Error::ApiError(error_text));
    }
    
    // 6. Parse da resposta
    let response_json: serde_json::Value = response.json().await?;
    
    // 7. Extrai content (ou reasoning_content para thinking models)
    let content = self.extract_content(&response_json)?;
    
    Ok(content)
}
```

### 4. Extração de Conteúdo (Fix para kimi-k2-thinking)

```rust
fn extract_content(&self, response: &serde_json::Value) -> Result<String> {
    let choice = &response["choices"][0];
    let message = &choice["message"];
    
    // 1. Tenta pegar content normal
    if let Some(content) = message["content"].as_str() {
        if !content.trim().is_empty() {
            return Ok(content.to_string());
        }
    }
    
    // 2. Se content vazio, tenta reasoning_content (thinking models)
    if let Some(reasoning) = message["reasoning_content"].as_str() {
        if !reasoning.trim().is_empty() {
            return Ok(reasoning.to_string());
        }
    }
    
    // 3. Fallback para role + reasoning
    if let Some(role) = message["role"].as_str() {
        if role == "assistant" {
            if let Some(reasoning) = message["reasoning_content"].as_str() {
                return Ok(reasoning.to_string());
            }
        }
    }
    
    Err(Error::EmptyResponse)
}
```

### 5. Parse da Resposta

```rust
fn parse_response(&self, response: &str) -> Result<ParsedResponse> {
    let mut thought = String::new();
    let mut action = None;
    let mut action_input = String::new();
    let mut final_answer = None;
    
    // Parse linha por linha
    for line in response.lines() {
        if line.starts_with("Thought:") {
            thought = line.strip_prefix("Thought:").unwrap().trim().to_string();
        } else if line.starts_with("Action:") {
            let action_str = line.strip_prefix("Action:").unwrap().trim();
            if action_str != "NONE" {
                action = Some(action_str.to_string());
            }
        } else if line.starts_with("Action Input:") {
            action_input = line.strip_prefix("Action Input:").unwrap().trim().to_string();
        } else if line.starts_with("Final Answer:") {
            final_answer = Some(line.strip_prefix("Final Answer:").unwrap().trim().to_string());
        }
    }
    
    Ok(ParsedResponse {
        thought,
        tool_name: action,
        tool_input: action_input,
        is_final_answer: final_answer.is_some(),
        content: final_answer.unwrap_or_default(),
    })
}
```

---

## Sistema de Memória

O RustClaw tem memória de **longo prazo** usando SQLite + embeddings.

### Arquitetura

```
┌─────────────────────────────────────────────────────┐
│           Memory System (src/memory/)               │
│                                                     │
│  ┌──────────────────────────────────────────────┐  │
│  │  SQLite Database (config/memory_cli.db)      │  │
│  │                                              │  │
│  │  Tables:                                     │  │
│  │  1. memories                                 │  │
│  │     - id, content, embedding, created_at    │  │
│  │  2. checkpoints                              │  │
│  │     - id, state_json, created_at            │  │
│  │  3. reminders                                │  │
│  │     - id, message, due_at, completed        │  │
│  │  4. scheduled_tasks                          │  │
│  │     - id, task, schedule, last_run          │  │
│  │  5. active_skills                            │  │
│  │     - id, skill_name, loaded_at             │  │
│  └──────────────────────────────────────────────┘  │
│                                                     │
│  ┌──────────────────────────────────────────────┐  │
│  │  Embedding Service (OpenAI)                  │  │
│  │                                              │  │
│  │  Model: text-embedding-3-small               │  │
│  │  Dimensions: 384                             │  │
│  │  Cost: $0.00002 / 1K tokens                  │  │
│  └──────────────────────────────────────────────┘  │
│                                                     │
│  ┌──────────────────────────────────────────────┐  │
│  │  Semantic Search                             │  │
│  │                                              │  │
│  │  1. Query → Embedding                        │  │
│  │  2. Cosine Similarity com stored embeddings │  │
│  │  3. Retorna top K resultados                │  │
│  └──────────────────────────────────────────────┘  │
│                                                     │
└─────────────────────────────────────────────────────┘
```

### 1. Schema do Banco de Dados

```sql
-- Tabela de memórias com embeddings
CREATE TABLE memories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content TEXT NOT NULL,
    embedding BLOB,  -- Vetor de 384 floats (384 * 4 bytes = 1536 bytes)
    metadata TEXT,   -- JSON com info adicional
    created_at INTEGER NOT NULL
);

-- Índice para busca temporal
CREATE INDEX idx_memories_created_at ON memories(created_at);

-- Tabela de checkpoints (snapshots do estado)
CREATE TABLE checkpoints (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    state_json TEXT NOT NULL,
    description TEXT,
    created_at INTEGER NOT NULL
);

-- Tabela de reminders
CREATE TABLE reminders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    message TEXT NOT NULL,
    due_at INTEGER NOT NULL,
    completed INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL
);

-- Tabela de tarefas agendadas
CREATE TABLE scheduled_tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task TEXT NOT NULL,
    schedule TEXT NOT NULL,  -- Cron expression
    last_run INTEGER,
    created_at INTEGER NOT NULL
);

-- Tabela de skills ativas
CREATE TABLE active_skills (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    skill_name TEXT NOT NULL,
    config TEXT,  -- JSON config
    loaded_at INTEGER NOT NULL
);
```

### 2. Armazenamento de Memória

```rust
impl MemoryStore {
    pub async fn store(
        &self,
        content: &str,
        metadata: Option<serde_json::Value>,
    ) -> Result<i64> {
        // 1. Gera embedding do conteúdo
        let embedding = self.generate_embedding(content).await?;
        
        // 2. Serializa embedding para BLOB
        let embedding_bytes = self.serialize_embedding(&embedding);
        
        // 3. Serializa metadata para JSON
        let metadata_json = metadata
            .map(|m| serde_json::to_string(&m).ok())
            .flatten();
        
        // 4. Insere no banco
        let id = sqlx::query(
            "INSERT INTO memories (content, embedding, metadata, created_at) 
             VALUES (?, ?, ?, ?)"
        )
        .bind(content)
        .bind(embedding_bytes)
        .bind(metadata_json)
        .bind(Utc::now().timestamp())
        .execute(&self.pool)
        .await?
        .last_insert_rowid();
        
        Ok(id)
    }
}
```

### 3. Geração de Embedding

```rust
async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
    // 1. Prepara request para OpenAI
    let request = json!({
        "model": "text-embedding-3-small",
        "input": text,
        "dimensions": 384,  // Reduzido para performance
    });
    
    // 2. Faz chamada à API
    let client = reqwest::Client::new();
    let response = client
        .post("https://api.openai.com/v1/embeddings")
        .header("Authorization", format!("Bearer {}", self.embedding_api_key))
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await?;
    
    // 3. Parse da resposta
    let response_json: serde_json::Value = response.json().await?;
    let embedding_array = response_json["data"][0]["embedding"]
        .as_array()
        .ok_or(Error::InvalidEmbedding)?;
    
    // 4. Converte para Vec<f32>
    let embedding: Vec<f32> = embedding_array
        .iter()
        .filter_map(|v| v.as_f64().map(|f| f as f32))
        .collect();
    
    Ok(embedding)
}
```

### 4. Busca Semântica

```rust
pub async fn search_similar(
    &self,
    query: &str,
    limit: usize,
) -> Result<Vec<Memory>> {
    // 1. Gera embedding da query
    let query_embedding = self.generate_embedding(query).await?;
    
    // 2. Busca TODAS as memórias do banco
    let all_memories: Vec<StoredMemory> = sqlx::query_as(
        "SELECT id, content, embedding, metadata, created_at 
         FROM memories 
         ORDER BY created_at DESC"
    )
    .fetch_all(&self.pool)
    .await?;
    
    // 3. Calcula similaridade para cada memória
    let mut scored_memories: Vec<(Memory, f32)> = all_memories
        .into_iter()
        .map(|stored| {
            let embedding = self.deserialize_embedding(&stored.embedding);
            let similarity = cosine_similarity(&query_embedding, &embedding);
            
            let memory = Memory {
                id: stored.id,
                content: stored.content,
                metadata: stored.metadata
                    .and_then(|m| serde_json::from_str(&m).ok()),
                created_at: stored.created_at,
            };
            
            (memory, similarity)
        })
        .collect();
    
    // 4. Ordena por similaridade (descendente)
    scored_memories.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    
    // 5. Retorna top K resultados
    Ok(scored_memories
        .into_iter()
        .take(limit)
        .map(|(memory, _score)| memory)
        .collect())
}
```

### 5. Cálculo de Cosine Similarity

```rust
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    // Produto interno (dot product)
    let dot_product: f32 = a.iter()
        .zip(b.iter())
        .map(|(x, y)| x * y)
        .sum();
    
    // Magnitude de A
    let magnitude_a: f32 = a.iter()
        .map(|x| x * x)
        .sum::<f32>()
        .sqrt();
    
    // Magnitude de B
    let magnitude_b: f32 = b.iter()
        .map(|x| x * x)
        .sum::<f32>()
        .sqrt();
    
    // Cosine similarity = dot / (mag_a * mag_b)
    if magnitude_a == 0.0 || magnitude_b == 0.0 {
        return 0.0;
    }
    
    dot_product / (magnitude_a * magnitude_b)
}
```

### 6. Uso no Agent

```rust
// Quando recebe input do usuário
pub async fn run(&mut self, user_input: &str) -> Result<String> {
    // Busca top 5 memórias relevantes
    let relevant_memories = self.memory
        .search_similar(user_input, 5)
        .await?;
    
    // Formata memórias para incluir no prompt
    let memory_context = relevant_memories
        .iter()
        .enumerate()
        .map(|(i, mem)| format!("[Memory {}] {}", i+1, mem.content))
        .collect::<Vec<_>>()
        .join("\n");
    
    // Adiciona ao prompt do LLM
    self.system_context.push(format!(
        "Relevant past context:\n{}", 
        memory_context
    ));
    
    // ... continua com ReAct loop
}
```

---

## Sistema de Segurança

RustClaw tem **5 camadas** de defesa contra execução maliciosa.

### Camadas de Segurança

```
┌─────────────────────────────────────────────────────┐
│              Security Layers                        │
├─────────────────────────────────────────────────────┤
│                                                     │
│  Layer 1: Input Validation                         │
│  ├─ Sanitiza input do usuário                      │
│  ├─ Remove caracteres perigosos                    │
│  ├─ Valida encoding (UTF-8)                        │
│  └─ Limite de tamanho (10MB)                       │
│                                                     │
│  Layer 2: Path Sanitization                        │
│  ├─ Resolve paths absolutos                        │
│  ├─ Bloqueia path traversal (../)                  │
│  ├─ Valida que path está em allowed_dirs           │
│  └─ Normaliza separadores (/ vs \)                 │
│                                                     │
│  Layer 3: Command Validation                       │
│  ├─ Whitelist de comandos permitidos               │
│  ├─ Bloqueia comandos perigosos (rm -rf, etc)     │
│  ├─ Valida argumentos                              │
│  └─ Timeout de execução (30s)                      │
│                                                     │
│  Layer 4: Tool Parameter Validation                │
│  ├─ Valida contra schema JSON                      │
│  ├─ Type checking                                  │
│  ├─ Required fields                                │
│  └─ Regex patterns                                 │
│                                                     │
│  Layer 5: Output Filtering                         │
│  ├─ Remove informações sensíveis                   │
│  ├─ Filtra paths absolutos                         │
│  ├─ Reduz output muito grande (>50KB)             │
│  └─ Sanitiza caracteres especiais                  │
│                                                     │
└─────────────────────────────────────────────────────┘
```

### Implementação (`src/security/mod.rs`)

```rust
pub struct SecurityManager {
    allowed_dirs: Vec<PathBuf>,
    blocked_commands: HashSet<String>,
    max_input_size: usize,
    max_output_size: usize,
}

impl SecurityManager {
    pub fn new() -> Self {
        Self {
            allowed_dirs: vec![
                std::env::current_dir().unwrap(),
                PathBuf::from("/tmp"),
            ],
            blocked_commands: [
                "rm", "rmdir", "del", "format",
                "dd", "mkfs", "fdisk",
                "sudo", "su", "chmod", "chown",
            ].iter().map(|s| s.to_string()).collect(),
            max_input_size: 10 * 1024 * 1024,  // 10MB
            max_output_size: 50 * 1024,         // 50KB
        }
    }
    
    // Layer 1: Input Validation
    pub fn validate_input(&self, input: &str) -> Result<String> {
        // Verifica tamanho
        if input.len() > self.max_input_size {
            return Err(Error::InputTooLarge);
        }
        
        // Valida UTF-8
        if !input.is_char_boundary(0) {
            return Err(Error::InvalidEncoding);
        }
        
        // Remove null bytes
        let sanitized = input.replace('\0', "");
        
        // Remove control characters perigosos
        let sanitized = sanitized
            .chars()
            .filter(|c| !c.is_control() || c.is_whitespace())
            .collect();
        
        Ok(sanitized)
    }
    
    // Layer 2: Path Sanitization
    pub fn sanitize_path(&self, path: &str) -> Result<PathBuf> {
        // 1. Converte para PathBuf
        let path_buf = PathBuf::from(path);
        
        // 2. Resolve para path absoluto
        let absolute = if path_buf.is_absolute() {
            path_buf
        } else {
            std::env::current_dir()?.join(path_buf)
        };
        
        // 3. Canonicalize (resolve symlinks, .., .)
        let canonical = absolute.canonicalize()
            .map_err(|_| Error::InvalidPath)?;
        
        // 4. Verifica se está em diretório permitido
        let is_allowed = self.allowed_dirs
            .iter()
            .any(|allowed| canonical.starts_with(allowed));
        
        if !is_allowed {
            return Err(Error::PathNotAllowed);
        }
        
        // 5. Detecta path traversal
        if path.contains("..") {
            return Err(Error::PathTraversalDetected);
        }
        
        Ok(canonical)
    }
    
    // Layer 3: Command Validation
    pub fn validate_command(&self, command: &str) -> Result<()> {
        // 1. Extrai comando base
        let cmd_parts: Vec<&str> = command.split_whitespace().collect();
        let base_cmd = cmd_parts.first().ok_or(Error::EmptyCommand)?;
        
        // 2. Verifica blocklist
        if self.blocked_commands.contains(*base_cmd) {
            return Err(Error::CommandBlocked);
        }
        
        // 3. Bloqueia pipes e redirecionamentos perigosos
        if command.contains("|") || 
           command.contains(">") || 
           command.contains("&") {
            return Err(Error::CommandNotAllowed);
        }
        
        // 4. Bloqueia command injection
        if command.contains(";") || 
           command.contains("$") || 
           command.contains("`") {
            return Err(Error::InjectionDetected);
        }
        
        Ok(())
    }
    
    // Layer 4: Tool Parameter Validation
    pub fn validate_tool_params(
        &self,
        tool_name: &str,
        params: &serde_json::Value,
        schema: &serde_json::Value,
    ) -> Result<()> {
        // Usa jsonschema para validar
        let compiled_schema = jsonschema::JSONSchema::compile(schema)
            .map_err(|e| Error::InvalidSchema(e.to_string()))?;
        
        if let Err(errors) = compiled_schema.validate(params) {
            let error_msgs: Vec<String> = errors
                .map(|e| e.to_string())
                .collect();
            return Err(Error::InvalidParams(error_msgs.join(", ")));
        }
        
        Ok(())
    }
    
    // Layer 5: Output Filtering
    pub fn sanitize_output(&self, output: &str) -> Result<String> {
        let mut sanitized = output.to_string();
        
        // 1. Limita tamanho
        if sanitized.len() > self.max_output_size {
            // Encontra boundary seguro para truncar (UTF-8)
            let mut truncate_at = self.max_output_size;
            while truncate_at > 0 && !sanitized.is_char_boundary(truncate_at) {
                truncate_at -= 1;
            }
            
            sanitized.truncate(truncate_at);
            sanitized.push_str("\n[... output truncated ...]");
        }
        
        // 2. Filtra paths absolutos do sistema
        let home_dir = std::env::var("HOME").unwrap_or_default();
        sanitized = sanitized.replace(&home_dir, "~");
        
        // 3. Remove tokens/keys expostos (regex)
        let key_pattern = regex::Regex::new(r"[A-Za-z0-9]{32,}").unwrap();
        sanitized = key_pattern.replace_all(&sanitized, "[REDACTED]").to_string();
        
        Ok(sanitized)
    }
}
```

---

## Sistema de Skills

Skills são "personalidades" que modificam o comportamento do agente.

### Conceito

```
┌─────────────────────────────────────────────────────┐
│                  Skill System                       │
├─────────────────────────────────────────────────────┤
│                                                     │
│  Skills são arquivos Markdown em skills/           │
│                                                     │
│  Estrutura:                                         │
│  ┌─────────────────────────────────────────────┐   │
│  │ # Skill Name                                │   │
│  │                                             │   │
│  │ ## Description                              │   │
│  │ Brief description of what this skill does  │   │
│  │                                             │   │
│  │ ## Instructions                             │   │
│  │ Detailed instructions for the agent        │   │
│  │ - How to behave                             │   │
│  │ - What to prioritize                        │   │
│  │ - Special rules                             │   │
│  │                                             │   │
│  │ ## Examples                                 │   │
│  │ Example interactions                        │   │
│  └─────────────────────────────────────────────┘   │
│                                                     │
│  Quando skill é carregada:                          │
│  - Instruções são injetadas no system prompt       │
│  - Agent adapta comportamento                       │
│  - Persiste até ser descarregada                   │
│                                                     │
└─────────────────────────────────────────────────────┘
```

### Exemplo de Skill (`skills/code_reviewer.md`)

```markdown
# Code Reviewer

## Description
Expert code reviewer focused on best practices, security, and performance.

## Instructions

You are now acting as a senior code reviewer. Your responsibilities:

1. **Code Quality**
   - Check for code smells and anti-patterns
   - Suggest improvements for readability
   - Identify overly complex logic

2. **Security**
   - Look for potential vulnerabilities
   - Check for input validation
   - Review authentication/authorization

3. **Performance**
   - Identify inefficient algorithms
   - Suggest optimization opportunities
   - Check for memory leaks

4. **Best Practices**
   - Ensure proper error handling
   - Verify logging is adequate
   - Check for proper documentation

## Review Format

For each file reviewed, provide:

### File: [path]

**Issues Found:**
- [Severity] [Description]

**Suggestions:**
- [Improvement suggestion]

**Positive Points:**
- [What is done well]

## Examples

User: Review src/agent.rs
Assistant: 
Thought: I need to read and analyze the agent.rs file
Action: read_file
Action Input: {"path": "src/agent.rs"}

[After reading]
Thought: I have the code, now I'll perform a comprehensive review
Final Answer:

### File: src/agent.rs

**Issues Found:**
- MEDIUM: The `call_llm()` function has no retry logic for API failures
- LOW: Some error messages could be more descriptive

**Suggestions:**
- Add exponential backoff for API retries
- Create custom error types for better error handling
- Consider extracting prompt building to separate module

**Positive Points:**
- Excellent separation of concerns
- Good use of async/await
- Comprehensive error handling structure
```

### Implementação (`src/skills/mod.rs`)

```rust
pub struct SkillManager {
    skills_dir: PathBuf,
    loaded_skills: HashMap<String, Skill>,
}

#[derive(Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub examples: Vec<String>,
}

impl SkillManager {
    pub fn new(skills_dir: impl Into<PathBuf>) -> Self {
        Self {
            skills_dir: skills_dir.into(),
            loaded_skills: HashMap::new(),
        }
    }
    
    pub fn load_skill(&mut self, skill_name: &str) -> Result<()> {
        // 1. Lê arquivo markdown
        let skill_path = self.skills_dir.join(format!("{}.md", skill_name));
        let content = std::fs::read_to_string(&skill_path)?;
        
        // 2. Parse do markdown
        let skill = self.parse_skill_markdown(&content)?;
        
        // 3. Armazena skill carregada
        self.loaded_skills.insert(skill_name.to_string(), skill);
        
        Ok(())
    }
    
    fn parse_skill_markdown(&self, content: &str) -> Result<Skill> {
        let mut name = String::new();
        let mut description = String::new();
        let mut instructions = String::new();
        let mut examples = Vec::new();
        
        let mut current_section = None;
        
        for line in content.lines() {
            if line.starts_with("# ") {
                name = line.strip_prefix("# ").unwrap().to_string();
            } else if line.starts_with("## Description") {
                current_section = Some("description");
            } else if line.starts_with("## Instructions") {
                current_section = Some("instructions");
            } else if line.starts_with("## Examples") {
                current_section = Some("examples");
            } else {
                match current_section {
                    Some("description") => description.push_str(line),
                    Some("instructions") => instructions.push_str(line),
                    Some("examples") => {
                        if !line.trim().is_empty() {
                            examples.push(line.to_string());
                        }
                    },
                    _ => {}
                }
            }
        }
        
        Ok(Skill {
            name,
            description,
            instructions,
            examples,
        })
    }
    
    pub fn get_active_skill(&self) -> Option<&Skill> {
        self.loaded_skills.values().next()
    }
    
    pub fn unload_all(&mut self) {
        self.loaded_skills.clear();
    }
}
```

### Hot Reload (Recarregar skills em runtime)

```rust
impl SkillManager {
    pub fn watch_for_changes(&self) -> Result<()> {
        use notify::{Watcher, RecursiveMode, Event};
        
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = notify::recommended_watcher(tx)?;
        
        // Monitora diretório de skills
        watcher.watch(&self.skills_dir, RecursiveMode::NonRecursive)?;
        
        // Thread para processar eventos
        std::thread::spawn(move || {
            for event in rx {
                if let Ok(Event { kind, paths, .. }) = event {
                    for path in paths {
                        if path.extension() == Some("md".as_ref()) {
                            // Recarrega skill modificada
                            println!("Skill modified: {:?}", path);
                        }
                    }
                }
            }
        });
        
        Ok(())
    }
}
```

---

## Fluxo Completo de uma Requisição

Vamos acompanhar uma requisição completa do usuário até a resposta final.

### Exemplo: "Liste os arquivos em src/ e leia config.rs"

```
┌─────────────────────────────────────────────────────────────────┐
│  1. USER INPUT                                                  │
│  "Liste os arquivos em src/ e leia config.rs"                  │
└────────────┬────────────────────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────────────────────────────┐
│  2. CLI (src/cli.rs)                                            │
│  - Captura input via readline                                   │
│  - Valida que não está vazio                                    │
│  - Passa para agent.run()                                       │
└────────────┬────────────────────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────────────────────────────┐
│  3. AGENT: Security Layer 1 - Input Validation                 │
│  - Valida encoding UTF-8 ✓                                      │
│  - Verifica tamanho (52 bytes < 10MB) ✓                        │
│  - Remove caracteres perigosos: nenhum encontrado ✓             │
└────────────┬────────────────────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────────────────────────────┐
│  4. AGENT: Memory Search                                        │
│  - Gera embedding do input via OpenAI                           │
│  - Busca top 5 memórias similares no SQLite                     │
│  - Resultado: 2 memórias relevantes encontradas                 │
│    * "Listei arquivos em src/ anteriormente"                    │
│    * "config.rs contém configurações da API"                    │
└────────────┬────────────────────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────────────────────────────┐
│  5. AGENT: Build Prompt                                         │
│  Messages:                                                      │
│  [0] role: "system"                                             │
│      content: "You are RustClaw... [tools list]..."            │
│  [1] role: "system"                                             │
│      content: "Relevant memories:\n[mem1]\n[mem2]"             │
│  [2] role: "user"                                               │
│      content: "Liste os arquivos em src/ e leia config.rs"     │
└────────────┬────────────────────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────────────────────────────┐
│  6. LLM CALL #1                                                 │
│  POST https://api.moonshot.ai/v1/chat/completions               │
│  {                                                              │
│    "model": "kimi-k2-thinking",                                 │
│    "messages": [...],                                           │
│    "temperature": 0.7,                                          │
│    "max_tokens": 4000                                           │
│  }                                                              │
│                                                                 │
│  Response:                                                      │
│  {                                                              │
│    "choices": [{                                                │
│      "message": {                                               │
│        "role": "assistant",                                     │
│        "content": "Thought: Preciso listar arquivos em src/... │
│                    Action: list_files                           │
│                    Action Input: {\"path\": \"src/\"}"          │
│      }                                                          │
│    }]                                                           │
│  }                                                              │
└────────────┬────────────────────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────────────────────────────┐
│  7. AGENT: Parse Response                                       │
│  Extracted:                                                     │
│  - Thought: "Preciso listar arquivos em src/"                   │
│  - Action: "list_files"                                         │
│  - Action Input: "{\"path\": \"src/\"}"                         │
│  - Is Final Answer: false                                       │
└────────────┬────────────────────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────────────────────────────┐
│  8. TOOL EXECUTION: list_files                                  │
│                                                                 │
│  a) Security Layer 2 - Path Sanitization                        │
│     - Input: "src/"                                             │
│     - Canonicalize: "/Users/user/project/src"                   │
│     - Check allowed dirs: ✓ in project dir                      │
│     - Check path traversal: ✓ no ".."                           │
│                                                                 │
│  b) Security Layer 4 - Parameter Validation                     │
│     - Validate against schema: ✓                                │
│     - Required fields present: ✓                                │
│                                                                 │
│  c) Execute                                                     │
│     - Read directory entries                                    │
│     - Filter out hidden files (.git, etc)                       │
│     - Format output                                             │
│                                                                 │
│  d) Security Layer 5 - Output Filtering                         │
│     - Size: 234 bytes < 50KB ✓                                  │
│     - Sanitize paths: /Users/user → ~                           │
│     - Remove sensitive data: none found                         │
│                                                                 │
│  Result:                                                        │
│  "Files in src/:                                                │
│   - agent.rs (1655 lines)                                       │
│   - cli.rs (234 lines)                                          │
│   - config.rs (55 lines)                                        │
│   - main.rs (89 lines)                                          │
│   - memory/ (directory)                                         │
│   - tools/ (directory)"                                         │
└────────────┬────────────────────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────────────────────────────┐
│  9. AGENT: Add Observation to History                           │
│  conversation_history.push(Message {                            │
│    role: "user",                                                │
│    content: "Observation: Files in src/:\n..."                  │
│  })                                                             │
└────────────┬────────────────────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────────────────────────────┐
│  10. LLM CALL #2                                                │
│  POST https://api.moonshot.ai/v1/chat/completions               │
│  Messages now include:                                          │
│  [0] system                                                     │
│  [1] system (memories)                                          │
│  [2] user (original request)                                    │
│  [3] assistant (thought + action)                               │
│  [4] user (observation from list_files)                         │
│                                                                 │
│  Response:                                                      │
│  "Thought: Agora preciso ler config.rs                          │
│   Action: read_file                                             │
│   Action Input: {\"path\": \"src/config.rs\"}"                  │
└────────────┬────────────────────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────────────────────────────┐
│  11. TOOL EXECUTION: read_file                                  │
│  [Same security flow as before]                                 │
│                                                                 │
│  Result:                                                        │
│  "use serde::Deserialize;                                       │
│   #[derive(Debug, Deserialize)]                                 │
│   pub struct Config {                                           │
│       pub api_key: String,                                      │
│       ..."                                                      │
└────────────┬────────────────────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────────────────────────────┐
│  12. LLM CALL #3                                                │
│  Messages now include observation from read_file                │
│                                                                 │
│  Response:                                                      │
│  "Thought: Tenho todas as informações necessárias               │
│   Final Answer: Encontrei os seguintes arquivos em src/:        │
│   - agent.rs (1655 linhas)                                      │
│   - cli.rs (234 linhas)                                         │
│   - config.rs (55 linhas)                                       │
│   - main.rs (89 linhas)                                         │
│   - memory/ (diretório)                                         │
│   - tools/ (diretório)                                          │
│                                                                 │
│   O arquivo config.rs contém a struct Config com campos:        │
│   - api_key: String (MOONSHOT_API_KEY)                          │
│   - base_url: String (https://api.moonshot.ai/v1)               │
│   - model: String (kimi-k2-thinking)                            │
│   - temperature, max_tokens, top_p, etc."                       │
└────────────┬────────────────────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────────────────────────────┐
│  13. AGENT: Parse Final Answer                                  │
│  - Detected "Final Answer:" prefix                              │
│  - is_final_answer = true                                       │
│  - Extract content after "Final Answer:"                        │
└────────────┬────────────────────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────────────────────────────┐
│  14. AGENT: Store in Memory                                     │
│  memory.store(                                                  │
│    input: "Liste os arquivos em src/ e leia config.rs",        │
│    output: "Encontrei os seguintes arquivos...",                │
│    metadata: {                                                  │
│      "tools_used": ["list_files", "read_file"],                 │
│      "iterations": 3,                                           │
│      "timestamp": 1234567890                                    │
│    }                                                            │
│  )                                                              │
│                                                                 │
│  - Generates embedding                                          │
│  - Stores in SQLite with embedding BLOB                         │
└────────────┬────────────────────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────────────────────────────┐
│  15. AGENT: Return to CLI                                       │
│  return Ok("Encontrei os seguintes arquivos...")                │
└────────────┬────────────────────────────────────────────────────┘
             │
             ▼
┌─────────────────────────────────────────────────────────────────┐
│  16. CLI: Display to User                                       │
│  - Formats output with colors/styling                           │
│  - Prints to terminal                                           │
│  - Waits for next input                                         │
└─────────────────────────────────────────────────────────────────┘
```

### Métricas desta Requisição

- **Total de chamadas LLM**: 3
- **Ferramentas executadas**: 2 (list_files, read_file)
- **Iterações do loop**: 3
- **Tokens usados**: ~2,500 tokens (estimado)
- **Memórias consultadas**: 2
- **Nova memória armazenada**: 1
- **Tempo total**: ~4 segundos

---

## Como Estender o Sistema

### Adicionar uma Nova Ferramenta

**1. Crie o arquivo da ferramenta em `src/tools/`**

```rust
// src/tools/weather.rs
use async_trait::async_trait;
use serde_json::json;

pub struct WeatherTool;

#[async_trait]
impl Tool for WeatherTool {
    fn name(&self) -> &str {
        "get_weather"
    }
    
    fn description(&self) -> &str {
        "Gets the current weather for a given location"
    }
    
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "location": {
                    "type": "string",
                    "description": "City name or coordinates"
                },
                "units": {
                    "type": "string",
                    "enum": ["metric", "imperial"],
                    "default": "metric"
                }
            },
            "required": ["location"]
        })
    }
    
    async fn execute(
        &self,
        params: serde_json::Value,
        context: &ToolContext,
    ) -> Result<String> {
        let location = params["location"].as_str().unwrap();
        let units = params["units"].as_str().unwrap_or("metric");
        
        // Chama API de clima (exemplo)
        let api_key = std::env::var("WEATHER_API_KEY")?;
        let url = format!(
            "https://api.openweathermap.org/data/2.5/weather?q={}&units={}&appid={}",
            location, units, api_key
        );
        
        let client = reqwest::Client::new();
        let response = client.get(&url).send().await?;
        let data: serde_json::Value = response.json().await?;
        
        let temp = data["main"]["temp"].as_f64().unwrap();
        let description = data["weather"][0]["description"].as_str().unwrap();
        
        Ok(format!(
            "Weather in {}: {}°C, {}",
            location, temp, description
        ))
    }
}
```

**2. Registre a ferramenta em `src/agent.rs`**

```rust
use crate::tools::weather::WeatherTool;

impl Agent {
    fn register_tools() -> HashMap<String, Box<dyn Tool>> {
        let mut tools = HashMap::new();
        
        // ... ferramentas existentes ...
        
        // Nova ferramenta
        tools.insert("get_weather".to_string(), Box::new(WeatherTool));
        
        tools
    }
}
```

**3. Adicione ao módulo de tools em `src/tools/mod.rs`**

```rust
pub mod weather;
pub use weather::WeatherTool;
```

**4. Teste a ferramenta**

```bash
cargo test weather_tool
```

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_weather_tool() {
        let tool = WeatherTool;
        let params = json!({
            "location": "London",
            "units": "metric"
        });
        
        let context = ToolContext::default();
        let result = tool.execute(params, &context).await;
        
        assert!(result.is_ok());
        assert!(result.unwrap().contains("Weather in London"));
    }
}
```

### Adicionar uma Nova Skill

**1. Crie arquivo markdown em `skills/`**

```markdown
# Tester

## Description
Expert in writing and running tests, ensuring code quality through comprehensive test coverage.

## Instructions

You are now a testing expert. Focus on:

1. **Test Coverage**
   - Write unit tests for all functions
   - Create integration tests for workflows
   - Add edge case tests

2. **Test Quality**
   - Use descriptive test names
   - Follow AAA pattern (Arrange, Act, Assert)
   - Mock external dependencies

3. **Testing Tools**
   - Prefer `cargo test` for Rust
   - Use `pytest` for Python
   - Use `jest` for JavaScript

## Examples

User: Write tests for the add function
Assistant:
Thought: I need to create comprehensive tests
Action: write_file
Action Input: {"path": "tests/add_test.rs", "content": "..."}
```

**2. Teste carregar a skill**

```bash
# No CLI do RustClaw
> load_skill tester
Skill 'tester' loaded successfully!

> write tests for config.rs
[Agent agora se comporta como expert em testes]
```

### Adicionar Campo na Memória

**1. Adicione coluna no schema SQLite**

```sql
-- Crie migration em src/memory/migrations/003_add_tags.sql
ALTER TABLE memories ADD COLUMN tags TEXT;
CREATE INDEX idx_memories_tags ON memories(tags);
```

**2. Atualize struct Memory**

```rust
// src/memory/types.rs
pub struct Memory {
    pub id: i64,
    pub content: String,
    pub embedding: Vec<f32>,
    pub metadata: Option<serde_json::Value>,
    pub tags: Option<Vec<String>>,  // NOVO
    pub created_at: i64,
}
```

**3. Atualize queries**

```rust
// src/memory/store.rs
pub async fn store_with_tags(
    &self,
    content: &str,
    tags: Vec<String>,
) -> Result<i64> {
    let tags_json = serde_json::to_string(&tags)?;
    
    sqlx::query(
        "INSERT INTO memories (content, embedding, tags, created_at) 
         VALUES (?, ?, ?, ?)"
    )
    .bind(content)
    .bind(embedding_bytes)
    .bind(tags_json)
    .bind(Utc::now().timestamp())
    .execute(&self.pool)
    .await?
    .last_insert_rowid()
}

pub async fn search_by_tags(&self, tags: &[String]) -> Result<Vec<Memory>> {
    // Implementa busca por tags
}
```

### Adicionar Nova Camada de Segurança

```rust
// src/security/rate_limiter.rs
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct RateLimiter {
    limits: HashMap<String, (usize, Duration)>,
    usage: HashMap<String, Vec<Instant>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        let mut limits = HashMap::new();
        
        // 100 chamadas LLM por hora
        limits.insert("llm_calls".to_string(), (100, Duration::from_secs(3600)));
        
        // 1000 tool executions por hora
        limits.insert("tool_calls".to_string(), (1000, Duration::from_secs(3600)));
        
        Self {
            limits,
            usage: HashMap::new(),
        }
    }
    
    pub fn check_limit(&mut self, resource: &str) -> Result<()> {
        let (max_calls, window) = self.limits
            .get(resource)
            .ok_or(Error::UnknownResource)?;
        
        // Remove entradas antigas
        let now = Instant::now();
        self.usage
            .entry(resource.to_string())
            .or_insert_with(Vec::new)
            .retain(|&timestamp| now.duration_since(timestamp) < *window);
        
        let usage_count = self.usage[resource].len();
        
        if usage_count >= *max_calls {
            return Err(Error::RateLimitExceeded);
        }
        
        // Registra uso
        self.usage.get_mut(resource).unwrap().push(now);
        
        Ok(())
    }
}
```

**Integre no Agent:**

```rust
impl Agent {
    async fn call_llm(&mut self) -> Result<String> {
        // Verifica rate limit
        self.rate_limiter.check_limit("llm_calls")?;
        
        // ... resto do código ...
    }
}
```

---

## Resumo dos Principais Arquivos

| Arquivo | Linhas | Responsabilidade |
|---------|--------|------------------|
| `src/main.rs` | 89 | Entry point, inicialização |
| `src/agent.rs` | 1655 | Core do agente, ReAct loop, LLM calls |
| `src/cli.rs` | 234 | Interface CLI, input/output |
| `src/config.rs` | 55 | Configuração, .env loading |
| `src/memory/store.rs` | 412 | Storage SQLite, embeddings |
| `src/memory/search.rs` | 156 | Busca semântica |
| `src/tools/mod.rs` | 89 | Registry de ferramentas |
| `src/tools/file_ops.rs` | 234 | Ferramentas de arquivo |
| `src/tools/shell.rs` | 123 | Execução de comandos |
| `src/security/mod.rs` | 345 | Validação e sanitização |
| `src/skills/mod.rs` | 198 | Sistema de skills |

---

## Próximos Passos Sugeridos

1. **Melhorias de Performance**
   - Implementar cache de embeddings
   - Paralelizar tool execution quando possível
   - Otimizar queries SQL com índices

2. **Novas Funcionalidades**
   - Suporte a imagens (vision models)
   - Streaming de respostas (SSE)
   - Multi-agente (vários agents colaborando)

3. **Monitoramento**
   - Adicionar telemetria (OpenTelemetry)
   - Dashboard de métricas (Grafana)
   - Logging estruturado (tracing)

4. **Testes**
   - Aumentar cobertura de testes (>80%)
   - Testes de integração end-to-end
   - Benchmarks de performance

---

**Documentação criada em:** 2026-03-20  
**Versão do RustClaw:** 0.1.0 (feat/moonshot branch)  
**Autor:** OpenCode
