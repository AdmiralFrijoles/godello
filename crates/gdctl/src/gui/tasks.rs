//! The seam between the GUI and the async core calls.
//!
//! Each function here wraps a core call in a task and maps the core error to a
//! string at the boundary, because an iced message must be Clone and the core
//! errors are not. The synchronous core calls (listing and removing installs)
//! are not here. Those run inline in the update step since they are quick local
//! work and need no task.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use godello_core::{EngineRepository, GodotVersion, InstallManager, Target, Variant};
use iced::Task;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::context::Repository;
use crate::gui::cache;
use crate::gui::message::Message;
use crate::gui::progress::{ChannelProgress, ProgressEvent};
use crate::net::WebClient;

/// Fetch the list of versions available to install, through the cache.
///
/// When force is false a fresh enough cached list is used and no network call is
/// made. Otherwise, or when the cache is missing or stale, it fetches the list
/// and writes it back to the cache. The command line does not go through here, so
/// it always fetches live.
pub fn load_remote(
    repository: Repository,
    include_pre: bool,
    cache_path: PathBuf,
    force: bool,
) -> Task<Message> {
    Task::perform(
        async move {
            if !force {
                if let Some(releases) = cache::load(&cache_path, cache::TTL_SECS) {
                    return Ok(releases);
                }
            }
            let releases = repository
                .list_releases(include_pre)
                .await
                .map_err(|err| err.to_string())?;
            cache::store(&cache_path, &releases);
            Ok(releases)
        },
        Message::RemoteLoaded,
    )
}

/// Resolve the download for a version and variant, then install it.
///
/// This returns two tasks batched together. One drains progress events from a
/// channel into messages while the download runs. The other performs the install
/// and yields the final result. The progress sink is moved into the install
/// future and dropped when it finishes, which ends the progress stream on its
/// own, so there is no lifecycle to manage by hand.
pub fn install_engine(
    repository: Repository,
    manager: InstallManager,
    downloader: WebClient,
    variant: Variant,
    version: GodotVersion,
    cancel: Arc<AtomicBool>,
) -> Task<Message> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<ProgressEvent>();
    let sink = ChannelProgress::new(tx);

    let progress = Task::run(UnboundedReceiverStream::new(rx), move |event| {
        Message::InstallProgress {
            variant,
            version,
            event,
        }
    });

    let install = Task::perform(
        async move {
            let target = Target::current(variant);
            let asset = repository
                .asset(version, target)
                .await
                .map_err(|err| err.to_string())?;
            let checksum = asset.checksum.clone();
            // Download. This is cancelable by aborting the task.
            let archive = manager
                .fetch(&asset, variant, version, &downloader, &sink)
                .await
                .map_err(|err| err.to_string())?;
            // Verify and extract on a blocking thread so the hashing and unzip do
            // not freeze the executor, which keeps the UI responsive and lets the
            // cancel flag take effect.
            let outcome = tokio::task::spawn_blocking(move || {
                manager.install_archive(&archive, variant, version, checksum.as_ref(), &cancel)
            })
            .await;
            match outcome {
                Ok(Ok(_engine)) => Ok(()),
                Ok(Err(err)) => Err(err.to_string()),
                Err(_join) => Err("the install was interrupted".to_string()),
            }
        },
        move |result| Message::Installed {
            variant,
            version,
            result,
        },
    );

    Task::batch([progress, install])
}
