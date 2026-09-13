use crate::backend::Backend;
use crate::item::{Item, Secret};
use crate::model::Envelope;
use std::collections::HashMap;
use uuid::Uuid;
use zbus::object_server::ObjectServer;
use zbus::{
    fdo, interface,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

#[derive(Clone)]
pub struct Collection {
    pub backend: Backend,
    pub name: String,
}

impl Collection {
    fn prefix(&self) -> String {
        format!("secretservice/{}/", self.name)
    }

    fn item_path(&self, id: &str) -> OwnedObjectPath {
        OwnedObjectPath::try_from(format!(
            "/org/freedesktop/secrets/collection/{}/{id}",
            self.name
        ))
        .expect("valid object path")
    }

    async fn item_targets(&self) -> fdo::Result<Vec<String>> {
        self.backend
            .list(&self.prefix())
            .await
            .map_err(|e| fdo::Error::Failed(e.to_string()))
    }

    async fn matching_items(
        &self,
        attributes: &HashMap<String, String>,
    ) -> fdo::Result<Vec<(String, Envelope)>> {
        let mut out = Vec::new();
        for target in self.item_targets().await? {
            let Some(raw) = self
                .backend
                .get(&target)
                .await
                .map_err(|e| fdo::Error::Failed(e.to_string()))?
            else {
                continue;
            };
            let Ok(env) = serde_json::from_str::<Envelope>(&raw) else {
                continue;
            };
            if env.matches(attributes) {
                out.push((target, env));
            }
        }
        Ok(out)
    }
}

#[interface(name = "org.freedesktop.Secret.Collection")]
impl Collection {
    pub async fn search_items(
        &self,
        attributes: HashMap<String, String>,
    ) -> fdo::Result<Vec<OwnedObjectPath>> {
        let matches = self.matching_items(&attributes).await?;
        Ok(matches
            .into_iter()
            .map(|(target, _)| {
                let id = target.rsplit('/').next().unwrap_or_default();
                self.item_path(id)
            })
            .collect())
    }

    async fn create_item(
        &self,
        properties: HashMap<String, OwnedValue>,
        secret: Secret,
        replace: bool,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> fdo::Result<(OwnedObjectPath, OwnedObjectPath)> {
        let label = properties
            .get("org.freedesktop.Secret.Item.Label")
            .and_then(|v| <&str>::try_from(v).ok())
            .map(str::to_owned)
            .unwrap_or_default();
        let attributes: HashMap<String, String> = properties
            .get("org.freedesktop.Secret.Item.Attributes")
            .and_then(|v| Value::try_from(v).ok())
            .and_then(|v| HashMap::<String, String>::try_from(v).ok())
            .unwrap_or_default();

        let existing_id = if replace {
            self.matching_items(&attributes)
                .await?
                .into_iter()
                .next()
                .map(|(target, _)| target.rsplit('/').next().unwrap().to_string())
        } else {
            None
        };
        let id = existing_id.unwrap_or_else(|| Uuid::new_v4().simple().to_string());

        let env = Envelope::from_bytes(label, attributes, secret.content_type, &secret.value);
        let json = serde_json::to_string(&env).map_err(|e| fdo::Error::Failed(e.to_string()))?;
        let target = format!("{}{id}", self.prefix());
        self.backend
            .set(&target, "", &json)
            .await
            .map_err(|e| fdo::Error::Failed(e.to_string()))?;

        let path = self.item_path(&id);
        let item = Item {
            backend: self.backend.clone(),
            target,
        };
        // Ignore "already registered" -- happens on `replace` of a known item.
        let _ = server.at(path.clone(), item).await;

        Ok((path, OwnedObjectPath::try_from("/").unwrap()))
    }

    async fn delete(&self) -> fdo::Result<OwnedObjectPath> {
        Err(fdo::Error::NotSupported(
            "deleting the collection itself is not supported yet".into(),
        ))
    }

    #[zbus(property)]
    async fn items(&self) -> fdo::Result<Vec<OwnedObjectPath>> {
        let targets = self.item_targets().await?;
        Ok(targets
            .iter()
            .map(|t| self.item_path(t.rsplit('/').next().unwrap_or_default()))
            .collect())
    }

    #[zbus(property)]
    async fn label(&self) -> String {
        self.name.clone()
    }

    #[zbus(property)]
    async fn locked(&self) -> bool {
        false
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
