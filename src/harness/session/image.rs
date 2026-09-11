//! Image attachment support for multimodal (vision) requests.
//!
//! Reads an image file from disk and encodes it as base64 with a media type
//! detected from the file extension. Providers that support vision convert
//! `Part::Image` into the appropriate content block; unsupported providers
//! degrade to a text placeholder.

use base64::Engine as _;

/// Supported image media types (Anthropic/OpenAI vision subset).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageMediaType {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl ImageMediaType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ImageMediaType::Png => "image/png",
            ImageMediaType::Jpeg => "image/jpeg",
            ImageMediaType::Gif => "image/gif",
            ImageMediaType::Webp => "image/webp",
        }
    }

    /// Detects the media type from a file extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "png" => Some(ImageMediaType::Png),
            "jpg" | "jpeg" => Some(ImageMediaType::Jpeg),
            "gif" => Some(ImageMediaType::Gif),
            "webp" => Some(ImageMediaType::Webp),
            _ => None,
        }
    }
}

/// A decoded image ready to be embedded in a provider request.
#[derive(Clone, Debug)]
pub struct ImageData {
    pub media_type: &'static str,
    /// Base64-encoded file contents.
    pub data: String,
}

/// Loads and encodes an image from disk. Returns a human-readable error when
/// the file does not exist or the extension is unsupported — callers degrade
/// to a text block instead of failing the request.
pub fn load_image(path: &str) -> Result<ImageData, String> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let media = ImageMediaType::from_extension(ext)
        .ok_or_else(|| format!("unsupported image extension: .{} (png/jpeg/gif/webp)", ext))?;
    let bytes = std::fs::read(path).map_err(|e| format!("failed to read image {}: {}", path, e))?;
    Ok(ImageData {
        media_type: media.as_str(),
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

/// Text placeholder used when an image cannot be sent to the model.
pub fn image_fallback_text(path: &str, reason: &str) -> String {
    format!("[image: {} — {}]", path, reason)
}

/// Directory used for pasted screenshots and other ephemeral image attachments.
pub fn attachments_dir() -> std::path::PathBuf {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("rustclaw")
        .join("attachments");
    let _ = std::fs::create_dir_all(&base);
    base
}

/// Writes raw PNG bytes under the attachments dir and returns the absolute path.
pub fn save_png_bytes(bytes: &[u8], stem: &str) -> Result<std::path::PathBuf, String> {
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S-%3f");
    let name = format!("{stem}-{ts}.png");
    let path = attachments_dir().join(name);
    std::fs::write(&path, bytes)
        .map_err(|e| format!("failed to write image {}: {e}", path.display()))?;
    Ok(path)
}

/// Encodes an RGBA8 buffer as PNG bytes.
pub fn encode_rgba_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| "image dimensions overflow".to_string())?;
    if rgba.len() < expected {
        return Err(format!(
            "image buffer too small: got {} bytes, need {expected} for {width}x{height}",
            rgba.len()
        ));
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("png header failed: {e}"))?;
        writer
            .write_image_data(&rgba[..expected])
            .map_err(|e| format!("png encode failed: {e}"))?;
    }
    Ok(out)
}

/// Saves an RGBA screenshot (e.g. from the system clipboard) as a PNG file.
pub fn save_rgba_screenshot(
    width: usize,
    height: usize,
    rgba: &[u8],
) -> Result<std::path::PathBuf, String> {
    let w = u32::try_from(width).map_err(|_| "image width too large".to_string())?;
    let h = u32::try_from(height).map_err(|_| "image height too large".to_string())?;
    let png = encode_rgba_png(w, h, rgba)?;
    save_png_bytes(&png, "paste")
}

/// Reads an image from the system clipboard and saves it as a PNG under the
/// attachments directory. Returns the absolute path on success.
pub fn paste_clipboard_image() -> Result<std::path::PathBuf, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("clipboard unavailable: {e}"))?;
    let img = cb
        .get_image()
        .map_err(|e| format!("no image on clipboard: {e}"))?;
    save_rgba_screenshot(img.width, img.height, &img.bytes)
}

