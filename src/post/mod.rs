mod body;
pub mod file;

use std::collections::HashMap;

use crate::{
    api::FanboxClient,
    config::Config,
    fanbox::{Comment, Post, PostListItem},
};
use file::{download_files, FanboxFileMeta};
use futures::future::join;
use log::info;
use post_archiver::{
    importer::{file_meta::UnsyncFileMeta, post::UnsyncPost, UnsyncCollection, UnsyncTag},
    manager::{PostArchiverConnection, PostArchiverManager},
    AuthorId, PlatformId,
};
use rusqlite::Connection;
use serde_json::json;
use tokio::task::JoinSet;

pub async fn get_post_urls(
    config: &Config,
    client: &FanboxClient,
    creator_id: &str,
) -> Result<Vec<PostListItem>, Box<dyn std::error::Error>> {
    let mut items = client.get_posts(creator_id).await?;
    items.retain(|item| config.filter_post(item));
    Ok(items)
}

pub fn filter_unsynced_posts(
    manager: &mut PostArchiverManager<impl PostArchiverConnection>,
    mut posts: Vec<PostListItem>,
) -> Result<Vec<PostListItem>, rusqlite::Error> {
    posts.retain(|post| {
        let source = get_source_link(&post.creator_id, &post.id);
        let post_updated = manager
            .find_post_with_updated(&source, &post.updated_datetime)
            .expect("Failed to check post");
        post_updated.is_none()
    });
    Ok(posts)
}

pub async fn get_posts(
    config: &Config,
    client: &FanboxClient,
    posts: Vec<PostListItem>,
) -> Result<Vec<(Post, Vec<Comment>)>, Box<dyn std::error::Error>> {
    let pb = config.progress("posts");
    pb.set_length(posts.len() as u64);

    let mut tasks = JoinSet::new();

    for post in posts {
        let client = client.clone();
        tasks.spawn(async move {
            join(
                client.get_post(&post.id),
                client.get_post_comments(&post.id, post.comment_count),
            )
            .await
        });
    }

    let mut posts = Vec::new();
    while let Some(Ok((post, comments))) = tasks.join_next().await {
        let (post, comments) = (post?, comments?);
        posts.push((post, comments));
        pb.inc(1);
    }

    Ok(posts)
}

pub async fn sync_posts(
    manager: &mut PostArchiverManager<Connection>,
    client: &FanboxClient,
    config: &Config,
    author: AuthorId,
    posts: Vec<(Post, Vec<Comment>)>,
) -> Result<(), Box<dyn std::error::Error>> {
    let manager = manager.transaction()?;
    let total_posts = posts.len();

    let platform = manager.import_platform("fanbox".to_string())?;

    let posts = posts
        .into_iter()
        .map(|(post, comments)| conversion_post(platform, author, post, comments))
        .collect::<Result<Vec<_>, _>>()?;

    let (_posts, post_files) = manager.import_posts(posts, true)?;

    let pb = config.progress("files");
    pb.set_length(post_files.len() as u64);
    download_files(&pb, client, post_files).await?;

    manager.commit()?;

    info!("{total_posts} total");

    fn conversion_post(
        platform: PlatformId,
        author: AuthorId,
        post: Post,
        comments: Vec<Comment>,
    ) -> Result<UnsyncPost<String>, Box<dyn std::error::Error>> {
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
