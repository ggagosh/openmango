use std::fs::File;
use std::io::{BufReader, BufWriter, Read as _, Write};
use std::path::Path;

use aes_gcm::aead::{Aead as _, KeyInit as _, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context as _, Result, bail};
use mongodb::bson::{Bson, Document, doc};
use sha2::{Digest as _, Sha256};

use super::model::{DocumentTarget, OperationId, RecoveryPayload, StoredPayload};

const LEGACY_PAYLOAD_VERSION: u32 = 1;
const PAYLOAD_VERSION: u32 = 2;
const NONCE_LEN: usize = 12;
const ARTIFACT_MAGIC: &[u8; 8] = b"OMHART01";
const ARTIFACT_CHUNK_BYTES: usize = 1024 * 1024;
const ARTIFACT_DATA_RECORD: u8 = 1;
const ARTIFACT_END_RECORD: u8 = 0;

pub(crate) struct HistoryCipher {
    cipher: Aes256Gcm,
}

impl HistoryCipher {
    pub(crate) fn new(key: [u8; 32]) -> Result<Self> {
        let cipher = Aes256Gcm::new_from_slice(&key).context("invalid history key")?;
        Ok(Self { cipher })
    }

    pub(crate) fn encrypt(
        &self,
        operation_id: OperationId,
        payload: &RecoveryPayload,
    ) -> Result<StoredPayload> {
        let target_hash = target_hash(&payload.target)?;
        let before_hash = document_state_hash(payload.before.as_ref())?;
        let after_hash = document_state_hash(payload.after.as_ref())?;
        let plaintext = mongodb::bson::to_vec(&doc! {
            "version": PAYLOAD_VERSION as i64,
            "target": {
                "connection_id": payload.target.connection_id.to_string(),
                "database": payload.target.database.clone(),
                "collection": payload.target.collection.clone(),
                "id": payload.target.id.clone(),
            },
            "before": payload.before.clone().map(Bson::Document).unwrap_or(Bson::Null),
            "after": payload.after.clone().map(Bson::Document).unwrap_or(Bson::Null),
        })
        .context("could not encode recovery payload")?;
        let nonce_bytes: [u8; NONCE_LEN] = rand::random();
        let aad = associated_data(PAYLOAD_VERSION, operation_id, &target_hash);
        let ciphertext = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), Payload { msg: &plaintext, aad: &aad })
            .map_err(|_| anyhow::anyhow!("could not encrypt recovery payload"))?;
        let mut encrypted = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        encrypted.extend_from_slice(&nonce_bytes);
        encrypted.extend_from_slice(&ciphertext);

        Ok(StoredPayload {
            version: PAYLOAD_VERSION,
            encrypted,
            target_hash,
            before_hash,
            after_hash,
        })
    }

    pub(crate) fn encrypt_artifact(
        &self,
        operation_id: OperationId,
        source: &Path,
        destination: &mut File,
    ) -> Result<()> {
        let mut source = BufReader::new(File::open(source)?);
        let mut destination = BufWriter::new(destination);
        destination.write_all(ARTIFACT_MAGIC)?;
        let mut buffer = vec![0u8; ARTIFACT_CHUNK_BYTES];
        let mut chunk = 0u64;
        loop {
            let bytes = source.read(&mut buffer)?;
            if bytes == 0 {
                write_artifact_record(
                    &self.cipher,
                    operation_id,
                    chunk,
                    ARTIFACT_END_RECORD,
                    &[],
                    &mut destination,
                )?;
                break;
            }
            write_artifact_record(
                &self.cipher,
                operation_id,
                chunk,
                ARTIFACT_DATA_RECORD,
                &buffer[..bytes],
                &mut destination,
            )?;
            chunk = chunk.checked_add(1).context("history artifact is too large")?;
        }
        destination.flush()?;
        Ok(())
    }

    pub(crate) fn decrypt_artifact(
        &self,
        operation_id: OperationId,
        source: &Path,
        destination: &Path,
    ) -> Result<()> {
        let mut source = BufReader::new(File::open(source)?);
        let mut magic = [0u8; ARTIFACT_MAGIC.len()];
        source.read_exact(&mut magic)?;
        if &magic != ARTIFACT_MAGIC {
            bail!("unsupported history artifact");
        }
        let mut destination = BufWriter::new(File::create(destination)?);
        let mut chunk = 0u64;
        loop {
            let mut record_type = [0u8; 1];
            source.read_exact(&mut record_type).context("history artifact is truncated")?;
            let record_type = record_type[0];
            if !matches!(record_type, ARTIFACT_DATA_RECORD | ARTIFACT_END_RECORD) {
                bail!("history artifact contains an invalid record");
            }
            let mut nonce = [0u8; NONCE_LEN];
            source.read_exact(&mut nonce)?;
            let mut length = [0u8; 4];
            source.read_exact(&mut length)?;
            let plaintext_len = u32::from_be_bytes(length) as usize;
            if plaintext_len > ARTIFACT_CHUNK_BYTES
                || (record_type == ARTIFACT_END_RECORD && plaintext_len != 0)
            {
                bail!("history artifact contains an invalid chunk");
            }
            let mut ciphertext = vec![0u8; plaintext_len + 16];
            source.read_exact(&mut ciphertext)?;
            let aad = artifact_associated_data(operation_id, chunk, record_type);
            let plaintext = self
                .cipher
                .decrypt(Nonce::from_slice(&nonce), Payload { msg: &ciphertext, aad: &aad })
                .map_err(|_| anyhow::anyhow!("could not authenticate history artifact"))?;
            if record_type == ARTIFACT_END_RECORD {
                let mut trailing = [0u8; 1];
                if source.read(&mut trailing)? != 0 {
                    bail!("history artifact contains trailing data");
                }
                break;
            }
            destination.write_all(&plaintext)?;
            chunk = chunk.checked_add(1).context("history artifact is too large")?;
        }
        destination.flush()?;
        Ok(())
    }

    pub(crate) fn decrypt(
        &self,
        operation_id: OperationId,
        connection_name: String,
        payload: &StoredPayload,
    ) -> Result<RecoveryPayload> {
        if !matches!(payload.version, LEGACY_PAYLOAD_VERSION | PAYLOAD_VERSION)
            || payload.encrypted.len() <= NONCE_LEN
        {
            bail!("unsupported recovery payload");
        }
        let (nonce, ciphertext) = payload.encrypted.split_at(NONCE_LEN);
        let aad = associated_data(payload.version, operation_id, &payload.target_hash);
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), Payload { msg: ciphertext, aad: &aad })
            .map_err(|_| anyhow::anyhow!("could not authenticate recovery payload"))?;
        let envelope: Document =
            mongodb::bson::from_slice(&plaintext).context("could not decode recovery payload")?;
        if envelope.get_i64("version").ok() != Some(payload.version as i64) {
            bail!("unsupported recovery payload");
        }
        let target = envelope.get_document("target").context("recovery payload has no target")?;
        let connection_id = target
            .get_str("connection_id")
            .context("recovery payload has no connection")?
            .parse()
            .context("recovery payload has an invalid connection")?;
        let target = DocumentTarget {
            connection_id,
            connection_name,
            database: target
                .get_str("database")
                .context("recovery payload has no database")?
                .to_string(),
            collection: target
                .get_str("collection")
                .context("recovery payload has no collection")?
                .to_string(),
            id: target.get("id").cloned().context("recovery payload has no document id")?,
        };
        let before = document_state(&envelope, "before", payload.version)?;
        let after = document_state(&envelope, "after", payload.version)?;

        if target_hash(&target)? != payload.target_hash
            || document_state_hash(before.as_ref())? != payload.before_hash
            || document_state_hash(after.as_ref())? != payload.after_hash
        {
            bail!("recovery payload integrity check failed");
        }

        Ok(RecoveryPayload { target, before, after })
    }
}

