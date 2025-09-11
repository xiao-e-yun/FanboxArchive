#![feature(iterator_try_collect)]

mod api;
mod config;
mod creator;
mod post;

pub mod fanbox;

use std::{error::Error, rc::Rc};

use api::FanboxClient;
use config::Progress;
use creator::get_creators;
use dashmap::DashMap;
use fanbox::PostListItem;
use log::{info, warn};
use plyne::define_tasks;
use post::get_posts;
use post_archiver::{manager::PostArchiverManager, utils::VERSION, AuthorId};
use post_archiver_utils::display_metadata;
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
            ("Overwrite", config.overwrite().to_string().as_str()),
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
    let authors_pb = config.progress("authors");
    let posts_pb = config.progress("posts");
    let files_pb = config.progress("files");

    FanboxSystem::new(
        manager,
        config,
        client,
        Default::default(),
        authors_pb,
        posts_pb,
        files_pb,
    )
    .execute()
    .await;

    info!("All done!");
    Ok(())
}

define_tasks! {
    FanboxSystem
    pipelines {
        PostsPipeline: Vec<PostListItem>,
        FilePipeline: u32,
    }
    vars {
        Manager: Rc<Mutex<PostArchiverManager>>,
        Config: config::Config,
        Client: FanboxClient,
        Authors: Rc<DashMap<String, AuthorId>>,
        AuthorsProgress: Progress,
        PostsProgress: Progress,
        FilesProgress: Progress,
    }
    tasks {
        get_creators,
        get_posts,
    }
}
