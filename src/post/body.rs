use std::{collections::HashMap, path::Path};

use futures::future::try_join_all;
use log::error;
use post_archiver::importer::{file_meta::UnsyncFileMeta, post::UnsyncContent};
use post_archiver_utils::{Error, Result};
use serde_json::json;
use tokio::try_join;

use crate::{
    api::FanboxDownloader,
    fanbox::{PostBlock, PostBlockStyle, PostBody, PostEmbed, PostTextEmbed, PostVideo},
    post::get_source_link,
};

impl PostBody {
    pub async fn content(
        &self,
        with_in: &Path,
        client: &FanboxDownloader,
    ) -> Result<Vec<UnsyncContent>> {
        let ((images, files), unordereds) = try_join![
            self.inline_files(with_in, client),
            self.unordered_files(with_in, client)
        ]?;

        let mut content = self.text(&images, &files);
        content.extend(unordereds.into_iter().map(UnsyncContent::File));

        for video in self.videos.iter().flatten() {
            content.push(UnsyncContent::Text(video.to_text()));
        }

        if let Some(video) = &self.video {
            content.push(UnsyncContent::Text(video.to_text()));
        }

        Ok(content)
    }

    pub fn text(
        &self,
        images: &HashMap<String, UnsyncFileMeta>,
        files: &HashMap<String, UnsyncFileMeta>,
    ) -> Vec<UnsyncContent> {
        let mut content = vec![];
        if let Some(text) = self.text.clone() {
            let text = text.replace("\n", "  \n");
            if !text.is_empty() {
                content.push(UnsyncContent::Text(text));
            }
        }

        if let Some(blocks) = self.blocks.as_ref() {
            for block in blocks.clone() {
                content.push(block.to_text(self, images, files));
            }
        }

        content
    }

    async fn inline_files(
        &self,
        with_in: &Path,
        client: &FanboxDownloader,
    ) -> Result<(
        HashMap<String, UnsyncFileMeta>,
        HashMap<String, UnsyncFileMeta>,
    )> {
        let images = self.image_map.iter().flatten().map(|(id, image)| async {
            let filename = image.filename();
            let mime = image.mime();
            let extra = HashMap::from([
                ("width".to_string(), json!(image.width)),
                ("height".to_string(), json!(image.height)),
            ]);
            let source = client.download(with_in, &image.original_url).await?;
            Ok::<(String, UnsyncFileMeta), Error>((
                id.clone(),
                UnsyncFileMeta::new(filename, mime, source).extra(extra),
            ))
        });

        let files = self.file_map.iter().flatten().map(|(id, file)| async {
            let filename = file.filename();
            let mime = file.mime();
            let source = client.download(with_in, &file.url).await?;
            Ok((id.clone(), UnsyncFileMeta::new(filename, mime, source)))
        });

        let (images, files) = try_join!(try_join_all(images), try_join_all(files))?;
        Ok((images.into_iter().collect(), files.into_iter().collect()))
    }

    async fn unordered_files(
        &self,
        with_in: &Path,
        client: &FanboxDownloader,
    ) -> Result<Vec<UnsyncFileMeta>> {
        let images = self.images.iter().flatten().map(|image| async {
            let filename = image.filename();
            let mime = image.mime();
            let extra = HashMap::from([
                ("width".to_string(), json!(image.width)),
                ("height".to_string(), json!(image.height)),
            ]);
            let source = client.download(with_in, &image.original_url).await?;
            Ok::<UnsyncFileMeta, Error>(UnsyncFileMeta::new(filename, mime, source).extra(extra))
        });

        let files = self.files.iter().flatten().map(|file| async {
            let filename = file.filename();
            let mime = file.mime();
            let source = client.download(with_in, &file.url).await?;
            Ok(UnsyncFileMeta::new(filename, mime, source))
        });

        let (mut images, files) = try_join!(try_join_all(images), try_join_all(files))?;
        images.extend(files);
        Ok(images)
    }
}

impl PostBlock {
    pub fn to_text(
        self,
        body: &PostBody,
        images: &HashMap<String, UnsyncFileMeta>,
        files: &HashMap<String, UnsyncFileMeta>,
    ) -> UnsyncContent {
        match self {
            PostBlock::P { text, styles } => {
                if text.is_empty() {
                    UnsyncContent::Text("  \n".to_string())
                } else {
                    UnsyncContent::Text(Self::style_text(text, styles))
                }
            }
            PostBlock::Header { text, styles } => {
                UnsyncContent::Text(format!("# {}", Self::style_text(text, styles)))
            }
            PostBlock::Image { image_id } => match images.get(&image_id) {
                Some(image) => UnsyncContent::File(image.clone()),
                None => UnsyncContent::Text(format!("[Image Mismatch: {image_id}]")),
            },
            PostBlock::File { file_id } => match files.get(&file_id) {
                Some(file) => UnsyncContent::File(file.clone()),
                None => UnsyncContent::Text(format!("[File Mismatch: {file_id}]")),
            },
            PostBlock::Embed { embed_id } => {
                let Some(embed) = body.embed_map.as_ref().unwrap().get(&embed_id) else {
                    return UnsyncContent::Text(format!("[Embed not found: {embed_id}]"));
                };
                UnsyncContent::Text(embed.to_text())
            }
            PostBlock::Video { video_id } => {
                let videos = body.videos.as_ref().unwrap();
                let video = videos.iter().find(|v| v.video_id == video_id).unwrap();
                UnsyncContent::Text(video.to_text())
            }
            PostBlock::UrlEmbed { url_embed_id } => {
                let Some(url_embed) = body.url_embed_map.as_ref().unwrap().get(&url_embed_id)
                else {
                    return UnsyncContent::Text(format!("[URL Embed not found: {url_embed_id}]"));
                };
                UnsyncContent::Text(url_embed.to_text())
            }
        }
    }

