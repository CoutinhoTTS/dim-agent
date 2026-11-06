//! The bound side of one media call: a job's validated parameters become an
//! aimux call, and what aimux returns becomes files in the job's work
//! directory.
//!
//! Every option here is the one aimux declares for that model type; a
//! provider's own keys travel in `params.providerOptions`, which is the
//! namespaced map aimux already reads. Nothing reads or writes a credential.

use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageOutputs, ImageResult,
};
use aimux_core::shared::{AspectRatio, SharedProviderOptions, Size};
use dim_modality::{AdapterError, Job, JobFile};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::vendor::Vendor;

/// One image call's options.
pub fn image_options(job: &Job, vendor: &Vendor) -> Result<ImageCallOptions, AdapterError> {
    let params = &job.params;
    let mut options = ImageCallOptions::new(text(params, "prompt").unwrap_or_default());
    if text(params, "prompt").is_none() {
        options.prompt = None;
    }
    options.n = integer(params, "n")?.unwrap_or(1) as u32;
    options.size = text(params, "size")
        .map(Size::parse)
        .transpose()
        .map_err(invalid_input)?;
    options.aspect_ratio = text(params, "aspectRatio")
        .map(AspectRatio::parse)
        .transpose()
        .map_err(invalid_input)?;
    options.seed = integer(params, "seed")?;
    options.provider_options = provider_options(params, vendor);
    if let (Some(key), Some(format)) = (vendor.format, text(params, "outputFormat")) {
        options
            .provider_options
            .entry(vendor.options.to_owned())
            .or_default()
            .insert(key.into(), Value::String(format.into()));
    }
    let mut files = Vec::new();
    for file in &job.files {
        files.push(image_file(file)?);
    }
    if !files.is_empty() {
        options.files = Some(files);
    }
    if let Some(mask) = text(params, "mask") {
        let file = job
            .files
            .iter()
            .find(|file| file.name == mask)
            .ok_or_else(|| {
                AdapterError::invalid_input(format!("`{mask}` is not one of the job's files"))
            })?;
        options.mask = Some(image_file(file)?);
    }
    Ok(options)
}

/// The requested `outputFormat`, for naming an output whose bytes name none.
pub fn requested_format(job: &Job) -> Option<&str> {
    text(&job.params, "outputFormat")
}

/// The caller's provider options, under the package's own namespace.
///
/// `params.providerOptions` is aimux's namespaced map when it carries the
/// package's key, and otherwise the package's own options: a caller never has
/// to know that Black Forest Labs calls its namespace `blackForestLabs`.
fn provider_options(params: &Value, vendor: &Vendor) -> SharedProviderOptions {
    let mut options = SharedProviderOptions::new();
    let Some(object) = params.get("providerOptions").and_then(Value::as_object) else {
        return options;
    };
    match object.get(vendor.options).and_then(Value::as_object) {
        Some(namespaced) => {
            options.insert(vendor.options.to_owned(), namespaced.clone());
        }
        None if !object.is_empty() => {
            options.insert(vendor.options.to_owned(), object.clone());
        }
        None => {}
    }
    options
}

fn image_file(file: &dim_modality::JobFile) -> Result<ImageFile, AdapterError> {
    Ok(ImageFile::File {
        media_type: file.media_type.clone(),
        data: ImageFileData::Binary(read(file.path.as_path())?),
    })
}

fn read(path: &Path) -> Result<Vec<u8>, AdapterError> {
    std::fs::read(path).map_err(|error| {
        AdapterError::invalid_input(format!(
            "{} cannot be read: {error}",
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        ))
    })
}

fn text<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}

fn integer(params: &Value, key: &str) -> Result<Option<u64>, AdapterError> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            AdapterError::invalid_input(format!("`{key}` must be a non-negative integer"))
        }),
    }
}

fn invalid_input(message: String) -> AdapterError {
    AdapterError::invalid_input(message)
}

/// The files one image call produced.
pub fn image_output(
    result: ImageResult,
    work_dir: &Path,
    requested: Option<&str>,
) -> Result<Vec<JobFile>, AdapterError> {
    let images: Vec<Vec<u8>> = match result.images {
        ImageOutputs::Base64(images) => images
            .into_iter()
            .map(|image| {
                use base64::Engine as _;
                base64::engine::general_purpose::STANDARD
                    .decode(image)
                    .map_err(|error| {
                        AdapterError::provider_error(format!(
                            "The provider returned an image that is not base64: {error}"
                        ))
                    })
            })
            .collect::<Result<_, _>>()?,
        ImageOutputs::Binary(images) => images,
    };
    if images.is_empty() {
        return Err(AdapterError::provider_error(
            "The provider returned no images",
        ));
    }
    let mut files = Vec::new();
    for (index, bytes) in images.iter().enumerate() {
        let (extension, media_type) = image_kind(bytes, requested);
        let name = format!("image-{}.{extension}", index + 1);
        let path = write(work_dir, &name, bytes)?;
        files.push(JobFile {
            name,
            media_type: media_type.to_owned(),
            path,
        });
    }
    Ok(files)
}

pub fn write(work_dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, AdapterError> {
    let path = work_dir.join(name);
    std::fs::write(&path, bytes).map_err(|error| {
        AdapterError::provider_error(format!("The result file cannot be written: {error}"))
    })?;
    Ok(path)
}

/// The extension and media type of an image, from its bytes (A.2): a provider
/// that ignores the requested format is named by what it returned.
pub fn image_kind(bytes: &[u8], requested: Option<&str>) -> (&'static str, &'static str) {
    if let Ok(format) = image::guess_format(bytes) {
        let extension = format.extensions_str().first().copied().unwrap_or("bin");
        return (extension, format.to_mime_type());
    }
    requested_image_kind(requested)
}

fn requested_image_kind(requested: Option<&str>) -> (&'static str, &'static str) {
    match requested {
        Some("jpeg") => ("jpg", "image/jpeg"),
        Some("webp") => ("webp", "image/webp"),
        _ => ("png", "image/png"),
    }
}
