//! 确定性占位符与值表（E-02）。
//!
//! 与 Magpie `internal/redact/redact.go` 的 `tag`/`placeholder`/`makeRoom`
//! （commit f11bf27c）逐位一致：
//! - 占位符 `{{KIND_xxxxxxxx}}`，后缀为 HMAC-SHA256(key, `kind\0value`) 摘要的
//!   base32（RFC 4648、无填充）前 8 个字符，转小写；
//! - 密钥 32 字节随机，存 Keychain 账户 `privacy-hmac-key`，首次生成后固定；
//! - 值表（占位符 → 原值）内存保存，上限 200,000，满时淘汰最旧一半；
//! - 同值同占位符：HMAC 确定性保证跨重启、跨淘汰不变，淘汰后原值再次出现即重新入表。
//!
//! 注意：本文件只实现占位符机制本身；检测规则见 `rules/`（E-03）。

use std::collections::{HashMap, VecDeque};

use data_encoding::BASE32_NOPAD;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::aicli::secrets::SecretStore;

/// Magpie `redact.go` 的占位符形态：`{{KIND_base32前8位小写}}`。
pub const PLACEHOLDER_PATTERN: &str = r"\{\{[A-Z][A-Z0-9_]*_[a-z2-7]{8}\}\}";
/// Keychain 账户名（第 0.4 节固定取值）。
pub const KEY_ACCOUNT: &str = "privacy-hmac-key";
/// 值表上限；满时淘汰最旧一半（Magpie `maxValues`）。
pub const MAX_VALUES: usize = 200_000;

type HmacSha256 = Hmac<Sha256>;

/// 生成新值用的密钥字节数。
pub const KEY_LEN: usize = 32;

fn tag_with(key: &[u8], kind: &str, value: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(kind.as_bytes());
    mac.update(b"\x00");
    mac.update(value.as_bytes());
    let digest = mac.finalize().into_bytes();
    // base32（无填充）的前 8 个字符再转小写，与 Magpie `tag` 一致。
    let mut suffix = BASE32_NOPAD.encode(&digest[..]);
    suffix.truncate(8);
    format!("{{{{{kind}_{}}}}}", suffix.to_lowercase())
}

/// 占位符引擎：密钥 + 值表。可跨请求复用；表只在内存中。
pub struct PlaceholderEngine {
    key: Vec<u8>,
    values: HashMap<String, String>,
    order: VecDeque<String>,
    /// 值表上限（默认 [`MAX_VALUES`]；测试可注入小值验证淘汰）。
    max_values: usize,
}

impl PlaceholderEngine {
    /// 固定密钥（测试与 golden 对照用）。
    pub fn with_key(key: [u8; KEY_LEN]) -> Self {
        Self {
            key: key.to_vec(),
            values: HashMap::new(),
            order: VecDeque::new(),
            max_values: MAX_VALUES,
        }
    }

    /// 生产构造：从 Keychain 读 `privacy-hmac-key`，没有则生成 32 字节随机并写回。
    pub fn from_store(store: &dyn SecretStore) -> Result<Self, String> {
        let key: Vec<u8> = match store.get(KEY_ACCOUNT) {
            // 存储形态为 32 字节的十六进制文本（64 个字符）。
            Ok(Some(existing)) if existing.len() >= KEY_LEN * 2 => {
                let bytes = existing.as_bytes();
                (0..KEY_LEN)
                    .map(|index| {
                        u8::from_str_radix(
                            std::str::from_utf8(&bytes[index * 2..index * 2 + 2]).unwrap_or("00"),
                            16,
                        )
                        .unwrap_or(0)
                    })
                    .collect()
            }
            Ok(_) => {
                let mut fresh = vec![0u8; KEY_LEN];
                getrandom::getrandom(&mut fresh).map_err(|e| format!("entropy: {e}"))?;
                let text = fresh
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                store
                    .set(KEY_ACCOUNT, &text)
                    .map_err(|error| format!("store privacy key: {error}"))?;
                fresh
            }
            Err(error) => return Err(format!("read privacy key: {error}")),
        };
        Ok(Self {
            key,
            values: HashMap::new(),
            order: VecDeque::new(),
            max_values: MAX_VALUES,
        })
    }

    /// 引擎密钥的只读引用（shape 等模块用于 HMAC 流密码）。
    pub fn key_bytes(&self) -> &[u8] {
        &self.key
    }

    /// 计算占位符但不入值表（对应 Magpie `tag`）。
    pub fn tag(&self, kind: &str, value: &str) -> String {
        tag_with(&self.key, kind, value)
    }

    /// 计算占位符并记住原值（对应 Magpie `placeholder`）：表满先淘汰最旧一半。
    pub fn mask(&mut self, kind: &str, value: &str) -> String {
        let placeholder = self.tag(kind, value);
        if !self.values.contains_key(&placeholder) {
            self.make_room();
            self.values.insert(placeholder.clone(), value.to_owned());
            self.order.push_back(placeholder.clone());
        }
        placeholder
    }

    /// 还原：查值表（对应 Magpie `Value`）。
    pub fn restore(&self, placeholder: &str) -> Option<&str> {
        self.values.get(placeholder).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// 满时淘汰最旧一半（Magpie `makeRoom`；Go 版按 map 迭代删除，这里按插入序
    /// 精确淘汰「最旧」，淘汰后同值重入得到同一占位符，行为等价且确定）。
    fn make_room(&mut self) {
        if self.len() < self.max_values {
            return;
        }
        let evict = self.max_values / 2;
        for _ in 0..evict {
            if let Some(oldest) = self.order.pop_front() {
                self.values.remove(&oldest);
            }
        }
    }

    /// 仅测试：注入小值表上限，验证「满时淘汰最旧一半」与淘汰后恢复。
    #[cfg(test)]
    pub(crate) fn with_limit_for_test(mut self, max_values: usize) -> Self {
        self.max_values = max_values;
        self
    }
}

#[cfg(test)]
mod tests;
