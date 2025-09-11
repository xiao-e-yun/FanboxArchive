mod body;
pub mod file;

use std::collections::HashMap;

use crate::{
    api::FanboxClient, fanbox::{Comment, Post, PostListItem}, Authors, Client, Config, FilesProgress, Manager, PostsPipelineOutput, PostsProgress
};
use file::{download_files, FanboxFileMeta};
use futures::future::join_all;
use log::{debug, error};
use post_archiver::{
    importer::{file_meta::UnsyncFileMeta, post::UnsyncPost, UnsyncCollection, UnsyncTag},
    manager::{PostArchiverConnection, PostArchiverManager},
    AuthorId, PlatformId,
};
use post_archiver_utils::Result;
use serde_json::json;
use tokio::join;

pub const POST_CHUNK_SIZE: usize = 12;

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
    manager: Manager,
    config: Config,
    client: Client,
    authors: Authors,
    mut posts_pipeline: PostsPipelineOutput,
    post_pb: PostsProgress,
    file_pb: FilesProgress,
) {
    let download_client = FanboxClient::new(&config);
    while let Some(posts) = posts_pipeline.recv().await {
        let client = &client;
        let post_pb = &post_pb;

        let posts = join_all(posts.into_iter().map(|post| async move {
            let result = join![
                client.get_post(&post.id),
                client.get_post_comments(&post.id, post.comment_count)
            ];
            debug!("Fetched post {}", post.id);
            post_pb.inc(1);
            match result {
                (Ok(post), comments) => {
                    let comments = comments.unwrap_or_else(|e| {
                        error!("Failed to fetch comments for post {}: {}", post.id, e);
                        vec![]
                    });
                    Some((post, comments))
                }
                (Err(e), _) => {
                    error!("Failed to fetch post {}: {}", post.id, e);
                    None
                }
            }
        }))
        .await
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

        if posts.is_empty() {
            continue;
        }

        let creator = posts.first().map(|(p, _)| p.creator_id.as_str()).unwrap();
        let author = *authors.get(creator).expect("Author not found");

        sync_posts(&manager, &download_client, &file_pb, author, posts).await.unwrap();
    }

    post_pb.finish();
    file_pb.finish();
}

pub async fn sync_posts(
    manager: &Manager,
    client: &FanboxClient,
    file_pb: &FilesProgress,
    author: AuthorId,
    posts: Vec<(Post, Vec<Comment>)>,
) -> Result<()> {
    let mut manager = manager.lock().await;
    let manager = manager.transaction()?;

    let platform = manager.import_platform("fanbox".to_string())?;

    let posts = posts
        .into_iter()
        .map(|(post, comments)| conversion_post(platform, author, post, comments))
        .collect::<Result<Vec<_>>>()?;

    let (_posts, post_files) = manager.import_posts(posts, true)?;

    file_pb.inc_length(post_files.len() as u64);
    download_files(file_pb, client, post_files).await?;

    manager.commit()?;

    fn conversion_post(
        platform: PlatformId,
        author: AuthorId,
        post: Post,
        comments: Vec<Comment>,
    ) -> Result<UnsyncPost<String>> {
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
            (meta, url)
        });

        let content = post.body.content();

        let comments = comments.into_iter().map(|c| c.into()).collect();

        let post = UnsyncPost::new(platform, source, post.title, content)
            .thumb(thumb.map(|v| v.0))
            .comments(comments)
            .collections(collections)
            .authors(vec![author])
            .published(post.published_datetime)
            .updated(post.updated_datetime)
            .tags(tags);

        Ok(post)
    }

    Ok(())
}

pub fn get_source_link(creator_id: &str, post_id: &str) -> String {
    format!("https://{creator_id}.fanbox.cc/posts/{post_id}")
}
