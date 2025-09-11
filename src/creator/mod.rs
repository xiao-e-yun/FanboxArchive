use std::collections::HashSet;

use itertools::Itertools;
use log::{error, info, warn};
use post_archiver::importer::{UnsyncAlias, UnsyncAuthor};
use post_archiver_utils::Result;
use tokio::{join, task::JoinSet};

use crate::{
    fanbox::Creator, post::{filter_unsynced_post, POST_CHUNK_SIZE}, Authors, AuthorsProgress, Client, Config, Manager, PostsPipelineInput, PostsProgress
};

pub async fn get_creators(
    config: Config,
    client: Client,
    manager: Manager,
    authors: Authors,
    posts_pipeline: PostsPipelineInput,
    authors_pb: AuthorsProgress,
    posts_pb: PostsProgress,
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
    if let Err(e) = sync_creators(&manager, creators, &authors).await {
        error!("Failed to sync creators: {e}");
        return;
    };

    authors_pb.set_length(filtered as u64);
    let mut set = authors
        .iter()
        .map(|entry| (client.clone(), entry.key().clone()))
        .map(|(client, creator)| async move { client.get_posts(&creator).await })
        .collect::<JoinSet<_>>();

    while let Some(post) = set.join_next().await {
        authors_pb.inc(1);
        match post.unwrap() {
            Ok(items) => {
                posts_pb.inc_length(items.len() as u64);
                let manager = manager.lock().await;
                let chunks = items
                    .into_iter()
                    .filter(|item| config.filter_post(item))
                    .filter(|item| filter_unsynced_post(&manager, item))
                    .chunks(POST_CHUNK_SIZE);
                for chunk in &chunks {
                    posts_pipeline.send(chunk.collect()).unwrap();
                }
            }
            Err(e) => {
                error!("Failed to get posts: {e}");
            }
        }
    }

    authors_pb.finish();
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
            "+-{}-+-{}--+------------ - -",
            "-".to_string().repeat(id_width),
            "-".to_string().repeat(fee_width)
        );
        info!("");
    }
}

pub async fn sync_creators(
    manager: &Manager,
    creators: HashSet<Creator>,
    authors: &Authors,
) -> Result<()> {
    let mut manager = manager.lock().await;
    let manager = manager.transaction()?;

    let fanbox_platform = manager.import_platform("fanbox".to_string())?;
    let pixiv_platform = manager.import_platform("pixiv".to_string())?;

    for creator in creators {
        let Ok(author) = UnsyncAuthor::new(creator.name.to_string())
            .aliases(vec![
                UnsyncAlias::new(fanbox_platform, creator.creator_id.clone())
                    .link(format!("https://{}.fanbox.cc/", creator.creator_id)),
                UnsyncAlias::new(pixiv_platform, creator.user.user_id.clone()).link(format!(
                    "https://www.pixiv.net/users/{}",
                    creator.user.user_id
                )),
            ])
            .sync(&manager)
        else {
            error!("Failed to sync author: {}", creator.creator_id);
            continue;
        };

        authors.insert(creator.creator_id, author);
    }

    manager.commit().unwrap();
    Ok(())
}
