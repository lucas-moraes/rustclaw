//! Microphone capture via cpal (feature `voice`).
//!
//! On macOS, CoreAudio/HAL aborts the process when AudioUnits are created or
//! destroyed on arbitrary threads (e.g. tokio worker threads). To avoid this,
//! the entire stream lifecycle (open → play → pause → drop) runs on a single
//! dedicated thread spawned by [`Recorder::start`]; callers only talk to it
//! through channels.

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

/// Maximum recording length (payload/cost guard).
pub const MAX_RECORDING: Duration = Duration::from_secs(60);

/// Push-to-talk recorder: accumulates mono f32 samples from the default
/// input device until [`Recorder::stop`] is called.
///
/// The cpal stream lives on a dedicated thread; `stop()` signals it and joins.
pub struct Recorder {
    /// Commands the capture thread: `true` = stop and report samples.
    stop_tx: Option<std_mpsc::Sender<bool>>,
    /// Result of the capture thread (samples + rate), set on stop.
    result_rx: std_mpsc::Receiver<(Vec<f32>, u32)>,
    /// Live input level (RMS, 0..=1000), updated by the capture callback.
    level: std::sync::Arc<std::sync::atomic::AtomicU32>,
    started_at: Instant,
}

impl Recorder {
    /// Opens the default input device and starts capturing.
    pub fn start() -> Result<Self> {
        // Channel for the capture thread to report back (stop result).
        let (result_tx, result_rx) = std_mpsc::channel::<(Vec<f32>, u32)>();
        let (stop_tx, stop_rx) = std_mpsc::channel::<bool>();
        let level = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let level_thread = std::sync::Arc::clone(&level);

        // Spawn the dedicated capture thread. All CoreAudio work happens here.
        let handle = std::thread::Builder::new()
            .name("rustclaw-voice-capture".into())
            .spawn(move || {
                let result = run_capture(&stop_rx, level_thread);
                // Report samples (or empty on error); then the thread ends,
                // dropping the stream on *this* thread.
                let _ = result_tx.send(result);
            })
            .context("voice: failed to spawn capture thread")?;

        // Wait briefly for the stream to come up: the capture thread sends an
        // early error via the result channel if device/stream setup fails.
        // We detect failure by polling a readiness flag through a second
        // channel piggybacked on `result_tx`? Simpler: setup errors are sent
        // as an empty sample list with rate 0.
        // Give the thread a moment; if it dies immediately (no device), the
        // result arrives with rate 0 and we surface the error.
        std::thread::sleep(Duration::from_millis(150));
        if handle.is_finished() {
            // Thread already exited: setup failed.
            let (_, rate) = result_rx.try_recv().unwrap_or((Vec::new(), 0));
            if rate == 0 {
                anyhow::bail!("voice: capture setup failed (no microphone or stream error)");
            }
            return Ok(Self {
                stop_tx: None,
                result_rx,
                level,
                started_at: Instant::now(),
            });
        }

        Ok(Self {
            stop_tx: Some(stop_tx),
            result_rx,
            level,
            started_at: Instant::now(),
        })
    }

    /// Stops capture and returns `(samples, sample_rate)`. Synchronous and fast;
    /// only the HTTP transcription call is async.
    pub fn stop(mut self) -> (Vec<f32>, u32) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(true);
        }

        // Join the thread so the stream drop is observed before returning.
        self.result_rx
            .recv()
            .unwrap_or((Vec::new(), self.sample_rate_hint()))
    }

    fn sample_rate_hint(&self) -> u32 {
        16_000
    }

    /// Current input level (RMS scaled to 0..=1000). Cheap: one atomic load,
    /// safe to call every frame from the UI loop.
    pub fn level(&self) -> u32 {
        self.level.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Duration since recording started.
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Whether the recording hit the max length (caller should auto-stop).
    pub fn exceeded_max(&self) -> bool {
        self.elapsed() >= MAX_RECORDING
    }
}

/// Scales an RMS value (0..1) to the 0..=1000 level reported to the UI.
fn rms_to_level(rms: f32) -> u32 {
    // Perceptual boost: sqrt makes quiet speech visible on the wave.
    (rms.sqrt().clamp(0.0, 1.0) * 1000.0) as u32
}

