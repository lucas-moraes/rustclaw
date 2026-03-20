use std::path::Path;

/// Representa o tipo de projeto detectado e seus comandos de build/teste
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectType {
    Rust,
    JavaScript,
    TypeScript,
    Python,
    Go,
    Java,
    Unknown,
}

/// Informações sobre como buildar e testar um projeto
#[derive(Debug, Clone)]
pub struct BuildInfo {
    pub project_type: ProjectType,
    pub build_command: String,
    pub test_command: Option<String>,
    pub working_dir: String,
}

/// Detecta o tipo de projeto baseado nos arquivos do diretório
pub struct BuildDetector;

impl BuildDetector {
    /// Detecta o tipo de projeto analisando arquivos no diretório
    pub fn detect(dir: &str) -> BuildInfo {
        let path = Path::new(dir);

        // Verifica Rust (Cargo.toml)
        if path.join("Cargo.toml").exists() {
            return BuildInfo {
                project_type: ProjectType::Rust,
                build_command: "cargo build".to_string(),
                test_command: Some("cargo test".to_string()),
                working_dir: dir.to_string(),
            };
        }

        // Verifica TypeScript (tsconfig.json)
        if path.join("tsconfig.json").exists() {
            let build_cmd = if path.join("package.json").exists() {
                // Verifica se tem script de build no package.json
                "npm run build".to_string()
            } else {
                "tsc".to_string()
            };

            return BuildInfo {
                project_type: ProjectType::TypeScript,
                build_command: build_cmd,
                test_command: Some("npm test".to_string()),
                working_dir: dir.to_string(),
            };
        }

        // Verifica JavaScript (package.json sem tsconfig.json)
        if path.join("package.json").exists() {
            return BuildInfo {
                project_type: ProjectType::JavaScript,
                build_command: "npm run build".to_string(),
                test_command: Some("npm test".to_string()),
                working_dir: dir.to_string(),
            };
        }

        // Verifica Python
        if path.join("pyproject.toml").exists() || path.join("setup.py").exists() {
            return BuildInfo {
                project_type: ProjectType::Python,
                build_command: "python -m build".to_string(),
                test_command: Some("pytest".to_string()),
                working_dir: dir.to_string(),
            };
        }

        // Verifica Go
        if path.join("go.mod").exists() {
            return BuildInfo {
                project_type: ProjectType::Go,
                build_command: "go build".to_string(),
                test_command: Some("go test ./...".to_string()),
                working_dir: dir.to_string(),
            };
        }

        // Verifica Java (Maven)
        if path.join("pom.xml").exists() {
            return BuildInfo {
                project_type: ProjectType::Java,
                build_command: "mvn compile".to_string(),
                test_command: Some("mvn test".to_string()),
                working_dir: dir.to_string(),
            };
        }

        // Verifica Java (Gradle)
        if path.join("build.gradle").exists() || path.join("build.gradle.kts").exists() {
            return BuildInfo {
                project_type: ProjectType::Java,
                build_command: "./gradlew build".to_string(),
                test_command: Some("./gradlew test".to_string()),
                working_dir: dir.to_string(),
            };
        }

        // Projeto desconhecido
        BuildInfo {
            project_type: ProjectType::Unknown,
            build_command: String::new(),
            test_command: None,
            working_dir: dir.to_string(),
        }
    }

    /// Verifica se o diretório é um projeto buildável
    pub fn is_buildable(dir: &str) -> bool {
        let info = Self::detect(dir);
        info.project_type != ProjectType::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_detect_rust_project() {
        let temp_dir = std::env::temp_dir().join("test_rust_project");
        fs::create_dir_all(&temp_dir).unwrap();
        fs::write(temp_dir.join("Cargo.toml"), "[package]\nname = \"test\"\n").unwrap();

        let info = BuildDetector::detect(temp_dir.to_str().unwrap());
        assert_eq!(info.project_type, ProjectType::Rust);
        assert_eq!(info.build_command, "cargo build");
        assert_eq!(info.test_command, Some("cargo test".to_string()));

        fs::remove_dir_all(temp_dir).ok();
    }

    #[test]
    fn test_detect_unknown_project() {
        let temp_dir = std::env::temp_dir().join("test_unknown_project");
        fs::create_dir_all(&temp_dir).unwrap();

        let info = BuildDetector::detect(temp_dir.to_str().unwrap());
        assert_eq!(info.project_type, ProjectType::Unknown);
        assert_eq!(info.build_command, "");

        fs::remove_dir_all(temp_dir).ok();
    }

    #[test]
    fn test_is_buildable() {
        let temp_dir = std::env::temp_dir().join("test_buildable");
        fs::create_dir_all(&temp_dir).unwrap();

        assert!(!BuildDetector::is_buildable(temp_dir.to_str().unwrap()));

        fs::write(temp_dir.join("Cargo.toml"), "[package]\n").unwrap();
        assert!(BuildDetector::is_buildable(temp_dir.to_str().unwrap()));

        fs::remove_dir_all(temp_dir).ok();
    }
}
