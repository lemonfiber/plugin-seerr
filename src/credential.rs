//! The key this adapter presents to Seerr, as lemonfiber writes it into the adapter's
//! configuration directory.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use lemonfiber_contract::adapter::UPSTREAM_FILE;

/// The part of `upstream.json` holding the key Seerr answers its API to.
pub const API_KEY: &str = "api_key";

/// The longest key taken.
pub const KEY_LIMIT: usize = 256;

const KEY_MARKS: &[u8] = b"-_.+/=";

/// Where the key every call presents comes from.
pub trait Holding: Send + Sync {
    /// The key held now.
    ///
    /// # Errors
    ///
    /// Why no key is held, in words that name no part of one.
    fn key(&self) -> Result<String, String>;
}

impl Holding for String {
    fn key(&self) -> Result<String, String> {
        Ok(self.clone())
    }
}

/// The key lemonfiber wrote into one configuration directory.
#[derive(Debug, Clone)]
pub struct Upstream(PathBuf);

impl Upstream {
    /// The key lemonfiber writes into `configuration`.
    #[must_use]
    pub fn in_directory(configuration: &Path) -> Self {
        Self(configuration.to_path_buf())
    }
}

impl Holding for Upstream {
    fn key(&self) -> Result<String, String> {
        let mut parts = lemonfiber_adapter_kit::upstream_in(&self.0).map_err(|why| {
            if why.kind() == ErrorKind::InvalidData {
                format!("{UPSTREAM_FILE} is not an object of named strings")
            } else {
                format!("{UPSTREAM_FILE} could not be read: {why}")
            }
        })?;
        let key = parts
            .remove(API_KEY)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| format!("{UPSTREAM_FILE} names no {API_KEY}"))?;
        if spelled_as_a_key(&key) {
            Ok(key)
        } else {
            Err(format!(
                "the {API_KEY} in {UPSTREAM_FILE} is not spelled as a key is"
            ))
        }
    }
}

/// Whether `key` is spelled the way a generated key is: letters, digits and the marks
/// hex, base64 and a UUID are written in, and no longer than [`KEY_LIMIT`].
#[must_use]
pub fn spelled_as_a_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= KEY_LIMIT
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || KEY_MARKS.contains(&byte))
}

#[cfg(test)]
mod tests;
