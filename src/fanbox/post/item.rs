use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::fanbox::User;

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(rename_all = "camelCase")]
pub struct PostListItem {
    pub id: String,
    pub title: String,
    pub fee_required: u32,
    pub published_datetime: DateTime<Utc>,
    pub updated_datetime: DateTime<Utc>,
    pub tags: Vec<String>,
    pub is_liked: bool,
    pub like_count: u32,
    pub is_commenting_restricted: bool,
    pub comment_count: u32,
    pub is_restricted: bool,
    pub user: User,
    pub creator_id: String,
    pub has_adult_content: bool,
    pub cover: Option<Cover>,
    pub excerpt: String,
    #[serde(default)]
    pub is_pinned: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Cover {
    CoverImage { url: String },
    PostImage { url: String },
    Video { video: Video },
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(rename_all = "camelCase")]
pub struct Video {
    pub service_provider: String,
    pub video_id: String,
}
