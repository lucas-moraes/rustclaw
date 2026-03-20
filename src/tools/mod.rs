use serde_json::Value;
use std::collections::HashMap;

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn call(&self, args: Value) -> Result<String, String>;
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<&Box<dyn Tool>> {
        self.tools.get(name)
    }

    pub fn list(&self) -> String {
        self.tools
            .values()
            .map(|t| format!("- {}: {}", t.name(), t.description()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub mod capabilities;
pub mod clear_memory;
pub mod datetime;
pub mod echo;
pub mod file_list;
pub mod file_read;
pub mod file_search;
pub mod file_write;
pub mod http;
pub mod location;
pub mod reminder;
pub mod reminder_parser;
pub mod shell;
pub mod skill_import;
pub mod skill_manager;
pub mod system;
pub mod browser;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::tools::browser::BrowserTool;
    use crate::tools::capabilities::CapabilitiesTool;
    use crate::tools::clear_memory::ClearMemoryTool;
    use crate::tools::datetime::DateTimeTool;
    use crate::tools::echo::EchoTool;
    use crate::tools::file_list::FileListTool;
    use crate::tools::file_read::FileReadTool;
    use crate::tools::file_search::FileSearchTool;
    use crate::tools::file_write::FileWriteTool;
    use crate::tools::http::{HttpGetTool, HttpPostTool};
    use crate::tools::location::LocationTool;
    use crate::tools::reminder::{AddReminderTool, CancelReminderTool, ListRemindersTool};
    use crate::tools::shell::ShellTool;
    use crate::tools::skill_import::SkillImportFromUrlTool;
    use crate::tools::skill_manager::{
        SkillCreateTool, SkillDeleteTool, SkillEditTool, SkillListTool, SkillRenameTool,
        SkillValidateTool,
    };
    use crate::tools::system::SystemInfoTool;
    use serde_json::json;
    use std::sync::Arc;

    fn base_config() -> Config {
        Config {
            api_key: "test".to_string(),
            base_url: "https://router.huggingface.co/v1".to_string(),
            model: "test".to_string(),
            max_tokens: 100,
            max_iterations: 2,
            plan_auto_threshold: 4,
            max_retries: 5,
            tavily_api_key: None,
            timezone: "America/Sao_Paulo".to_string(),
        }
    }

    fn network_enabled() -> bool {
        std::env::var("RUN_NETWORK_TESTS")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    fn browser_enabled() -> bool {
        std::env::var("RUN_BROWSER_TESTS")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn tool_file_operations() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        let file_path = root.join("test.txt");

        let write = FileWriteTool::new();
        let read = FileReadTool::new();
        let list = FileListTool::new();
        let search = FileSearchTool::new();

        let write_res = write
            .call(json!({"path": file_path.to_string_lossy(), "content": "hello"}))
            .await
            .unwrap();
        assert!(write_res.contains("Criado") || write_res.contains("Atualizado"));

        let read_res = read
            .call(json!({"path": file_path.to_string_lossy()}))
            .await
            .unwrap();
        assert!(read_res.contains("hello"));

        let list_res = list
            .call(json!({"path": root.to_string_lossy()}))
            .await
            .unwrap();
        assert!(list_res.contains("test.txt"));

        let search_res = search
            .call(json!({"path": root.to_string_lossy(), "pattern": "*.txt"}))
            .await
            .unwrap();
        assert!(search_res.contains("test.txt"));
    }

    #[tokio::test]
    async fn tool_shell() {
        let shell = ShellTool::new();
        let output = shell
            .call(json!({"command": "echo hello"}))
            .await
            .unwrap();
        assert!(output.contains("hello"));
    }

    #[tokio::test]
    async fn tool_system_and_capabilities() {
        let capabilities = CapabilitiesTool::new();
        let system = SystemInfoTool::new();
        let datetime = DateTimeTool::new();
        let echo = EchoTool;

        let caps = capabilities.call(json!({})).await.unwrap();
        assert!(caps.contains("file_list"));

        let sys = system.call(json!({})).await.unwrap();
        assert!(sys.contains("CPU") || sys.contains("Memória") || sys.contains("Discos"));

        let cpu = system.call(json!({"detail": "cpu"})).await.unwrap();
        assert!(cpu.contains("CPU"));

        let mem = system.call(json!({"detail": "memory"})).await.unwrap();
        assert!(mem.contains("Memória"));

        let disk = system.call(json!({"detail": "disk"})).await.unwrap();
        assert!(disk.contains("Discos"));

        let dt = datetime.call(json!({})).await.unwrap();
        assert!(!dt.is_empty());

        let echo_out = echo.call(json!({"text": "ping"})).await.unwrap();
        assert!(echo_out.contains("ping"));
    }

    #[tokio::test]
    async fn tool_clear_memory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("mem.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE memories (id TEXT PRIMARY KEY, content TEXT NOT NULL, embedding BLOB NOT NULL, timestamp TEXT NOT NULL, importance REAL NOT NULL DEFAULT 0.5, memory_type TEXT NOT NULL, metadata TEXT NOT NULL DEFAULT '{}', search_count INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE scheduled_tasks (id TEXT PRIMARY KEY, name TEXT NOT NULL, cron_expression TEXT NOT NULL, task_type TEXT NOT NULL, is_active INTEGER NOT NULL DEFAULT 1, created_at TEXT NOT NULL, last_run TEXT, next_run TEXT, metadata TEXT NOT NULL DEFAULT '{}');
            CREATE TABLE reminders (id TEXT PRIMARY KEY, message TEXT NOT NULL, remind_at TEXT NOT NULL, created_at TEXT NOT NULL, is_recurring INTEGER NOT NULL DEFAULT 0, cron_expression TEXT, chat_id INTEGER NOT NULL, is_sent INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE checkpoints (id TEXT PRIMARY KEY, user_input TEXT NOT NULL, current_iteration INTEGER NOT NULL, messages_json TEXT, completed_tools_json TEXT, plan_text TEXT, project_dir TEXT, plan_file TEXT, phase TEXT NOT NULL, state TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
            CREATE TABLE active_skills (chat_id INTEGER PRIMARY KEY, skill_name TEXT NOT NULL DEFAULT 'general', activated_at TEXT NOT NULL, last_used TEXT NOT NULL);
            "#
        ).unwrap();

        let clear = ClearMemoryTool::new(&db_path);
        let resp = clear.call(json!({"confirm": false})).await.unwrap();
        assert!(resp.contains("confirm"));

        let resp_ok = clear.call(json!({"confirm": true})).await.unwrap();
        assert!(resp_ok.contains("sucesso"));
    }

    #[tokio::test]
    async fn tool_skills() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        let old_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(root).unwrap();

        let list = SkillListTool::new();
        let create = SkillCreateTool::new();
        let edit = SkillEditTool::new("skills");
        let rename = SkillRenameTool::new();
        let validate = SkillValidateTool::new();
        let delete = SkillDeleteTool::new();

        let create_resp = create
            .call(json!({"name": "demo", "validate": false}))
            .await
            .unwrap();
        assert!(create_resp.contains("demo"));

        let list_resp = list.call(json!({})).await.unwrap();
        assert!(list_resp.contains("demo"));

        let edit_resp = edit
            .call(json!({"name": "demo", "content": "# Skill: demo\n\n## Descrição\nTeste\n\n## Contexto\nTeste"}))
            .await
            .unwrap();
        assert!(edit_resp.contains("atualizada") || edit_resp.contains("sucesso"));

        let validate_resp = validate.call(json!({"name": "demo"})).await.unwrap();
        assert!(validate_resp.contains("válida") || validate_resp.contains("valida"));

        let rename_resp = rename
            .call(json!({"old_name": "demo", "new_name": "demo2"}))
            .await
            .unwrap();
        assert!(rename_resp.contains("demo2"));

        let delete_resp = delete
            .call(json!({"name": "demo2", "confirm": true}))
            .await
            .unwrap();
        assert!(delete_resp.contains("removida"));

        std::env::set_current_dir(old_dir).unwrap();
    }

    #[tokio::test]
    async fn tool_reminders() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("mem.db");
        let cfg = Arc::new(base_config());
        let chat_id = 1;

        let add = AddReminderTool::new(cfg.clone(), &db_path, chat_id);
        let list = ListRemindersTool::new(&db_path, chat_id);
        let cancel = CancelReminderTool::new(&db_path, chat_id);

        let add_resp = add
            .call(json!({"text": "amanhã às 10h lembrar teste"}))
            .await
            .unwrap();
        assert!(add_resp.contains("Lembrete"));

        let list_resp = list.call(json!({})).await.unwrap();
        assert!(list_resp.contains("Lembretes") || list_resp.contains("lembrete"));

        let id_line = list_resp
            .lines()
            .find(|line| line.contains("🆔"))
            .unwrap_or("");
        let id = id_line.split_whitespace().last().unwrap_or("invalid");

        let cancel_resp = cancel
            .call(json!({"id": id}))
            .await
            .unwrap();
        assert!(cancel_resp.contains("cancelado") || cancel_resp.contains("Cancelado"));
    }

    #[tokio::test]
    async fn tool_http_and_location_optional() {
        if !network_enabled() {
            return;
        }

        let http_get = HttpGetTool::new();
        let http_post = HttpPostTool::new();
        let location = LocationTool::new();

        let get_resp = http_get
            .call(json!({"url": "https://httpbin.org/get"}))
            .await
            .unwrap();
        assert!(get_resp.contains("Status"));

        let post_resp = http_post
            .call(json!({"url": "https://httpbin.org/post", "body": {"ping": "pong"}}))
            .await
            .unwrap();
        assert!(post_resp.contains("Status"));

        let loc_resp = location.call(json!({})).await.unwrap();
        assert!(loc_resp.contains("Localização") || loc_resp.contains("localização"));
    }

    #[tokio::test]
    async fn tool_browser_optional() {
        if !browser_enabled() {
            return;
        }

        let temp_dir = tempfile::tempdir().unwrap();
        let browser = BrowserTool::new(temp_dir.path().to_path_buf());

        let result = browser
            .call(json!({"action": "navigate", "url": "https://example.com"}))
            .await;

        match result {
            Ok(_) => {}
            Err(e) => {
                assert!(
                    e.contains("Chromium") || e.contains("browser"),
                    "Unexpected browser error: {}",
                    e
                );
            }
        }
    }

    #[tokio::test]
    async fn tool_skill_import_optional() {
        if !network_enabled() {
            return;
        }

        let temp_dir = tempfile::tempdir().unwrap();
        let old_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(temp_dir.path()).unwrap();

        let importer = SkillImportFromUrlTool::new();
        let resp = importer
            .call(json!({"url": "https://raw.githubusercontent.com/vercel-labs/skills/main/skills/find-skills/SKILL.md", "skill_name": "find-skills"}))
            .await;

        assert!(resp.is_ok());
        std::env::set_current_dir(old_dir).unwrap();
    }
}
