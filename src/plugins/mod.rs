//! Plugin system for extensibility.
//!
//! Provides a trait and registry for dynamic tool loading.

use crate::tools::ToolRegistry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMetadata {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: Option<String>,
}

pub trait Plugin: Send + Sync {
    fn metadata(&self) -> PluginMetadata;
    fn register_tools(&self, registry: &mut ToolRegistry);
}

pub struct PluginRegistry {
    plugins: HashMap<String, Box<dyn Plugin>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    #[allow(dead_code)]
    pub fn register(&mut self, plugin: Box<dyn Plugin>) -> Result<(), String> {
        let name = plugin.metadata().name.clone();
        if self.plugins.contains_key(&name) {
            return Err(format!("Plugin '{}' is already registered", name));
        }
        self.plugins.insert(name, plugin);
        Ok(())
    }

    #[allow(dead_code)]
    pub fn unregister(&mut self, name: &str) -> Option<Box<dyn Plugin>> {
        self.plugins.remove(name)
    }

    #[allow(dead_code)]
    pub fn get(&self, name: &str) -> Option<&dyn Plugin> {
        self.plugins.get(name).map(|p| p.as_ref())
    }

    #[allow(dead_code)]
    pub fn list_plugins(&self) -> Vec<PluginMetadata> {
        self.plugins.values().map(|p| p.metadata()).collect()
    }

    #[allow(dead_code)]
    pub fn load_from_dir(&mut self, dir: &Path) -> Result<usize, String> {
        if !dir.exists() {
            return Ok(0);
        }

        let mut loaded = 0;
        for entry in
            std::fs::read_dir(dir).map_err(|e| format!("Failed to read plugin directory: {}", e))?
        {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("so") {
                tracing::info!("Found plugin: {:?}", path);
                loaded += 1;
            }
        }
        Ok(loaded)
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockPlugin {
        metadata: PluginMetadata,
    }

    impl MockPlugin {
        fn new(name: &str) -> Self {
            Self {
                metadata: PluginMetadata {
                    name: name.to_string(),
                    version: "1.0.0".to_string(),
                    description: "Mock plugin".to_string(),
                    author: None,
                },
            }
        }
    }

    impl Plugin for MockPlugin {
        fn metadata(&self) -> PluginMetadata {
            self.metadata.clone()
        }

        fn register_tools(&self, _registry: &mut ToolRegistry) {}
    }

    #[test]
    fn test_plugin_registry() {
        let mut registry = PluginRegistry::new();

        assert!(registry.list_plugins().is_empty());

        let plugin = Box::new(MockPlugin::new("test-plugin"));
        let name = plugin.metadata().name.clone();

        registry.register(plugin).unwrap();
        assert_eq!(registry.list_plugins().len(), 1);
        assert!(registry.get(&name).is_some());

        let removed = registry.unregister(&name);
        assert!(removed.is_some());
        assert!(registry.list_plugins().is_empty());
    }

    #[test]
    fn test_duplicate_plugin() {
        let mut registry = PluginRegistry::new();

        let plugin1 = Box::new(MockPlugin::new("test"));
        let result1 = registry.register(plugin1);
        assert!(result1.is_ok());

        let plugin2 = Box::new(MockPlugin::new("test"));
        let result2 = registry.register(plugin2);
        assert!(result2.is_err());
    }
}
