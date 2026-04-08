use crate::utils::colors::Colors;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::time::{interval, Duration};

pub struct Spinner {
    frames: Vec<String>,
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
        let frames = vec![
            format!("{}{}⠋{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠙{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠹{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠸{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠼{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠴{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠦{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠧{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠇{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠏{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
        ];
        Self {
            frames,
            message: "Thinking".to_string(),
            interval_ms: 100,
        }
    }

    #[allow(dead_code)]
    pub fn with_message(message: impl Into<String>) -> Self {
        let frames = vec![
            format!("{}{}⠋{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠙{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠹{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠸{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠼{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠴{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠦{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠧{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠇{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
            format!("{}{}⠏{}", Colors::BOLD, Colors::AMBER, Colors::RESET),
        ];
        Self {
            frames,
            message: message.into(),
            interval_ms: 100,
        }
    }

    pub async fn run<F, T>(self, future: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();
        let interval_ms = self.interval_ms;
        let frames = self.frames;
        let message = self.message;

        let spinner_task = tokio::spawn(async move {
            let mut ticker = interval(Duration::from_millis(interval_ms));
            let mut frame_idx = 0;

            while running_clone.load(Ordering::Relaxed) {
                ticker.tick().await;

                let frame = &frames[frame_idx % frames.len()];
                frame_idx += 1;

                print!(
                    "{}{}{} {}...{}",
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
