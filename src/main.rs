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
use log::{debug, info};
use plyne::define_tasks;
use post_archiver::{manager::PostArchiverManager, utils::VERSION};
use post_archiver_utils::{display_metadata, setup_database};
use tokio::sync::Mutex;

use crate::{context::UnimportedPost, post::posts};

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

    let output = config.output().clone();
    let manager = setup_database(&output)?;
    let context = Mutex::new(context::Context::load(&output));
    let manager = Arc::new(Mutex::new(manager));

    let client = FanboxClient::new(&config);
    let progress = ProgressSet::new(&config.multi);

    let system = FanboxSystem::new(manager, config, client, context, progress);
    let FanboxSystemContext { context, .. } = system.execute().await;

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
        posts_pipeline: UnimportedPost,
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
    }
}
