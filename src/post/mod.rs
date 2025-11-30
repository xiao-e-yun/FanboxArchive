mod body;
pub mod file;

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use crate::{
    api::FanboxClient,
    config::ProgressSet,
    context::Context,
    creator::sync_creator,
    fanbox::{Comment, Post, PostListItem},
    FileEvent, Manager, SyncEvent,
};
use file::FanboxFileMeta;
use futures::try_join;
use log::{debug, error, info, trace, warn};
use plyne::{Input, Output};
use post_archiver::{
    importer::{file_meta::UnsyncFileMeta, post::UnsyncPost, UnsyncCollection, UnsyncTag},
    manager::{PostArchiverConnection, PostArchiverManager},
    AuthorId, PlatformId,
};
use post_archiver_utils::Result;
use serde_json::json;
use tempfile::TempPath;
use tokio::{
    fs::{create_dir_all, File, OpenOptions},
    io, join,
    sync::{oneshot, Mutex},
    task::JoinSet,
};

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
    posts_input: Input<Vec<PostListItem>>,
    mut posts_pipeline: Output<Vec<PostListItem>>,
    files_pipeline: Input<FileEvent>,
    sync_piepline: Input<SyncEvent>,
    client: &FanboxClient,
    context: &Context,
    pb: &ProgressSet,
) {
    let mut join_set = JoinSet::new();

    // check failed posts
    check_failed_posts(posts_input, context, pb);

    let failed_posts = Arc::new(Mutex::new(vec![]));
    while let Some(posts) = posts_pipeline.recv().await {
        for post in posts {
            let posts_pb = pb.posts.clone();
            let files_pb = pb.files.clone();
            let client = client.clone();
            let failed_posts = failed_posts.clone();
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
                    (Err(e), _) => {
                        error!("Failed to fetch post {}: {}", post.id, e);
                        let mut failed_posts = failed_posts.lock().await;
                        failed_posts.push(post);
                    }
                };

                posts_pb.inc(1);
            });
        }
    }

    join_set.join_all().await;

    update_failed_posts(context, failed_posts).await;

    pb.posts.finish();
}

fn check_failed_posts(posts_input: Input<Vec<PostListItem>>, context: &Context, pb: &ProgressSet) {
    debug!("Checking failed posts");
    let failed_posts = context.failed_posts.borrow();
    if failed_posts.is_empty() { return }
    warn!("Retrying {} previously failed posts", failed_posts.len());
    posts_input.send(failed_posts.clone()).unwrap();
    pb.posts.inc(failed_posts.len() as u64);
}

async fn update_failed_posts(context: &Context, failed_posts: Arc<Mutex<Vec<PostListItem>>>) {
    debug!("Recording failed posts");
    let updated_failed_posts = Arc::into_inner(failed_posts).unwrap().into_inner();
    if !updated_failed_posts.is_empty() {
        error!("{} posts failed to download", updated_failed_posts.len());
    }
    *context.failed_posts.borrow_mut() = updated_failed_posts;
}

pub async fn sync_posts(mut sync_piepline: Output<SyncEvent>, manager: &Manager) {
    let mut authors = HashMap::new();
    'post: while let Some((post, comments, rx)) = sync_piepline.recv().await {
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

        let mut create_dir = true;
        for (path, url) in files {
            if let Err(e) = save_file(&mut file_map, &path, &url, create_dir).await {
                error!("Failed to save file {}: {}", path.display(), e);
                error!("Aborting post import due to file errors: {source}");
                continue 'post;
            };
            create_dir = false;
        }

        info!("Post imported: {source}");
        tx.commit().unwrap();
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
            let mut meta = UnsyncFileMeta::from_url(url);
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

    async fn save_file(
        file_map: &mut HashMap<String, TempPath>,
        path: &PathBuf,
        url: &str,
        create_dir: bool,
    ) -> Result<()> {
        if create_dir {
            let path = path.parent().unwrap();
            create_dir_all(path).await?;
        }

        let temp = file_map.remove(url).ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            format!("File not found in map: {url}"),
        ))?;

        let mut open_options = OpenOptions::new();
        let (mut src, mut dst) = try_join!(
            File::open(&temp),
            open_options
                .create(true)
                .write(true)
                .truncate(true)
                .open(&path)
        )?;

        io::copy(&mut src, &mut dst).await?;
        trace!("File saved: {url} -> {}", path.display());

        Ok(())
    }
}

pub fn get_source_link(creator_id: &str, post_id: &str) -> String {
    format!("https://{creator_id}.fanbox.cc/posts/{post_id}")
}
