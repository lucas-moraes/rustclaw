use dotenv::dotenv;
use reqwest::Client;
use serde_json::json;
use std::env;
use tracing::{info, Level};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    let client = Client::new();
    let api_key = env::var("HF_TOKEN")?;

    let response = client
        .post("https://router.huggingface.co/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&json!({
            "model": "moonshotai/Kimi-K2-Instruct-0905:groq",
            "messages": [
                {
                    "role": "user",
                    "content": "Olá, teste de conexão com RustClaw!"
                }
            ],
            "max_tokens": 50
        }))
        .send()
        .await?;

    let res = response;
    let status = res.status();
    let body = res.text().await?;

    if status.is_success() {
        let json: serde_json::Value = serde_json::from_str(&body)?;
        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("Sem conteúdo");
        info!("Resposta: {}", content.trim());
    } else {
        info!("Erro {}: {}", status, body);
    }

    Ok(())
}
