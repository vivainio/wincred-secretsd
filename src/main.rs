mod backend;
mod collection;
mod item;
mod model;
mod service;

use backend::Backend;
use collection::Collection;
use service::Service;
use std::future::pending;
use zbus::connection;
use zbus::zvariant::OwnedObjectPath;

const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const DEFAULT_COLLECTION: &str = "login";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let backend = Backend::new();

    let default_collection_path =
        OwnedObjectPath::try_from(format!("{SERVICE_PATH}/collection/{DEFAULT_COLLECTION}"))?;

    let service = Service {
        default_collection: default_collection_path.clone(),
    };
    let collection = Collection {
        backend: backend.clone(),
        name: DEFAULT_COLLECTION.to_string(),
    };

    // The "default" alias must be a live object too, not just resolvable via
    // Service.ReadAlias -- libsecret calls methods (e.g. CreateItem) directly
    // on /org/freedesktop/secrets/aliases/default.
    let default_alias_path = OwnedObjectPath::try_from(format!("{SERVICE_PATH}/aliases/default"))?;

    let conn = connection::Builder::session()?
        .name("org.freedesktop.secrets")?
        .serve_at(SERVICE_PATH, service)?
        .serve_at(default_collection_path.as_str(), collection.clone())?
        .serve_at(default_alias_path.as_str(), collection)?
        .build()
        .await?;

    // Re-register every existing item as a live D-Bus object so
    // Collection.Items / SearchItems results resolve to something.
    let targets = backend
        .list(&format!("secretservice/{DEFAULT_COLLECTION}/"))
        .await?;
    for target in targets {
        let id = target.rsplit('/').next().unwrap_or_default();
        let path = OwnedObjectPath::try_from(format!(
            "{SERVICE_PATH}/collection/{DEFAULT_COLLECTION}/{id}"
        ))?;
        let item = item::Item {
            backend: backend.clone(),
            target,
        };
        conn.object_server().at(path, item).await?;
    }

    eprintln!("wincred-secretsd: serving org.freedesktop.secrets");
    pending::<()>().await;
    Ok(())
}
