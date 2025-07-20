use std::{collections::HashMap, path::PathBuf, sync::Arc};

use log::error;
use mime_guess::MimeGuess;
use post_archiver::importer::file_meta::UnsyncFileMeta;
use serde_json::json;
use tokio::{sync::Semaphore, task::JoinSet};

use crate::{
    api::FanboxClient, config::{Progress}, fanbox::{PostBody, PostFile, PostImage}
};

pub async fn download_files(
    pb: &Progress, 
    client: &FanboxClient,
    files: Vec<(PathBuf, String)>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut tasks = JoinSet::new();

    let mut last_folder = PathBuf::new();
    let semphore = Arc::new(Semaphore::new(3));
    for (path, url) in files {
        // Create folder if it doesn't exist
        let folder = path.parent().unwrap();
        if last_folder != folder {
            last_folder = folder.to_path_buf();
            tokio::fs::create_dir_all(folder).await?;
        }

        let semphore = semphore.clone();
        let client = client.clone();
        let pb = pb.clone();
        tasks.spawn(async move {
            let permit = semphore.acquire().await.unwrap();
            if let Err(e) = client.download(&url, path.clone()).await {
                error!("Failed to download {} to {}: {}", url, path.display(), e);
            };
            pb.inc(1);
        });
    }

    tasks.join_all().await;

    Ok(())
}

pub trait FanboxFileMeta
where
    Self: Sized,
{
    fn from_url(url: String) -> Self;
    fn from_image(image: PostImage) -> Self;
    fn from_file(file: PostFile) -> Self;
}

impl FanboxFileMeta for UnsyncFileMeta<String> {
    fn from_url(url: String) -> Self {
        let filename = url.split('/').next_back().unwrap().to_string();
        let mime = MimeGuess::from_path(&filename)
            .first_or_octet_stream()
            .to_string();
        let extra = Default::default();

        Self {
            filename,
            mime,
            extra,
            data: url,
        }
    }
    fn from_image(image: PostImage) -> Self {
        let filename = image.filename();
        let mime = image.mime();
        let extra = HashMap::from([
            ("width".to_string(), json!(image.width)),
            ("height".to_string(), json!(image.height)),
        ]);

        Self {
            filename,
            mime,
            extra,
            data: image.original_url,
        }
    }
    fn from_file(file: PostFile) -> Self {
        let filename = file.filename();
        let mime = file.mime();
        let extra = Default::default();

        Self {
            filename,
            mime,
            extra,
            data: file.url,
        }
    }
}

impl PostBody {
    pub fn files(&self) -> Vec<UnsyncFileMeta<String>> {
        let mut files: Vec<UnsyncFileMeta<String>> = vec![];

        if let Some(list) = self.images.clone() {
            files.extend(post_images_to_files(list));
        }

        if let Some(map) = self.image_map.clone() {
            files.extend(post_images_to_files(map.into_values().collect()));
        };

        if let Some(list) = self.files.clone() {
            files.extend(post_files_to_files(list));
        }

        if let Some(map) = self.file_map.clone() {
            files.extend(post_files_to_files(map.into_values().collect()));
        };

        // util function
        fn post_images_to_files(images: Vec<PostImage>) -> Vec<UnsyncFileMeta<String>> {
            images.into_iter().map(UnsyncFileMeta::from_image).collect()
        }

        fn post_files_to_files(files: Vec<PostFile>) -> Vec<UnsyncFileMeta<String>> {
            files.into_iter().map(UnsyncFileMeta::from_file).collect()
        }

        files
    }
}
