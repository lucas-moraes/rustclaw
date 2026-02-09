use crate::tools::Tool;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub struct LocationTool;

impl LocationTool {
    pub fn new() -> Self {
        Self
    }

    async fn get_location_from_ip(&self) -> Result<LocationInfo, String> {
        let client = reqwest::Client::new();
        
        match client.get("https://ipapi.co/json/").send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<LocationInfo>().await {
                        Ok(info) => return Ok(info),
                        Err(e) => return Err(format!("Parse error: {}", e)),
                    }
                }
            }
            Err(_) => {}
        }

        match client.get("https://ipinfo.io/json").send().await {
            Ok(response) => {
                if response.status().is_success() {
                    match response.json::<LocationInfo>().await {
                        Ok(info) => return Ok(info),
                        Err(e) => return Err(format!("Parse error: {}", e)),
                    }
                }
            }
            Err(_) => {}
        }

        Err("Failed to get location".to_string())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LocationInfo {
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub country_name: String,
    #[serde(default)]
    pub timezone: String,
    #[serde(default)]
    pub loc: String,
}

#[async_trait::async_trait]
impl Tool for LocationTool {
    fn name(&self) -> &str {
        "location"
    }

    fn description(&self) -> &str {
        "Obtém localização geográfica do dispositivo. Input: {}"
    }

    async fn call(&self, _args: Value) -> Result<String, String> {
        match self.get_location_from_ip().await {
            Ok(info) => {
                let city = if info.city.is_empty() { "Desconhecida" } else { &info.city };
                let country = if !info.country_name.is_empty() {
                    &info.country_name
                } else if !info.country.is_empty() {
                    &info.country
                } else {
                    "Desconhecido"
                };

                Ok(format!(
                    "Localização:\nCidade: {}\nPaís: {}\nTimezone: {}",
                    city, country, info.timezone
                ))
            }
            Err(e) => Ok(format!("Não foi possível obter a localização: {}", e))
        }
    }
}
