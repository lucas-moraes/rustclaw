use std::collections::HashMap;
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub tool_name: String,
    pub output: String,
    pub success: bool,
}

pub struct ParallelExecutor {
    max_parallel: usize,
}

impl ParallelExecutor {
    pub fn new(max_parallel: usize) -> Self {
        Self { max_parallel }
    }

    pub fn from_env() -> Self {
        let max_parallel = std::env::var("MAX_PARALLEL_TOOLS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);
        Self::new(max_parallel)
    }

    pub async fn execute_parallel<F, Fut>(
        &self,
        actions: Vec<(String, String)>,
        executor: &mut F,
    ) -> Vec<ToolResult>
    where
        F: FnMut(String, String) -> Fut,
        Fut: std::future::Future<Output = String>,
    {
        if actions.is_empty() {
            return vec![];
        }

        let actions_to_run: Vec<_> = if actions.len() > self.max_parallel {
            tracing::info!(
                "Capping parallel execution from {} to {} tools",
                actions.len(),
                self.max_parallel
            );
            actions.into_iter().take(self.max_parallel).collect()
        } else {
            actions
        };

        let mut results = Vec::with_capacity(actions_to_run.len());
        
        let futures: Vec<_> = actions_to_run
            .into_iter()
            .map(|(tool_name, action_input)| async move {
                (tool_name, action_input)
            })
            .collect();

        let handles = Self::join_all(futures).await;
        
        for (tool_name, action_input) in handles {
            let output = executor(tool_name.clone(), action_input.clone()).await;
            results.push(ToolResult {
                tool_name,
                output,
                success: true,
            });
        }
        
        results
    }

    async fn join_all<F>(futures: Vec<F>) -> Vec<F::Output>
    where
        F: std::future::Future,
    {
        let mut results = Vec::with_capacity(futures.len());
        for future in futures {
            results.push(future.await);
        }
        results
    }

    pub fn analyze_dependencies(
        actions: &[(String, String)],
        parsed_inputs: &[Value],
    ) -> DependencyAnalysis {
        let mut file_writes: HashMap<String, usize> = HashMap::new();
        let mut file_reads: HashMap<String, Vec<usize>> = HashMap::new();
        let mut shell_commands: Vec<usize> = Vec::new();
        let mut safe_indices: Vec<usize> = Vec::new();
        let mut unsafe_indices: Vec<usize> = Vec::new();

        for (i, (action, input)) in actions.iter().enumerate() {
            let path = Self::extract_path(input);
            
            match action.as_str() {
                "file_write" | "write_file" => {
                    if let Some(p) = &path {
                        if file_writes.contains_key(p) {
                            unsafe_indices.push(i);
                        } else {
                            file_writes.insert(p.clone(), i);
                            safe_indices.push(i);
                        }
                    } else {
                        safe_indices.push(i);
                    }
                }
                "file_read" | "read_file" | "read_multiple_files" => {
                    if let Some(p) = &path {
                        if file_writes.contains_key(p) {
                            unsafe_indices.push(i);
                        } else {
                            file_reads.entry(p.clone()).or_default().push(i);
                            safe_indices.push(i);
                        }
                    } else {
                        safe_indices.push(i);
                    }
                }
                "shell" | "bash" | "command" => {
                    shell_commands.push(i);
                }
                _ => {
                    safe_indices.push(i);
                }
            }
        }

        DependencyAnalysis {
            safe_indices,
            unsafe_indices,
            file_writes,
            file_reads,
            shell_commands,
        }
    }

    fn extract_path(input: &str) -> Option<String> {
        if let Ok(value) = serde_json::from_str::<Value>(input) {
            if let Some(path) = value.get("path").and_then(|v| v.as_str()) {
                return Some(path.to_string());
            }
            if let Some(paths) = value.get("paths").and_then(|v| v.as_array()) {
                if let Some(first) = paths.first() {
                    return first.as_str().map(|s| s.to_string());
                }
            }
            if let Some(command) = value.get("command").and_then(|v| v.as_str()) {
                if let Some(idx) = command.find("cat ") {
                    let after_cat = &command[idx + 4..];
                    if after_cat.starts_with(">") {
                        let after_redirect = after_cat[1..].trim_start();
                        return after_redirect.split_whitespace().next().map(|s| s.to_string());
                    }
                    return after_cat.split_whitespace().next().map(|s| s.to_string());
                }
            }
        }
        None
    }

    pub fn split_by_dependencies(
        actions: Vec<(String, String)>,
    ) -> (Vec<(String, String)>, Vec<(String, String)>) {
        let parsed_inputs: Vec<Value> = actions
            .iter()
            .map(|(_, input)| serde_json::from_str(input).unwrap_or(json!({})))
            .collect();

        let analysis = Self::analyze_dependencies(&actions, &parsed_inputs);
        
        let independent: Vec<_> = analysis.safe_indices
            .into_iter()
            .filter(|&i| !analysis.shell_commands.contains(&i))
            .map(|i| actions[i].clone())
            .collect();
        
        let dependent: Vec<_> = analysis.unsafe_indices
            .into_iter()
            .chain(analysis.shell_commands.into_iter())
            .map(|i| actions[i].clone())
            .collect();

        (independent, dependent)
    }
}

#[derive(Debug, Clone)]
pub struct DependencyAnalysis {
    pub safe_indices: Vec<usize>,
    pub unsafe_indices: Vec<usize>,
    pub file_writes: HashMap<String, usize>,
    pub file_reads: HashMap<String, Vec<usize>>,
    pub shell_commands: Vec<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_path_from_json() {
        let input = r#"{"path": "/tmp/test.txt"}"#;
        let path = ParallelExecutor::extract_path(input);
        assert_eq!(path, Some("/tmp/test.txt".to_string()));
    }

