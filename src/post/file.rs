use std::{collections::HashMap, sync::Arc};

use futures::future::try_join_all;
use log::error;
use mime_guess::MimeGuess;
use post_archiver::importer::file_meta::UnsyncFileMeta;
use serde_json::json;
use tokio::{sync::Semaphore, task::JoinSet};

use crate::{
    api::FanboxClient,
    fanbox::{PostBody, PostFile, PostImage},
    Config, FilesPipelineOutput, Progress,
};

pub async fn download_files(mut files_pipeline: FilesPipelineOutput, config: Config, pb: Progress) {
    let mut tasks = JoinSet::new();
    let client = FanboxClient::new(&config);

    let semaphore = Arc::new(Semaphore::new(3));
    while let Some((urls, tx)) = files_pipeline.recv().await {
        if urls.is_empty() {
            tx.send(Default::default()).unwrap();
            continue;
        }

        let files_pb = pb.files.clone();
        let client = client.clone();
        let semaphore = semaphore.clone();
        tasks.spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();
            match try_join_all(urls.into_iter().map(|url| async {
                let download_path = client.download(&url);
                let result = download_path.await.map(|path| (url, path));
                files_pb.inc(1);
                result.inspect_err(|e| error!("Failed to download file: {e}"))
            }))
            .await
            {
                Ok(urls) => tx.send(urls.into_iter().collect()).unwrap(),
                Err(e) => error!("Failed to receive file URLs: {e}"),
            }
        });
    }

    tasks.join_all().await;
    pb.files.finish();
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
