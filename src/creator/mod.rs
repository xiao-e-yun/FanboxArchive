use std::collections::HashSet;

use futures::join;
use log::{error, info, warn};
use plyne::{Input, Output};
use post_archiver_utils::{Result, TaskSet};
use rusqlite::params;
use tokio::sync::Mutex;

use crate::{
    api::FanboxClient,
    config::{Config, ProgressSet},
    context::{CachedCreator, Context},
    fanbox::Creator,
    Manager,
};

pub async fn creators(
    config: &Config,
    client: &FanboxClient,
    progress: &ProgressSet,
    creator_pipeline: Input<Creator>,
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
            info!("Fetching following creators");
            match client.get_following_creators().await {
                Ok(list) => {
                    info!("Found {} following creators", list.len());
                    progress.creators.inc_length(list.len() as u64);
                    list
                }
                Err(e) => {
                    error!("Failed to get following creators: {e}");
                    vec![]
                }
            }
        },
        async {
            if !accepts.accept_supporting() {
                return vec![];
            }
            info!("Fetching supporting creators");
            match client.get_supporting_creators().await {
                Ok(list) => {
                    info!("Found {} supporting creators", list.len());
                    progress.creators.inc_length(list.len() as u64);
                    list
                }
                Err(e) => {
                    error!("Failed to get supporting creators: {e}");
                    vec![]
                }
            }
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

    for creator in creators {
        creator_pipeline.send(creator).unwrap();
    }
}

pub async fn creator_posts(
    mut creator_pipeline: Output<Creator>,
    posts_pipeline: Input<String>,
    context: &Mutex<Context>,
    manager: &Manager,
    config: &Config,
    client: &FanboxClient,
    progress: &ProgressSet,
) {
    let tasks: TaskSet<(String, CachedCreator), post_archiver_utils::error::Error> =
        TaskSet::new("creators").limit(2);

    while let Some(creator) = creator_pipeline.recv().await {
        let mut cached = context
            .lock()
            .await
            .get_creator(&creator.user_id, &creator.creator_id);

        if creator.creator_id != cached.creator_id {
            warn!(
                "Creator ID has changed: {} -> {}",
                cached.creator_id, creator.creator_id
            );
            let _ = patch_creator_source(manager, &cached, &creator)
                .await
                .inspect_err(|e| error!("Failed to update creator source: {}", e));
            cached.creator_id = creator.creator_id.clone();
        }

        let unimported_posts = cached.take_unimported_posts(creator.fee);
        progress.posts.inc_length(unimported_posts.len() as u64);
        for post in unimported_posts {
            posts_pipeline.send(post).unwrap();
        }

        let pb = progress.creators.clone();
        let post_pb = progress.posts.clone();
        let client = client.clone();
        let config = config.clone();
        let posts_pipeline = posts_pipeline.clone();
        tasks
            .spawn(async move {
                let urls = client
                    .list_post_paninate(&cached, config.strategy())
                    .await
                    .inspect_err(|e| error!("failed to fetch creator pagination: {e}"))?;

                for url in urls {
                    let posts = client
                        .list_post(&url, cached.last_updated, config.strategy())
                        .await
                        .inspect_err(|e| error!("failed to list creator posts: {e}"))?;

                    let mut count = 0;
                    for post in posts {
                        cached.last_updated =
                            cached.last_updated.max(post.published_datetime.timestamp());

                        if creator.fee < post.fee_required {
                            cached.push_unimport(post.fee_required, post.id);
                            continue;
                        }

                        if !config.filter_post(&post) {
                            continue;
                        }

                        posts_pipeline.send(post.id).unwrap();
                        count += 1;
                    }
                    post_pb.inc_length(count);
                }

                pb.inc(1);
                Ok((creator.user_id.clone(), cached))
            })
            .await;
    }

    let mut context = context.lock().await;
    for (id, cached) in tasks.join_all().await.into_iter().flatten() {
        context.creators.insert(id, cached);
    }
    progress.creators.finish();
}

async fn patch_creator_source(
    manager: &Manager,
    cached: &CachedCreator,
    creator: &Creator,
) -> Result<usize> {
    let mut manager = manager.lock().await;
    let mut importer = manager.importer();
    let importer = importer.savepoint().unwrap();

    let fanbox_platform = importer.import_platform("fanbox".to_string())?;

    // update creator alias
    let mut stmt = importer.conn.prepare("UPDATE author_aliases SET source = ?1, link = 'https://' || ?1 || '.fanbox.cc/' WHERE platform = ?2 AND source = ?3").unwrap();
    stmt.execute(params![
        creator.creator_id,
        fanbox_platform,
        cached.creator_id
    ])?;
    drop(stmt);

    // update posts
    let mut stmt = importer.conn.prepare("UPDATE posts SET source = replace(source, ?1, ?2) WHERE platform = ?3 AND source LIKE ?1 || '%'").unwrap();

    let old_source = format!("https://{}.fanbox.cc/posts/", cached.creator_id);
    let new_source = format!("https://{}.fanbox.cc/posts/", creator.creator_id);

    let cols = stmt.execute(params![old_source, new_source, fanbox_platform])?;
    drop(stmt);

    importer.commit()?;
    Ok(cols)
}

pub fn display_creators(creators: &HashSet<Creator>) {
    if !log::log_enabled!(log::Level::Info) {
        return;
    }

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
}