    #[test]
    fn test_extract_path_from_command() {
        let input = r#"{"command": "cat > /tmp/test.txt"}"#;
        let path = ParallelExecutor::extract_path(input);
        assert_eq!(path, Some("/tmp/test.txt".to_string()));
    }

    #[test]
    fn test_dependency_analysis_write_write() {
        let actions = vec![
            ("file_write".to_string(), r#"{"path": "/tmp/a.txt"}"#.to_string()),
            ("file_write".to_string(), r#"{"path": "/tmp/a.txt"}"#.to_string()),
        ];
        let inputs: Vec<Value> = actions
            .iter()
            .map(|(_, i)| serde_json::from_str(i).unwrap())
            .collect();
        let analysis = ParallelExecutor::analyze_dependencies(&actions, &inputs);
        assert_eq!(analysis.safe_indices, vec![0]);
        assert_eq!(analysis.unsafe_indices, vec![1]);
    }

    #[test]
    fn test_dependency_analysis_read_after_write() {
        let actions = vec![
            ("file_write".to_string(), r#"{"path": "/tmp/a.txt"}"#.to_string()),
            ("file_read".to_string(), r#"{"path": "/tmp/a.txt"}"#.to_string()),
        ];
        let inputs: Vec<Value> = actions
            .iter()
            .map(|(_, i)| serde_json::from_str(i).unwrap())
            .collect();
        let analysis = ParallelExecutor::analyze_dependencies(&actions, &inputs);
        assert_eq!(analysis.safe_indices, vec![0]);
        assert_eq!(analysis.unsafe_indices, vec![1]);
    }

    #[test]
    fn test_dependency_analysis_independent() {
        let actions = vec![
            ("file_read".to_string(), r#"{"path": "/tmp/a.txt"}"#.to_string()),
            ("file_read".to_string(), r#"{"path": "/tmp/b.txt"}"#.to_string()),
        ];
        let inputs: Vec<Value> = actions
            .iter()
            .map(|(_, i)| serde_json::from_str(i).unwrap())
            .collect();
        let analysis = ParallelExecutor::analyze_dependencies(&actions, &inputs);
        assert_eq!(analysis.safe_indices, vec![0, 1]);
        assert!(analysis.unsafe_indices.is_empty());
    }
}