/// Opens a native file dialog so the user can pick an image from disk.
///
/// `start_dir` is the folder the dialog opens in (typically the project cwd).
/// Returns `Ok(None)` when the dialog is cancelled.
pub fn pick_image_file(start_dir: &std::path::Path) -> Result<Option<std::path::PathBuf>, String> {
    let start = if start_dir.is_dir() {
        start_dir.to_path_buf()
    } else {
        start_dir
            .parent()
            .filter(|p| p.is_dir())
            .unwrap_or(start_dir)
            .to_path_buf()
    };

    #[cfg(target_os = "macos")]
    {
        // Bring the host process forward (terminal apps often bury osascript
        // panels) and open at the project directory. Cancel → empty string.
        let default_loc = start
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let script = format!(
            r#"
try
  tell application "System Events" to set frontmost of first process whose unix id is {pid} to true
end try
try
  set defaultFolder to POSIX file "{default_loc}" as alias
on error
  set defaultFolder to (path to desktop folder)
end try
try
  set theFile to choose file with prompt "Select an image" of type {{"public.png", "public.jpeg", "public.tiff", "public.gif", "public.webp", "png", "jpg", "jpeg", "gif", "webp"}} default location defaultFolder without multiple selections allowed
  return POSIX path of theFile
on error number -128
  return ""
end try
"#,
            pid = std::process::id(),
            default_loc = default_loc,
        );
        let out = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| format!("failed to open file picker: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(format!("file picker failed: {}", err.trim()));
        }
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if path.is_empty() {
            Ok(None)
        } else {
            Ok(Some(std::path::PathBuf::from(path)))
        }
    }
    #[cfg(target_os = "linux")]
    {
        // Prefer zenity, then kdialog. Both return the selected path on stdout.
        let start_s = start.to_string_lossy().into_owned();
        let zenity_args = vec![
            "--file-selection".into(),
            "--title=Select an image".into(),
            format!("--filename={}/", start_s.trim_end_matches('/')),
            "--file-filter=Images | *.png *.jpg *.jpeg *.gif *.webp".into(),
        ];
        let kdialog_args = vec![
            "--getopenfilename".into(),
            start_s.clone(),
            "Image Files (*.png *.jpg *.jpeg *.gif *.webp)".into(),
        ];
        let candidates: [(&str, Vec<String>); 2] =
            [("zenity", zenity_args), ("kdialog", kdialog_args)];
        for (bin, args) in candidates {
            let has = std::process::Command::new("which")
                .arg(bin)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if !has {
                continue;
            }
            let out = std::process::Command::new(bin)
                .args(&args)
                .output()
                .map_err(|e| format!("failed to open file picker ({bin}): {e}"))?;
            if !out.status.success() {
                return Ok(None);
            }
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if path.is_empty() {
                return Ok(None);
            }
            return Ok(Some(std::path::PathBuf::from(path)));
        }
        Err(
            "no file picker found (install zenity or kdialog, or pass a path: /image <path>)"
                .into(),
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = start;
        Err("native file picker is not available on this platform — use /image <path>".into())
    }
}

/// Resolves a user-supplied image path (absolute or relative to `cwd`) and
/// validates that it is a supported image file.
pub fn resolve_and_validate(
    path: &str,
    cwd: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let raw = std::path::PathBuf::from(path);
    let abs = if raw.is_absolute() {
        raw
    } else {
        cwd.join(raw)
    };
    let abs = std::fs::canonicalize(&abs).unwrap_or(abs);
    let _ = load_image(abs.to_str().unwrap_or(path))?;
    Ok(abs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_media_type_from_extension() {
        assert_eq!(
            ImageMediaType::from_extension("png"),
            Some(ImageMediaType::Png)
        );
        assert_eq!(
            ImageMediaType::from_extension("JPG"),
            Some(ImageMediaType::Jpeg)
        );
        assert_eq!(
            ImageMediaType::from_extension("webp"),
            Some(ImageMediaType::Webp)
        );
        assert_eq!(ImageMediaType::from_extension("txt"), None);
    }

    #[test]
    fn test_load_image_encodes_base64() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("img.png");
        std::fs::write(&path, [0x89, b'P', b'N', b'G']).unwrap();
        let img = load_image(path.to_str().unwrap()).unwrap();
        assert_eq!(img.media_type, "image/png");
        assert_eq!(
            img.data,
            base64::engine::general_purpose::STANDARD.encode([0x89, b'P', b'N', b'G'])
        );
    }

    #[test]
    fn test_load_image_missing_file() {
        assert!(load_image("/nonexistent/nope.png").is_err());
    }

    #[test]
    fn test_load_image_unsupported_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("img.txt");
        std::fs::write(&path, "hi").unwrap();
        assert!(load_image(path.to_str().unwrap()).is_err());
    }

    #[test]
    fn test_encode_rgba_png_roundtrip_header() {
        // 1x1 red pixel RGBA
        let rgba = [255u8, 0, 0, 255];
        let png = encode_rgba_png(1, 1, &rgba).unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
        let path = save_png_bytes(&png, "test").unwrap();
        assert!(path.exists());
        let img = load_image(path.to_str().unwrap()).unwrap();
        assert_eq!(img.media_type, "image/png");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_resolve_and_validate_relative() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        std::fs::write(&path, [0x89, b'P', b'N', b'G']).unwrap();
        let got = resolve_and_validate("shot.png", dir.path()).unwrap();
        assert_eq!(got.file_name().unwrap(), "shot.png");
    }
}
