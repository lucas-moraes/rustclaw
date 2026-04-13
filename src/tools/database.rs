use super::Tool;
use rusqlite::Connection;
use serde_json::Value;
use std::path::Path;

pub struct DatabaseQueryTool;

impl DatabaseQueryTool {
    pub fn new() -> Self {
        Self
    }

    fn execute_sqlite(&self, db_path: &str, query: &str) -> Result<String, String> {
        let conn = Connection::open(db_path)
            .map_err(|e| format!("Failed to open database: {}", e))?;

        let trimmed = query.trim().to_lowercase();

        if trimmed.starts_with("select") || trimmed.starts_with("pragma") || trimmed.starts_with("explain") {
            let mut stmt = conn.prepare(&query)
                .map_err(|e| format!("Failed to prepare query: {}", e))?;

            let column_count = stmt.column_count();
            let mut results = Vec::new();

            let headers: Vec<String> = (0..column_count)
                .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
                .collect();
            let header_line = headers.join(" | ");
            results.push(header_line.clone());
            results.push("-".repeat(header_line.len()));

            let mut rows = stmt.raw_query();
            while let Ok(Some(row)) = rows.next() {
                let row_ref = row;
                let mut row_data = Vec::new();
                for i in 0..column_count {
                    let value = match row_ref.get_ref(i) {
                        Ok(rusqlite::types::ValueRef::Null) => "NULL".to_string(),
                        Ok(rusqlite::types::ValueRef::Integer(i)) => i.to_string(),
                        Ok(rusqlite::types::ValueRef::Real(f)) => format!("{:.4}", f),
                        Ok(rusqlite::types::ValueRef::Text(s)) => String::from_utf8_lossy(s).to_string(),
                        Ok(rusqlite::types::ValueRef::Blob(b)) => format!("[BLOB: {} bytes]", b.len()),
                        Err(_) => "?".to_string(),
                    };
                    row_data.push(value);
                }
                results.push(row_data.join(" | "));
            }

            Ok(results.join("\n"))
        } else {
            let affected = conn.execute(query, [])
                .map_err(|e| format!("Failed to execute: {}", e))?;
            Ok(format!("OK. {} rows affected.", affected))
        }
    }
}

#[async_trait::async_trait]
impl Tool for DatabaseQueryTool {
    fn name(&self) -> &str {
        "db_query"
    }

    fn description(&self) -> &str {
        "Execute SQL queries on SQLite databases. Input: { \"db_path\": \"/path/to/db.sqlite\", \"query\": \"SELECT * FROM table\" }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let db_path = args["db_path"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'db_path' é obrigatório".to_string())?;

        let query = args["query"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'query' é obrigatório".to_string())?;

        if db_path.is_empty() || query.is_empty() {
            return Err("db_path e query não podem estar vazios".to_string());
        }

        if !Path::new(db_path).exists() {
            return Err(format!("Database not found: {}", db_path));
        }

        self.execute_sqlite(db_path, query)
    }
}
