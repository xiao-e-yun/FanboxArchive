mod api;
mod config;
mod creator;
mod post;

pub mod fanbox;

use std::error::Error;

use config::Config;
use creator::{display_creators, get_creators, sync_creators};
use log::{info, warn};
use post::{filter_unsynced_posts, get_post_urls, get_posts, sync_posts};
use post_archiver::{manager::PostArchiverManager, utils::VERSION};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = Config::parse();

    info!("# Fanbox Archive #");
    info!("");
    info!("==================================");
    info!("PostArchiver version: v{}", VERSION);
    info!("Overwrite: {}",config.overwrite());
    info!("Output: {}",config.output().display());
    info!("==================================");

    if !config.output().exists() {
        warn!("Creating output folder");
        std::fs::create_dir_all(config.output())?;
    }

    info!("Connecting to PostArchiver");
    let mut manager = PostArchiverManager::open_or_create(config.output())?;

    info!("Loading Creator List");
    let creators = get_creators(&config).await?;
    display_creators(&creators);

    info!("Syncing Creator List"); 
    let authors = sync_creators(&mut manager, creators)?;

    info!("Loading Creators Post");
    for (author, creator_id) in authors {
        info!("{}", creator_id);
        let mut posts = get_post_urls(&config, &creator_id).await?;

        let total_post = posts.len();
        let mut posts_count_info = format!("{} posts", total_post);
        if !config.force() {
            posts = filter_unsynced_posts(&mut manager, posts)?;
            posts_count_info += &format!(" ({} unsynced)", posts.len());
        };
        info!(" + {}", posts_count_info);

        let posts = get_posts(&config, posts).await?;
        if !posts.is_empty() {
            sync_posts(&mut manager, &config, author, posts).await?;
        }

        info!("");
    }

    info!("All done!");
    Ok(())
}
