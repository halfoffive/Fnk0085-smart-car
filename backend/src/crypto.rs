//! AES-128-GCM AEAD 加解密。
//!
//! 设备与后端共享 token；token 经 SHA-256 → 截断 16B 派生 AES-128 密钥。
//! 在线格式：`nonce(12B) || ciphertext || tag(16B)`，aes-gcm 默认把 tag 追加到密文末尾。
//!
//! `AeadKey` 为不可变句柄；`derive_key` 与 `seal`/`open` 均为纯函数（无内部状态）。

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Key, Nonce};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// AES-128-GCM 密钥（16 字节）
pub const KEY_LEN: usize = 16;
/// AES-128-GCM nonce 长度（12 字节）
pub const NONCE_LEN: usize = 12;
/// 认证 tag 长度（16 字节）
pub const TAG_LEN: usize = 16;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("密文长度不足（至少 nonce+tag={0}）")]
    CiphertextTooShort(usize),
    #[error("AEAD 解密失败")]
    DecryptFailed,
}

/// 不可变 AES-128-GCM 句柄
#[derive(Clone)]
pub struct AeadKey {
    cipher: Aes128Gcm,
}

impl AeadKey {
    /// 从原始 16 字节构造
    pub fn from_bytes(raw: &[u8; KEY_LEN]) -> Self {
        let key = Key::<Aes128Gcm>::from_slice(raw);
        Self {
            cipher: Aes128Gcm::new(key),
        }
    }

    /// 加密：返回 nonce(12) || ciphertext || tag(16) 的拼接字节
    pub fn seal(&self, nonce: &[u8; NONCE_LEN], plaintext: &[u8]) -> bytes::Bytes {
        let nonce = Nonce::from_slice(nonce);
        let ct = self
            .cipher
            .encrypt(nonce, plaintext)
            .expect("AES-GCM 加密不会失败（输入合法）");
        // 拼接 nonce + ciphertext_with_tag
        let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
        out.extend_from_slice(nonce);
        out.extend_from_slice(&ct);
        bytes::Bytes::from(out)
    }

    /// 解密：输入为 nonce(12) || ciphertext || tag(16)
    pub fn open(&self, wire: &[u8]) -> Result<bytes::Bytes, CryptoError> {
        if wire.len() < NONCE_LEN + TAG_LEN {
            return Err(CryptoError::CiphertextTooShort(NONCE_LEN + TAG_LEN));
        }
        let (nonce_bytes, ct) = wire.split_at(NONCE_LEN);
        let nonce = Nonce::from_slice(nonce_bytes.try_into().unwrap());
        match self.cipher.decrypt(nonce, ct) {
            Ok(pt) => Ok(bytes::Bytes::from(pt)),
            Err(_) => Err(CryptoError::DecryptFailed),
        }
    }
}

/// 从 token 派生 AES-128 密钥：SHA-256(token)[0..16]
pub fn derive_key(token: &str) -> AeadKey {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let digest = hasher.finalize();
    let mut raw = [0u8; KEY_LEN];
    raw.copy_from_slice(&digest[..KEY_LEN]);
    AeadKey::from_bytes(&raw)
}

/// 便利函数：用 token 派生密钥后加密
pub fn seal(plain: &[u8], key: &AeadKey, nonce: &[u8; NONCE_LEN]) -> bytes::Bytes {
    key.seal(nonce, plain)
}

/// 便利函数：用 token 派生密钥后解密
pub fn open(wire: &[u8], key: &AeadKey) -> Result<bytes::Bytes, CryptoError> {
    key.open(wire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = derive_key("change-me-please");
        let nonce = [0u8; NONCE_LEN];
        let plain = b"hello Fnk0085";
        let sealed = key.seal(&nonce, plain);
        assert_eq!(sealed.len(), NONCE_LEN + plain.len() + TAG_LEN);
        let opened = key.open(&sealed).expect("解密成功");
        assert_eq!(opened.as_ref(), plain);
    }

    #[test]
    fn rejects_tampered() {
        let key = derive_key("change-me-please");
        let nonce = [0u8; NONCE_LEN];
        let mut sealed = key.seal(&nonce, b"data").to_vec();
        sealed[NONCE_LEN] ^= 0xff; // 篡改密文
        assert!(key.open(&sealed).is_err());
    }

    #[test]
    fn different_tokens_yield_different_keys() {
        let a = derive_key("token-a");
        let b = derive_key("token-b");
        let nonce = [0u8; NONCE_LEN];
        let sealed = a.seal(&nonce, b"x");
        assert!(b.open(&sealed).is_err());
    }
}
