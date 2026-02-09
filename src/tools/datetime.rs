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
        "Obtém data e hora atual do sistema. Input: {} ou {\"format\": \"iso\"}"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let now = Local::now();
        
        let format = args["format"].as_str().unwrap_or("full");
        
        let result = match format {
            "iso" => now.to_rfc3339(),
            "date" => now.format("%Y-%m-%d").to_string(),
            "time" => now.format("%H:%M:%S").to_string(),
            _ => format!(
                "Data: {}\nHora: {}\nDia da semana: {}\nTimezone: {}",
                now.format("%d/%m/%Y"),
                now.format("%H:%M:%S"),
                now.format("%A"),
                now.format("%:z")
            ),
        };
        
        Ok(result)
    }
}
