mod body;
pub mod file;

use std::collections::HashMap;

use crate::{
    creator::sync_creator,
    fanbox::{Comment, Post, PostListItem},
    Client, FilesPipelineInput, Manager, PostsPipelineOutput, Progress, SyncPipelineInput,
    SyncPipelineOutput,
};
use file::FanboxFileMeta;
use log::{debug, error, trace};
use post_archiver::{
    importer::{file_meta::UnsyncFileMeta, post::UnsyncPost, UnsyncCollection, UnsyncTag},
    manager::{PostArchiverConnection, PostArchiverManager},
    AuthorId, PlatformId,
};
use serde_json::json;
use tokio::{join, sync::oneshot, task::JoinSet};

pub fn filter_unsynced_post(
    manager: &PostArchiverManager<impl PostArchiverConnection>,
    post: &PostListItem,
) -> bool {
    let source = get_source_link(&post.creator_id, &post.id);
    manager
        .find_post_with_updated(&source, &post.updated_datetime)
        .expect("Failed to check post")
        .is_none()
}

pub async fn get_posts(
    mut posts_pipeline: PostsPipelineOutput,
    files_pipeline: FilesPipelineInput,
    sync_piepline: SyncPipelineInput,
    client: Client,
    pb: Progress,
) {
    let mut join_set = JoinSet::new();

    while let Some(posts) = posts_pipeline.recv().await {
        for post in posts {
            let posts_pb = pb.posts.clone();
            let files_pb = pb.files.clone();
            let client = client.clone();
            let files_pipeline = files_pipeline.clone();
            let sync_piepline = sync_piepline.clone();
            join_set.spawn(async move {
                let result = join![
                    client.get_post(&post.id),
                    client.get_post_comments(&post.id, post.comment_count)
                ];
                debug!("Fetched post {}", post.id);

                match result {
                    (Ok(post), comments) => {
                        let comments = comments.unwrap_or_else(|e| {
                            error!("Failed to fetch comments for post {}: {}", post.id, e);
                            vec![]
                        });

                        let (tx, rx) = oneshot::channel();
                        let files = post
                            .body
                            .files()
                            .into_iter()
                            .map(|f| f.data)
                            .chain(post.cover_image_url.clone())
                            .collect::<Vec<_>>();

                        files_pb.inc_length(files.len() as u64);
                        files_pipeline.send((files, tx)).unwrap();
                        sync_piepline.send((post, comments, rx)).unwrap();
                    }
                    (Err(e), _) => error!("Failed to fetch post {}: {}", post.id, e),
                };

                posts_pb.inc(1);
            });
        }
    }

    join_set.join_all().await;
    pb.posts.finish();
}

pub async fn sync_posts(mut sync_piepline: SyncPipelineOutput, manager: Manager) {
    let mut authors = HashMap::new();
    while let Some((post, comments, rx)) = sync_piepline.recv().await {
        let mut manager = manager.lock().await;

        let fanbox_platform = manager.import_platform("fanbox".to_string()).unwrap();
        let pixiv_platform = manager.import_platform("pixiv".to_string()).unwrap();

        let tx = manager.transaction().unwrap();

        let Ok(author) = sync_creator(&tx, &mut authors, [fanbox_platform, pixiv_platform], &post)
        else {
            error!("Failed to sync creator for post: {}", post.id);
            continue;
        };

        let post = conversion_post(fanbox_platform, author, post, comments);
        let source = post.source.clone();

        let Ok((_, _, _, files)) = tx.import_post(post, true) else {
            error!("Failed to import post: {source}");
            continue;
        };

        let Ok(mut file_map) = rx.await else {
            error!("Failed to receive file map for post: {source}");
            continue;
        };

        
        let mut failed = false;
        let mut created = false;
        for (path, url) in files {
            if !created {
                let path = path.parent().unwrap();
                if let Err(e) = std::fs::create_dir_all(path) {
                    error!( "Failed to create directory {}: {}", path.display(), e);
                    failed = true;
                    break;
                }
                created = true;
            }
            match file_map.remove(&url) {
                Some(temp) => {
                    match std::fs::copy(&temp, &path) {
                        Ok(_) => trace!("File saved: {}", path.display()),
                        Err(e) => {
                            error!("Failed to save file {}: {}", path.display(), e);
                            failed = true;
                            break;
                        }
                    };
                    std::fs::remove_file(&temp).ok();
                },
                None => {
                    error!("File URL not found in map: {url}");
                    failed = true;
                    break;
                }
            }
        }

        if failed {
            error!("Aborting post import due to file errors: {source}");
        } else {
            debug!("Post imported: {source}");
            tx.commit().unwrap();
        }
    }

    fn conversion_post(
        platform: PlatformId,
        author: AuthorId,
        post: Post,
        comments: Vec<Comment>,
    ) -> UnsyncPost<String> {
        let source = get_source_link(&post.creator_id, &post.id);

        let mut tags = vec![];
        if post.fee_required == 0 {
            tags.push("free".to_string());
        }
        if post.has_adult_content {
            tags.push("r-18".to_string());
        }
        let tags = tags
            .into_iter()
            .map(|tag| UnsyncTag {
                name: tag,
                platform: None,
            })
            .collect::<Vec<_>>();

        let collections = post
            .tags
            .into_iter()
            .map(|tag| {
                UnsyncCollection::new(
                    tag.clone(),
                    format!("https://{}.fanbox.cc/tags/{}", post.creator_id, tag),
                )
            })
            .collect();

        let thumb = post.cover_image_url.clone().map(|url| {
            let mut meta = UnsyncFileMeta::from_url(url.clone());
            meta.extra = HashMap::from([
                ("width".to_string(), json!(1200)),
                ("height".to_string(), json!(630)),
            ]);
            meta
        });

        let content = post.body.content();

        let comments = comments.into_iter().map(|c| c.into()).collect();

        UnsyncPost::new(platform, source, post.title, content)
            .thumb(thumb)
            .comments(comments)
            .collections(collections)
            .authors(vec![author])
            .published(post.published_datetime)
            .updated(post.updated_datetime)
            .tags(tags)
    }
}

pub fn get_source_link(creator_id: &str, post_id: &str) -> String {
    format!("https://{creator_id}.fanbox.cc/posts/{post_id}")
}
