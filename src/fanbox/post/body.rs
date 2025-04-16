use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::super::FollowingCreator;
use super::PostListItem;

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(rename_all = "camelCase", tag = "type")]
pub struct PostBody {
    pub text: Option<String>,
    pub blocks: Option<Vec<PostBlock>>,
    pub images: Option<Vec<PostImage>>,
    pub videos: Option<Vec<PostVideo>>,
    pub video: Option<PostVideo>,
    pub files: Option<Vec<PostFile>>,
    pub image_map: Option<BTreeMap<String, PostImage>>,
    pub file_map: Option<BTreeMap<String, PostFile>>,
    pub embed_map: Option<BTreeMap<String, PostEmbed>>,
    pub url_embed_map: Option<BTreeMap<String, PostTextEmbed>>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum PostBlock {
    P {
        text: String,
        styles: Option<Vec<PostBlockStyle>>,
    },
    Header {
        text: String,
        styles: Option<Vec<PostBlockStyle>>,
    },
    #[serde(rename_all = "camelCase")]
    Image { image_id: String },
    #[serde(rename_all = "camelCase")]
    File { file_id: String },
    #[serde(rename_all = "camelCase")]
    Embed { embed_id: String },
    #[serde(rename_all = "camelCase")]
    UrlEmbed { url_embed_id: String },
    #[serde(rename_all = "camelCase")]
    Video { video_id: String },
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(rename_all = "camelCase")]
pub struct PostBlockStyle {
    #[serde(rename = "type")]
    pub ty: String,
    pub offset: u32,
    pub length: u32,
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(rename_all = "camelCase")]
pub struct PostImage {
    pub id: String,
    pub extension: String,
    pub width: u32,
    pub height: u32,
    pub original_url: String,
    pub thumbnail_url: String,
}

impl PostImage {
    pub fn filename(&self) -> String {
        format!("{}.{}", self.id, self.extension)
    }
    pub fn mime(&self) -> String {
        mime_guess::from_ext(&self.extension)
            .first_or_octet_stream()
            .to_string()
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(rename_all = "camelCase")]
pub struct PostVideo {
    pub service_provider: String,
    pub video_id: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(rename_all = "camelCase")]
pub struct PostFile {
    pub id: String,
    pub name: String,
    pub extension: String,
    pub size: u64,
    pub url: String,
}

impl PostFile {
    pub fn filename(&self) -> String {
        format!("{}.{}", self.name, self.extension)
    }
    pub fn mime(&self) -> String {
        mime_guess::from_ext(&self.extension)
            .first_or_octet_stream()
            .to_string()
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(rename_all = "camelCase")]
pub struct PostEmbed {
    pub id: String,
    pub service_provider: String,
    pub content_id: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PostTextEmbed {
    #[serde(rename = "html")]
    Html { id: String, html: String },
    #[serde(rename = "html.card")]
    HtmlCard { id: String, html: String },
    #[serde(rename = "fanbox.post", rename_all = "camelCase")]
    FanboxPost { id: String, post_info: PostListItem },
    #[serde(rename = "fanbox.creator")]
    FanboxCreator {
        id: String,
        profile: FollowingCreator,
    },
    Default {
        id: String,
        url: String,
        host: String,
    },
}
