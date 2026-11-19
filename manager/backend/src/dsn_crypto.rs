//! DSN 落库加密。
//!
//! `sys_store.pg_dsn` 含数据库口令，不能明文落库。算法由 `manager.xml`
//! 的 `security.dsn_cipher` 指定：
//!
//! * `des`（默认）— DES-CBC + PKCS#7；
//! * `sm4` — SM4-CTR（`libsm`）。
//!
//! 密钥取 `security.dsn_key`（不足补长、超长截断）；未配置时由 `jwt_secret`
//! 派生，保证零配置可启动。密文格式 `enc:v1:<alg>:<base64(iv||ct)>`，
//! 不带该前缀的历史明文原样返回，可平滑升级；算法按密文自带标签解析，
//! 因此切换 `dsn_cipher` 不会让旧数据读不出来（密钥需保持一致）。

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use rand::RngCore;

type DesEnc = cbc::Encryptor<des::Des>;
type DesDec = cbc::Decryptor<des::Des>;

const PREFIX: &str = "enc:v1:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DsnAlg {
    Des,
    Sm4,
}

impl DsnAlg {
    fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "des" => Some(Self::Des),
            "sm4" => Some(Self::Sm4),
            _ => None,
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Self::Des => "des",
            Self::Sm4 => "sm4",
        }
    }

    fn key_len(self) -> usize {
        match self {
            Self::Des => 8,
            Self::Sm4 => 16,
        }
    }

    fn iv_len(self) -> usize {
        self.key_len()
    }
}

#[derive(Debug, Clone)]
pub struct DsnCipher {
    alg: DsnAlg,
    /// 原始密钥材料（配置的 dsn_key 或 jwt_secret），按算法长度再派生。
    material: Vec<u8>,
}

impl DsnCipher {
    /// `method` 为 `des` / `sm4`（未知值回退 DES）；`key` 为空时用 `secret` 派生。
    pub fn from_config(method: &str, key: Option<&str>, secret: &str) -> Self {
        let alg = DsnAlg::parse(method).unwrap_or_else(|| {
            tracing::warn!("未知的 dsn_cipher='{method}'，回退 des");
            DsnAlg::Des
        });
        let material = key
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.as_bytes().to_vec())
            .unwrap_or_else(|| {
                tracing::warn!(
                    "security.dsn_key 未配置，DSN 加密密钥由 jwt_secret 派生；生产环境请显式配置"
                );
                secret.as_bytes().to_vec()
            });
        Self { alg, material }
    }

    /// 加密明文 DSN；空串直接返回（表示未配置）。
    pub fn encrypt(&self, plain: &str) -> Result<String, String> {
        if plain.is_empty() {
            return Ok(String::new());
        }
        let key = derive_key(&self.material, self.alg.key_len());
        let mut iv = vec![0u8; self.alg.iv_len()];
        rand::thread_rng().fill_bytes(&mut iv);
        let ct = match self.alg {
            DsnAlg::Des => des_encrypt(&key, &iv, plain.as_bytes()),
            DsnAlg::Sm4 => sm4_encrypt(&key, &iv, plain.as_bytes())?,
        };
        let mut buf = iv;
        buf.extend_from_slice(&ct);
        Ok(format!("{PREFIX}{}:{}", self.alg.tag(), STANDARD.encode(buf)))
    }

    /// 解密；历史明文（无前缀）原样返回。
    pub fn decrypt(&self, stored: &str) -> Result<String, String> {
        let Some(rest) = stored.strip_prefix(PREFIX) else {
            return Ok(stored.to_string());
        };
        let (tag, b64) = rest
            .split_once(':')
            .ok_or_else(|| "DSN 密文格式错误".to_string())?;
        let alg = DsnAlg::parse(tag).ok_or_else(|| format!("不支持的 DSN 加密算法：{tag}"))?;
        let buf = STANDARD
            .decode(b64)
            .map_err(|e| format!("DSN 密文 base64 解码失败：{e}"))?;
        if buf.len() <= alg.iv_len() {
            return Err("DSN 密文长度非法".into());
        }
        let (iv, ct) = buf.split_at(alg.iv_len());
        let key = derive_key(&self.material, alg.key_len());
        let out = match alg {
            DsnAlg::Des => des_decrypt(&key, iv, ct)?,
            DsnAlg::Sm4 => sm4_decrypt(&key, iv, ct)?,
        };
        String::from_utf8(out).map_err(|e| format!("DSN 明文非 UTF-8：{e}"))
    }
}

/// 把任意长度的密钥材料扩展/截断为 `n` 字节（确定性，保证同一密钥可复现）。
fn derive_key(material: &[u8], n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    if material.is_empty() {
        return out;
    }
    let mut x: u8 = 0x5a;
    for (i, slot) in out.iter_mut().enumerate() {
        x = x.wrapping_mul(31).wrapping_add(material[i % material.len()]);
        *slot = x;
    }
    out
}

fn des_encrypt(key: &[u8], iv: &[u8], plain: &[u8]) -> Vec<u8> {
    let enc = DesEnc::new_from_slices(key, iv).expect("DES key/iv 长度已归一化");
    enc.encrypt_padded_vec_mut::<Pkcs7>(plain)
}

fn des_decrypt(key: &[u8], iv: &[u8], ct: &[u8]) -> Result<Vec<u8>, String> {
    let dec = DesDec::new_from_slices(key, iv).expect("DES key/iv 长度已归一化");
    dec.decrypt_padded_vec_mut::<Pkcs7>(ct)
        .map_err(|_| "DSN 密文解密失败（密钥不匹配或数据损坏）".to_string())
}

fn sm4_encrypt(key: &[u8], iv: &[u8], plain: &[u8]) -> Result<Vec<u8>, String> {
    use libsm::sm4::{Cipher, Mode};
    let c = Cipher::new(key, Mode::Ctr).map_err(|e| format!("SM4 初始化失败：{e:?}"))?;
    c.encrypt(&[], plain, iv)
        .map_err(|e| format!("SM4 加密失败：{e:?}"))
}

fn sm4_decrypt(key: &[u8], iv: &[u8], ct: &[u8]) -> Result<Vec<u8>, String> {
    use libsm::sm4::{Cipher, Mode};
    let c = Cipher::new(key, Mode::Ctr).map_err(|e| format!("SM4 初始化失败：{e:?}"))?;
    c.decrypt(&[], ct, iv)
        .map_err(|e| format!("SM4 解密失败：{e:?}"))
}