/// Runs the capture loop on the dedicated thread. Returns `(samples, rate)`;
/// rate 0 signals a setup failure.
fn run_capture(
    stop_rx: &std_mpsc::Receiver<bool>,
    level: std::sync::Arc<std::sync::atomic::AtomicU32>,
) -> (Vec<f32>, u32) {
    let device = match cpal::default_host().default_input_device() {
        Some(d) => d,
        None => return (Vec::new(), 0),
    };
    let supported = match device.default_input_config() {
        Ok(c) => c,
        Err(_) => return (Vec::new(), 0),
    };
    let sample_rate = supported.sample_rate().0;
    let config: cpal::StreamConfig = supported.clone().into();
    let buffer: std::sync::Arc<std::sync::Mutex<Vec<f32>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let err_fn = |err| eprintln!("voice: capture error: {err}");
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => {
            let buf = std::sync::Arc::clone(&buffer);
            device.build_input_stream(
                &config,
                move |data: &[f32], _| {
                    if let Ok(mut b) = buf.lock() {
                        b.extend_from_slice(data);
                    }
                    let rms =
                        (data.iter().map(|s| s * s).sum::<f32>() / data.len().max(1) as f32).sqrt();
                    level.store(rms_to_level(rms), std::sync::atomic::Ordering::Relaxed);
                },
                err_fn,
                None,
            )
        }
        cpal::SampleFormat::I16 => {
            let buf = std::sync::Arc::clone(&buffer);
            device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    let f: Vec<f32> = data.iter().map(|s| *s as f32 / i16::MAX as f32).collect();
                    if let Ok(mut b) = buf.lock() {
                        b.extend_from_slice(&f);
                    }
                    let rms = (f.iter().map(|s| s * s).sum::<f32>() / f.len().max(1) as f32).sqrt();
                    level.store(rms_to_level(rms), std::sync::atomic::Ordering::Relaxed);
                },
                err_fn,
                None,
            )
        }
        cpal::SampleFormat::U16 => {
            let buf = std::sync::Arc::clone(&buffer);
            device.build_input_stream(
                &config,
                move |data: &[u16], _| {
                    let f: Vec<f32> = data
                        .iter()
                        .map(|s| (*s as f32 - u16::MAX as f32 / 2.0) / 32768.0)
                        .collect();
                    if let Ok(mut b) = buf.lock() {
                        b.extend_from_slice(&f);
                    }
                    let rms = (f.iter().map(|s| s * s).sum::<f32>() / f.len().max(1) as f32).sqrt();
                    level.store(rms_to_level(rms), std::sync::atomic::Ordering::Relaxed);
                },
                err_fn,
                None,
            )
        }
        _other => return (Vec::new(), 0),
    };
    let stream = match stream {
        Ok(s) => s,
        Err(_) => return (Vec::new(), 0),
    };
    if stream.play().is_err() {
        return (Vec::new(), 0);
    }

    // Wait for the stop signal, then pause + drop the stream on this thread.
    let _ = stop_rx.recv();
    let _ = stream.pause();
    let samples = buffer
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    // Give CoreAudio a moment to finish any in-flight callback before the
    // stream (and its closure) are freed.
    std::thread::sleep(Duration::from_millis(20));
    (samples, sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_recording_constant() {
        assert_eq!(MAX_RECORDING.as_secs(), 60);
    }

    #[test]
    fn test_recorder_start_fails_gracefully_without_device() {
        // In CI/headless there is no input device: must error, not panic.
        let result = Recorder::start();
        if let Err(e) = result {
            assert!(e.to_string().contains("voice:"));
        }
        // If a device exists (local dev), start() succeeding is fine.
    }
}

#[cfg(test)]
mod repro_tests {
    use super::*;

    #[test]
    #[ignore]
    fn repro_start_stop() {
        let r = Recorder::start().expect("start");
        std::thread::sleep(Duration::from_secs(2));
        let (samples, rate) = r.stop();
        println!("stopped ok: {} samples @ {}", samples.len(), rate);
        std::thread::sleep(Duration::from_secs(1));
        println!("still alive after drop");
    }

    /// Reproduces the TUI scenario: start/stop from a non-main thread
    /// (simulating a tokio worker). The dedicated capture thread keeps all
    /// CoreAudio work off the caller's thread.
    #[test]
    #[ignore]
    fn repro_start_stop_off_main_thread() {
        let handle = std::thread::spawn(|| {
            let r = Recorder::start().expect("start");
            std::thread::sleep(Duration::from_secs(2));
            let (samples, rate) = r.stop();
            println!("stopped ok: {} samples @ {}", samples.len(), rate);
        });
        handle.join().expect("worker thread must not abort");
        std::thread::sleep(Duration::from_secs(1));
        println!("still alive after drop");
    }
}
