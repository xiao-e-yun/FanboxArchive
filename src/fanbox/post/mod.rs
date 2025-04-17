pub mod body;
pub mod item;

pub use super::{PostType, User};
use chrono::{DateTime, Utc};

pub use body::*;
pub use item::*;

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(rename_all = "camelCase")]
pub struct Post {
    pub id: String,
    pub title: String,
    pub fee_required: u32,
    pub published_datetime: DateTime<Utc>,
    pub updated_datetime: DateTime<Utc>,
    pub tags: Vec<String>,
    pub is_liked: bool,
    pub like_count: u32,
    pub comment_count: u32,
    pub is_restricted: bool,
    pub user: User,
    pub creator_id: String,
    pub has_adult_content: bool,
    #[serde(rename = "type")]
    pub ty: PostType,
    pub cover_image_url: Option<String>,
    pub body: PostBody,
    pub excerpt: String,
    pub next_post: Option<PostShort>,
    pub prev_post: Option<PostShort>,
    pub image_for_share: String,
    #[serde(default)]
    pub is_pinned: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(rename_all = "camelCase")]
pub struct PostComments {
    pub view_mode: String,
    pub comment_list: Option<PostCommentList>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(rename_all = "camelCase")]
pub struct PostCommentList {
    pub items: Vec<Comment>,
    pub next_url: Option<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub body: String,
    pub created_datetime: DateTime<Utc>,
    pub id: String,
    pub is_liked: bool,
    pub is_own: bool,
    pub like_count: u32,
    pub parent_comment_id: String,
    #[serde(default)]
    pub replies: Vec<Comment>,
    pub root_comment_id: String,
    pub user: User,
}

impl Into<post_archiver::Comment> for Comment {
    fn into(self) -> post_archiver::Comment {
        post_archiver::Comment {
            user: self.user.name,
            text: self.body,
            replies: self.replies.into_iter().map(|c| c.into()).collect(),
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(rename_all = "camelCase")]
pub struct PostShort {
    id: String,
    title: String,
    published_datetime: DateTime<Utc>,
}