pub(crate) fn document_hash(document: &Document) -> Result<[u8; 32]> {
    let bytes = mongodb::bson::to_vec(document).context("could not encode document hash")?;
    Ok(Sha256::digest(bytes).into())
}

pub(crate) fn document_state_hash(document: Option<&Document>) -> Result<[u8; 32]> {
    match document {
        Some(document) => document_hash(document),
        None => Ok(Sha256::digest(b"openmango:document-absent:v1").into()),
    }
}

fn document_state(envelope: &Document, field: &str, version: u32) -> Result<Option<Document>> {
    if version == LEGACY_PAYLOAD_VERSION {
        return Ok(Some(
            envelope
                .get_document(field)
                .with_context(|| format!("recovery payload has no {field} image"))?
                .clone(),
        ));
    }
    match envelope.get(field) {
        Some(Bson::Document(document)) => Ok(Some(document.clone())),
        Some(Bson::Null) => Ok(None),
        _ => bail!("recovery payload has an invalid {field} image"),
    }
}

fn target_hash(target: &DocumentTarget) -> Result<[u8; 32]> {
    let identity = doc! {
        "connection_id": target.connection_id.to_string(),
        "database": target.database.clone(),
        "collection": target.collection.clone(),
        "id": Bson::Document(doc! { "value": target.id.clone() }),
    };
    let bytes = mongodb::bson::to_vec(&identity).context("could not encode target identity")?;
    Ok(Sha256::digest(bytes).into())
}

fn associated_data(version: u32, operation_id: OperationId, target_hash: &[u8; 32]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(4 + 16 + 32);
    aad.extend_from_slice(&version.to_be_bytes());
    aad.extend_from_slice(operation_id.as_bytes());
    aad.extend_from_slice(target_hash);
    aad
}

fn artifact_associated_data(operation_id: OperationId, chunk: u64, record_type: u8) -> Vec<u8> {
    let mut aad = Vec::with_capacity(16 + 8 + 1 + 24);
    aad.extend_from_slice(b"openmango:artifact:v1:");
    aad.extend_from_slice(operation_id.as_bytes());
    aad.extend_from_slice(&chunk.to_be_bytes());
    aad.push(record_type);
    aad
}

