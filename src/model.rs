use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// What actually gets stored as the wincred "secret" blob for one Secret
/// Service item. wincred only understands a single opaque UTF-8 string per
/// target, so the item's label/attributes/content-type ride along in this
/// JSON envelope; the real secret bytes are base64'd inside it since
/// Secret Service secrets are arbitrary bytes, not necessarily text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub label: String,
    #[serde(default)]
    pub attributes: HashMap<String, String>,
    pub content_type: String,
    pub secret_b64: String,
}

impl Envelope {
    pub fn from_bytes(
        label: String,
        attributes: HashMap<String, String>,
        content_type: String,
        bytes: &[u8],
    ) -> Self {
        use base64::Engine;
        Self {
            label,
            attributes,
            content_type,
            secret_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
        }
    }

    pub fn secret_bytes(&self) -> Vec<u8> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(&self.secret_b64)
            .unwrap_or_default()
    }

    pub fn set_secret_bytes(&mut self, bytes: &[u8]) {
        use base64::Engine;
        self.secret_b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    }

    pub fn matches(&self, query: &HashMap<String, String>) -> bool {
        query.iter().all(|(k, v)| self.attributes.get(k) == Some(v))
    }
}
