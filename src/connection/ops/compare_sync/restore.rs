//! Session-only encrypted undo records. A frame reaches durable storage before its write starts.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use mongodb::bson::{Binary, Bson, RawDocumentBuf, doc, spec::BinarySubtype};
use tempfile::NamedTempFile;

use super::Operation;
use crate::error::{Error, Result};

const MAGIC: &[u8; 8] = b"OMUNDO01";
const MAX_FRAME: usize = 40 * 1024 * 1024;
const READ_BUDGET: usize = 32 * 1024 * 1024;

pub struct UndoRecord {
    pub row: usize,
    pub operation: Operation,
    pub id: Bson,
    pub before: Option<RawDocumentBuf>,
    pub after_hash: Option<[u8; 32]>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Prepared,
    Possible,
    Confirmed,
    Inactive,
}

struct Frame {
    offset: u64,
    length: usize,
    sequence: u64,
    operation: Operation,
    status: Status,
}

struct Log {
    file: NamedTempFile,
    cipher: Aes256Gcm,
    frames: Vec<Frame>,
    nonce: u64,
}

/// A private file and ephemeral key. Clones of the Arc keep an in-flight batch's log alive.
pub struct RestoreHandle {
    log: Mutex<Log>,
    pending: AtomicUsize,
    uncertain: AtomicUsize,
}

