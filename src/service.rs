use crate::collection::Collection;
use crate::item::{Item, Secret};
use std::collections::HashMap;
use zbus::object_server::ObjectServer;
use zbus::{
    fdo, interface,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

pub struct Service {
    pub default_collection: OwnedObjectPath,
}

#[interface(name = "org.freedesktop.Secret.Service")]
impl Service {
    async fn open_session(
        &self,
        algorithm: String,
        _input: OwnedValue,
    ) -> fdo::Result<(OwnedValue, OwnedObjectPath)> {
        if algorithm != "plain" {
            return Err(fdo::Error::NotSupported(
                "only the \"plain\" algorithm is supported -- this daemon only ever runs \
                 on a local, already-authenticated session bus, so there's nothing for \
                 transport encryption to protect against"
                    .into(),
            ));
        }
        // No per-session crypto state to track in plain mode; a fixed path
        // is fine since GetSecret/SetSecret just echo it back unused.
        let path = OwnedObjectPath::try_from("/org/freedesktop/secrets/session/plain").unwrap();
        let output = OwnedValue::try_from(Value::from("")).expect("string always convertible");
        Ok((output, path))
    }

    async fn search_items(
        &self,
        attributes: HashMap<String, String>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> fdo::Result<(Vec<OwnedObjectPath>, Vec<OwnedObjectPath>)> {
        let iface = server
            .interface::<_, Collection>(&self.default_collection)
            .await
            .map_err(|e| fdo::Error::Failed(e.to_string()))?;
        let unlocked = iface.get().await.search_items(attributes).await?;
        Ok((unlocked, Vec::new()))
    }

    async fn get_secrets(
        &self,
        items: Vec<OwnedObjectPath>,
        session: OwnedObjectPath,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> fdo::Result<HashMap<OwnedObjectPath, Secret>> {
        let mut out = HashMap::new();
        for path in items {
            if let Ok(iface) = server.interface::<_, Item>(&path).await {
                let secret = iface.get().await.fetch_secret(session.clone()).await?;
                out.insert(path, secret);
            }
        }
        Ok(out)
    }

    async fn unlock(
        &self,
        objects: Vec<OwnedObjectPath>,
    ) -> fdo::Result<(Vec<OwnedObjectPath>, OwnedObjectPath)> {
        // Nothing is ever locked, so everything is trivially "unlocked".
        Ok((objects, OwnedObjectPath::try_from("/").unwrap()))
    }

    async fn lock(
        &self,
        _objects: Vec<OwnedObjectPath>,
    ) -> fdo::Result<(Vec<OwnedObjectPath>, OwnedObjectPath)> {
        // Locking isn't modeled; report nothing as locked rather than lie.
        Ok((Vec::new(), OwnedObjectPath::try_from("/").unwrap()))
    }

    async fn read_alias(&self, name: String) -> fdo::Result<OwnedObjectPath> {
        if name == "default" {
            Ok(self.default_collection.clone())
        } else {
            Ok(OwnedObjectPath::try_from("/").unwrap())
        }
    }

    async fn set_alias(&self, _name: String, _collection: OwnedObjectPath) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported(
            "only the default collection/alias exists so far".into(),
        ))
    }

    async fn create_collection(
        &self,
        _properties: HashMap<String, OwnedValue>,
        _alias: String,
    ) -> fdo::Result<(OwnedObjectPath, OwnedObjectPath)> {
        Err(fdo::Error::NotSupported(
            "only a single fixed collection is supported so far".into(),
        ))
    }

    #[zbus(property)]
    async fn collections(&self) -> Vec<OwnedObjectPath> {
        vec![self.default_collection.clone()]
    }
}
