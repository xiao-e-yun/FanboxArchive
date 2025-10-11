use std::collections::{HashMap, HashSet};

use futures::join;
use log::{error, info, warn};
use plyne::{Input, Output};
use post_archiver::{
    importer::{UnsyncAlias, UnsyncAuthor},
    manager::PostArchiverManager,
    AuthorId, PlatformId,
};
use post_archiver_utils::Result;
use rusqlite::Transaction;

use crate::{
    api::FanboxClient,
    config::{Config, ProgressSet, Strategy},
    context::Context,
    fanbox::{Creator, Post, PostListItem, User},
    post::filter_unsynced_post,
    Manager,
};

pub async fn get_creators(
    config: &Config,
    client: &FanboxClient,
    creator_pipeline: Input<Creator>,
    pb: &ProgressSet,
) {
    let accepts = config.accepts();
    info!("Accepts:");
    for accept in accepts.list() {
        info!(" + {accept}");
    }
    info!("");

    let mut creators: HashSet<Creator> = HashSet::new();
    info!("Checking creators");

    let (following, supporting) = join!(
        async {
            if !accepts.accept_following() {
                return vec![];
            }
            let list = client
                .get_following_creators()
                .await
                .inspect_err(|e| warn!("Failed to get following creators: {e}"))
                .unwrap_or_default();
            info!(" + Following: {} found", list.len());
            list
        },
        async {
            if !accepts.accept_supporting() {
                return vec![];
            }
            let list = client
                .get_supporting_creators()
                .await
                .inspect_err(|e| warn!("Failed to get supporting creators: {e}"))
                .unwrap_or_default();
            info!(" + Supporting: {} found", list.len());
            list
        }
    );
    creators.extend(following.into_iter().map(|c| c.into()));
    creators.extend(supporting.into_iter().map(|c| c.into()));
    info!("");

    let total = creators.len();
    info!("Total: {total} creators");
    creators.retain(|c| config.filter_creator(c));
    let filtered = creators.len();
    let offset = total - filtered;
    info!("Excluded: {offset} creators");
    info!("Included: {filtered} creators");
    info!("");

    display_creators(&creators);
    pb.authors.inc_length(total as u64);

    for creator in creators {
        creator_pipeline.send(creator).unwrap();
    }
}

pub async fn get_creator_posts(
    mut creator_pipeline: Output<Creator>,
    posts_pipeline: Input<Vec<PostListItem>>,
    context: &Context,
    manager: &Manager,
    config: &Config,
    client: &FanboxClient,
    pb: &ProgressSet,
) {
    while let Some(creator) = creator_pipeline.recv().await {
        let mut creator_record = context
            .creators
            .entry(creator.creator_id.clone())
            .or_default();

        let last_updated = creator_record
            .last_updated(creator.fee)
            .filter(|_| config.strategy() == Strategy::Increment);

        let Ok((posts, last_date)) = client.get_posts(&creator.creator_id, last_updated).await
        else {
            error!("Failed to get posts for creator: {}", creator.creator_id);
            return;
        };

        creator_record.update(last_date, creator.fee);

        let manager = manager.lock().await;
        let posts = posts
            .into_iter()
            .filter(|post| config.filter_post(post))
            .filter(|post| {
                config.strategy() == Strategy::Force || filter_unsynced_post(&manager, post)
            })
            .collect::<Vec<_>>();

        info!("Found {} posts ({})", posts.len(), creator.creator_id);
        pb.posts.inc_length(posts.len() as u64);
        posts_pipeline.send(posts).unwrap();
        pb.authors.inc(1);
    }

    pb.authors.finish();
}

pub fn display_creators(creators: &HashSet<Creator>) {
    if log::log_enabled!(log::Level::Info) {
        let mut creators = creators.iter().collect::<Vec<_>>();
        creators.sort_by(|a, b| a.creator_id.cmp(&b.creator_id));

        let (mut id_width, mut fee_width) = (11_usize, 5_usize);
        for creator in creators.iter() {
            id_width = creator.creator_id.len().max(id_width);
            fee_width = creator.fee.to_string().len().max(fee_width);
        }

        info!(
            "+-{:-<id_width$}-+-{:-<fee_width$}--+-{}------- - -",
            " CreatorId ", " Fee ", " Name "
        );
        for creator in creators.iter() {
            info!(
                "| {:id_width$} | {:fee_width$}$ | {}",
                creator.creator_id, creator.fee, creator.name
            );
        }
        info!(
            "+-{}-+-{}--+-------------- - -",
            "-".to_string().repeat(id_width),
            "-".to_string().repeat(fee_width)
        );
        info!("");
    }
}

pub fn sync_creator(
    manager: &PostArchiverManager<Transaction<'_>>,
    authors: &mut HashMap<String, AuthorId>,
    platforms: [PlatformId; 2],
    post: &Post,
) -> Result<AuthorId> {
    let creator_id = post.creator_id.clone();
    if let Some(author) = authors.get(&creator_id) {
        return Ok(*author);
    }

    let [fanbox_platform, pixiv_platform] = platforms;
    let User { name, user_id, .. } = &post.user;

    match UnsyncAuthor::new(name.to_string())
        .aliases(vec![
            UnsyncAlias::new(fanbox_platform, creator_id.clone())
                .link(format!("https://{creator_id}.fanbox.cc/")),
            UnsyncAlias::new(pixiv_platform, user_id.clone())
                .link(format!("https://www.pixiv.net/users/{user_id}")),
        ])
        .sync(manager)
    {
        Ok(author) => {
            info!("Synced author: {creator_id} ({name})");
            authors.insert(creator_id, author);
            Ok(author)
        }
        Err(e) => {
            error!("Failed to sync author: {creator_id} ({name}): {e}");
            Result::Err(e.into())
        }
    }
}
