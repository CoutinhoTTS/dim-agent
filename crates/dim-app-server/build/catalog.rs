use crate::catalog_tokens::{split_display, split_id, split_tokens};
use flate2::{Compression, write::GzEncoder};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fmt::Write as _, fs, io::Write, path::Path};

pub fn build(out: &Path) {
    let source = "resources/default-catalog.json";
    println!("cargo::rerun-if-changed={source}");
    let bytes = fs::read(source).unwrap();
    let mut catalog: Value = serde_json::from_slice(&bytes).unwrap();
    let providers = catalog["providers"].as_array_mut().unwrap();
    let mut headers = String::new();
    let mut rows = String::new();
    let mut count = 0;
    for (provider, header) in providers.iter_mut().enumerate() {
        let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(header).unwrap()));
        let models = header["models"].take();
        header["models"] = serde_json::json!([]);
        let id = header["providerId"].as_str().unwrap();
        let first = count;
        for model in models.as_array().unwrap() {
            let model_id = model["modelId"].as_str().unwrap();
            let name = model["displayName"].as_str().unwrap();
            let canonical = model
                .pointer("/metadata/raw/canonical_model_id")
                .and_then(Value::as_str);
            let canonical_tokens =
                canonical.map(|id| split_tokens(id.to_lowercase().rsplit('/').next().unwrap()));
            let own_namespace = header
                .pointer("/metadata/vendorNamespace")
                .and_then(Value::as_str);
            let vendor = header.pointer("/metadata/role").and_then(Value::as_str) == Some("vendor")
                && own_namespace.is_none_or(|own| {
                    canonical
                        .and_then(|id| id.split_once('/'))
                        .is_none_or(|(namespace, _)| namespace == own)
                });
            let index = if vendor {
                "Some(id::Index::Vendor)"
            } else if id == "custom-provider" {
                "Some(id::Index::CustomProvider)"
            } else {
                "None"
            };
            let file = format!("catalog-model-{count}.gz");
            compressed(out, &file, model);
            writeln!(rows, "ModelData {{ provider: {provider}, id: {model_id:?}, index: {index}, tokens: &{:?}, tag: {:?}, canonical: {}, display: &{:?}, bytes: include_bytes!({:?}) }},", split_id(model_id), model_id.rsplit_once(':').map(|(_, tag)| tag.to_lowercase()), canonical_tokens.map_or_else(|| "None".into(), |tokens| format!("Some(&{tokens:?})")), split_display(name), out.join(file)).unwrap();
            count += 1;
        }
        let file = format!("catalog-provider-{provider}.gz");
        compressed(out, &file, header);
        writeln!(headers, "ProviderData {{ id: {id:?}, fingerprint: {fingerprint:?}, first: {first}, end: {count}, bytes: include_bytes!({:?}) }},", out.join(file)).unwrap();
    }
    fs::write(out.join("catalog.rs"), format!("const REVISION: &str = \"{:x}\";\nstatic DATA: CatalogData = CatalogData {{ providers: &[{headers}], models: &[{rows}] }};", Sha256::digest(bytes))).unwrap();
}

fn compressed(out: &Path, name: &str, value: &Value) {
    let mut gzip = GzEncoder::new(Vec::new(), Compression::best());
    serde_json::to_writer(&mut gzip, value).unwrap();
    gzip.flush().unwrap();
    fs::write(out.join(name), gzip.finish().unwrap()).unwrap();
}
