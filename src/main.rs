#![feature(iterator_try_collect)]

mod api;
mod config;
mod creator;
mod post;

pub mod fanbox;

use std::{collections::HashMap, error::Error, rc::Rc};

use api::FanboxClient;
use config::ProgressSet;
use creator::{get_creator_posts, get_creators};
use fanbox::{Creator, PostListItem};
use log::{info, warn};
use plyne::define_tasks;
use post::{file::download_files, get_posts, sync_posts};
use post_archiver::{manager::PostArchiverManager, utils::VERSION};
use post_archiver_utils::display_metadata;
use tempfile::{TempPath};
use tokio::sync::Mutex;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = config::Config::parse();

    display_metadata(
        "Fanbox Archive",
        &[
            (
                "Version",
                format!("v{}", env!("CARGO_PKG_VERSION")).as_str(),
            ),
            ("PostArchiver Version", VERSION),
            ("Force", config.force().to_string().as_str()),
            ("Limit", config.limit().to_string().as_str()),
            ("Skip Free", config.skip_free().to_string().as_str()),
            ("Accepts", config.accepts().list().join(", ").as_str()),
            ("Whitelist", config.whitelist().join(", ").as_str()),
            ("Blacklist", config.blacklist().join(", ").as_str()),
            ("Output", config.output().display().to_string().as_str()),
        ],
    );

    if !config.output().exists() {
        warn!("Creating output folder");
        std::fs::create_dir_all(config.output())?;
    }

    info!("Connecting to PostArchiver");
    let manager = PostArchiverManager::open_or_create(config.output())?;
    let manager = Rc::new(Mutex::new(manager));

    let client = FanboxClient::new(&config);
    let progress = ProgressSet::new(&config);

    FanboxSystem::new(manager, config, client, progress)
        .execute()
        .await;

    info!("All done!");
    Ok(())
}

define_tasks! {
    FanboxSystem
    pipelines {
        CreatorPipeline: Creator,
        PostsPipeline: Vec<PostListItem>,
        FilesPipeline: (Vec<String>, tokio::sync::oneshot::Sender<HashMap<String, TempPath>>),
        SyncPipeline: (fanbox::Post, Vec<fanbox::Comment>, tokio::sync::oneshot::Receiver<HashMap<String, TempPath>>),
    }
    vars {
        Manager: Rc<Mutex<PostArchiverManager>>,
        Config: config::Config,
        Client: FanboxClient,
        Progress: config::ProgressSet,
    }
    tasks {
        get_creators,
        get_creator_posts,
        get_posts,
        download_files,
        sync_posts
    }
}
