use aes_gcm::aead::{Aead as _, KeyInit as _, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context as _, Result, bail};
use mongodb::bson::{Bson, Document, doc};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

const PAYLOAD_VERSION: i64 = 1;
const NONCE_LEN: usize = 12;

pub(crate) struct HistoryCipher {
    cipher: Aes256Gcm,
}

#[derive(Debug, Clone)]
pub(crate) struct RecoveryPayload {
    pub document_key: Document,
    pub before: Option<Document>,
    pub after: Option<Document>,
}

impl HistoryCipher {
    pub(crate) fn new(key: [u8; 32]) -> Result<Self> {
        Ok(Self { cipher: Aes256Gcm::new_from_slice(&key).context("invalid History key")? })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encrypt_item(
        &self,
        item_id: Uuid,
        connection_id: Uuid,
        database: &str,
        collection: &str,
        family: &str,
        token_hash: &[u8],
        payload: &RecoveryPayload,
    ) -> Result<Vec<u8>> {
        let plaintext = mongodb::bson::to_vec(&doc! {
            "version": PAYLOAD_VERSION,
            "documentKey": payload.document_key.clone(),
            "before": payload.before.clone().map(Bson::Document).unwrap_or(Bson::Null),
            "after": payload.after.clone().map(Bson::Document).unwrap_or(Bson::Null),
        })?;
        self.encrypt(
            &item_aad(item_id, connection_id, database, collection, family, token_hash),
            &plaintext,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decrypt_item(
        &self,
        item_id: Uuid,
        connection_id: Uuid,
        database: &str,
        collection: &str,
        family: &str,
        token_hash: &[u8],
        encrypted: &[u8],
    ) -> Result<RecoveryPayload> {
        let plaintext = self.decrypt(
            &item_aad(item_id, connection_id, database, collection, family, token_hash),
            encrypted,
        )?;
        let document: Document = mongodb::bson::from_slice(&plaintext)?;
        if document.get_i64("version").ok() != Some(PAYLOAD_VERSION) {
            bail!("unsupported History payload");
        }
        Ok(RecoveryPayload {
            document_key: document.get_document("documentKey")?.clone(),
            before: optional_document(&document, "before")?,
            after: optional_document(&document, "after")?,
        })
    }

    pub(crate) fn encrypt_cursor(
        &self,
        connection_id: Uuid,
        database: &str,
        token: &[u8],
    ) -> Result<Vec<u8>> {
        self.encrypt(&cursor_aad(connection_id, database), token)
    }

    pub(crate) fn decrypt_cursor(
        &self,
        connection_id: Uuid,
        database: &str,
        encrypted: &[u8],
    ) -> Result<Vec<u8>> {
        self.decrypt(&cursor_aad(connection_id, database), encrypted)
    }

    fn encrypt(&self, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
        let nonce: [u8; NONCE_LEN] = rand::random();
        let ciphertext = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad })
            .map_err(|_| anyhow::anyhow!("could not encrypt History payload"))?;
        let mut stored = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        stored.extend_from_slice(&nonce);
        stored.extend_from_slice(&ciphertext);
        Ok(stored)
    }

    fn decrypt(&self, aad: &[u8], encrypted: &[u8]) -> Result<Vec<u8>> {
        if encrypted.len() <= NONCE_LEN {
            bail!("History payload is truncated");
        }
        self.cipher
            .decrypt(
                Nonce::from_slice(&encrypted[..NONCE_LEN]),
                Payload { msg: &encrypted[NONCE_LEN..], aad },
            )
            .map_err(|_| anyhow::anyhow!("could not authenticate History payload"))
    }
}

pub(crate) fn token_hash(token: &[u8]) -> [u8; 32] {
    Sha256::digest(token).into()
}

fn item_aad(
    item_id: Uuid,
    connection_id: Uuid,
    database: &str,
    collection: &str,
    family: &str,
    token_hash: &[u8],
) -> Vec<u8> {
    let mut aad = format!(
        "openmango-history-item-v1\0{item_id}\0{connection_id}\0{database}\0{collection}\0{family}\0"
    )
    .into_bytes();
    aad.extend_from_slice(token_hash);
    aad
}

fn cursor_aad(connection_id: Uuid, database: &str) -> Vec<u8> {
    format!("openmango-history-cursor-v1\0{connection_id}\0{database}").into_bytes()
}

fn optional_document(document: &Document, key: &str) -> Result<Option<Document>> {
    match document.get(key) {
        Some(Bson::Document(value)) => Ok(Some(value.clone())),
        Some(Bson::Null) => Ok(None),
        _ => bail!("History payload has invalid {key}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_payload_authenticates_ciphertext_and_aad() {
        let cipher = HistoryCipher::new([7; 32]).unwrap();
        let item_id = Uuid::new_v4();
        let connection_id = Uuid::new_v4();
        let hash = [3; 32];
        let payload = RecoveryPayload {
            document_key: doc! { "_id": 1 },
            before: Some(doc! { "_id": 1, "value": "before" }),
            after: Some(doc! { "_id": 1, "value": "after" }),
        };
        let encrypted = cipher
            .encrypt_item(item_id, connection_id, "db", "items", "update", &hash, &payload)
            .unwrap();
        assert_eq!(
            cipher
                .decrypt_item(item_id, connection_id, "db", "items", "update", &hash, &encrypted)
                .unwrap()
                .after,
            payload.after
        );

        let mut tampered = encrypted.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(
            cipher
                .decrypt_item(item_id, connection_id, "db", "items", "update", &hash, &tampered)
                .is_err()
        );
        assert!(
            cipher
                .decrypt_item(item_id, connection_id, "db", "other", "update", &hash, &encrypted)
                .is_err()
        );
    }
}
