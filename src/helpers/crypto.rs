//! Password encryption/decryption using AES-256-GCM + Argon2id.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context, Result, bail};
use argon2::Argon2;
use base64::Engine as _;
use rand::RngExt as _;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

/// Encrypt a password with a passphrase using Argon2id key derivation + AES-256-GCM.
/// Returns base64-encoded `salt || nonce || ciphertext+tag`.
pub fn encrypt_password(password: &str, passphrase: &str) -> Result<String> {
    let mut rng = rand::rng();
    let mut salt = [0u8; SALT_LEN];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rng.fill(&mut salt);
    rng.fill(&mut nonce_bytes);

    let key = derive_key(passphrase, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key).context("failed to create cipher")?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext =
        cipher.encrypt(nonce, password.as_bytes()).map_err(|e| anyhow::anyhow!("{e}"))?;

    let mut blob = Vec::with_capacity(SALT_LEN + NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(&salt);
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext);

    Ok(base64::engine::general_purpose::STANDARD.encode(&blob))
}

/// Decrypt a base64-encoded `salt || nonce || ciphertext+tag` blob with a passphrase.
pub fn decrypt_password(encrypted_b64: &str, passphrase: &str) -> Result<String> {
    let blob = base64::engine::general_purpose::STANDARD
        .decode(encrypted_b64)
        .context("invalid base64")?;

    if blob.len() < SALT_LEN + NONCE_LEN + 1 {
        bail!("encrypted data too short");
    }

    let salt = &blob[..SALT_LEN];
    let nonce_bytes = &blob[SALT_LEN..SALT_LEN + NONCE_LEN];
    let ciphertext = &blob[SALT_LEN + NONCE_LEN..];

    let key = derive_key(passphrase, salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key).context("failed to create cipher")?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("decryption failed — wrong passphrase?"))?;

    String::from_utf8(plaintext).context("decrypted data is not valid UTF-8")
}

fn derive_key(passphrase: &str, salt: &[u8]) -> Result<[u8; KEY_LEN]> {
    let mut key = [0u8; KEY_LEN];
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| anyhow::anyhow!("key derivation failed: {e}"))?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let password = "my_secret_p@ssw0rd!";
        let passphrase = "export-passphrase-123";
        let encrypted = encrypt_password(password, passphrase).unwrap();
        let decrypted = decrypt_password(&encrypted, passphrase).unwrap();
        assert_eq!(decrypted, password);
    }

    #[test]
    fn wrong_passphrase_fails() {
        let encrypted = encrypt_password("secret", "correct-passphrase").unwrap();
        let result = decrypt_password(&encrypted, "wrong-passphrase");
        assert!(result.is_err());
    }

    #[test]
    fn empty_password() {
        let encrypted = encrypt_password("", "passphrase").unwrap();
        let decrypted = decrypt_password(&encrypted, "passphrase").unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn different_encryptions_differ() {
        let a = encrypt_password("same", "pass").unwrap();
        let b = encrypt_password("same", "pass").unwrap();
        assert_ne!(a, b, "random salt/nonce should produce different ciphertexts");
    }
}
