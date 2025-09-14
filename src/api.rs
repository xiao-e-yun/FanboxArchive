
use log::debug;
use post_archiver_utils::{ArchiveClient, Error, Result};
use reqwest::{
    header::{self, HeaderMap},
    Client,
};
use serde::{de::DeserializeOwned, Deserialize};
use tempfile::TempPath;
use tokio::task::JoinSet;

use crate::{
    config::Config,
    fanbox::{Comment, FollowingCreator, Post, PostComments, PostListItem, SupportingCreator},
};

pub type APIPost = Post;
pub type APIPostComments = Vec<Comment>;
pub type APIListCreatorPost = Vec<PostListItem>;
pub type APIListSupportingCreator = Vec<SupportingCreator>;
pub type APIListFollowingCreator = Vec<FollowingCreator>;
pub type APIListCreatorPaginate = Vec<String>;

#[derive(Debug, Clone)]
pub struct FanboxClient {
    inner: ArchiveClient,
}

impl FanboxClient {
    pub fn new(config: &Config) -> Self {
        let inner = ArchiveClient::builder(
            Client::builder()
                .user_agent(config.user_agent())
                .default_headers(HeaderMap::from_iter([
                    (header::COOKIE, config.cookies().parse().unwrap()),
                    (
                        header::ORIGIN,
                        "https://www.fanbox.cc".to_string().parse().unwrap(),
                    ),
                ]))
                .build()
                .unwrap(),
            config.limit(),
        )
        .pre_sec_limit(2)
        .build();

        Self { inner }
    }

    pub async fn fetch<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        let response = self.inner.fetch::<FanboxAPIResponse<T>>(url).await?;

        match response.body {
            Some(body) => Ok(body),
            None => Err(match response.error.as_str() {
                "general_error" => Error::InvalidSession,
                _ => Error::InvalidResponse(response.error),
            }),
        }
    }

    pub async fn download(&self, url: &str) -> Result<TempPath> {
        let path = self.inner.download(url).await?;
        debug!("Downloaded {url}");
        Ok(path)
    }

    pub async fn get_supporting_creators(&self) -> Result<APIListSupportingCreator> {
        let url = "https://api.fanbox.cc/plan.listSupporting";
        self.fetch(url).await
    }

    pub async fn get_following_creators(&self) -> Result<APIListFollowingCreator> {
        let url = "https://api.fanbox.cc/creator.listFollowing";
        self.fetch(url).await
    }

    pub async fn get_posts(&self, creator: &str) -> Result<APIListCreatorPost> {
        let url = format!("https://api.fanbox.cc/post.paginateCreator?creatorId={creator}",);
        let urls: APIListCreatorPaginate = self.fetch(&url).await?;

        let mut tasks = JoinSet::new();
        for url in urls {
            let client = self.clone();
            tasks.spawn(async move { client.fetch::<APIListCreatorPost>(&url).await });
        }

        tasks
            .join_all()
            .await
            .into_iter()
            .try_collect::<Vec<APIListCreatorPost>>()
            .map(|posts| posts.into_iter().flatten().collect::<APIListCreatorPost>())
    }

    pub async fn get_post(&self, post_id: &str) -> Result<APIPost> {
        let url = format!("https://api.fanbox.cc/post.info?postId={post_id}");
        self.fetch(&url).await
    }

    pub async fn get_post_comments(&self, id: &str, comment_count: u32) -> Result<APIPostComments> {
        if comment_count == 0 {
            return Ok(vec![]);
        };

        let mut next_url = Some(format!(
            "https://api.fanbox.cc/post.getComments?postId={id}&limit=10",
        ));
        let mut comments = vec![];
        while let Some(url) = next_url {
            let PostComments { comment_list, .. } = self.fetch(&url).await?;
            let response = match comment_list {
                Some(list) => list,
                None => break,
            };

            next_url = response.next_url;
            comments.extend(response.items.into_iter());
        }
        Ok(comments)
    }
}

#[derive(Deserialize, Debug)]
pub struct FanboxAPIResponse<T> {
    body: Option<T>,
    #[serde(default)]
    error: String,
}
