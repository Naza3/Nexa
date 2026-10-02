//! Synthetic Windows observation, not native inference or an immutability proof.
//! A writable view predates registration and its original writer is closed.
//! Either scan refusal or admission is reported; a successful scan's ordinary
//! sharing guard must not be mistaken for protection against preexisting views.
#![cfg(windows)]

use model_store::library::{ScanControl, scan_directory};
use runtime_types::ErrorCode;
use std::{
    fs::{self, File, OpenOptions},
    os::windows::io::AsRawHandle,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION, HANDLE},
    System::Memory::{
        CreateFileMappingW, FILE_MAP_WRITE, FlushViewOfFile, MEMORY_MAPPED_VIEW_ADDRESS,
        MapViewOfFile, PAGE_READWRITE, UnmapViewOfFile,
    },
};

fn string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend((value.len() as u64).to_le_bytes());
    bytes.extend(value.as_bytes());
}

// Same bounded, structurally valid fixture as external.rs. Its last byte is
// tensor payload, so the observation never changes a parser length or offset.
fn gguf() -> Vec<u8> {
    let mut bytes = b"GGUF".to_vec();
    bytes.extend(3_u32.to_le_bytes());
    bytes.extend(1_u64.to_le_bytes());
    bytes.extend(4_u64.to_le_bytes());
    for (key, value) in [
        ("general.architecture", "qwen3"),
        ("tokenizer.chat_template", "synthetic 中文 template"),
    ] {
        string(&mut bytes, key);
        bytes.extend(8_u32.to_le_bytes());
        string(&mut bytes, value);
    }
    for (key, value) in [
        ("general.file_type", 7_u32),
        ("qwen3.context_length", 40960),
    ] {
        string(&mut bytes, key);
        bytes.extend(4_u32.to_le_bytes());
        bytes.extend(value.to_le_bytes());
    }
    string(&mut bytes, "synthetic.weight");
    bytes.extend(1_u32.to_le_bytes());
    bytes.extend(32_u64.to_le_bytes());
    bytes.extend(0_u32.to_le_bytes());
    bytes.extend(0_u64.to_le_bytes());
    bytes.resize(bytes.len().next_multiple_of(32), 0);
    bytes.resize(bytes.len() + 128, 0);
    bytes
}

// Keep failure diagnostics fixed: no source paths or platform error strings.
fn required<T, E>(result: Result<T, E>, phase: &'static str) -> T {
    match result {
        Ok(value) => value,
        Err(_) => panic!("windows_external_mapping setup_failed phase={phase}"),
    }
}

struct WritableMapping {
    handle: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    length: usize,
}

impl WritableMapping {
    fn new(file: &File, length: usize) -> Self {
        assert!(length > 0);
        // SAFETY: file owns a live read/write handle. Null attributes/name create
        // an unnamed, non-inheritable mapping; zero size uses the existing EOF.
        let handle = unsafe {
            CreateFileMappingW(
                file.as_raw_handle().cast(),
                ptr::null(),
                PAGE_READWRITE,
                0,
                0,
                ptr::null(),
            )
        };
        assert!(!handle.is_null(), "mapping creation failed");
        let mut mapping = Self {
            handle,
            view: MEMORY_MAPPED_VIEW_ADDRESS::default(),
            length,
        };
        // SAFETY: mapping owns the handle; the requested view is within EOF.
        mapping.view = unsafe { MapViewOfFile(mapping.handle, FILE_MAP_WRITE, 0, 0, length) };
        assert!(
            !mapping.view.Value.is_null(),
            "mapping view creation failed"
        );
        mapping
    }

    fn replace_last_byte(&mut self, value: u8) -> bool {
        assert!(!self.view.Value.is_null());
        // SAFETY: this live PAGE_READWRITE view covers length bytes. Only this
        // thread accesses the last payload byte, and no Rust reference aliases
        // it. The original file handle need not outlive the mapped view.
        unsafe {
            let byte = self.view.Value.cast::<u8>().add(self.length - 1);
            ptr::write_volatile(byte, value);
            ptr::read_volatile(byte) == value
        }
    }

    fn flush(&self) -> bool {
        // SAFETY: the whole requested range is inside the live mapped view.
        // Success is only a flush observation, not a crash-durability promise.
        unsafe { FlushViewOfFile(self.view.Value, self.length) != 0 }
    }

    fn release(&mut self) -> (bool, bool) {
        let unmapped = self.view.Value.is_null() || {
            // SAFETY: this is the exact view returned by MapViewOfFile and has
            // not been unmapped. Clear it only after successful release.
            let ok = unsafe { UnmapViewOfFile(self.view) != 0 };
            if ok {
                self.view.Value = ptr::null_mut();
            }
            ok
        };
        let closed = self.handle.is_null() || {
            // SAFETY: this object alone owns the live mapping handle.
            let ok = unsafe { CloseHandle(self.handle) != 0 };
            if ok {
                self.handle = ptr::null_mut();
            }
            ok
        };
        (unmapped, closed)
    }
}

