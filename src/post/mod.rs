mod body;

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::{
    api::{FanboxClient, FanboxDownloader},
    config::{Config, IncOnDrop, ProgressSet},
    context::{Context, UnimportedPost},
    fanbox::Post,
    Manager,
};
use log::{debug, error, info, warn};
use mime_guess::{mime::IMAGE_JPEG, MimeGuess};
use plyne::Output;
use post_archiver::importer::{
    file_meta::UnsyncFileMeta, post::UnsyncPost, UnsyncAlias, UnsyncAuthor, UnsyncCollection,
    UnsyncTag,
};
use post_archiver_utils::{Result, Strategy, TaskSet};
use serde_json::json;
use tempfile::tempdir;
use tokio::{sync::Mutex, try_join};

pub async fn posts(
    mut posts_pipeline: Output<UnimportedPost>,
    client: FanboxClient,
    context: &Mutex<Context>,
    manager: Manager,
    config: &Config,
    progress: ProgressSet,
) {
    let post_tasks = TaskSet::new("posts").limit(4);
    let downloader = FanboxDownloader::new(config, progress.files.clone());

    let mut seen = HashSet::new();

    while let Some(unimported) = posts_pipeline.recv().await {
        let guard = IncOnDrop(progress.posts.clone());

        if !seen.insert(unimported.id.clone()) {
            debug!("Skipping duplicate post {}", unimported.id);
            continue;
        }

        if matches!(config.strategy(), Strategy::Increment) {
            let mut manager = manager.lock().await;
            let importer = manager.importer();
            let source = get_source_link(&unimported.creator, &unimported.id);
            if let Ok(Some(_)) = importer.lookup_post(&source) {
                debug!("Skipping already imported post {}", unimported.id);
                continue;
            }
        }

        let client = client.clone();
        let downloader = downloader.clone();
        let manager = manager.clone();
        post_tasks
            .spawn(post_task(client, downloader, manager, unimported, guard))
            .await
    }

    let results = post_tasks.join_all().await;
    progress.posts.finish();
    progress.files.finish();

    let mut ctx = context.lock().await;
    for failed in results.into_iter().flat_map(std::result::Result::err) {
        let creator = ctx.creators.get_mut(&failed.user).unwrap();
        creator
            .unimports
            .entry(failed.fee)
            .or_default()
            .insert(failed.id);
    }
}

async fn post_task(
    client: FanboxClient,
    downloader: FanboxDownloader,
    manager: Manager,
    mut unimported: UnimportedPost,
    _guard: IncOnDrop,
) -> std::result::Result<(), UnimportedPost> {
    let Ok(temp_dir) = tempdir() else {
        error!(
            "Failed to create temporary directory for post {}",
            unimported.id
        );
        return Err(unimported);
    };

    let post = match client.get_post(&unimported.id).await {
        Ok(post) if post.is_restricted => {
            warn!("Post {} is restricted, skipping", post.id);
            unimported.fee = post.fee_required;
            return Err(unimported);
        }
        Ok(post) if post.body.is_none() => {
            warn!("Post {} has no body, skipping\n{post:#?}", post.id);
            unimported.fee = post.fee_required;
            return Err(unimported);
        }
        Ok(post) => {
            debug!("Fetched post {}", post.id);
            post
        }
        Err(e) => {
            error!("Failed to fetch post: {e}");
            return Err(unimported);
        }
    };

    let post = match resolve_post(client, downloader, post, temp_dir.path()).await {
        Ok(post) => {
            debug!("Post downloaded: {}", post.source);
            post
        }
        Err(e) => {
            error!("Failed to download post: {e}");
            return Err(unimported);
        }
    };

    let source = post.source.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut manager = manager.blocking_lock();
        let result = manager.importer().import_post(post);
        drop(temp_dir);
        result
    })
    .await;

    match result {
        Ok(Ok(_)) => {
            info!("Post imported: {source}");
            Ok(())
        }
        Ok(Err(e)) => {
            error!("Failed to import post: {source}\n{e}");
            Err(unimported)
        }
        Err(e) => {
            error!("Import task panicked: {source}\n{e}");
            Err(unimported)
        }
    }
}

async fn resolve_post(
    client: FanboxClient,
    downloader: FanboxDownloader,
    post: Post,
    with_in: &Path,
) -> Result<UnsyncPost> {
    let comments = client
        .get_post_comments(&post.id, post.comment_count)
        .await
        .inspect_err(|e| error!("Failed to fetch comments for post {}: {e}", post.id))
        .unwrap_or_default();

    let (thumb, content) = try_join![
        download_cover(&post, with_in, &downloader),
        post.body.as_ref().unwrap().content(with_in, &downloader)
    ]?;

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

pub fn get_source_link(creator: &str, post: &str) -> String {
    format!("https://{creator}.fanbox.cc/posts/{post}")
}
