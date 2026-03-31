use std::sync::Arc;
use tokio::sync::OnceCell;

static PREFETCH_RESULTS: OnceCell<PrefetchData> = OnceCell::const_new();

#[derive(Debug, Clone)]
pub struct PrefetchData {
    pub config: Option<Arc<crate::config::Config>>,
    pub api_key: Option<String>,
    pub home_dir: Option<String>,
    pub current_dir: Option<String>,
}

impl PrefetchData {
    pub async fn load() -> Self {
        let (config_result, api_key_result, home_result, dir_result) = tokio::join!(
            Self::load_config(),
            Self::load_api_key(),
            Self::load_home_dir(),
            Self::load_current_dir(),
        );

        Self {
            config: config_result.ok(),
            api_key: api_key_result,
            home_dir: home_result,
            current_dir: dir_result,
        }
    }

    async fn load_config() -> anyhow::Result<Arc<crate::config::Config>> {
        tokio::task::spawn_blocking(|| {
            crate::config::Config::from_env()
        })
        .await?
        .map(Arc::new)
    }

    async fn load_api_key() -> Option<String> {
        std::env::var("OPENCODE_API_KEY")
            .or_else(|_| std::env::var("TOKEN"))
            .ok()
    }

    async fn load_home_dir() -> Option<String> {
        dirs::home_dir().map(|p| p.to_string_lossy().to_string())
    }

    async fn load_current_dir() -> Option<String> {
        std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    }
}

pub async fn prefetch() -> &'static PrefetchData {
    PREFETCH_RESULTS.get_or_init(|| async {
        PrefetchData::load().await
    }).await
}

pub fn get_cached_config() -> Option<Arc<crate::config::Config>> {
    PREFETCH_RESULTS.get().and_then(|data| data.config.clone())
}

pub fn get_cached_api_key() -> Option<String> {
    PREFETCH_RESULTS.get().and_then(|data| data.api_key.clone())
}

pub fn get_cached_home_dir() -> Option<String> {
    PREFETCH_RESULTS.get().and_then(|data| data.home_dir.clone())
}

pub fn get_cached_current_dir() -> Option<String> {
    PREFETCH_RESULTS.get().and_then(|data| data.current_dir.clone())
}

pub fn start_background_prefetch() {
    tokio::spawn(async {
        let _ = prefetch().await;
    });
}
