use crate::utils::colors::Colors;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::{interval, Duration};

pub struct Spinner {
    frame_bold: String,
    frame_normal: String,
    message: String,
    interval_ms: u64,
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

impl Spinner {
    pub fn new() -> Self {
        Self {
            frame_bold: format!("{}{} {}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            frame_normal: format!("{} ", Colors::AMBER),
            message: "Thinking".to_string(),
            interval_ms: 500,
        }
    }

    #[allow(dead_code)]
    pub fn with_message(message: impl Into<String>) -> Self {
        Self {
            frame_bold: format!("{}{} {}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            frame_normal: format!("{} ", Colors::AMBER),
            message: message.into(),
            interval_ms: 500,
        }
    }

    pub async fn run<F, T>(self, future: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();
        let interval_ms = self.interval_ms;
        let frame_bold = self.frame_bold.clone();
        let frame_normal = self.frame_normal.clone();
        let message = self.message.clone();

        let spinner_task = tokio::spawn(async move {
            let mut ticker = interval(Duration::from_millis(interval_ms));
            let mut bold = true;

            while running_clone.load(Ordering::Relaxed) {
                ticker.tick().await;

                let frame = if bold { &frame_bold } else { &frame_normal };
                bold = !bold;

                print!(
                    "{}{}{}{}...{}\x1b[0m",
                    Colors::CLEAR_LINE,
                    Colors::DIM,
                    frame,
                    message,
                    Colors::RESET
                );
                std::io::stdout().flush().unwrap();
            }
        });

        let result = future.await;

        running.store(false, Ordering::Relaxed);
        let _ = spinner_task.await;

        print!("{}", Colors::CLEAR_LINE);
        std::io::stdout().flush().unwrap();

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_spinner_runs() {
        let spinner = Spinner::new();
        let result = spinner
            .run(async {
                sleep(Duration::from_millis(500)).await;
                42
            })
            .await;
        assert_eq!(result, 42);
    }
}
