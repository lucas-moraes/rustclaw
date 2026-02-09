use crate::tools::Tool;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub struct LocationTool {
    client: reqwest::Client,
}

impl LocationTool {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");
        
        Self { client }
    }

    async fn get_location_from_ip(&self) -> Result<LocationInfo, String> {
        // Try multiple IP geolocation services
        let services = [
            "https://ipapi.co/json/",
            "https://ipinfo.io/json",
        ];

        for service in &services {
            match self.client.get(*service).send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        match response.json::<LocationInfo>().await {
                            Ok(info) => return Ok(info),
                            Err(e) => {
                                tracing::warn!("Failed to parse location from {}: {}", service, e);
                                continue;
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to fetch location from {}: {}", service, e);
                    continue;
                }
            }
        }

        Err("Failed to get location from all services".to_string())
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
    pub latitude: Option<f64>,
    #[serde(default)]
    pub longitude: Option<f64>,
    #[serde(default)]
    pub loc: String, // ipinfo format: "lat,lon"
    #[serde(default)]
    pub timezone: String,
    #[serde(default)]
    pub org: String,
}

impl LocationInfo {
    fn get_lat_lon(&self) -> (Option<f64>, Option<f64>) {
        if let (Some(lat), Some(lon)) = (self.latitude, self.longitude) {
            return (Some(lat), Some(lon));
        }
        
        // Try parsing from loc field (ipinfo format)
        if !self.loc.is_empty() {
            let parts: Vec<&str> = self.loc.split(',').collect();
            if parts.len() == 2 {
                if let (Ok(lat), Ok(lon)) = (parts[0].parse::<f64>(), parts[1].parse::<f64>()) {
                    return (Some(lat), Some(lon));
                }
            }
        }
        
        (None, None)
    }
}

#[async_trait::async_trait]
impl Tool for LocationTool {
    fn name(&self) -> &str {
        "location"
    }

    fn description(&self) -> &str {
        "Obtém localização geográfica do dispositivo baseado no IP. Input: {}"
    }

    async fn call(&self, _args: Value) -> Result<String, String> {
        match self.get_location_from_ip().await {
            Ok(info) => {
                let (lat, lon) = info.get_lat_lon();
                
                let city = if info.city.is_empty() { "Desconhecida" } else { &info.city };
                let region = if info.region.is_empty() { "" } else { &info.region };
                let country = if !info.country_name.is_empty() {
                    &info.country_name
                } else if !info.country.is_empty() {
                    &info.country
                } else {
                    "Desconhecido"
                };
                
                let location_str = if !region.is_empty() {
                    format!("{}, {}, {}", city, region, country)
                } else {
                    format!("{}, {}", city, country)
                };
                
                let coords = match (lat, lon) {
                    (Some(lat), Some(lon)) => format!("{}° {}, {}° {}", 
                        lat.abs(), 
                        if lat >= 0.0 { "N" } else { "S" },
                        lon.abs(),
                        if lon >= 0.0 { "E" } else { "W" }
                    ),
                    _ => "Coordenadas não disponíveis".to_string(),
                };

                let result = format!(
                    "Localização do dispositivo:\n\
                    Cidade: {}\n\
                    País: {}\n\
                    Coordenadas: {}\n\
                    Timezone: {}\n\
                    IP: {}",
                    location_str,
                    country,
                    coords,
                    if info.timezone.is_empty() { "Desconhecido" } else { &info.timezone },
                    if info.ip.is_empty() { "Oculto" } else { &info.ip }
                );
                
                Ok(result)
            }
            Err(e) => {
                tracing::error!("Failed to get location: {}", e);
                Ok(format!("Não foi possível obter a localização. Erro: {}\n\
                           O dispositivo pode estar sem acesso à internet ou os serviços de geolocalização estão indisponíveis.", e))
            }
        }
    }
}
