mod body;
pub mod file;

use crate::{
    api::fanbox::FanboxClient,
    config::Config,
    creator::SyncedCreator,
    fanbox::{Creator, Post, PostListItem},
};
use chrono::{DateTime, Utc};
use file::{download_files, get_files, sync_files, SyncedFile};
use log::{error, info};
use post_archiver::{AuthorId, Content, PostId, PostTagId};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

pub async fn get_post_urls(
    config: &Config,
    creator: &Creator,
) -> Result<Vec<PostListItem>, Box<dyn std::error::Error>> {
    let client = FanboxClient::new(&config);
    let mut items = client.get_posts(creator).await?;
    items.retain(|item| config.filter_post(item));
    Ok(items)
}

pub fn filter_unsynced_posts(
    conn: &mut Connection,
    mut posts: Vec<PostListItem>,
) -> Result<Vec<PostListItem>, rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT updated FROM posts WHERE source = ?")?;
    posts.retain(|post| {
        let source = get_source_link(&post.creator_id, &post.id);
        let updated = post.updated_datetime;

        let post_updated: Option<DateTime<Utc>> = stmt
            .query_row(params![source], |row| row.get(0))
            .optional()
            .unwrap();

        match post_updated {
            Some(post_updated) => post_updated < updated,
            None => true,
        }
    });
    Ok(posts)
}

pub async fn get_posts(
    config: &Config,
    posts: Vec<PostListItem>,
) -> Result<Vec<Post>, Box<dyn std::error::Error>> {
    let client = FanboxClient::new(&config);
    let mut tasks = vec![];
    for post in posts {
        let client = client.clone();
        tasks.push(tokio::spawn(async move {
            client.get_post(post.id).await.expect("Failed to get post")
        }));
    }

    let mut posts = Vec::new();

    for task in tasks {
        posts.push(task.await?);
    }

    Ok(posts)
}

pub async fn sync_posts(
    conn: &mut Connection,
    config: &Config,
    creator: &SyncedCreator,
    posts: Vec<Post>,
    fanbox_and_free_tag: (PostTagId, PostTagId),
) -> Result<(), Box<dyn std::error::Error>> {
    let total_posts = posts.len();
    let mut synced_posts = 0;

    let mut all_files = vec![];
    let author = creator.author().id;
    let mut tx = conn.transaction()?;
    for post in posts {
        info!(" syncing {}", post.title());
        match sync_post(&mut tx, author, post, fanbox_and_free_tag) {
            Ok(files) => {
                synced_posts += 1;
                if !files.is_empty() {
                    all_files.extend(files);
                }
            }
            Err(e) => error!(" + failed: {}", e),
        }
    }

    info!("{} total", total_posts);
    info!("{} success", synced_posts);
    if total_posts != synced_posts {
        info!("{} failed", total_posts - synced_posts);
    }

    if !all_files.is_empty() {
        info!("");
        info!("Downloading {} files", all_files.len());
        let client = FanboxClient::new(&config);
        download_files(all_files, client, config.output()).await?;
    }
    tx.commit()?;

    fn sync_post(
        tx: &mut Transaction,
        author: AuthorId,
        post: Post,
        fanbox_and_free_tag: (PostTagId, PostTagId),
    ) -> Result<Vec<SyncedFile>, Box<dyn std::error::Error>> {
        let post_id = sync_post_meta(tx, author, &post, fanbox_and_free_tag)?;
        let body = post.body();

        let cover_url = post.cover_image_url.as_ref();
        let files = get_files(cover_url, &body, author, post_id);
        let files = sync_files(tx, files)?;

        let files_map = files
            .iter()
            .map(|file| (file.raw_id.clone(), file.id.into()))
            .collect();
        sync_post_content(tx, post_id, body.content(&files_map))?;
        info!(" + {} files", files.len());
        Ok(files)
    }

    fn sync_post_meta(
        tx: &mut Transaction,
        author: AuthorId,
        post: &Post,
        (fanbox_tag, free_tag): (PostTagId, PostTagId),
    ) -> Result<PostId, Box<dyn std::error::Error>> {
        let mut select_post_stmt = tx.prepare_cached("SELECT id FROM posts WHERE source = ?")?;
        let mut update_post_stmt =
            tx.prepare_cached("UPDATE posts SET updated = ? WHERE id = ?")?;
        let mut insert_post_stmt = tx.prepare_cached("INSERT INTO posts (author,source,title,content,updated,published) VALUES (?,?,?,?,?,?) RETURNING id")?;
        let mut insert_tag_stmt =
            tx.prepare_cached("INSERT OR IGNORE INTO post_tags (post,tag) VALUES (?,?)")?;

        let source = get_source_link(&post.creator(), &post.id());
        let title = post.title();
        let content = "[\"UNSYNCED\"]".to_string();
        let updated = post.updated_datetime;
        let published = post.published_datetime;

        let post_id: PostId = match select_post_stmt
            .query_row(params![source], |row| row.get(0))
            .optional()
            .unwrap()
        {
            Some(id) => {
                update_post_stmt.execute(params![updated, id]).unwrap();
                id
            }
            None => insert_post_stmt
                .query_row(
                    params![author, source, title, content, updated, published],
                    |row| row.get(0),
                )
                .unwrap(),
        };

        insert_tag_stmt
            .execute(params![post_id, fanbox_tag])
            .unwrap();
        if post.fee_required == 0 {
            insert_tag_stmt.execute(params![post_id, free_tag]).unwrap();
        }

        Ok(post_id)
    }

    fn sync_post_content(
        tx: &mut Transaction,
        post_id: PostId,
        content: Vec<Content>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut insert_post_stmt =
            tx.prepare_cached("UPDATE posts SET content = ? WHERE id = ?")?;
        insert_post_stmt.execute(params![serde_json::to_string(&content)?, post_id])?;
        Ok(())
    }

    Ok(())
}

pub fn get_or_insert_tag(conn: &mut Connection, name: &str) -> Result<PostTagId, rusqlite::Error> {
    match conn
        .query_row("SELECT id FROM tags WHERE name = ?", [name], |row| {
            row.get(0)
        })
        .optional()?
    {
        Some(id) => Ok(id),
        None => conn.query_row(
            "INSERT INTO tags (name) VALUES (?) RETURNING id",
            [name],
            |row| row.get(0),
        ),
    }
}

pub fn get_source_link(creator_id: &str, post_id: &str) -> String {
    format!("https://{}.fanbox.cc/posts/{}", creator_id, post_id)
}
