use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::{interval, Duration};

/// Spinner animation for CLI
pub struct Spinner {
    frames: Vec<&'static str>,
    message: String,
    color_code: String,
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

impl Spinner {
    /// Create a new spinner with default settings
    pub fn new() -> Self {
        Self {
            frames: vec!["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            message: "Pensando".to_string(),
            color_code: "\x1b[34m".to_string(), // Blue
        }
    }

    /// Create a spinner with custom message
    #[allow(dead_code)]
    pub fn with_message(message: impl Into<String>) -> Self {
        Self {
            frames: vec!["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            message: message.into(),
            color_code: "\x1b[34m".to_string(),
        }
    }

    /// Create a spinner with custom color
    #[allow(dead_code)]
    pub fn with_color(color: SpinnerColor) -> Self {
        Self {
            frames: vec!["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            message: "Pensando".to_string(),
            color_code: color.to_ansi_code(),
        }
    }

    /// Run the spinner animation while executing the given future
    pub async fn run<F, T>(self, future: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();

        // Spinner task
        let spinner_task = tokio::spawn(async move {
            let mut interval = interval(Duration::from_millis(100));
            let mut frame_idx = 0;

            while running_clone.load(Ordering::Relaxed) {
                interval.tick().await;

                let frame = self.frames[frame_idx % self.frames.len()];
                print!(
                    "\r\x1b[K{}{} {}...\x1b[0m",
                    self.color_code, frame, self.message
                );
                std::io::stdout().flush().unwrap();

                frame_idx += 1;
            }
        });

        // Execute the future
        let result = future.await;

        // Stop the spinner
        running.store(false, Ordering::Relaxed);
        let _ = spinner_task.await;

        // Clear the line
        print!("\r\x1b[K");
        std::io::stdout().flush().unwrap();

        result
    }
}

/// Available colors for the spinner
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum SpinnerColor {
    Blue,
    Green,
    Yellow,
    Red,
    Magenta,
    Cyan,
    White,
}

impl SpinnerColor {
    #[allow(dead_code)]
    fn to_ansi_code(&self) -> String {
        match self {
            SpinnerColor::Blue => "\x1b[34m",
            SpinnerColor::Green => "\x1b[32m",
            SpinnerColor::Yellow => "\x1b[33m",
            SpinnerColor::Red => "\x1b[31m",
            SpinnerColor::Magenta => "\x1b[35m",
            SpinnerColor::Cyan => "\x1b[36m",
            SpinnerColor::White => "\x1b[37m",
        }
        .to_string()
    }
}

/// Convenience function to run a future with a spinner
#[allow(dead_code)]
pub async fn with_spinner<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    Spinner::new().run(future).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_spinner_runs() {
        let spinner = Spinner::new();
        let result = spinner.run(async {
            sleep(Duration::from_millis(500)).await;
            42
        }).await;
        
        assert_eq!(result, 42);
    }

    #[tokio::test]
    async fn test_spinner_with_message() {
        let spinner = Spinner::with_message("Processando");
        let result = spinner.run(async {
            sleep(Duration::from_millis(300)).await;
            "done"
        }).await;
        
        assert_eq!(result, "done");
    }
}
