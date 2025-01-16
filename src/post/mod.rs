mod body;

use std::path::PathBuf;

use crate::{
    api::fanbox::FanboxClient,
    config::Config,
    creator::SyncedCreator,
    fanbox::{Creator, Post, PostBody, PostListItem},
};
use futures::future::join_all;
use log::{error, info};
use post_archiver::{AuthorId, Content, PostId};
use rusqlite::{params, Connection, OptionalExtension, ToSql, Transaction};

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
    let mut stmt = conn.prepare("SELECT id FROM posts WHERE source = ? AND updated != ?")?;
    posts.retain(|post| {
        let source = get_source_link(&post.creator_id, &post.id);
        let updated = post.updated_datetime.to_sql().unwrap();
        !stmt.exists(params![source, updated]).unwrap()
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
    tag: u32,
    creator: &SyncedCreator,
    posts: Vec<Post>,
) -> Result<(), Box<dyn std::error::Error>> {
    let total_posts = posts.len();
    let mut synced_posts = 0;

    let mut all_files = vec![];
    let author = creator.author().id.raw();
    let mut tx = conn.transaction()?;
    for post in posts {
        info!(" syncing {}", post.title());
        match sync_post(&mut tx, author, tag, post) {
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

    info!("downloading {} files", all_files.len());
    let client = FanboxClient::new(&config);
    download_files(all_files, client, config.output()).await?;
    tx.commit()?;

    fn sync_post(
        tx: &mut Transaction,
        author: u32,
        tag: u32,
        post: Post,
    ) -> Result<Vec<SyncedFile>, Box<dyn std::error::Error>> {
        let post_id = sync_post_meta(tx, author, &post, tag)?;
        let body = post.body();
        let files = sync_files(tx, &body, author, post_id)?;
        let mapped = files
            .iter()
            .map(|file| (file.raw_id.clone(), file.id.into()))
            .collect();
        sync_post_content(tx, post_id, body.content(&mapped))?;
        info!(" + {} files", files.len());
        Ok(files)
    }

    fn sync_post_meta(
        tx: &mut Transaction,
        author: u32,
        post: &Post,
        tag: u32,
    ) -> Result<u32, Box<dyn std::error::Error>> {
        let mut insert_post_stmt = tx.prepare_cached("INSERT INTO posts (author,source,title,content,updated,published) VALUES (?,?,?,?,?,?) RETURNING id")?;
        let mut insert_tag_stmt =
            tx.prepare_cached("INSERT INTO post_tags (post,tag) VALUES (?,?)")?;

        let source = get_source_link(&post.creator(), &post.id());
        let title = post.title();
        let content = "UNSYNCED".to_string();
        let updated = post.updated_datetime;
        let published = post.published_datetime;

        let post_id: u32 = insert_post_stmt
            .query_row(
                params![author, source, title, content, updated, published],
                |row| row.get(0),
            )
            .unwrap();
        insert_tag_stmt.execute(params![post_id, tag]).unwrap();

        Ok(post_id)
    }

    fn sync_post_content(
        tx: &mut Transaction,
        post_id: u32,
        content: Vec<Content>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut insert_post_stmt =
            tx.prepare_cached("UPDATE posts SET content = ? WHERE id = ?")?;
        insert_post_stmt.execute(params![serde_json::to_string(&content)?, post_id])?;
        Ok(())
    }

    Ok(())
}

fn sync_files(
    tx: &mut Transaction,
    post_body: &PostBody,
    author: u32,
    post: u32,
) -> Result<Vec<SyncedFile>, Box<dyn std::error::Error>> {
    let mut insert_file_stmt = tx.prepare_cached(
        "INSERT INTO file_metas (filename,author,post,mime,extra) VALUES (?,?,?,?,?) RETURNING id",
    )?;
    let files = post_body.files(AuthorId::from(author), PostId::from(post));
    let mut collect = vec![];
    for file in files {
        let id: u32 = insert_file_stmt
            .query_row(
                params![
                    &file.filename,
                    file.author.raw(),
                    file.post.raw(),
                    &file.mime,
                    serde_json::to_string(&file.extra).unwrap(),
                ],
                |row| row.get(0),
            )
            .unwrap();

        let path = PathBuf::from(file.author.to_string())
            .join(file.post.to_string())
            .join(&file.filename);
        let url = file.url.clone();
        collect.push(SyncedFile {
            id,
            path,
            url,
            raw_id: file.id,
        });
    }
    Ok(collect)
}

async fn download_files(
    files: Vec<SyncedFile>,
    client: FanboxClient,
    output: &PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut tasks = vec![];

    let mut last_folder = PathBuf::new();
    for file in files {
        let path = output.join(&file.path);

        // Create folder if it doesn't exist
        let folder = path.parent().unwrap();
        if last_folder != folder {
            last_folder = folder.to_path_buf();
            tokio::fs::create_dir_all(folder).await?;
        }

        let client = client.clone();
        tasks.push(tokio::spawn(async move {
            client
                .download(&file.url, path)
                .await
                .expect("Failed to download file");
        }));
    }

    join_all(tasks).await;
    Ok(())
}

pub fn get_or_insert_tag(conn: &mut Connection) -> Result<u32, rusqlite::Error> {
    const TAG_NAME: &str = "fanbox";

    match conn
        .query_row("SELECT id FROM tags WHERE name = ?", [TAG_NAME], |row| {
            row.get(0)
        })
        .optional()?
    {
        Some(id) => Ok(id),
        None => conn.query_row(
            "INSERT INTO tags (name) VALUES (?) RETURNING id",
            [TAG_NAME],
            |row| row.get(0),
        ),
    }
}

pub fn get_source_link(creator_id: &str, post_id: &str) -> String {
    format!("https://{}.fanbox.cc/posts/{}", creator_id, post_id)
}

#[derive(Debug)]
pub struct SyncedFile {
    pub path: PathBuf,
    pub url: String,
    pub raw_id: String,
    pub id: u32,
}