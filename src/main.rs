mod api;
mod config;
mod context;
mod creator;
mod post;

pub mod fanbox;

use std::{error::Error, sync::Arc};

use api::FanboxClient;
use config::{Config, ProgressSet};
use context::Context;
use creator::{creator_posts, creators};
use fanbox::Creator;
use log::{debug, info, warn};
use plyne::define_tasks;
use post_archiver::{manager::PostArchiverManager, utils::VERSION};
use post_archiver_utils::{display_metadata};
use tokio::sync::Mutex;

use crate::post::{failed_posts, posts};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = config::Config::parse();

    display_metadata!("Fanbox Archive" {
        "Version": format!("v{}", env!("CARGO_PKG_VERSION")),
        "PostArchiver Version": VERSION,
        "Strategy": config.strategy(),
        "Limit": config.limit(),
        "Skip Free": config.skip_free(),
        "Accepts": config.accepts().list().join(", "),
        "Whitelist": config.whitelist().join(", "),
        "Blacklist": config.blacklist().join(", "),
        "Output": config.output().display(),
    });

    if !config.output().exists() {
        warn!("Creating output folder");
        std::fs::create_dir_all(config.output())?;
    }

    info!("Connecting to PostArchiver");
    let output = config.output().clone();

    let manager = PostArchiverManager::open_or_create(&output)?;

    let context = Mutex::new(context::Context::load(&output));
    let manager = Arc::new(Mutex::new(manager));

    let client = FanboxClient::new(&config);
    let progress = ProgressSet::new(&config.multi);

    let FanboxSystemContext { context, .. } =
        FanboxSystem::new(manager, config, client, context, progress)
            .execute()
            .await;

    info!("All done!");

    context.into_inner().save(&output);
    debug!("Context saved");
    Ok(())
}

pub type Manager = Arc<Mutex<PostArchiverManager>>;

define_tasks! {
    FanboxSystem
    pipelines {
        creator_pipeline: Creator,
        posts_pipeline: String,
    }
    vars {
        manager: Manager,
        config: Config,
        client: FanboxClient,
        context: Mutex<Context>,
        progress: ProgressSet,
    }
    tasks {
        creators,
        creator_posts,
        posts,
        failed_posts,
    }
}