    pub fn style_text(text: String, styles: Option<Vec<PostBlockStyle>>) -> String {
        let Some(mut styles) = styles else {
            return text;
        };

        let mut insert_map: HashMap<usize, String> = HashMap::new();
        styles.sort_by_key(|a| a.offset);
        while let Some(style) = styles.pop() {
            let offset = style.offset as usize;
            let length = style.length as usize;
            let (prefix, suffix) = match style.ty.as_str() {
                "bold" => ("**", "**"),
                _ => {
                    error!("Unknown style: {style:?}");
                    ("", "")
                }
            };
            let prefix_entry = insert_map.entry(offset).or_default();
            *prefix_entry += prefix;

            let suffix_entry = insert_map.entry(offset + length).or_default();
            *suffix_entry = suffix.to_string() + suffix_entry;
        }
        // Insert the styles in reverse order to avoid messing up the offsets.
        let mut output = String::new();
        for (i, char) in text.chars().enumerate() {
            if let Some(insert) = insert_map.get(&i) {
                output += insert;
            }
            output.push(char);
        }
        output
    }
}

impl PostVideo {
    pub fn to_text(&self) -> String {
        match self.service_provider.as_str() {
            "youtube" => {
                format!("[<img alt=\"Youtube thumbnail\" src=\"https://img.youtube.com/vi/{}/0.jpg\">](https://www.youtube.com/watch?v={})",self.video_id, self.video_id)
            }
            _ => {
                error!("Unknown video provider ({})", self.service_provider);
                error!("video_id: {}", self.video_id);
                format!("{}'s video: {}", self.service_provider, self.video_id)
            }
        }
    }
}

impl PostEmbed {
    pub fn to_text(&self) -> String {
        match self.service_provider.as_str() {
            "youtube" => {
                format!("[![youtube](https://img.youtube.com/vi/{}/0.jpg)](https://www.youtube.com/watch?v={})",self.content_id, self.content_id)
            }
            "google_forms" => {
                format!(
                    "[Google Form](https://docs.google.com/forms/d/e/{}/viewform)",
                    self.content_id
                )
            }
            "fanbox" => {
                fn deconstruct(input: &str) -> std::result::Result<(String, String), &'static str> {
                    let parts: Vec<&str> = input.split('/').collect();
                    if parts.len() == 4 && parts[0] == "creator" && parts[2] == "post" {
                        let creator = parts[1].to_string();
                        let post = parts[3].to_string();
                        Ok((creator, post))
                    } else {
                        Err("The input string does not match the expected format.")
                    }
                }

                let (creator, post) = deconstruct(&self.content_id).unwrap();
                format!(
                    "[Fanbox Post ({}/{})]({})",
                    creator,
                    post,
                    get_source_link(&creator, &post)
                )
            }
            "twitter" => {
                format!(
                    "[Tweet](https://twitter.com/i/web/status/{})",
                    self.content_id
                )
            }
            provider => {
                error!("Unknown embed provider ({provider})");
                error!("id: {}", self.id);
                error!("content_id: {}", self.content_id);
                format!("{}'s embed: {}", provider, self.content_id)
            }
        }
    }
}

impl PostTextEmbed {
    pub fn to_text(&self) -> String {
        match self {
            PostTextEmbed::Html { id: _, html } => {
                let Some(start) = html.find("<iframe src=\"") else {
                    return "[Invalid URL Embed]".to_string();
                };
                let mut src = html.split_at(start + 13).1;
                let Some(end) = src.find("\"") else {
                    return "[Invalid URL Embed]".to_string();
                };
                src = src.split_at(end).0;

                format!("[{src}]({src})")
            }
            PostTextEmbed::HtmlCard { id: _, html } => {
                let Some(start) = html.find("<iframe src=\"") else {
                    return "[Invalid URL Embed]".to_string();
                };
                let mut src = html.split_at(start + 13).1;

                let Some(end) = src.find("\"") else {
                    return "[Invalid URL Embed]".to_string();
                };
                src = src.split_at(end).0;

                format!("[{src}]({src})")
            }
            PostTextEmbed::FanboxPost { id: _id, post_info } => {
                format!(
                    "[Fanbox Post {}]({})",
                    post_info.title,
                    get_source_link(&post_info.creator_id, &post_info.id)
                )
            }
            PostTextEmbed::FanboxCreator { id: _, profile } => {
                format!(
                    "[Creator {}](https://{}.fanbox.cc)",
                    profile.name(),
                    profile.creator_id()
                )
            }
            PostTextEmbed::Default {
                id: _,
                url,
                host: _,
            } => {
                format!("[{url}]({url})")
            }
        }
    }
}
