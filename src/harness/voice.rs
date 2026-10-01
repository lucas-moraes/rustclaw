//! Speech-to-text (push-to-talk) support.
//!
//! Self-contained module: audio capture ([`Recorder`], behind the `voice`
//! cargo feature), WAV encoding, and transcription via the DeepInfra
//! whisper endpoint. The TUI only touches [`Recorder`] and [`transcribe`];
//! it knows nothing about cpal, WAV or HTTP.

use anyhow::{Context, Result};
use base64::Engine;

#[cfg(feature = "voice")]
mod recorder;
#[cfg(feature = "voice")]
pub use recorder::Recorder;

pub use crate::config::DEFAULT_STT_MODEL;

/// DeepInfra inference endpoint for a given model.
pub fn stt_endpoint(model: &str) -> String {
    format!("https://api.deepinfra.com/v1/inference/{model}")
}

/// Encodes mono `f32` samples as a 16-bit PCM WAV file in memory.
pub fn encode_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    let data_len = (samples.len() * 2) as u32;
    // RIFF header (44 bytes), little-endian.
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        out.extend_from_slice(&((clamped * 32767.0) as i16).to_le_bytes());
    }
    out
}

/// Linear resampler: `from_hz` -> `to_hz` (good enough for speech).
pub fn resample(samples: &[f32], from_hz: u32, to_hz: u32) -> Vec<f32> {
    if from_hz == to_hz || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = from_hz as f64 / to_hz as f64;
    let out_len = ((samples.len() as f64) / ratio).floor() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let i0 = pos.floor() as usize;
        let i1 = (i0 + 1).min(samples.len() - 1);
        let frac = (pos - pos.floor()) as f32;
        out.push(samples[i0] * (1.0 - frac) + samples[i1] * frac);
    }
    out
}

/// Transcribes a WAV file via the DeepInfra whisper endpoint.
pub async fn transcribe(
    http: &reqwest::Client,
    api_key: &str,
    wav: &[u8],
    model: &str,
) -> Result<String> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(wav);
    let body = serde_json::json!({
        "audio": format!("data:audio/wav;base64,{b64}"),
    });
    let resp = http
        .post(stt_endpoint(model))
        .bearer_auth(api_key)
        .timeout(std::time::Duration::from_secs(30))
        .json(&body)
        .send()
        .await
        .context("voice: failed to reach transcription endpoint")?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .context("voice: failed to read transcription response")?;
    if !status.is_success() {
        let hint = match status.as_u16() {
            401 => " (missing or invalid token — run /auth to set the deepinfra key)",
            429 => " (rate limited — try again in a moment)",
            _ => "",
        };
        anyhow::bail!("voice: transcription failed with HTTP {status}{hint}: {text}");
    }
    let parsed: serde_json::Value =
        serde_json::from_str(&text).context("voice: invalid JSON from transcription endpoint")?;
    parsed["text"]
        .as_str()
        .map(|s| s.trim().to_string())
        .context("voice: transcription response missing 'text' field")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_wav_header_and_size() {
        let samples = vec![0.0f32, 0.5, -0.5, 1.0];
        let wav = encode_wav(&samples, 16_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        // 44-byte header + 4 samples * 2 bytes.
        assert_eq!(wav.len(), 44 + 8);
        // RIFF size = 36 + data_len.
        let riff = u32::from_le_bytes(wav[4..8].try_into().unwrap());
        assert_eq!(riff, 36 + 8);
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap());
        assert_eq!(data_len, 8);
    }

    #[test]
    fn test_encode_wav_sample_roundtrip() {
        let samples = vec![0.0f32, 0.5, -0.5, 1.0, -1.0];
        let wav = encode_wav(&samples, 16_000);
        for (i, s) in samples.iter().enumerate() {
            let off = 44 + i * 2;
            let v = i16::from_le_bytes(wav[off..off + 2].try_into().unwrap());
            let back = v as f32 / 32767.0;
            assert!((back - s).abs() < 0.001, "sample {i}: {back} != {s}");
        }
    }

    #[test]
    fn test_encode_wav_clamps() {
        let wav = encode_wav(&[2.0f32, -2.0], 16_000);
        let v0 = i16::from_le_bytes(wav[44..46].try_into().unwrap());
        let v1 = i16::from_le_bytes(wav[46..48].try_into().unwrap());
        assert_eq!(v0, 32767);
        assert_eq!(v1, -32767);
    }

    #[test]
    fn test_resample_identity_and_downsample() {
        let s = vec![0.0f32, 1.0, 0.0, -1.0];
        assert_eq!(resample(&s, 16_000, 16_000), s);
        let down = resample(&s, 48_000, 16_000);
        assert_eq!(down.len(), s.len() / 3);
        // First sample preserved.
        assert!((down[0] - s[0]).abs() < 1e-6);
    }

    #[test]
    fn test_parse_transcription_response() {
        let body = serde_json::json!({
            "text": "  olá mundo  ",
            "segments": [],
            "words": []
        });
        let text = body["text"].as_str().unwrap().trim().to_string();
        assert_eq!(text, "olá mundo");
    }

    #[test]
    fn test_endpoint_url() {
        assert_eq!(
            stt_endpoint(DEFAULT_STT_MODEL),
            "https://api.deepinfra.com/v1/inference/openai/whisper-large-v3"
        );
    }

    #[test]
    fn test_error_hints() {
        let status = reqwest::StatusCode::UNAUTHORIZED;
        let hint = match status.as_u16() {
            401 => " (missing or invalid token — run /auth to set the deepinfra key)",
            429 => " (rate limited — try again in a moment)",
            _ => "",
        };
        assert!(hint.contains("/auth"));
        let status = reqwest::StatusCode::TOO_MANY_REQUESTS;
        let hint = match status.as_u16() {
            401 => " (missing or invalid token — run /auth to set the deepinfra key)",
            429 => " (rate limited — try again in a moment)",
            _ => "",
        };
        assert!(hint.contains("rate limited"));
    }

    /// Real call against DeepInfra using the token from the auth store.
    /// Run: RUSTCLAW_HOME="$HOME/Library/Application Support/rustclaw" \
    ///      cargo test smoke_voice_transcribe -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn smoke_voice_transcribe() {
        let store = crate::harness::auth::AuthStore::load();
        let key = store
            .get_key("deepinfra")
            .expect("no deepinfra token in auth store (run /auth)");
        // 0.5s of silence at 16 kHz — should transcribe to empty text.
        let samples = vec![0.0f32; 8_000];
        let wav = encode_wav(&samples, 16_000);
        let http = reqwest::Client::new();
        let text = transcribe(&http, &key, &wav, DEFAULT_STT_MODEL)
            .await
            .expect("transcription failed");
        println!("transcribed: {text:?}");
    }
}
