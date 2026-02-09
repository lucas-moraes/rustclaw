use crate::tools::Tool;
use chrono::Local;
use serde_json::Value;

pub struct DateTimeTool;

impl DateTimeTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for DateTimeTool {
    fn name(&self) -> &str {
        "datetime"
    }

    fn description(&self) -> &str {
        "Obtém data e hora atual do sistema. Input: {}"
    }

    async fn call(&self, _args: Value) -> Result<String, String> {
        let now = Local::now();
        
        let result = format!(
            "Data: {}\nHora: {}\nDia da semana: {}\nTimezone: {}",
            now.format("%d/%m/%Y"),
            now.format("%H:%M:%S"),
            now.format("%A"),
            now.format("%:z")
        );
        
        Ok(result)
    }
}
