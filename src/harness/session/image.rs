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
}
