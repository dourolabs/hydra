use aes_gcm::{
    Aes256Gcm, Key, KeyInit, Nonce,
    aead::{Aead, OsRng, rand_core::RngCore},
};

/// A reference to a secret, including its name and whether it is system-internal.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SecretRef {
    pub name: String,
    pub internal: bool,
}

/// Well-known secret names used for auto-injection logic.
pub const SECRET_OPENAI_API_KEY: &str = "OPENAI_API_KEY";
pub const SECRET_ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";
pub const SECRET_CLAUDE_CODE_OAUTH_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";
pub const SECRET_GH_TOKEN: &str = "GH_TOKEN";
pub const SECRET_GITHUB_TOKEN: &str = "GITHUB_TOKEN";
pub const SECRET_GITHUB_REFRESH_TOKEN: &str = "GITHUB_REFRESH_TOKEN";

/// Validates a user-provided secret name.
///
/// Rules:
/// - 1 to 128 characters
/// - Only uppercase ASCII letters, digits, and underscores
/// - Must start with an uppercase letter
/// - Must not start with `HYDRA_` (reserved for system env vars)
pub fn validate_secret_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 128 {
        return Err("secret name must be between 1 and 128 characters".to_string());
    }
    if !name.starts_with(|c: char| c.is_ascii_uppercase()) {
        return Err("secret name must start with an uppercase letter".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(
            "secret name must contain only uppercase letters, digits, and underscores".to_string(),
        );
    }
    if name.starts_with("HYDRA_") {
        return Err("secret name must not start with HYDRA_ (reserved prefix)".to_string());
    }
    Ok(())
}

const NONCE_SIZE: usize = 12;

/// Manages encryption and decryption of user secrets using AES-256-GCM.
pub struct SecretManager {
    cipher: Aes256Gcm,
}

impl SecretManager {
    /// Creates a new SecretManager from a 32-byte encryption key.
    #[allow(deprecated)] // GenericArray from aes-gcm 0.10.x
    pub fn new(key: [u8; 32]) -> Self {
        let aes_key: Key<Aes256Gcm> = key.into();
        let cipher = Aes256Gcm::new(&aes_key);
        Self { cipher }
    }

