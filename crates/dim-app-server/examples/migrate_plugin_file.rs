//! Offline migration of one explicitly identified unversioned plugin file.
use dim_agent_capabilities::file_storage::{FileStorage, FileStoragePlugin};
use dim_agent_runtime::capability_plugin;
use dim_app_server::plugin_files::migrate_unversioned_file;
use std::{io, path::Path};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [mode, root, path] = args.as_slice() else {
        return Err(io::Error::other(
            "usage: migrate_plugin_file --from-unversioned ROOT RELATIVE_FILE",
        )
        .into());
    };
    if mode != "--from-unversioned" {
        return Err(
            io::Error::other("explicit --from-unversioned confirmation is required").into(),
        );
    }
    let directory = Path::new(root).canonicalize()?;
    let ctx = rutis::Ctx::root_with(tokio::runtime::Handle::current());
    (&ctx.plugin(capability_plugin(FileStoragePlugin::new(vec![directory])))).await?;
    let storage = ctx.get::<FileStorage>().unwrap();
    let result = async {
        let path = storage.path(0, Path::new(path))?;
        let lease = storage.lock(&path, &CancellationToken::new()).await?;
        migrate_unversioned_file(lease).await
    }
    .await;
    ctx.shutdown().await?;
    result?;
    println!("Migrated one plugin file; original bytes are retained in its .before-v1 backup.");
    Ok(())
}
