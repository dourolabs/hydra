#[allow(deprecated)]
use aes_gcm::aead::generic_array::GenericArray;
use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, OsRng, rand_core::RngCore},
};

const NONCE_SIZE: usize = 12;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("encryption failed: {0}")]
    EncryptionFailed(String),
    #[error("decryption failed: {0}")]
    DecryptionFailed(String),
    #[error("invalid ciphertext: too short")]
    CiphertextTooShort,
    #[error("invalid encryption key: {0}")]
    InvalidKey(String),
}

pub struct SecretManager {
    cipher: Aes256Gcm,
}

impl SecretManager {
    /// Creates a new SecretManager from a 32-byte encryption key.
    pub fn new(key: &[u8; 32]) -> Self {
        let cipher = Aes256Gcm::new(key.into());
        Self { cipher }
    }

    /// Creates a new SecretManager from a base64-encoded 32-byte key.
    pub fn from_base64(encoded_key: &str) -> Result<Self, SecretError> {
        use base64::Engine;
        let key_bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded_key.trim())
            .map_err(|e| SecretError::InvalidKey(format!("base64 decode error: {e}")))?;
        if key_bytes.len() != 32 {
            return Err(SecretError::InvalidKey(format!(
                "expected 32 bytes, got {}",
                key_bytes.len()
            )));
        }
        let key: [u8; 32] = key_bytes.try_into().unwrap();
        Ok(Self::new(&key))
    }

    /// Encrypts a plaintext string and returns `nonce || ciphertext`.
    #[allow(deprecated)] // GenericArray from aes-gcm 0.10.x dependency
    pub fn encrypt(&self, plaintext: &str) -> Result<Vec<u8>, SecretError> {
        let mut nonce_bytes = [0u8; NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = GenericArray::from(nonce_bytes);

        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|e| SecretError::EncryptionFailed(e.to_string()))?;

        let mut result = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);
        Ok(result)
    }

    /// Decrypts a ciphertext produced by `encrypt()` and returns the plaintext string.
    #[allow(deprecated)] // GenericArray from aes-gcm 0.10.x dependency
    pub fn decrypt(&self, data: &[u8]) -> Result<String, SecretError> {
        if data.len() < NONCE_SIZE {
            return Err(SecretError::CiphertextTooShort);
        }

        let (nonce_bytes, ciphertext) = data.split_at(NONCE_SIZE);
        let nonce_arr: [u8; NONCE_SIZE] = nonce_bytes.try_into().unwrap();
        let nonce = GenericArray::from(nonce_arr);

        let plaintext_bytes = self
            .cipher
            .decrypt(&nonce, ciphertext)
            .map_err(|e| SecretError::DecryptionFailed(e.to_string()))?;

        String::from_utf8(plaintext_bytes)
            .map_err(|e| SecretError::DecryptionFailed(format!("invalid UTF-8: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        [42u8; 32]
    }

    fn different_key() -> [u8; 32] {
        [99u8; 32]
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let manager = SecretManager::new(&test_key());
        let plaintext = "sk-abc123-secret-api-key";

        let encrypted = manager.encrypt(plaintext).unwrap();
        let decrypted = manager.decrypt(&encrypted).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encrypt_produces_different_ciphertexts() {
        let manager = SecretManager::new(&test_key());
        let plaintext = "same-secret";

        let encrypted1 = manager.encrypt(plaintext).unwrap();
        let encrypted2 = manager.encrypt(plaintext).unwrap();

        // Different nonces produce different ciphertexts
        assert_ne!(encrypted1, encrypted2);

        // But both decrypt to the same plaintext
        assert_eq!(manager.decrypt(&encrypted1).unwrap(), plaintext);
        assert_eq!(manager.decrypt(&encrypted2).unwrap(), plaintext);
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let manager1 = SecretManager::new(&test_key());
        let manager2 = SecretManager::new(&different_key());

        let encrypted = manager1.encrypt("secret-value").unwrap();
        let result = manager2.decrypt(&encrypted);

        assert!(result.is_err());
        assert!(matches!(result, Err(SecretError::DecryptionFailed(_))));
    }

    #[test]
    fn decrypt_too_short_data_fails() {
        let manager = SecretManager::new(&test_key());
        let result = manager.decrypt(&[0u8; 5]);

        assert!(result.is_err());
        assert!(matches!(result, Err(SecretError::CiphertextTooShort)));
    }

    #[test]
    fn decrypt_empty_data_fails() {
        let manager = SecretManager::new(&test_key());
        let result = manager.decrypt(&[]);

        assert!(result.is_err());
        assert!(matches!(result, Err(SecretError::CiphertextTooShort)));
    }

    #[test]
    fn from_base64_valid_key() {
        use base64::Engine;
        let key = test_key();
        let encoded = base64::engine::general_purpose::STANDARD.encode(key);

        let manager = SecretManager::from_base64(&encoded).unwrap();
        let encrypted = manager.encrypt("test").unwrap();
        let decrypted = manager.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, "test");
    }

    #[test]
    fn from_base64_wrong_length_fails() {
        use base64::Engine;
        let short_key = [0u8; 16];
        let encoded = base64::engine::general_purpose::STANDARD.encode(short_key);

        let result = SecretManager::from_base64(&encoded);
        assert!(matches!(result, Err(SecretError::InvalidKey(_))));
    }

    #[test]
    fn from_base64_invalid_encoding_fails() {
        let result = SecretManager::from_base64("not-valid-base64!!!");
        assert!(matches!(result, Err(SecretError::InvalidKey(_))));
    }

    #[test]
    fn encrypt_decrypt_empty_string() {
        let manager = SecretManager::new(&test_key());
        let encrypted = manager.encrypt("").unwrap();
        let decrypted = manager.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn encrypt_decrypt_unicode() {
        let manager = SecretManager::new(&test_key());
        let plaintext = "secret-with-unicode-\u{1F512}";
        let encrypted = manager.encrypt(plaintext).unwrap();
        let decrypted = manager.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }
}