    /// Creates a new SecretManager from a base64-encoded key string.
    ///
    /// Returns an error if the key is not valid base64 or is not exactly 32 bytes.
    pub fn from_base64(encoded: &str) -> Result<Self, SecretManagerError> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .map_err(|e| SecretManagerError::InvalidKey(format!("invalid base64: {e}")))?;
        let key: [u8; 32] = bytes.try_into().map_err(|v: Vec<u8>| {
            SecretManagerError::InvalidKey(format!("key must be exactly 32 bytes, got {}", v.len()))
        })?;
        Ok(Self::new(key))
    }

    /// Encrypts a plaintext string, returning `nonce || ciphertext`.
    #[allow(deprecated)] // GenericArray from aes-gcm 0.10.x
    pub fn encrypt(&self, plaintext: &str) -> Result<Vec<u8>, SecretManagerError> {
        let mut nonce_bytes = [0u8; NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| SecretManagerError::EncryptionFailed(e.to_string()))?;

        let mut result = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);
        Ok(result)
    }

    /// Decrypts a `nonce || ciphertext` blob back to a plaintext string.
    #[allow(deprecated)] // GenericArray from aes-gcm 0.10.x
    pub fn decrypt(&self, data: &[u8]) -> Result<String, SecretManagerError> {
        if data.len() < NONCE_SIZE {
            return Err(SecretManagerError::DecryptionFailed(
                "data too short to contain nonce".to_string(),
            ));
        }

        let (nonce_bytes, ciphertext) = data.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = self
            .cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| SecretManagerError::DecryptionFailed(e.to_string()))?;

        String::from_utf8(plaintext)
            .map_err(|e| SecretManagerError::DecryptionFailed(format!("invalid UTF-8: {e}")))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecretManagerError {
    #[error("Invalid encryption key: {0}")]
    InvalidKey(String),
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),
    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        [42u8; 32]
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let manager = SecretManager::new(test_key());
        let plaintext = "sk-test-key-12345";

        let encrypted = manager.encrypt(plaintext).unwrap();
        let decrypted = manager.decrypt(&encrypted).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encrypt_produces_different_ciphertexts() {
        let manager = SecretManager::new(test_key());
        let plaintext = "same-value";

        let a = manager.encrypt(plaintext).unwrap();
        let b = manager.encrypt(plaintext).unwrap();

        // Different nonces should produce different ciphertexts
        assert_ne!(a, b);
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let manager1 = SecretManager::new([1u8; 32]);
        let manager2 = SecretManager::new([2u8; 32]);

        let encrypted = manager1.encrypt("secret").unwrap();
        assert!(manager2.decrypt(&encrypted).is_err());
    }

    #[test]
    fn decrypt_too_short_data_fails() {
        let manager = SecretManager::new(test_key());
        assert!(manager.decrypt(&[0u8; 5]).is_err());
    }

    #[test]
    fn from_base64_valid_key() {
        use base64::Engine;
        let key = [7u8; 32];
        let encoded = base64::engine::general_purpose::STANDARD.encode(key);
        let manager = SecretManager::from_base64(&encoded).unwrap();

        let encrypted = manager.encrypt("hello").unwrap();
        let decrypted = manager.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, "hello");
    }

    #[test]
    fn from_base64_wrong_length_fails() {
        use base64::Engine;
        let short = base64::engine::general_purpose::STANDARD.encode([0u8; 16]);
        assert!(SecretManager::from_base64(&short).is_err());
    }

    #[test]
    fn from_base64_invalid_base64_fails() {
        assert!(SecretManager::from_base64("not-valid-base64!!!").is_err());
    }

    #[test]
    fn encrypt_decrypt_empty_string() {
        let manager = SecretManager::new(test_key());
        let encrypted = manager.encrypt("").unwrap();
        let decrypted = manager.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn encrypt_decrypt_unicode() {
        let manager = SecretManager::new(test_key());
        let plaintext = "secret-with-unicode-🔑";
        let encrypted = manager.encrypt(plaintext).unwrap();
        let decrypted = manager.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn validate_secret_name_valid() {
        assert!(validate_secret_name("OPENAI_API_KEY").is_ok());
        assert!(validate_secret_name("MY_SECRET").is_ok());
        assert!(validate_secret_name("A").is_ok());
        assert!(validate_secret_name("SECRET123").is_ok());
        assert!(validate_secret_name("A_B_C_1_2_3").is_ok());
    }

    #[test]
    fn validate_secret_name_empty() {
        assert!(validate_secret_name("").is_err());
    }

    #[test]
    fn validate_secret_name_too_long() {
        let long_name = "A".repeat(129);
        assert!(validate_secret_name(&long_name).is_err());
        // Exactly 128 should be fine
        let max_name = "A".repeat(128);
        assert!(validate_secret_name(&max_name).is_ok());
    }

    #[test]
    fn validate_secret_name_must_start_with_uppercase() {
        assert!(validate_secret_name("1SECRET").is_err());
        assert!(validate_secret_name("_SECRET").is_err());
        assert!(validate_secret_name("aSecret").is_err());
    }

    #[test]
    fn validate_secret_name_invalid_chars() {
        assert!(validate_secret_name("MY-SECRET").is_err());
        assert!(validate_secret_name("MY SECRET").is_err());
        assert!(validate_secret_name("my_secret").is_err());
        assert!(validate_secret_name("Secret").is_err());
    }

    #[test]
    fn validate_secret_name_hydra_prefix_reserved() {
        assert!(validate_secret_name("HYDRA_TOKEN").is_err());
        assert!(validate_secret_name("HYDRA_SERVER_URL").is_err());
        // METIS without underscore is fine
        assert!(validate_secret_name("METISKEY").is_ok());
    }
}
