use crate::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use dim_agent_types::{ContentBlock, ImageSource, ImageSourceType};
use std::{fs::File, io::Read, path::PathBuf};

pub fn load(paths: &[PathBuf]) -> Result<Vec<ContentBlock>> {
    paths
        .iter()
        .map(|path| {
            let read = || -> Result<ContentBlock> {
                if !std::fs::metadata(path)?.is_file() {
                    return Err("Expected a local image file".into());
                }
                let mut bytes = Vec::new();
                File::open(path)?
                    .take(5 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() > 5 * 1024 * 1024 {
                    return Err("Image exceeds 5 MiB".into());
                }
                let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes.as_slice()))
                    .with_guessed_format()?;
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
                let (width, height) = reader.into_dimensions().map_err(|_| {
                    "Invalid or truncated image header; dimensions must be 1–8000 pixels per side"
                })?;
                if width == 0 || height == 0 {
                    return Err("Image dimensions must be nonzero".into());
                }
                Ok(ContentBlock::Image {
                    source: ImageSource {
                        source_type: ImageSourceType::Base64,
                        media_type: media_type.into(),
                        data: Some(STANDARD.encode(bytes)),
                        url: None,
                    },
                })
            };
            read().map_err(|error| format!("Cannot use image {}: {error}", path.display()).into())
        })
        .collect()
}
