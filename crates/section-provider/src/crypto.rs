//! AES-256-GCM encryption for credential storage.
//!
//! Encrypted format: hex(nonce ‖ ciphertext ‖ tag)
//! - nonce: 12 bytes
//! - ciphertext: variable length (same as plaintext)
//! - tag: 16 bytes (appended by `seal_in_place_append_tag`)

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};
use std::path::Path;

/// AES-256-GCM key length in bytes.
const KEY_LEN: usize = 32;

/// Load an existing key from `path`, or generate a new one and persist it.
pub fn load_or_generate_key(path: &Path) -> anyhow::Result<Vec<u8>> {
    if path.exists() {
        let key = std::fs::read(path)?;
        if key.len() != KEY_LEN {
            anyhow::bail!(
                "encryption key file has invalid length {} (expected {})",
                key.len(),
                KEY_LEN,
            );
        }
        Ok(key)
    } else {
        let rng = SystemRandom::new();
        let mut key = vec![0u8; KEY_LEN];
        rng.fill(&mut key)
            .map_err(|_| anyhow::anyhow!("failed to generate random key"))?;

        // Write atomically-ish: write to a temp file, then rename.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, &key)?;

        // Restrict permissions on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }

        Ok(key)
    }
}

/// Encrypt `plaintext` with AES-256-GCM. Returns hex-encoded `nonce ‖ ciphertext ‖ tag`.
pub fn encrypt(key_bytes: &[u8], plaintext: &[u8]) -> anyhow::Result<String> {
    let unbound = UnboundKey::new(&AES_256_GCM, key_bytes)
        .map_err(|_| anyhow::anyhow!("invalid AES-256-GCM key"))?;
    let key = LessSafeKey::new(unbound);

    let rng = SystemRandom::new();
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rng.fill(&mut nonce_bytes)
        .map_err(|_| anyhow::anyhow!("failed to generate nonce"))?;

    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    let mut in_out = plaintext.to_vec();
    key.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
        .map_err(|_| anyhow::anyhow!("encryption failed"))?;

    // Prepend nonce to ciphertext+tag.
    let mut result = Vec::with_capacity(NONCE_LEN + in_out.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&in_out);

    Ok(to_hex(&result))
}

/// Decrypt hex-encoded `nonce ‖ ciphertext ‖ tag`. Returns the plaintext bytes.
pub fn decrypt(key_bytes: &[u8], hex_encoded: &str) -> anyhow::Result<Vec<u8>> {
    let data = from_hex(hex_encoded)?;

    if data.len() < NONCE_LEN {
        anyhow::bail!("encrypted data too short");
    }

    let (nonce_bytes, ciphertext_and_tag) = data.split_at(NONCE_LEN);
    let nonce = Nonce::try_assume_unique_for_key(nonce_bytes)
        .map_err(|_| anyhow::anyhow!("invalid nonce"))?;

    let unbound = UnboundKey::new(&AES_256_GCM, key_bytes)
        .map_err(|_| anyhow::anyhow!("invalid AES-256-GCM key"))?;
    let key = LessSafeKey::new(unbound);

    let mut in_out = ciphertext_and_tag.to_vec();
    let plaintext = key
        .open_in_place(nonce, Aad::empty(), &mut in_out)
        .map_err(|_| anyhow::anyhow!("decryption failed (wrong key or corrupted data)"))?;

    Ok(plaintext.to_vec())
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn from_hex(s: &str) -> anyhow::Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        anyhow::bail!("hex string has odd length");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|e| anyhow::anyhow!("invalid hex at position {}: {}", i, e))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let rng = SystemRandom::new();
        let mut key = [0u8; KEY_LEN];
        rng.fill(&mut key).unwrap();

        let plaintext = b"hello, world!";
        let encrypted = encrypt(&key, plaintext).unwrap();
        let decrypted = decrypt(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn wrong_key_fails() {
        let rng = SystemRandom::new();
        let mut key1 = [0u8; KEY_LEN];
        let mut key2 = [0u8; KEY_LEN];
        rng.fill(&mut key1).unwrap();
        rng.fill(&mut key2).unwrap();

        let encrypted = encrypt(&key1, b"secret").unwrap();
        assert!(decrypt(&key2, &encrypted).is_err());
    }

    #[test]
    fn key_file_round_trip() {
        let dir = tempfile::TempDir::new().unwrap();
        let key_path = dir.path().join("section.key");

        let key1 = load_or_generate_key(&key_path).unwrap();
        assert_eq!(key1.len(), KEY_LEN);

        let key2 = load_or_generate_key(&key_path).unwrap();
        assert_eq!(key1, key2);
    }
}
