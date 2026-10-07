//! Opaque bounded native leases. No source path is serialized to the WebView.
use serde::Serialize;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const SELECTION_LIFETIME: Duration = Duration::from_secs(600);
#[derive(Clone, Debug, Serialize)]
pub struct PickedFile {
    pub selection_index: usize,
    pub file_name: String,
    pub size_bytes: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct PickedFiles {
    pub selection_id: Uuid,
    pub files: Vec<PickedFile>,
    pub expires_in_seconds: u64,
}
pub struct FileSelection<T> {
    pub id: Uuid,
    pub files: Option<Vec<T>>,
    expires: Instant,
}
impl<T> FileSelection<T> {
    pub fn new(files: Vec<T>, summaries: Vec<PickedFile>) -> (Self, PickedFiles) {
        let id = Uuid::new_v4();
        (
            Self {
                id,
                files: Some(files),
                expires: Instant::now() + SELECTION_LIFETIME,
            },
            PickedFiles {
                selection_id: id,
                files: summaries,
                expires_in_seconds: SELECTION_LIFETIME.as_secs(),
            },
        )
    }
    pub fn expire(slot: &mut Option<Self>) {
        if slot.as_ref().is_some_and(|s| s.expires <= Instant::now()) {
            *slot = None;
        }
    }
    pub fn get(slot: &mut Option<Self>, id: Uuid) -> Result<&mut Self, &'static str> {
        Self::expire(slot);
        slot.as_mut()
            .filter(|s| s.id == id && s.files.is_some())
            .ok_or("selection_expired")
    }
    pub fn consume_pair(slot: &mut Option<Self>, id: Uuid) -> Result<Vec<T>, &'static str> {
        let selection = Self::get(slot, id)?;
        if selection
            .files
            .as_ref()
            .is_none_or(|files| files.len() != 2)
        {
            return Err("invalid_request");
        }
        let files = selection.files.take().ok_or("selection_expired")?;
        *slot = None;
        Ok(files)
    }
    pub fn discard(slot: &mut Option<Self>, id: Uuid) -> bool {
        if slot.as_ref().is_some_and(|s| s.id == id) {
            *slot = None;
            true
        } else {
            false
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Lease(Arc<AtomicUsize>);
    impl Drop for Lease {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    #[test]
    fn pair_requires_two_files_and_is_single_use() {
        let (selection, dto) = FileSelection::new(vec![1], vec![]);
        let mut slot = Some(selection);
        assert_eq!(
            FileSelection::consume_pair(&mut slot, dto.selection_id),
            Err("invalid_request")
        );
        assert!(slot.is_some());
        let (selection, dto) = FileSelection::new(vec![1, 2], vec![]);
        slot = Some(selection);
        assert_eq!(
            FileSelection::consume_pair(&mut slot, Uuid::new_v4()),
            Err("selection_expired")
        );
        assert_eq!(
            FileSelection::consume_pair(&mut slot, dto.selection_id).unwrap(),
            vec![1, 2]
        );
        assert_eq!(
            FileSelection::consume_pair(&mut slot, dto.selection_id),
            Err("selection_expired")
        );
    }
    #[test]
    fn wrong_id_preserves_and_discard_releases_leases() {
        let drops = Arc::new(AtomicUsize::new(0));
        let (selection, dto) = FileSelection::new(vec![Lease(drops.clone())], vec![]);
        let mut slot = Some(selection);
        assert!(FileSelection::get(&mut slot, Uuid::new_v4()).is_err());
        assert!(!FileSelection::discard(&mut slot, Uuid::new_v4()));
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(FileSelection::discard(&mut slot, dto.selection_id));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn expiry_and_success_consume_but_failed_admission_preserves() {
        let (mut selection, dto) = FileSelection::new(vec![7], vec![]);
        selection.expires = Instant::now();
        let mut slot = Some(selection);
        assert!(FileSelection::get(&mut slot, dto.selection_id).is_err());
        assert!(slot.is_none());
        let (selection, dto) = FileSelection::new(vec![7], vec![]);
        let mut slot = Some(selection);
        assert_eq!(
            FileSelection::get(&mut slot, dto.selection_id)
                .unwrap()
                .files
                .as_ref()
                .unwrap(),
            &[7]
        );
        FileSelection::get(&mut slot, dto.selection_id)
            .unwrap()
            .files
            .take();
        assert!(FileSelection::get(&mut slot, dto.selection_id).is_err());
    }
}
