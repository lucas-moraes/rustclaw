use crate::error::{AgentError, ToolError};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisionMessage {
    pub image_data: String,
    pub mime_type: String,
}

pub struct VisionTool;

impl VisionTool {
    pub fn new() -> Self {
        Self
    }

    pub fn capture_screen(&self) -> Result<VisionMessage, AgentError> {
        #[cfg(target_os = "macos")]
        {
            let output = Command::new("screencapture")
                .args(["-x", "-"])
                .output()
                .map_err(|e| {
                    AgentError::Tool(ToolError::ExecutionFailed(format!(
                        "Failed to capture screen: {}",
                        e
                    )))
                })?;

            if !output.status.success() {
                return Err(AgentError::Tool(ToolError::ExecutionFailed(
                    "screencapture failed".to_string(),
                )));
            }

            let image_data = STANDARD.encode(&output.stdout);
            Ok(VisionMessage {
                image_data,
                mime_type: "image/png".to_string(),
            })
        }

        #[cfg(target_os = "linux")]
        {
            let output = Command::new("gnome-screenshot")
                .args(["-f", "/tmp/screenshot.png"])
                .output()
                .map_err(|e| {
                    AgentError::Tool(ToolError::ExecutionFailed(format!(
                        "Failed to capture screen: {}",
                        e
                    )))
                })?;

            if !output.status.success() {
                return Err(AgentError::Tool(ToolError::ExecutionFailed(
                    "gnome-screenshot failed".to_string(),
                )));
            }

            let image_bytes = std::fs::read("/tmp/screenshot.png").map_err(|e| {
                AgentError::Tool(ToolError::ExecutionFailed(format!(
                    "Failed to read screenshot: {}",
                    e
                )))
            })?;

            let image_data = STANDARD.encode(&image_bytes);
            Ok(VisionMessage {
                image_data,
                mime_type: "image/png".to_string(),
            })
        }

        #[cfg(target_os = "windows")]
        {
            let output = Command::new("powershell")
                .args([
                    "-Command",
                    "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Screen]::PrimaryScreen | ForEach-Object { $_.Bounds.Width }; [System.Windows.Forms.Screen]::CaptureAreaSave('png', [System.IO.Path]::GetTempPath() + 'screenshot.png')"
                ])
                .output()
                .map_err(|e| AgentError::Tool(ToolError::ExecutionFailed(format!(
                    "Failed to capture screen: {}",
                    e
                ))))?;

            if !output.status.success() {
                return Err(AgentError::Tool(ToolError::ExecutionFailed(
                    "PowerShell screen capture failed".to_string(),
                )));
            }

            let image_path = std::env::temp_dir().join("screenshot.png");
            let image_bytes = std::fs::read(&image_path).map_err(|e| {
                AgentError::Tool(ToolError::ExecutionFailed(format!(
                    "Failed to read screenshot: {}",
                    e
                )))
            })?;

            let image_data = STANDARD.encode(&image_bytes);
            Ok(VisionMessage {
                image_data,
                mime_type: "image/png".to_string(),
            })
        }
    }

    pub fn load_image_from_path<P: AsRef<Path>>(
        &self,
        path: P,
    ) -> Result<VisionMessage, AgentError> {
        let path = path.as_ref();

        if !path.exists() {
            return Err(AgentError::Tool(ToolError::ExecutionFailed(format!(
                "Image file not found: {:?}",
                path
            ))));
        }

        let mime_type = match path.extension().and_then(|e| e.to_str()) {
            Some("png") => "image/png",
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            Some("bmp") => "image/bmp",
            _ => "image/png",
        };

        let image_bytes = std::fs::read(path).map_err(|e| {
            AgentError::Tool(ToolError::ExecutionFailed(format!(
                "Failed to read image: {}",
                e
            )))
        })?;

        let image_data = STANDARD.encode(&image_bytes);

        Ok(VisionMessage {
            image_data,
            mime_type: mime_type.to_string(),
        })
    }

    pub fn build_multimodal_content(
        text: &str,
        images: Vec<VisionMessage>,
    ) -> Vec<serde_json::Value> {
        let mut content = Vec::new();

        if !text.is_empty() {
            content.push(serde_json::json!({
                "type": "text",
                "text": text
            }));
        }

        for image in images {
            content.push(serde_json::json!({
                "type": "image_url",
                "image_url": {
                    "url": format!("data:{};base64,{}", image.mime_type, image.image_data)
                }
            }));
        }

        content
    }
}

impl Default for VisionTool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_multimodal_content_with_text() {
        let text = "What do you see in this image?";
        let content = VisionTool::build_multimodal_content(text, vec![]);

        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], text);
    }

    #[test]
    fn test_build_multimodal_content_with_image() {
        let text = "Describe this";
        let images = vec![VisionMessage {
            image_data: "abc123".to_string(),
            mime_type: "image/png".to_string(),
        }];
        let content = VisionTool::build_multimodal_content(text, images);

        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert!(content[1]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }

    #[test]
    fn test_load_image_from_path_missing() {
        let tool = VisionTool::new();
        let result = tool.load_image_from_path("/nonexistent/image.png");
        assert!(result.is_err());
    }
}