fn write_artifact_record(
    cipher: &Aes256Gcm,
    operation_id: OperationId,
    chunk: u64,
    record_type: u8,
    plaintext: &[u8],
    destination: &mut impl Write,
) -> Result<()> {
    let nonce: [u8; NONCE_LEN] = rand::random();
    let aad = artifact_associated_data(operation_id, chunk, record_type);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad: &aad })
        .map_err(|_| anyhow::anyhow!("could not encrypt history artifact"))?;
    destination.write_all(&[record_type])?;
    destination.write_all(&nonce)?;
    destination.write_all(&(plaintext.len() as u32).to_be_bytes())?;
    destination.write_all(&ciphertext)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use mongodb::bson::{Bson, doc};
    use tempfile::{NamedTempFile, TempDir};
    use uuid::Uuid;

    use super::*;

    fn target() -> DocumentTarget {
        DocumentTarget {
            connection_id: Uuid::new_v4(),
            connection_name: "Local".into(),
            database: "app".into(),
            collection: "users".into(),
            id: Bson::Int32(1),
        }
    }

    #[test]
    fn payload_round_trip_and_aad_binding() {
        let cipher = HistoryCipher::new([7; 32]).unwrap();
        let operation_id = Uuid::new_v4();
        let payload = RecoveryPayload {
            target: target(),
            before: Some(doc! { "_id": 1, "secret": "before" }),
            after: Some(doc! { "_id": 1, "secret": "after" }),
        };
        let stored = cipher.encrypt(operation_id, &payload).unwrap();
        let restored = cipher.decrypt(operation_id, "Local".into(), &stored).unwrap();
        assert_eq!(restored.before, payload.before);
        assert_eq!(restored.after, payload.after);
        assert!(cipher.decrypt(Uuid::new_v4(), "Local".into(), &stored).is_err());

        let mut tampered = stored.clone();
        tampered.encrypted[NONCE_LEN] ^= 1;
        assert!(cipher.decrypt(operation_id, "Local".into(), &tampered).is_err());

        let mut tampered = stored;
        tampered.target_hash[0] ^= 1;
        assert!(cipher.decrypt(operation_id, "Local".into(), &tampered).is_err());

        let absent = RecoveryPayload { after: None, ..payload };
        let absent_id = Uuid::new_v4();
        let stored = cipher.encrypt(absent_id, &absent).unwrap();
        assert_eq!(cipher.decrypt(absent_id, "Local".into(), &stored).unwrap().after, None);
    }

    #[test]
    fn chunked_artifact_round_trip_authenticates_every_chunk_and_operation() {
        let directory = TempDir::new().unwrap();
        let source = directory.path().join("source.archive");
        let restored = directory.path().join("restored.archive");
        let bytes =
            (0..ARTIFACT_CHUNK_BYTES * 2 + 37).map(|index| (index % 251) as u8).collect::<Vec<_>>();
        std::fs::write(&source, &bytes).unwrap();
        let cipher = HistoryCipher::new([8; 32]).unwrap();
        let operation_id = Uuid::new_v4();
        let mut encrypted = NamedTempFile::new_in(directory.path()).unwrap();

        cipher.encrypt_artifact(operation_id, &source, encrypted.as_file_mut()).unwrap();
        encrypted.as_file_mut().sync_all().unwrap();
        cipher.decrypt_artifact(operation_id, encrypted.path(), &restored).unwrap();

        assert_eq!(std::fs::read(restored).unwrap(), bytes);
        assert!(
            cipher
                .decrypt_artifact(
                    Uuid::new_v4(),
                    encrypted.path(),
                    &directory.path().join("wrong.archive"),
                )
                .is_err()
        );
    }

    #[test]
    fn decrypts_legacy_document_only_payloads() {
        let cipher = HistoryCipher::new([9; 32]).unwrap();
        let operation_id = Uuid::new_v4();
        let target = target();
        let before = doc! { "_id": 1, "value": "before" };
        let after = doc! { "_id": 1, "value": "after" };
        let target_hash = target_hash(&target).unwrap();
        let plaintext = mongodb::bson::to_vec(&doc! {
            "version": LEGACY_PAYLOAD_VERSION as i64,
            "target": {
                "connection_id": target.connection_id.to_string(),
                "database": target.database.clone(),
                "collection": target.collection.clone(),
                "id": target.id.clone(),
            },
            "before": before.clone(),
            "after": after.clone(),
        })
        .unwrap();
        let nonce = [3; NONCE_LEN];
        let aad = associated_data(LEGACY_PAYLOAD_VERSION, operation_id, &target_hash);
        let ciphertext = cipher
            .cipher
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: &plaintext, aad: &aad })
            .unwrap();
        let stored = StoredPayload {
            version: LEGACY_PAYLOAD_VERSION,
            encrypted: nonce.into_iter().chain(ciphertext).collect(),
            target_hash,
            before_hash: document_hash(&before).unwrap(),
            after_hash: document_hash(&after).unwrap(),
        };

        let restored = cipher.decrypt(operation_id, "Local".into(), &stored).unwrap();

        assert_eq!(restored.before, Some(before));
        assert_eq!(restored.after, Some(after));
    }
}
