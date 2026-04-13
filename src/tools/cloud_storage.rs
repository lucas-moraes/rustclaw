use super::Tool;
use serde_json::Value;
use std::process::Command;

pub struct S3Tool;

impl S3Tool {
    pub fn new() -> Self {
        Self
    }

    fn run_aws_command(&self, args: &[&str]) -> Result<String, String> {
        let output = Command::new("aws")
            .args(args)
            .output()
            .map_err(|e| format!("Failed to run aws CLI: {}", e))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).to_string())
        }
    }
}

#[async_trait::async_trait]
impl Tool for S3Tool {
    fn name(&self) -> &str {
        "s3"
    }

    fn description(&self) -> &str {
        "Interact with S3-compatible storage. Actions: ls, cp, mv, rm, sync. Input: { \"action\": \"ls\", \"bucket\": \"my-bucket\" }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let action = args["action"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'action' é obrigatório".to_string())?;

        match action {
            "ls" => {
                let bucket = args["bucket"].as_str().ok_or("'bucket' é obrigatório")?;
                let prefix = args["prefix"].as_str().unwrap_or("");
                self.run_aws_command(&["s3", "ls", &format!("s3://{}/{}", bucket, prefix)])
            }
            "cp" => {
                let source = args["source"].as_str().ok_or("'source' é obrigatório")?;
                let dest = args["dest"].as_str().ok_or("'dest' é obrigatório")?;
                self.run_aws_command(&["s3", "cp", source, dest])
            }
            "mv" => {
                let source = args["source"].as_str().ok_or("'source' é obrigatório")?;
                let dest = args["dest"].as_str().ok_or("'dest' é obrigatório")?;
                self.run_aws_command(&["s3", "mv", source, dest])
            }
            "rm" => {
                let path = args["path"].as_str().ok_or("'path' é obrigatório")?;
                self.run_aws_command(&["s3", "rm", path])
            }
            "sync" => {
                let source = args["source"].as_str().ok_or("'source' é obrigatório")?;
                let dest = args["dest"].as_str().ok_or("'dest' é obrigatório")?;
                let delete_flag = if args["delete"].as_bool().unwrap_or(false) {
                    vec!["--delete"]
                } else {
                    vec![]
                };
                let mut cmd_args = vec!["s3", "sync", source, dest];
                cmd_args.extend(delete_flag);
                self.run_aws_command(&cmd_args)
            }
            "mb" => {
                let bucket = args["bucket"].as_str().ok_or("'bucket' é obrigatório")?;
                self.run_aws_command(&["s3", "mb", &format!("s3://{}", bucket)])
            }
            "rb" => {
                let bucket = args["bucket"].as_str().ok_or("'bucket' é obrigatório")?;
                self.run_aws_command(&["s3", "rb", &format!("s3://{}", bucket)])
            }
            _ => Err(format!("Unknown action: {}. Use: ls, cp, mv, rm, sync, mb, rb", action)),
        }
    }
}

pub struct GcsTool;

impl GcsTool {
    pub fn new() -> Self {
        Self
    }

    fn run_gcloud_command(&self, args: &[&str]) -> Result<String, String> {
        let output = Command::new("gsutil")
            .args(args)
            .output()
            .map_err(|e| format!("Failed to run gsutil: {}", e))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).to_string())
        }
    }
}

#[async_trait::async_trait]
impl Tool for GcsTool {
    fn name(&self) -> &str {
        "gcs"
    }

    fn description(&self) -> &str {
        "Interact with Google Cloud Storage. Actions: ls, cp, mv, rm. Input: { \"action\": \"ls\", \"bucket\": \"my-bucket\" }"
    }

    async fn call(&self, args: Value) -> Result<String, String> {
        let action = args["action"]
            .as_str()
            .ok_or_else(|| "Parâmetro 'action' é obrigatório".to_string())?;

        match action {
            "ls" => {
                let bucket = args["bucket"].as_str().ok_or("'bucket' é obrigatório")?;
                let prefix = args["prefix"].as_str().unwrap_or("");
                self.run_gcloud_command(&["ls", &format!("gs://{}/{}", bucket, prefix)])
            }
            "cp" => {
                let source = args["source"].as_str().ok_or("'source' é obrigatório")?;
                let dest = args["dest"].as_str().ok_or("'dest' é obrigatório")?;
                self.run_gcloud_command(&["cp", source, dest])
            }
            "mv" => {
                let source = args["source"].as_str().ok_or("'source' é obrigatório")?;
                let dest = args["dest"].as_str().ok_or("'dest' é obrigatório")?;
                self.run_gcloud_command(&["mv", source, dest])
            }
            "rm" => {
                let path = args["path"].as_str().ok_or("'path' é obrigatório")?;
                self.run_gcloud_command(&["rm", path])
            }
            "sync" => {
                let source = args["source"].as_str().ok_or("'source' é obrigatório")?;
                let dest = args["dest"].as_str().ok_or("'dest' é obrigatório")?;
                self.run_gcloud_command(&["rsync", "-r", source, dest])
            }
            "mb" => {
                let bucket = args["bucket"].as_str().ok_or("'bucket' é obrigatório")?;
                self.run_gcloud_command(&["mb", &format!("gs://{}", bucket)])
            }
            "rb" => {
                let bucket = args["bucket"].as_str().ok_or("'bucket' é obrigatório")?;
                self.run_gcloud_command(&["rb", &format!("gs://{}", bucket)])
            }
            _ => Err(format!("Unknown action: {}. Use: ls, cp, mv, rm, sync, mb, rb", action)),
        }
    }
}
