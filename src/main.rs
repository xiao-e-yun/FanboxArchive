#![feature(iterator_try_collect)]

mod api;
mod config;
mod creator;
mod post;

pub mod fanbox;

use std::error::Error;

use api::FanboxClient;
use config::Config;
use creator::{display_creators, get_creators, sync_creators};
use log::{info, warn};
use post::{filter_unsynced_posts, get_post_urls, get_posts, sync_posts};
use post_archiver::{manager::PostArchiverManager, utils::VERSION};
use post_archiver_utils::display_metadata;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = Config::parse();

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
    let mut manager = PostArchiverManager::open_or_create(config.output())?;

    let client = FanboxClient::new(&config);

    info!("Loading Creator List");
    let creators = get_creators(&config, &client).await?;
    display_creators(&creators);

    info!("Syncing Creator List");
    let authors = sync_creators(&mut manager, creators)?;

    info!("Loading Creators Post");
    for (author, creator_id) in authors {
        info!("{creator_id}",);
        let mut posts = get_post_urls(&config, &client, &creator_id).await?;

        let total_post = posts.len();
        let mut posts_count_info = format!("{total_post} posts",);
        if !config.force() {
            posts = filter_unsynced_posts(&mut manager, posts)?;
            posts_count_info += &format!(" ({} unsynced)", posts.len());
        };
        info!(" + {posts_count_info}",);

        let posts = get_posts(&client, posts).await?;
        if !posts.is_empty() {
            sync_posts(&mut manager, &config, author, posts).await?;
        }

        info!("");
    }

    info!("All done!");
    Ok(())
}
