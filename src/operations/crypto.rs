use aes_gcm::aead::{Aead as _, KeyInit as _, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context as _, Result, bail};
use mongodb::bson::{Bson, Document, doc};
use sha2::{Digest as _, Sha256};

use super::model::{DocumentTarget, OperationId, RecoveryPayload, StoredPayload};

const PAYLOAD_VERSION: u32 = 1;
const NONCE_LEN: usize = 12;

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
        let before_hash = document_hash(&payload.before)?;
        let after_hash = document_hash(&payload.after)?;
        let plaintext = mongodb::bson::to_vec(&doc! {
            "version": PAYLOAD_VERSION as i64,
            "target": {
                "connection_id": payload.target.connection_id.to_string(),
                "database": payload.target.database.clone(),
                "collection": payload.target.collection.clone(),
                "id": payload.target.id.clone(),
            },
            "before": payload.before.clone(),
            "after": payload.after.clone(),
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

    pub(crate) fn decrypt(
        &self,
        operation_id: OperationId,
        connection_name: String,
        payload: &StoredPayload,
    ) -> Result<RecoveryPayload> {
        if payload.version != PAYLOAD_VERSION || payload.encrypted.len() <= NONCE_LEN {
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
        if envelope.get_i64("version").ok() != Some(PAYLOAD_VERSION as i64) {
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
        let before = envelope
            .get_document("before")
            .context("recovery payload has no before image")?
            .clone();
        let after =
            envelope.get_document("after").context("recovery payload has no after image")?.clone();

        if target_hash(&target)? != payload.target_hash
            || document_hash(&before)? != payload.before_hash
            || document_hash(&after)? != payload.after_hash
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

#[cfg(test)]
mod tests {
    use mongodb::bson::{Bson, doc};
    use uuid::Uuid;

    use super::*;

    #[test]
    fn payload_round_trip_and_aad_binding() {
        let cipher = HistoryCipher::new([7; 32]).unwrap();
        let operation_id = Uuid::new_v4();
        let payload = RecoveryPayload {
            target: DocumentTarget {
                connection_id: Uuid::new_v4(),
                connection_name: "Local".into(),
                database: "app".into(),
                collection: "users".into(),
                id: Bson::Int32(1),
            },
            before: doc! { "_id": 1, "secret": "before" },
            after: doc! { "_id": 1, "secret": "after" },
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
    }
}
