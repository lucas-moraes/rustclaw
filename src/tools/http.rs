use super::Tool;
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

const MAX_RESPONSE_SIZE: usize = 100_000;
const TIMEOUT_SECONDS: u64 = 30;

pub struct HttpGetTool;
pub struct HttpPostTool;

impl HttpGetTool {
    pub fn new() -> Self {
        Self
    }
}

impl HttpPostTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Tool for HttpGetTool {
    fn name(&self) -> &str {
        "http_get"
    }

    fn description(&self) -> &str {
        "Faz requisição HTTP GET. Input: { \"url\": \"https://api.example.com/data\", \"headers\": {} } (headers opcional)"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let url = args["url"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'url' é obrigatório".to_string())?;

        let headers = args["headers"].as_object();

        let client = Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECONDS))
            .build()
            .map_err(|e| format!("Erro ao criar client HTTP: {}", e))?;

        let mut request = client.get(url);

        if let Some(headers) = headers {
            for (key, value) in headers {
                if let Some(val_str) = value.as_str() {
                    request = request.header(key, val_str);
                }
            }
        }

        let response = request
            .send()
            .await
            .map_err(|e| format!("Erro na requisição: {}", e))?;

        let status = response.status();
        let content = response
            .text()
            .await
            .map_err(|e| format!("Erro ao ler resposta: {}", e))?;

        let truncated = if content.len() > MAX_RESPONSE_SIZE {
            format!(
                "{}\n\n[RESPOSTA TRUNCADA - {} bytes de {} total]",
                &content[..MAX_RESPONSE_SIZE],
                MAX_RESPONSE_SIZE,
                content.len()
            )
        } else {
            content
        };

        if status.is_success() {
            Ok(format!("Status {}\n\n{}", status, truncated))
        } else {
            Err(format!(
                "Erro HTTP {}:\n{}",
                status,
                truncated.chars().take(500).collect::<String>()
            ))
        }
    }
}

#[async_trait::async_trait]
impl Tool for HttpPostTool {
    fn name(&self) -> &str {
        "http_post"
    }

    fn description(&self) -> &str {
        "Faz requisição HTTP POST. Input: { \"url\": \"...\", \"body\": {...}, \"headers\": {...}, \"content_type\": \"application/json\" }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let url = args["url"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'url' é obrigatório".to_string())?;

        let body = &args["body"];
        let headers = args["headers"].as_object();
        let content_type = args["content_type"].as_str().unwrap_or("application/json");

        let client = Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECONDS))
            .build()
            .map_err(|e| format!("Erro ao criar client HTTP: {}", e))?;

        let mut request = client.post(url);

        if let Some(headers) = headers {
            for (key, value) in headers {
                if let Some(val_str) = value.as_str() {
                    request = request.header(key, val_str);
                }
            }
        }

        if content_type == "application/json" {
            request = request.header("Content-Type", "application/json");
            request = request.json(body);
        } else if content_type == "application/x-www-form-urlencoded" {
            request = request.header("Content-Type", "application/x-www-form-urlencoded");
            if let Some(obj) = body.as_object() {
                let params: Vec<(String, String)> = obj
                    .iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect();
                request = request.form(&params);
            }
        } else {
            request = request.header("Content-Type", content_type);
            if let Some(text) = body.as_str() {
                request = request.body(text.to_string());
            } else {
                request = request.json(body);
            }
        }

        let response = request
            .send()
            .await
            .map_err(|e| format!("Erro na requisição: {}", e))?;

        let status = response.status();
        let content = response
            .text()
            .await
            .map_err(|e| format!("Erro ao ler resposta: {}", e))?;

        let truncated = if content.len() > MAX_RESPONSE_SIZE {
            format!(
                "{}\n\n[RESPOSTA TRUNCADA - {} bytes de {} total]",
                &content[..MAX_RESPONSE_SIZE],
                MAX_RESPONSE_SIZE,
                content.len()
            )
        } else {
            content
        };

        if status.is_success() {
            Ok(format!("Status {}\n\n{}", status, truncated))
        } else {
            Err(format!(
                "Erro HTTP {}:\n{}",
                status,
                truncated.chars().take(500).collect::<String>()
            ))
        }
    }
}

impl Default for HttpGetTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for HttpPostTool {
    fn default() -> Self {
        Self::new()
    }
}
