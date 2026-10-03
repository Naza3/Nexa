//! Pinned model sources adapted into the independent transfer contract.
use crate::*;
use download_engine::{DownloadSpec, Url};
use std::collections::BTreeSet;

fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
/// Only catalog entry URLs use provider hosts; the engine validates redirect routes.
fn allowed_url(source: DownloadSource, url: &Url) -> bool {
    if url.scheme() != "https"
        || url.port().is_some_and(|p| p != 443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    match source {
        DownloadSource::Modelscope => matches!(url.host_str(), Some("modelscope.cn")),
        DownloadSource::Huggingface => matches!(url.host_str(), Some("huggingface.co")),
    }
}
pub(super) fn catalog() -> Result<ModelCatalog> {
    let catalog: ModelCatalog = serde_json::from_str(include_str!("model-catalog.json"))
        .map_err(|_| BridgeError::new("model_catalog_invalid"))?;
    let mut ids = BTreeSet::new();
    if catalog.entries.is_empty() || catalog.entries.len() > 64 {
        return Err(BridgeError::new("model_catalog_invalid"));
    }
    for entry in &catalog.entries {
        if runtime_types::ModelId::new(&entry.catalog_id).is_err()
            || !ids.insert(entry.catalog_id.clone())
            || entry.file_name.len() > 200
            || entry.file_name.contains(['/', '\\', ':', '\0'])
            || !entry.file_name.ends_with(".gguf")
            || entry.size_bytes < 4
            || entry.size_bytes > model_store::library::MAX_MODEL_BYTES
            || !hex(&entry.sha256, 64)
            || entry.sources.is_empty()
            || entry.sources.len() > 2
        {
            return Err(BridgeError::new("model_catalog_invalid"));
        }
        let mut seen = Vec::new();
        for source in &entry.sources {
            let url =
                Url::parse(&source.url).map_err(|_| BridgeError::new("model_catalog_invalid"))?;
            if !hex(&source.revision, 40)
                || !source.url.contains(&source.revision)
                || !allowed_url(source.source, &url)
                || seen.contains(&source.source)
            {
                return Err(BridgeError::new("model_catalog_invalid"));
            }
            seen.push(source.source);
        }
    }
    Ok(catalog)
}
pub(super) fn specification(entry: &CatalogEntry, source: &CatalogSource) -> Result<DownloadSpec> {
    let url = Url::parse(&source.url).map_err(|_| BridgeError::new("model_catalog_invalid"))?;
    if !allowed_url(source.source, &url)
        || !hex(&source.revision, 40)
        || !source.url.contains(&source.revision)
        || !hex(&entry.sha256, 64)
    {
        return Err(BridgeError::new("model_catalog_invalid"));
    }
    let mut sha256 = [0; 32];
    for (index, byte) in sha256.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&entry.sha256[index * 2..index * 2 + 2], 16)
            .map_err(|_| BridgeError::new("model_catalog_invalid"))?;
    }
    Ok(DownloadSpec {
        url,
        expected_size: entry.size_bytes,
        sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_catalog_adapts_both_initial_sources_without_redirect_allowlists() {
        let catalog = catalog().unwrap();
        assert!(catalog.entries.len() >= 2);
        for entry in catalog.entries {
            assert_eq!(entry.sources.len(), 2);
            for source in &entry.sources {
                let spec = specification(&entry, source).unwrap();
                assert_eq!(spec.expected_size, entry.size_bytes);
                assert_eq!(spec.url.as_str(), source.url);
            }
        }
    }
    #[test]
    fn initial_urls_must_match_the_selected_provider() {
        for url in [
            "http://huggingface.co/file",
            "https://huggingface.co.evil.invalid/file",
            "https://127.0.0.1/file",
            "https://user:pass@huggingface.co/file",
            "https://huggingface.co:444/file",
            "https://us.aws.cdn.hf.co/file",
            "https://modelscope.cn/file",
            "https://huggingface.co/file#secret",
        ] {
            assert!(!allowed_url(
                DownloadSource::Huggingface,
                &Url::parse(url).unwrap()
            ));
        }
    }
}