impl RestoreHandle {
    pub fn create(directory: &Path) -> Result<Self> {
        fs::create_dir_all(directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        }
        let mut file = tempfile::Builder::new()
            .prefix("compare-")
            .suffix(".restore")
            .tempfile_in(directory)?;
        // Startup cleanup skips locked files belonging to another live app instance.
        file.as_file()
            .try_lock()
            .map_err(|error| Error::Parse(format!("Cannot lock undo file: {error}")))?;
        file.write_all(MAGIC)?;
        let key: [u8; 32] = rand::random();
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| Error::Parse("Cannot initialize undo encryption".into()))?;
        Ok(Self {
            log: Mutex::new(Log { file, cipher, frames: Vec::new(), nonce: 0 }),
            pending: AtomicUsize::new(0),
            uncertain: AtomicUsize::new(0),
        })
    }

    pub fn pending(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }
    pub fn uncertain(&self) -> usize {
        self.uncertain.load(Ordering::Acquire)
    }

    /// Call from spawn_blocking. On any error, none of this batch may be written.
    pub fn prepare(&self, records: &[UndoRecord]) -> Result<Vec<usize>> {
        let mut log = self.log.lock().map_err(|_| Error::Parse("Undo file lock failed".into()))?;
        let mut entries = Vec::with_capacity(records.len());
        for record in records {
            let plaintext = mongodb::bson::to_vec(&doc! {
                "row": record.row as i64, "op": record.operation.code(), "id": record.id.clone(),
                "before": record.before.as_ref().map(|d| binary(d.as_bytes().to_vec())).unwrap_or(Bson::Null),
                "after": record.after_hash.map(|h| binary(h.to_vec())).unwrap_or(Bson::Null),
            }).map_err(|e| Error::Parse(e.to_string()))?;
            if plaintext.len() > MAX_FRAME - 16 {
                return Err(Error::Parse("Undo record exceeds the size limit".into()));
            }
            let sequence = log.nonce;
            log.nonce = log
                .nonce
                .checked_add(1)
                .ok_or_else(|| Error::Parse("Undo nonce space exhausted".into()))?;
            let nonce = nonce(sequence);
            let encrypted = log
                .cipher
                .encrypt(Nonce::from_slice(&nonce), Payload { msg: &plaintext, aad: MAGIC })
                .map_err(|_| Error::Parse("Cannot encrypt undo record".into()))?;
            let offset = log.file.seek(SeekFrom::End(0))?;
            log.file.write_all(&sequence.to_le_bytes())?;
            log.file.write_all(&(encrypted.len() as u32).to_le_bytes())?;
            log.file.write_all(&encrypted)?;
            entries.push(log.frames.len());
            log.frames.push(Frame {
                offset,
                length: encrypted.len(),
                sequence,
                operation: record.operation,
                status: Status::Prepared,
            });
        }
        log.file.flush()?;
        log.file.as_file().sync_data()?;
        Ok(entries)
    }

    /// Mark possible BEFORE sending a write, so lost acknowledgements still have a guarded undo.
    pub fn started(&self, index: usize) -> Result<()> {
        self.transition(index, Status::Possible)
    }
    pub fn confirmed(&self, index: usize) -> Result<()> {
        self.transition(index, Status::Confirmed)
    }
    pub fn inactive(&self, index: usize) -> Result<()> {
        self.transition(index, Status::Inactive)
    }

    fn transition(&self, index: usize, status: Status) -> Result<()> {
        let mut log = self.log.lock().map_err(|_| Error::Parse("Undo file lock failed".into()))?;
        let frame =
            log.frames.get_mut(index).ok_or_else(|| Error::Parse("Missing undo record".into()))?;
        let pending = |status| matches!(status, Status::Possible | Status::Confirmed);
        if pending(frame.status) && !pending(status) {
            self.pending.fetch_sub(1, Ordering::Release);
        }
        if !pending(frame.status) && pending(status) {
            self.pending.fetch_add(1, Ordering::Release);
        }
        if frame.status == Status::Possible && status != Status::Possible {
            self.uncertain.fetch_sub(1, Ordering::Release);
        }
        if frame.status != Status::Possible && status == Status::Possible {
            self.uncertain.fetch_add(1, Ordering::Release);
        }
        frame.status = status;
        Ok(())
    }

    /// Remove inserted documents first, then revert replacements, then restore deletions.
    /// This handles reused _ids without relying on execution order within an unordered bulk.
    pub fn read_pending(&self, mut cursor: usize) -> Result<(usize, Vec<(usize, UndoRecord)>)> {
        let mut log = self.log.lock().map_err(|_| Error::Parse("Undo file lock failed".into()))?;
        let mut records = Vec::new();
        let mut bytes = 0;
        let mut identities = HashSet::new();
        let frames = log.frames.len();
        while cursor < frames * 3 && records.len() < super::BATCH_ROWS {
            if cursor.is_multiple_of(frames) && !records.is_empty() {
                break;
            }
            let phase = [Operation::Insert, Operation::Replace, Operation::Delete][cursor / frames];
            let index = frames - 1 - cursor % frames;
            let frame = &log.frames[index];
            if frame.operation != phase
                || !matches!(frame.status, Status::Possible | Status::Confirmed)
            {
                cursor += 1;
                continue;
            }
            if !records.is_empty() && bytes + frame.length > READ_BUDGET {
                break;
            }
            let offset = frame.offset;
            let expected_length = frame.length;
            let expected_sequence = frame.sequence;
            log.file.seek(SeekFrom::Start(offset))?;
            let mut header = [0; 12];
            log.file.read_exact(&mut header)?;
            let sequence = u64::from_le_bytes(header[..8].try_into().unwrap());
            let length = u32::from_le_bytes(header[8..].try_into().unwrap()) as usize;
            if sequence != expected_sequence || length != expected_length || length > MAX_FRAME {
                return Err(Error::Parse("Undo file is damaged".into()));
            }
            let mut encrypted = vec![0; length];
            log.file.read_exact(&mut encrypted)?;
            let plaintext = log
                .cipher
                .decrypt(
                    Nonce::from_slice(&nonce(sequence)),
                    Payload { msg: &encrypted, aad: MAGIC },
                )
                .map_err(|_| Error::Parse("Undo record failed authentication".into()))?;
            let document: mongodb::bson::Document =
                mongodb::bson::from_slice(&plaintext).map_err(|e| Error::Parse(e.to_string()))?;
            let record = decode(document)?;
            let identity = mongodb::bson::to_vec(&doc! {"_id": &record.id})
                .map_err(|e| Error::Parse(e.to_string()))?;
            if !identities.insert(identity) {
                break;
            }
            records.push((index, record));
            bytes += length;
            cursor += 1;
        }
        Ok((cursor, records))
    }
}

fn binary(bytes: Vec<u8>) -> Bson {
    Bson::Binary(Binary { subtype: BinarySubtype::Generic, bytes })
}
fn nonce(sequence: u64) -> [u8; 12] {
    let mut nonce = [0; 12];
    nonce[4..].copy_from_slice(&sequence.to_be_bytes());
    nonce
}

