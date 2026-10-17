use crate::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use dim_agent_types::{ContentBlock, ImageSource, ImageSourceType};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    fs::OpenOptions,
    io::Read,
    path::{Path, PathBuf},
};

pub fn load(paths: &[PathBuf]) -> Result<Vec<ContentBlock>> {
    let mut encoded_size = 0;
    paths
        .iter()
        .map(|path| {
            file(path)
                .and_then(|content| {
                    let ContentBlock::Image { ref source } = content else {
                        return Err("Expected PNG, JPEG, WebP or GIF image data".into());
                    };
                    encoded_size += source.data.as_ref().expect("local image data").len();
                    if encoded_size > 16 * 1024 * 1024 {
                        return Err("Images exceed 16 MiB after base64 encoding".into());
                    }
                    Ok(content)
                })
                .map_err(|error| format!("Cannot use image {}: {error}", path.display()).into())
        })
        .collect()
}

pub fn file(path: &Path) -> Result<ContentBlock> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NONBLOCK);
    let mut file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err("Expected a local ordinary file".into());
    }
    let mut bytes = Vec::new();
    (&mut file).take(32).read_to_end(&mut bytes)?;
    let image_intent = matches!(
        image::guess_format(&bytes),
        Ok(image::ImageFormat::Png
            | image::ImageFormat::Jpeg
            | image::ImageFormat::WebP
            | image::ImageFormat::Gif)
    ) || path.extension().is_some_and(|extension| {
        image::ImageFormat::from_extension(extension).is_some()
            || extension
                .to_str()
                .is_some_and(|s| matches!(s.to_ascii_lowercase().as_str(), "svg" | "heic" | "heif"))
    });
    if !image_intent {
        return Ok(ContentBlock::Text {
            text: path.to_str().ok_or("File paths must be UTF-8")?.into(),
        });
    }
    file.take((5 * 1024 * 1024 + 1 - bytes.len()) as u64)
        .read_to_end(&mut bytes)?;
    encoded(bytes).map(|source| ContentBlock::Image { source })
}

pub fn encoded(bytes: Vec<u8>) -> Result<ImageSource> {
    if bytes.len() > 5 * 1024 * 1024 {
        return Err("Image exceeds 5 MiB".into());
    }
    let mut reader =
        image::ImageReader::new(std::io::Cursor::new(bytes.as_slice())).with_guessed_format()?;
    let media_type = match reader.format() {
        Some(image::ImageFormat::Png) => "image/png",
        Some(image::ImageFormat::Jpeg) => "image/jpeg",
        Some(image::ImageFormat::WebP) => "image/webp",
        Some(image::ImageFormat::Gif) => "image/gif",
        _ => return Err("Expected PNG, JPEG, WebP or GIF image data".into()),
    };
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8000);
    limits.max_image_height = Some(8000);
    reader.limits(limits);
    let (width, height) = reader.into_dimensions().map_err(
        |_| "Invalid or truncated image header; dimensions must be 1–8000 pixels per side",
    )?;
    if width == 0 || height == 0 {
        return Err("Image dimensions must be nonzero".into());
    }
    Ok(ImageSource {
        source_type: ImageSourceType::Base64,
        media_type: media_type.into(),
        data: Some(STANDARD.encode(bytes)),
        url: None,
    })
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn pixels(image: image::DynamicImage) -> Result<ImageSource> {
    if image.width() == 0 || image.height() == 0 || image.width() > 8000 || image.height() > 8000 {
        return Err("Clipboard image dimensions must be 1–8000 pixels per side".into());
    }
    let mut encoded = std::io::Cursor::new(Vec::new());
    image.write_to(&mut encoded, image::ImageFormat::Png)?;
    let bytes = encoded.into_inner();
    if bytes.len() > 5 * 1024 * 1024 {
        return Err("Image exceeds 5 MiB".into());
    }
    Ok(ImageSource {
        source_type: ImageSourceType::Base64,
        media_type: "image/png".into(),
        data: Some(STANDARD.encode(bytes)),
        url: None,
    })
}
