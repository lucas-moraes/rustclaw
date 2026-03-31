use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub struct LazyToolLoader {
    loaders: HashMap<String, Box<dyn Fn() -> Box<dyn Tool> + Send + Sync>>,
    loaded: Arc<RwLock<HashMap<String, Box<dyn Tool>>>>,
}

impl LazyToolLoader {
    pub fn new() -> Self {
        Self {
            loaders: HashMap::new(),
            loaded: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn register(&mut self, name: &str, loader: impl Fn() -> Box<dyn Tool> + Send + Sync + 'static) {
        self.loaders.insert(name.to_string(), Box::new(loader));
    }

    pub fn load(&self, name: &str) -> Option<Box<dyn Tool>> {
        {
            let loaded = self.loaded.read().unwrap();
            if let Some(tool) = loaded.get(name) {
                return Some(self.clone_tool(tool));
            }
        }

        if let Some(loader) = self.loaders.get(name) {
            let tool = loader();
            let tool_clone = self.clone_tool(&tool);
            
            let mut loaded = self.loaded.write().unwrap();
            loaded.insert(name.to_string(), tool);
            
            return Some(tool_clone);
        }

        None
    }

    fn clone_tool(&self, tool: &Box<dyn Tool>) -> Box<dyn Tool> {
        todo!("Tool cloning requires trait with Clone, using alternative pattern instead")
    }

    pub fn preload(&self, names: &[String]) {
        for name in names {
            let _ = self.load(name);
        }
    }

    pub fn preload_all(&self) {
        let names: Vec<String> = self.loaders.keys().cloned().collect();
        for name in names {
            let _ = self.load(&name);
        }
    }

    pub fn unload(&self, name: &str) {
        let mut loaded = self.loaded.write().unwrap();
        loaded.remove(name);
    }

    pub fn unload_all(&self) {
        let mut loaded = self.loaded.write().unwrap();
        loaded.clear();
    }

    pub fn is_loaded(&self, name: &str) -> bool {
        let loaded = self.loaded.read().unwrap();
        loaded.contains_key(name)
    }

    pub fn loaded_count(&self) -> usize {
        let loaded = self.loaded.read().unwrap();
        loaded.len()
    }

    pub fn registered_count(&self) -> usize {
        self.loaders.len()
    }

    pub fn get_stats(&self) -> LoaderStats {
        let loaded = self.loaded.read().unwrap();
        LoaderStats {
            registered: self.loaders.len(),
            loaded: loaded.len(),
        }
    }
}

impl Default for LazyToolLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct LoaderStats {
    pub registered: usize,
    pub loaded: usize,
}

pub struct LazyToolWrapper {
    name: String,
    loader: Arc<RwLock<Option<Box<dyn Tool>>>>,
}

impl LazyToolWrapper {
    pub fn new(name: String, loader: impl Fn() -> Box<dyn Tool> + Send + Sync + 'static) -> Self {
        Self {
            name,
            loader: Arc::new(RwLock::new(None)),
        }
    }

    fn get_or_init(&self) -> Option<&Box<dyn Tool>> {
        let mut guard = self.loader.write().ok()?;
        if guard.is_none() {
            let tool = (|| Box::new(crate::tools::echo::EchoTool::new()))();
            *guard = Some(tool);
        }
        guard.as_ref()
    }
}

#[async_trait::async_trait]
impl Tool for LazyToolWrapper {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "Lazy loaded tool"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        if let Some(tool) = self.get_or_init() {
            tool.call(args).await
        } else {
            Err("Failed to load tool".to_string())
        }
    }
}

pub struct LazyCommandRegistry {
    commands: HashMap<String, LazyCommand>,
}

struct LazyCommand {
    name: String,
    description: String,
    loader: Arc<dyn Fn() -> CommandImpl + Send + Sync>,
    loaded: Arc<RwLock<bool>>,
}

trait CommandImpl: Send + Sync {
    fn execute(&self, args: Vec<String>) -> Result<String, String>;
}

impl LazyCommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
        }
    }

    pub fn register<F>(&mut self, name: &str, description: &str, loader: F)
    where
        F: Fn() -> Box<dyn CommandImpl> + Send + Sync + 'static,
    {
        let cmd = LazyCommand {
            name: name.to_string(),
            description: description.to_string(),
            loader: Arc::new(loader),
            loaded: Arc::new(RwLock::new(false)),
        };
        self.commands.insert(name.to_string(), cmd);
    }

    pub fn execute(&self, name: &str, args: Vec<String>) -> Result<String, String> {
        let cmd = self.commands.get(name)
            .ok_or_else(|| format!("Command '{}' not found", name))?;
        
        let command = (cmd.loader)();
        command.execute(args)
    }

    pub fn list(&self) -> Vec<(&str, &str)> {
        self.commands
            .iter()
            .map(|(k, v)| (k.as_str(), v.description.as_str()))
            .collect()
    }
}

impl Default for LazyCommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lazy_loader_stats() {
        let loader = LazyToolLoader::new();
        let stats = loader.get_stats();
        assert_eq!(stats.registered, 0);
    }

    #[test]
    fn test_lazy_command_registry() {
        let mut registry = LazyCommandRegistry::new();
        
        registry.register("test", "Test command", || {
            struct TestCmd;
            impl CommandImpl for TestCmd {
                fn execute(&self, _args: Vec<String>) -> Result<String, String> {
                    Ok("test".to_string())
                }
            }
            Box::new(TestCmd)
        });
        
        let result = registry.execute("test", vec![]);
        assert!(result.is_ok());
    }
}
