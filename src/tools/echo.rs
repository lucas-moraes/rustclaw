use super::Tool;
use serde_json::Value;

pub struct EchoTool;

#[async_trait::async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Repete o texto recebido. Input: { \"text\": \"mensagem\" }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let text = args["text"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'text' é obrigatório e deve ser uma string".to_string())?;
        Ok(format!("Você disse: {}", text))
    }
}
