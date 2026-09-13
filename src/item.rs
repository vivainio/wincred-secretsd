use crate::backend::Backend;
use crate::model::Envelope;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use zbus::{
    fdo, interface,
    zvariant::{OwnedObjectPath, Type},
};

/// The Secret Service `Secret` struct, returned/accepted as a single D-Bus
/// struct out/in-param (signature `(oayays)`) -- NOT a plain Rust tuple,
/// which zbus would instead unpack into four separate out-params.
/// `parameters` only carries meaning for encrypted sessions; always empty
/// here since only the "plain" session algorithm is supported.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Secret {
    pub session: OwnedObjectPath,
    pub parameters: Vec<u8>,
    pub value: Vec<u8>,
    pub content_type: String,
}

pub struct Item {
    pub backend: Backend,
    pub target: String,
}

impl Item {
    async fn load(&self) -> fdo::Result<Envelope> {
        let raw = self
            .backend
            .get(&self.target)
            .await
            .map_err(|e| fdo::Error::Failed(e.to_string()))?
            .ok_or_else(|| fdo::Error::Failed(format!("item {} vanished", self.target)))?;
        serde_json::from_str(&raw)
            .map_err(|e| fdo::Error::Failed(format!("corrupt item envelope: {e}")))
    }

    async fn store(&self, env: &Envelope) -> fdo::Result<()> {
        let json = serde_json::to_string(env).map_err(|e| fdo::Error::Failed(e.to_string()))?;
        self.backend
            .set(&self.target, "", &json)
            .await
            .map_err(|e| fdo::Error::Failed(e.to_string()))
    }

    /// Plain Rust entry point (used by `Service.GetSecrets` too). Kept
    /// separate from the `#[interface]`-exposed `get_secret` because a
    /// D-Bus method's sole OUT arg being a struct must come back as a
    /// 1-tuple -- otherwise zbus serializes the reply body as the struct's
    /// bare fields (wire signature `oayays`) instead of one nested struct
    /// arg (`(oayays)`), which GDBus-based clients like libsecret reject.
    pub async fn fetch_secret(&self, session: OwnedObjectPath) -> fdo::Result<Secret> {
        let env = self.load().await?;
        Ok(Secret {
            session,
            parameters: Vec::new(),
            value: env.secret_bytes(),
            content_type: env.content_type,
        })
    }
}

#[interface(name = "org.freedesktop.Secret.Item")]
impl Item {
    async fn delete(&self) -> fdo::Result<OwnedObjectPath> {
        self.backend
            .delete(&self.target)
            .await
            .map_err(|e| fdo::Error::Failed(e.to_string()))?;
        // "/" is the well-known no-op prompt path -- nothing to confirm.
        Ok(OwnedObjectPath::try_from("/").unwrap())
    }

    async fn get_secret(&self, session: OwnedObjectPath) -> fdo::Result<(Secret,)> {
        self.fetch_secret(session).await.map(|s| (s,))
    }

    async fn set_secret(&self, secret: Secret) -> fdo::Result<()> {
        let mut env = self.load().await?;
        env.content_type = secret.content_type;
        env.set_secret_bytes(&secret.value);
        self.store(&env).await
    }

    #[zbus(property)]
    async fn locked(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn attributes(&self) -> fdo::Result<HashMap<String, String>> {
        Ok(self.load().await?.attributes)
    }

    #[zbus(property)]
    async fn label(&self) -> fdo::Result<String> {
        Ok(self.load().await?.label)
    }

    #[zbus(property)]
    async fn set_label(&self, label: String) -> fdo::Result<()> {
        let mut env = self.load().await?;
        env.label = label;
        self.store(&env).await
    }

    #[zbus(property)]
    async fn created(&self) -> u64 {
        0
    }

    #[zbus(property)]
    async fn modified(&self) -> u64 {
        0
    }
}
