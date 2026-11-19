mod body;

use std::{
    collections::HashMap, mem, path::{Path, PathBuf},
};

use crate::{
    api::{FanboxClient, FanboxDownloader},
    config::ProgressSet,
    context::Context,
    fanbox::Post,
    Manager,
};
use log::{debug, error, info, warn};
use mime_guess::{mime::IMAGE_JPEG, MimeGuess};
use plyne::{Input, Output};
use post_archiver::importer::{
    file_meta::UnsyncFileMeta, post::UnsyncPost, UnsyncAlias, UnsyncAuthor, UnsyncCollection,
    UnsyncTag,
};
use post_archiver_utils::{Result, TaskSet};
use serde_json::json;
use tempfile::{tempdir};
use tokio::sync::Mutex;

pub async fn posts(
    mut posts_pipeline: Output<String>,
    client: FanboxClient,
    context: &Mutex<Context>,
    manager: Manager,
    progress: ProgressSet,
) {
    let post_tasks = TaskSet::new("posts").limit(4);
    let downloader = FanboxDownloader::new(progress.files.clone());

    while let Some(post_id) = posts_pipeline.recv().await {
        let progress = progress.clone();
        let client = client.clone();
        let downloader = downloader.clone();
        let manager = manager.clone();
        post_tasks
            .spawn(async move {
                let Ok(temp_dir) = tempdir() else {
                    error!("Failed to create temporary directory for post {post_id}");
                    return Err(post_id);
                };

                let result = post(client, downloader, post_id.clone(), temp_dir.path()).await;
                progress.posts.inc(1);
                let post = match result {
                    Ok(post) => {
                        debug!("Post downloaded: {}", post.source);
                        post
                    }
                    Err(e) => {
                        error!("Failed to download post: {e}");
                        return Err(post_id);
                    }
                };

                let mut manager = manager.lock().await;
                let source = post.source.clone();
                if let Err(e) = manager.importer().import_post(post) {
                    error!("Failed to import post: {source}\n{e}");
                    return Err(post_id);
                }
                info!("Post imported: {source}");
                Ok(())
            })
            .await;
    }

    let results = post_tasks.join_all().await;
    progress.posts.finish();
    progress.files.finish();

    context.lock().await.failed_posts = results.into_iter().flat_map(|r| r.err()).collect();
}

async fn post(
    client: FanboxClient,
    downloader: FanboxDownloader,
    post_id: String,
    with_in: &Path,
) -> Result<UnsyncPost> {
    let post = client.get_post(&post_id).await?;
    debug!("Fetched post {}", post.id);

    let comments = client
        .get_post_comments(&post_id, post.comment_count)
        .await
        .inspect_err(|e| error!("Failed to fetch comments for post {post_id}: {e}"))
        .unwrap_or_default();

    let thumb = download_cover(&post, with_in, &downloader).await?;
    let content = post.body.content(with_in, &downloader).await?;

    let source = get_source_link(&post.creator_id, &post.id);

    let tags = collect_tags(&post);
    let authors = collect_authors(&post);
    let collections = collect_collections(&post);

    let comments = comments.into_iter().map(|c| c.into()).collect();

    Ok(
        UnsyncPost::new("fanbox".to_string(), source, post.title, content)
            .thumb(thumb)
            .updated(post.updated_datetime)
            .published(post.published_datetime)
            .tags(tags)
            .authors(authors)
            .collections(collections)
            .comments(comments),
    )
}

async fn download_cover(
    post: &Post,
    with_in: &Path,
    downloader: &FanboxDownloader,
) -> Result<Option<UnsyncFileMeta>> {
    let Some(url) = &post.cover_image_url else {
        return Ok(None);
    };

    let path = downloader.download(with_in, url).await?;
    let filename = url.split('/').next_back().unwrap().to_string();
    let mime = MimeGuess::from_path(&filename)
        .first_or(IMAGE_JPEG)
        .to_string();
    let extra = HashMap::from([
        ("width".to_string(), json!(1200)),
        ("height".to_string(), json!(630)),
    ]);
    Ok(Some(UnsyncFileMeta::new(filename, mime, path).extra(extra)))
}

fn collect_tags(post: &Post) -> Vec<UnsyncTag> {
    let mut tags = vec![];
    if post.fee_required == 0 {
        tags.push(UnsyncTag {
            name: "free".to_string(),
            platform: None,
        })
    }

    if post.has_adult_content {
        tags.push(UnsyncTag {
            name: "r-18".to_string(),
            platform: None,
        })
    }

    tags
}

fn collect_collections(post: &Post) -> Vec<UnsyncCollection> {
    let mut collections = Vec::with_capacity(post.tags.len());
    for tag in &post.tags {
        let encoded = urlencoding::encode(tag);
        collections.push(UnsyncCollection::new(
            tag.clone(),
            format!("https://{}.fanbox.cc/tags/{encoded}", post.creator_id),
        ));
    }
    collections
}

fn collect_authors(post: &Post) -> Vec<UnsyncAuthor> {
    vec![UnsyncAuthor::new(post.user.name.to_string()).aliases(vec![
        UnsyncAlias::new("fanbox".to_string(), post.creator_id.clone())
            .link(format!("https://{}.fanbox.cc/", post.creator_id.clone())),
        UnsyncAlias::new("pixiv".to_string(), post.user.user_id.clone()).link(format!(
            "https://www.pixiv.net/users/{}",
            post.user.user_id.clone()
        )),
    ])]
}

pub async fn failed_posts(posts_input: Input<String>, context: &Mutex<Context>, progress: &ProgressSet) {
    let failed_posts = &mut context.lock().await.failed_posts;
    if failed_posts.is_empty() {
        return;
    }
    warn!("Retrying {} previously failed posts", failed_posts.len());
    progress.posts.inc_length(failed_posts.len() as u64);
    for post in mem::take(failed_posts) {
        posts_input.send(post).unwrap();
    }
}

pub fn get_source_link(creator: &str, post: &str) -> String {
    format!("https://{creator}.fanbox.cc/posts/{post}")
}
