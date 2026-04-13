use super::Tool;
use serde_json::Value;
use std::process::Command;

pub struct DockerTool;

impl DockerTool {
    pub fn new() -> Self {
        Self
    }

    fn run_docker_command(&self, args: &[&str]) -> Result<String, String> {
        let output = Command::new("docker")
            .args(args)
            .output()
            .map_err(|e| format!("Failed to run docker: {}", e))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(stderr)
        }
    }
}

#[async_trait::async_trait]
impl Tool for DockerTool {
    fn name(&self) -> &str {
        "docker"
    }

    fn description(&self) -> &str {
        "Execute Docker commands. Actions: status, ps, images, build, run, stop, rm. Input: { \"action\": \"ps\", \"args\": {} }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let action = args["action"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'action' é obrigatório".to_string())?;

        match action {
            "status" => {
                let output = Command::new("docker")
                    .args(["info"])
                    .output()
                    .map_err(|e| e.to_string())?;

                if output.status.success() {
                    Ok("Docker is running".to_string())
                } else {
                    Err("Docker is not running".to_string())
                }
            }
            "ps" => {
                self.run_docker_command(&["ps", "--format", "table {{.ID}}\t{{.Image}}\t{{.Status}}\t{{.Names}}"])
            }
            "images" => {
                self.run_docker_command(&["images", "--format", "table {{.Repository}}\t{{.Tag}}\t{{.Size}}"])
            }
            "build" => {
                let path = args["path"].as_str().unwrap_or(".");
                let tag = args["tag"].as_str().unwrap_or("latest");
                self.run_docker_command(&["build", path, "-t", tag])
            }
            "run" => {
                let image = args["image"].as_str().ok_or("'image' é obrigatório")?;
                let container_name = args["name"].as_str();
                
                let mut cmd_args = vec!["run", "-d"];
                if let Some(name) = container_name {
                    cmd_args.push("--name");
                    cmd_args.push(name);
                }
                cmd_args.push(image);
                
                self.run_docker_command(&cmd_args.iter().map(|s| *s).collect::<Vec<_>>())
            }
            "stop" => {
                let container = args["container"].as_str().ok_or("'container' é obrigatório")?;
                self.run_docker_command(&["stop", container])
            }
            "rm" => {
                let container = args["container"].as_str().ok_or("'container' é obrigatório")?;
                self.run_docker_command(&["rm", "-f", container])
            }
            "logs" => {
                let container = args["container"].as_str().ok_or("'container' é obrigatório")?;
                let lines = args["lines"].as_u64().unwrap_or(100);
                self.run_docker_command(&["logs", "--tail", &lines.to_string(), container])
            }
            "exec" => {
                let container = args["container"].as_str().ok_or("'container' é obrigatório")?;
                let cmd = args["command"].as_str().ok_or("'command' é obrigatório")?;
                self.run_docker_command(&["exec", container, "sh", "-c", cmd])
            }
            _ => Err(format!("Unknown action: {}. Use: status, ps, images, build, run, stop, rm, logs, exec", action)),
        }
    }
}

pub struct DockerComposeTool;

impl DockerComposeTool {
    pub fn new() -> Self {
        Self
    }

    fn run_compose_command(&self, args: &[&str], dir: Option<&str>) -> Result<String, String> {
        let mut cmd = Command::new("docker-compose");
        if let Some(d) = dir {
            cmd.current_dir(d);
        }
        let output = cmd.args(args)
            .output()
            .map_err(|e| format!("Failed to run docker-compose: {}", e))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).to_string())
        }
    }
}

#[async_trait::async_trait]
impl Tool for DockerComposeTool {
    fn name(&self) -> &str {
        "docker_compose"
    }

    fn description(&self) -> &str {
        "Execute Docker Compose commands. Actions: up, down, ps, logs, build. Input: { \"action\": \"up\", \"dir\": \".\" }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let action = args["action"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'action' é obrigatório".to_string())?;

        let dir = args["dir"].as_str();

        match action {
            "up" => {
                let detached = args["detached"].as_bool().unwrap_or(true);
                let mut cmd_args = vec!["up"];
                if detached {
                    cmd_args.push("-d");
                }
                if args["build"].as_bool().unwrap_or(false) {
                    cmd_args.push("--build");
                }
                self.run_compose_command(&cmd_args, dir)
            }
            "down" => {
                self.run_compose_command(&["down"], dir)
            }
            "ps" => {
                self.run_compose_command(&["ps"], dir)
            }
            "logs" => {
                let lines = args["lines"].as_u64().unwrap_or(100);
                let service = args["service"].as_str();
                
                let output = if let Some(svc) = service {
                    self.run_compose_command(&["logs", "--tail", &lines.to_string(), svc], dir)
                } else {
                    self.run_compose_command(&["logs", "--tail", &lines.to_string()], dir)
                }?;
                Ok(output)
            }
            "build" => {
                let no_cache = args["no_cache"].as_bool().unwrap_or(false);
                let mut cmd_args = vec!["build"];
                if no_cache {
                    cmd_args.push("--no-cache");
                }
                self.run_compose_command(&cmd_args, dir)
            }
            "pull" => {
                self.run_compose_command(&["pull"], dir)
            }
            "restart" => {
                let service = args["service"].as_str();
                let mut cmd_args = vec!["restart"];
                if let Some(s) = service {
                    cmd_args.push(s);
                }
                self.run_compose_command(&cmd_args, dir)
            }
            _ => Err(format!("Unknown action: {}. Use: up, down, ps, logs, build, pull, restart", action)),
        }
    }
}
