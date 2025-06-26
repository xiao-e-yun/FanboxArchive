mod body;
pub mod file;

use std::{collections::HashMap, fmt::format};

use crate::{
    api::fanbox::FanboxClient,
    config::Config,
    fanbox::{Comment, Post, PostListItem},
};
use file::{download_files, FanboxFileMeta};
use log::info;
use post_archiver::{
    importer::{file_meta::UnsyncFileMeta, post::UnsyncPost, UnsyncCollection, UnsyncTag},
    manager::{PostArchiverConnection, PostArchiverManager},
    AuthorId, PlatformId,
};
use rusqlite::Connection;
use serde_json::json;

pub async fn get_post_urls(
    config: &Config,
    creator_id: &str,
) -> Result<Vec<PostListItem>, Box<dyn std::error::Error>> {
    let client = FanboxClient::new(config);
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
    posts: Vec<PostListItem>,
) -> Result<Vec<(Post, Vec<Comment>)>, Box<dyn std::error::Error>> {
    let client = FanboxClient::new(config);
    let mut tasks = vec![];
    for post in posts {
        let client = client.clone();
        tasks.push(tokio::spawn(async move {
            let post_meta = client.get_post(&post.id);
            let comments = client.get_post_comments(&post.id, post.comment_count);

            (
                post_meta.await.expect("failed to get post"),
                comments.await.expect("failed to get comments of post"),
            )
        }));
    }

    let mut posts = Vec::new();

    for task in tasks {
        posts.push(task.await?);
    }

    Ok(posts)
}

pub async fn sync_posts(
    manager: &mut PostArchiverManager<Connection>,
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

    let client = FanboxClient::new(config);
    download_files(post_files, &client).await?;

    manager.commit()?;

    info!("{} total", total_posts);

    fn conversion_post(
        platform: PlatformId,
        author: AuthorId,
        post: Post,
        comments: Vec<Comment>,
    ) -> Result<(UnsyncPost, HashMap<String, String>), Box<dyn std::error::Error>> {
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

        let collections = post.tags.into_iter().map(|tag| 
            UnsyncCollection::new(tag.clone(), format!("https://{}.fanbox.cc/tags/{}", post.creator_id, tag))
        ).collect();

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

        let mut files = post
            .body
            .files()
            .into_iter()
            .map(|(meta, url)| (meta.filename.clone(), url))
            .collect::<HashMap<_, _>>();

        if let Some((thumb, url)) = &thumb {
            files.insert(thumb.filename.clone(), url.clone());
        }

        let post = UnsyncPost::new(platform, source, post.title, content)
            .thumb(thumb.map(|v| v.0))
            .comments(comments)
            .collections(collections)
            .authors(vec![author])
            .published(post.published_datetime)
            .updated(post.updated_datetime)
            .tags(tags);

        Ok((post, files))
    }

    Ok(())
}

pub fn get_source_link(creator_id: &str, post_id: &str) -> String {
    format!("https://{}.fanbox.cc/posts/{}", creator_id, post_id)
}
