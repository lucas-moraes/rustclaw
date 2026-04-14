use std::path::{Path, PathBuf};

fn normalize_path(path: &Path) -> PathBuf {
    let mut components = path.components().peekable();
    let mut result = PathBuf::new();

    for component in components {
        match component {
            std::path::Component::ParentDir => {
                result.pop();
            }
            std::path::Component::CurDir => {}
            std::path::Component::RootDir => {
                result.push(component.as_os_str());
            }
            std::path::Component::Normal(s) => {
                result.push(s);
            }
            std::path::Component::Prefix(_) => {
                result.push(component.as_os_str());
            }
        }
    }

    if result.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        result
    }
}

#[derive(Clone, Debug)]
pub struct ProjectSandbox {
    allowed_dir: Option<PathBuf>,
}

impl ProjectSandbox {
    pub fn new() -> Self {
        Self { allowed_dir: None }
    }

    pub fn set_project_dir(&mut self, path: PathBuf) {
        let canonical = if path.is_absolute() {
            path.canonicalize().unwrap_or(path.clone())
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(&path))
                .unwrap_or(path.clone())
                .canonicalize()
                .unwrap_or(path)
        };
        self.allowed_dir = Some(canonical);
    }

    pub fn clear(&mut self) {
        self.allowed_dir = None;
    }

    pub fn is_active(&self) -> bool {
        self.allowed_dir.is_some()
    }

    pub fn allowed_dir(&self) -> Option<&PathBuf> {
        self.allowed_dir.as_ref()
    }

    pub fn validate_path(&self, path: &Path) -> Result<PathBuf, String> {
        let allowed = self
            .allowed_dir
            .as_ref()
            .ok_or_else(|| "Sandbox não está ativa".to_string())?;

        // When sandbox is active, use allowed_dir as base for relative paths
        // This ensures relative paths are resolved relative to the project directory
        let base_dir = allowed;

        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            base_dir.join(path)
        };

        // Canonicalize to resolve ".." and symlinks
        let canonical = if resolved.exists() {
            resolved.canonicalize().map_err(|e| {
                format!(
                    "Não foi possível resolver caminho '{}': {}",
                    resolved.display(),
                    e
                )
            })?
        } else if path.is_absolute() {
            // For absolute paths that don't exist, try to canonicalize parent
            if let Some(parent) = path.parent() {
                if parent.exists() {
                    let canonical_parent = parent.canonicalize().map_err(|e| {
                        format!(
                            "Não foi possível resolver caminho '{}': {}",
                            parent.display(),
                            e
                        )
                    })?;
                    let filename = path
                        .file_name()
                        .map(|n| canonical_parent.join(n))
                        .unwrap_or_else(|| canonical_parent);
                    return if filename.starts_with(allowed) {
                        Ok(filename)
                    } else {
                        Err(format!(
                            "'{}' está fora do diretório do projeto",
                            path.display()
                        ))
                    };
                }
            }
            normalize_path(&resolved)
        } else {
            // For relative paths that don't exist, canonicalize parent and check
            let normalized = normalize_path(&resolved);
            if let Some(parent) = normalized.parent() {
                if parent.exists() {
                    let canonical_parent = parent.canonicalize().map_err(|e| {
                        format!(
                            "Não foi possível resolver caminho '{}': {}",
                            parent.display(),
                            e
                        )
                    })?;
                    let filename = normalized
                        .file_name()
                        .map(|n| canonical_parent.join(n))
                        .unwrap_or_else(|| canonical_parent);
                    return if filename.starts_with(allowed) {
                        Ok(filename)
                    } else {
                        Err(format!(
                            "'{}' está fora do diretório do projeto",
                            path.display()
                        ))
                    };
                }
            }
            normalized
        };

        if canonical.starts_with(allowed) {
            Ok(canonical)
        } else {
            Err(format!(
                "'{}' está fora do diretório do projeto",
                path.display()
            ))
        }
    }

    pub fn validate_path_string(&self, path_str: &str) -> Result<PathBuf, String> {
        self.validate_path(Path::new(path_str))
    }

    pub fn is_path_allowed(&self, path: &Path) -> bool {
        self.validate_path(path).is_ok()
    }
}

impl Default for ProjectSandbox {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_not_active_by_default() {
        let sandbox = ProjectSandbox::new();
        assert!(!sandbox.is_active());
        assert!(sandbox.allowed_dir().is_none());
    }

    #[test]
    fn test_set_project_dir() {
        let mut sandbox = ProjectSandbox::new();
        sandbox.set_project_dir(PathBuf::from("/tmp/test_project"));
        assert!(sandbox.is_active());
        assert!(sandbox.allowed_dir().is_some());
    }

    #[test]
    fn test_clear_sandbox() {
        let mut sandbox = ProjectSandbox::new();
        sandbox.set_project_dir(PathBuf::from("/tmp/test_project"));
        sandbox.clear();
        assert!(!sandbox.is_active());
        assert!(sandbox.allowed_dir().is_none());
    }

    #[test]
    fn test_validate_path_inside_sandbox() {
        let mut sandbox = ProjectSandbox::new();
        let temp_dir = std::env::temp_dir();
        sandbox.set_project_dir(temp_dir.clone());

        let result = sandbox.validate_path(&temp_dir.join("file.txt"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_path_outside_sandbox() {
        let mut sandbox = ProjectSandbox::new();
        sandbox.set_project_dir(PathBuf::from("/tmp/test_project"));

        let result = sandbox.validate_path(Path::new("/etc/passwd"));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("está fora do diretório do projeto"));
    }

    #[test]
    fn test_validate_path_relative_inside_project() {
        let mut sandbox = ProjectSandbox::new();
        let temp_dir = std::env::temp_dir();
        sandbox.set_project_dir(temp_dir.clone());

        // Relative path should now be resolved against allowed_dir, not cwd
        let result = sandbox.validate_path(Path::new("file.txt"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_path_parent_directory() {
        let mut sandbox = ProjectSandbox::new();
        sandbox.set_project_dir(PathBuf::from("/tmp/test_project"));

        let result = sandbox.validate_path(Path::new("/tmp/test_project/../other_file.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn test_sandbox_inactive_allows_all() {
        let sandbox = ProjectSandbox::new();
        let result = sandbox.validate_path(Path::new("/any/path"));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Sandbox não está ativa");
    }

    #[test]
    fn test_normalize_path() {
        assert_eq!(
            normalize_path(Path::new("/a/b/../c")),
            PathBuf::from("/a/c")
        );
        assert_eq!(normalize_path(Path::new("/a/./b")), PathBuf::from("/a/b"));
    }
}