fn decode(document: mongodb::bson::Document) -> Result<UndoRecord> {
    let invalid = || Error::Parse("Invalid undo record".into());
    let row =
        document.get_i64("row").ok().and_then(|n| usize::try_from(n).ok()).ok_or_else(invalid)?;
    let operation =
        Operation::from_code(document.get_i32("op").map_err(|_| invalid())?).ok_or_else(invalid)?;
    let id = document.get("id").cloned().ok_or_else(invalid)?;
    let before = match document.get("before") {
        Some(Bson::Null) => None,
        Some(Bson::Binary(bytes)) => {
            Some(RawDocumentBuf::from_bytes(bytes.bytes.clone()).map_err(|_| invalid())?)
        }
        _ => return Err(invalid()),
    };
    let after_hash = match document.get("after") {
        Some(Bson::Null) => None,
        Some(Bson::Binary(bytes)) => {
            Some(bytes.bytes.as_slice().try_into().map_err(|_| invalid())?)
        }
        _ => return Err(invalid()),
    };
    Ok(UndoRecord { row, operation, id, before, after_hash })
}

/// Orphaned keys are gone after exit; remove only our unlocked encrypted files, not live logs.
pub fn sweep(directory: &Path) -> Result<()> {
    if !directory.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !entry.file_type()?.is_file()
            || !name.starts_with("compare-")
            || !name.ends_with(".restore")
        {
            continue;
        }
        let file: File = OpenOptions::new().read(true).write(true).open(entry.path())?;
        if file.try_lock().is_ok() {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Windows locks are mandatory: a second handle cannot read the locked file, ours can.
    fn contents(log: &mut Log) -> Vec<u8> {
        let mut bytes = Vec::new();
        log.file.seek(SeekFrom::Start(0)).unwrap();
        log.file.read_to_end(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn records_are_encrypted_authenticated_and_cleaned_without_touching_live_logs() {
        let directory = tempfile::tempdir().unwrap();
        let restore = RestoreHandle::create(directory.path()).unwrap();
        let path = restore.log.lock().unwrap().file.path().to_owned();
        let document = RawDocumentBuf::from_document(
            &doc! {"_id": 1, "secret": "before-image-secret", "n": i64::MAX},
        )
        .unwrap();
        let entry = restore
            .prepare(&[UndoRecord {
                row: 7,
                operation: Operation::Replace,
                id: 1.into(),
                before: Some(document.clone()),
                after_hash: Some([9; 32]),
            }])
            .unwrap()[0];
        restore.started(entry).unwrap();
        restore.confirmed(entry).unwrap();
        assert_eq!(restore.pending(), 1);
        assert_eq!(
            restore.read_pending(0).unwrap().1[0].1.before.as_ref().unwrap().as_bytes(),
            document.as_bytes()
        );
        let bytes = contents(&mut restore.log.lock().unwrap());
        assert!(!bytes.windows(19).any(|b| b == b"before-image-secret"));
        sweep(directory.path()).unwrap();
        assert!(path.exists());
        restore.log.lock().unwrap().cipher = Aes256Gcm::new_from_slice(&[0; 32]).unwrap();
        assert!(restore.read_pending(0).is_err());
        drop(restore);
        assert!(!path.exists());
        let orphan = directory.path().join("compare-orphan.restore");
        fs::write(&orphan, MAGIC).unwrap();
        sweep(directory.path()).unwrap();
        assert!(!orphan.exists());
    }

    #[test]
    fn undo_rejects_reordered_frames_and_incomplete_logs() {
        let directory = tempfile::tempdir().unwrap();
        let restore = RestoreHandle::create(directory.path()).unwrap();
        let record = || UndoRecord {
            row: 0,
            operation: Operation::Insert,
            id: 1.into(),
            before: None,
            after_hash: Some([1; 32]),
        };
        let entries = restore.prepare(&[record(), record()]).unwrap();
        for entry in entries {
            restore.started(entry).unwrap();
        }
        assert_eq!(restore.uncertain(), 2);
        let mut log = restore.log.lock().unwrap();
        let original = contents(&mut log);
        let start = log.frames[0].offset as usize;
        let second = log.frames[1].offset as usize;
        let mut changed = original.clone();
        changed[start..second].copy_from_slice(&original[second..]);
        log.file.seek(SeekFrom::Start(0)).unwrap();
        log.file.write_all(&changed).unwrap();
        drop(log);
        assert!(restore.read_pending(0).is_err());
        let mut log = restore.log.lock().unwrap();
        log.file.seek(SeekFrom::Start(0)).unwrap();
        log.file.write_all(&original).unwrap();
        log.file.as_file().set_len((original.len() - 1) as u64).unwrap();
        drop(log);
        assert!(restore.read_pending(0).is_err());
    }
}
