//! Bounded, read-only catalog view. Never opens a store, locks it, recovers
//! imports, creates directories, or reads model payload bytes.
use crate::{
    ModelManifest, ModelStorage, Result, invalid_manifest,
    library::{self, ModelLibrary},
};
use runtime_types::{ErrorCode, ModelId};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, io::Read, path::Path};
use uuid::Uuid;

pub const MAX_INVENTORY_MODELS: usize = 1024;
pub struct InventoryEntry {
    pub manifest: ModelManifest,
    pub availability_error: Option<ErrorCode>,
    pub file_stamp: Option<library::FileIdentity>,
}
pub struct Inventory {
    pub generation: Uuid,
    pub entries: Vec<InventoryEntry>,
}
/// Validate only an application data directory. External picker/scan sources
/// must use library::validate_directory_candidate and its stricter policy.
pub fn validate_data_directory(root: &Path) -> Result<()> {
    library::DirectoryGuard::open_data_directory(root).map(|_| ())
}
pub fn read(root: &Path) -> Result<Inventory> {
    let mut entries = Vec::new();
    let mut registered_ids = BTreeSet::new();
    let mut metadata_bytes = 0u64;
    let _root = root
        .exists()
        .then(|| library::DirectoryGuard::open_data_directory(root))
        .transpose()?;
    let library = ModelLibrary::read(root)?;
    let managed = root.join("models");
    if managed.exists() {
        let _managed = library::DirectoryGuard::open_data_directory(&managed)?;
        for (count, entry) in fs::read_dir(&managed).map_err(crate::io_error)?.enumerate() {
            if count >= MAX_INVENTORY_MODELS {
                return Err(library::library_error(ErrorCode::ModelLibraryLimit));
            }
            let entry = entry.map_err(crate::io_error)?;
            let name = entry.file_name();
            let id = ModelId::new(
                name.to_str()
                    .ok_or_else(|| invalid_manifest("invalid model ID"))?,
            )?;
            if !registered_ids.insert(id.clone()) {
                return Err(library::library_error(ErrorCode::ModelLibraryChanged));
            }
            let directory = entry.path();
            let _directory = library::DirectoryGuard::open_data_directory(&directory)?;
            let file = library::open_read_file(&directory.join("manifest.json"), false)?;
            metadata_bytes =
                metadata_bytes.saturating_add(file.metadata().map_err(crate::io_error)?.len());
            if metadata_bytes > 16 * 1024 * 1024
                || file.metadata().map_err(crate::io_error)?.len() > 1024 * 1024
            {
                return Err(invalid_manifest("manifest too large"));
            }
            let mut bytes = Vec::new();
            file.take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(crate::io_error)?;
            if bytes.len() > 1024 * 1024 {
                return Err(invalid_manifest("manifest too large"));
            }
            let manifest: ModelManifest =
                serde_json::from_slice(&bytes).map_err(|_| invalid_manifest("invalid manifest"))?;
            manifest.validate()?;
            if manifest.id != id || manifest.storage != ModelStorage::Managed {
                return Err(invalid_manifest("manifest identity mismatch"));
            }
            if library
                .as_ref()
                .is_some_and(|library| library.is_unregistered(&manifest))
            {
                continue;
            }
            let stamp = library::open_read_file(&directory.join("model.gguf"), false)
                .and_then(|f| library::identity(&f));
            let availability_error = match &stamp {
                Ok(stamp) if stamp.size == manifest.size_bytes => None,
                Ok(_) => Some(ErrorCode::ModelFileChanged),
                Err(e) => Some(e.code),
            };
            entries.push(InventoryEntry {
                manifest,
                availability_error,
                file_stamp: stamp.ok(),
            });
        }
    }
    if let Some(library) = &library {
        for entry in &library.models {
            if !registered_ids.insert(entry.manifest.id.clone()) {
                return Err(library::library_error(ErrorCode::ModelLibraryChanged));
            }
            if library.is_unregistered(&entry.manifest) {
                continue;
            }
            if entries.len() >= MAX_INVENTORY_MODELS {
                return Err(library::library_error(ErrorCode::ModelLibraryLimit));
            }
            let availability_error = library.availability(entry);
            entries.push(InventoryEntry {
                manifest: entry.manifest.clone(),
                availability_error,
                file_stamp: availability_error.is_none().then(|| entry.identity.clone()),
            });
        }
    }
    entries.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    let mut ids = BTreeSet::new();
    let mut digest = Sha256::new();
    // The source discriminator makes a runtime cursor impossible to reuse here.
    digest.update(b"nexa-offline-inventory-v1");
    if let Some(library) = &library {
        digest.update(library.library_generation.as_bytes());
    }
    for entry in &entries {
        if !ids.insert(&entry.manifest.id) {
            return Err(library::library_error(ErrorCode::ModelLibraryChanged));
        }
        digest.update(
            serde_json::to_vec(&entry.manifest)
                .map_err(|_| invalid_manifest("invalid manifest"))?,
        );
        digest.update(
            serde_json::to_vec(&entry.file_stamp)
                .map_err(|_| invalid_manifest("invalid identity"))?,
        );
        digest.update(entry.availability_error.map_or("", |e| e.as_str()));
    }
    let bytes: [u8; 16] = digest.finalize()[..16].try_into().unwrap();
    Ok(Inventory {
        generation: Uuid::from_bytes(bytes),
        entries,
    })
}