impl Drop for WritableMapping {
    fn drop(&mut self) {
        // Also release on assertion failure; the normal path checks both API
        // results and explicit deletion rather than relying on best-effort Drop.
        let _ = self.release();
    }
}

#[derive(Default, serde::Serialize)]
struct Observation {
    phase: &'static str,
    writer_closed: bool,
    scan_admitted: bool,
    scan_rejected_in_use: bool,
    ordinary_writer_blocked: bool,
    mapping_write_attempted: bool,
    mapped_byte_changed: bool,
    flush_succeeded: bool,
    file_read_succeeded: bool,
    file_changed_while_guard: bool,
    view_unmapped: bool,
    mapping_handle_closed: bool,
    ordinary_write_after_release: bool,
    scan_after_release_admitted: bool,
    source_removed: bool,
    directories_removed: bool,
}

impl Observation {
    fn emit(&mut self, phase: &'static str) {
        self.phase = phase;
        println!(
            "windows_external_mapping {}",
            required(serde_json::to_string(self), "encode_observation")
        );
    }
}

#[test]
fn windows_preexisting_writable_mapping_observation() {
    let data = required(tempfile::tempdir(), "create_data");
    let source = required(tempfile::tempdir(), "create_source");
    let path = source.path().join("synthetic.gguf");
    let original = gguf();
    required(fs::write(&path, &original), "write_fixture");
    let writer = required(
        OpenOptions::new().read(true).write(true).open(&path),
        "open_original_writer",
    );
    let mut mapping = WritableMapping::new(&writer, original.len());
    drop(writer);
    let mut observed = Observation {
        writer_closed: true,
        ..Default::default()
    };
    observed.emit("mapping_ready");

    let scan = scan_directory(data.path(), source.path(), None, &ScanControl::default());
    match scan {
        Ok(scanned) => {
            observed.scan_admitted = true;
            assert_eq!(scanned.library().models.len(), 1);
            // This open has no truncate flag and never writes, even if a
            // regression unexpectedly permits it. Drop it before the mapping
            // observation so it cannot be the writer of the changed byte.
            observed.ordinary_writer_blocked = match OpenOptions::new().write(true).open(&path) {
                Ok(writer) => {
                    drop(writer);
                    false
                }
                Err(error) => matches!(
                    error.raw_os_error().map(|code| code as u32),
                    Some(ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
                ),
            };
            observed.mapping_write_attempted = true;
            observed.emit("guard_held_before_mapping_write");
            const CHANGED_BYTE: u8 = 0x5a;
            assert_ne!(original[original.len() - 1], CHANGED_BYTE);
            observed.mapped_byte_changed = mapping.replace_last_byte(CHANGED_BYTE);
            observed.flush_succeeded = mapping.flush();
            // A fresh ordinary read observes file visibility separately from
            // the mapped-view readback. Neither outcome is hardcoded.
            if let Ok(bytes) = fs::read(&path) {
                observed.file_read_succeeded = true;
                let mut changed = original.clone();
                *changed.last_mut().unwrap() = CHANGED_BYTE;
                observed.file_changed_while_guard = bytes == changed;
                assert!(
                    bytes == original || bytes == changed,
                    "unexpected fixture change"
                );
            }
            observed.emit("guard_held_after_mapping_write");
            // Explicitly after the mapped write, flush and fresh file read.
            // Cloning library metadata alone would not retain the guards.
            drop(scanned);
        }
        Err(error) => {
            observed.scan_rejected_in_use = error.code == ErrorCode::ModelFileInUse;
            observed.emit("scan_rejected");
        }
    }

    (observed.view_unmapped, observed.mapping_handle_closed) = mapping.release();
    drop(mapping);
    observed.ordinary_write_after_release = fs::write(&path, &original).is_ok();
    // A refusal above is meaningful only if the same valid fixture can be
    // scanned after release. This temporary scan guard is dropped here.
    observed.scan_after_release_admitted =
        scan_directory(data.path(), source.path(), None, &ScanControl::default())
            .map(|scanned| scanned.library().models.len() == 1)
            .unwrap_or(false);
    observed.source_removed = fs::remove_file(&path).is_ok() && !path.exists();
    let source_path = source.path().to_path_buf();
    let data_path = data.path().to_path_buf();
    let source_closed = source.close().is_ok();
    let data_closed = data.close().is_ok();
    observed.directories_removed =
        source_closed && data_closed && !source_path.exists() && !data_path.exists();
    observed.emit("finished");

    assert!(observed.scan_admitted || observed.scan_rejected_in_use);
    if observed.scan_admitted {
        assert!(observed.ordinary_writer_blocked);
        assert!(observed.mapping_write_attempted && observed.mapped_byte_changed);
        assert!(observed.file_read_succeeded);
        // Deliberately no expected value for file_changed_while_guard or flush:
        // they report this host's observation, never an immutability guarantee.
    } else {
        assert!(!observed.mapping_write_attempted);
    }
    assert!(observed.view_unmapped && observed.mapping_handle_closed);
    assert!(observed.ordinary_write_after_release);
    assert!(observed.scan_after_release_admitted);
    assert!(observed.source_removed && observed.directories_removed);
}
