//! Various small utilities accumulated over time for the WebPush server
use std::collections::HashMap;
use std::hash::Hash;

use base64::Engine;

pub mod timing;
pub mod user_agent;

pub use self::timing::{ms_since_epoch, sec_since_epoch, us_since_epoch};

pub trait InsertOpt<K: Eq + Hash, V> {
    /// Insert an item only if it exists
    fn insert_opt(&mut self, key: impl Into<K>, value: Option<impl Into<V>>);
}

impl<K: Eq + Hash, V> InsertOpt<K, V> for HashMap<K, V> {
    fn insert_opt(&mut self, key: impl Into<K>, value: Option<impl Into<V>>) {
        if let Some(value) = value {
            self.insert(key.into(), value.into());
        }
    }
}

/// Convenience wrapper for base64 decoding
/// *note* The `base64` devs are HIGHLY opinionated and the method to encode/decode
/// changes frequently. This function encapsulates that as much as possible.
pub fn b64_decode_url(input: &str) -> Result<Vec<u8>, base64::DecodeError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(input.trim_end_matches('='))
}

pub fn b64_encode_url(input: &Vec<u8>) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(input)
}

pub fn b64_decode_std(input: &str) -> Result<Vec<u8>, base64::DecodeError> {
    base64::engine::general_purpose::STANDARD_NO_PAD.decode(input.trim_end_matches('='))
}

pub fn b64_encode_std(input: &Vec<u8>) -> String {
    base64::engine::general_purpose::STANDARD_NO_PAD.encode(input)
}
