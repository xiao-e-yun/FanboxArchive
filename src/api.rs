use std::{
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Once,
    },
};

use chrono::NaiveDateTime;
use indicatif::ProgressBar;
use log::{debug, error};
use post_archiver_utils::{browser_headers, ArchiveClient, Limit, RequestError, Result, Strategy};
use reqwest::{
    header::{self, HeaderMap, HeaderValue},
    Client, StatusCode, Url,
};
use serde::{
    de::{DeserializeOwned, IgnoredAny},
    Deserialize,
};

use crate::{
    config::Config,
    context::CachedCreator,
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
    available: Arc<AtomicBool>,
}

impl FanboxClient {
    pub fn new(config: &Config) -> Self {
        let user_agent = config.user_agent();

        let mut default_headers = Self::generate_user_headers(&user_agent);
        debug!("Using headers: {default_headers:#?} (without cookies)");

        default_headers.insert(header::COOKIE, config.cookies().parse().unwrap());

        Self {
            inner: ArchiveClient::new(
                Client::builder()
                    .default_headers(default_headers)
                    .build()
                    .unwrap(),
                Limit {
                    per_min: config.limit(),
                    ..Default::default()
                },
            ),
            available: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn generate_user_headers(user_agent: &str) -> HeaderMap {
        let mut headers = browser_headers(user_agent);
        let referer: HeaderValue = "https://www.fanbox.cc/".parse().unwrap();
        headers.insert(header::ORIGIN, referer.clone());
        headers.insert(header::REFERER, referer);
        headers
    }

    pub async fn fetch<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        if !self.available.load(Ordering::SeqCst) {
            return Err(RequestError::InvalidSession.into());
        }

        let response = match self.inner.fetch::<FanboxAPIResponse<T>>(url).await {
            Ok(response) => response,
            Err(RequestError::UnexpectedStatus(status, body)) => {
                return Err(self.status_error(status, body).into())
            }
            Err(e) => return Err(e.into()),
        };

        match response.body {
            Some(body) => Ok(body),
            None => Err(match response.error.as_str() {
                "general_error" => RequestError::InvalidSession,
                _ => RequestError::InvalidResponse(response.error),
            }
            .into()),
        }
    }

    /// Map a non-success response using Fanbox's JSON `error` field when present.
    fn status_error(&self, status: StatusCode, body: String) -> RequestError {
        let error = serde_json::from_str::<FanboxAPIResponse<IgnoredAny>>(&body)
            .map(|r| r.error)
            .unwrap_or_default();

        static ONCE: Once = Once::new();
        match error.as_str() {
            "pop_key_registration_required" | "pop_signature_required" => {
                ONCE.call_once(|| {
                    error!("Fanbox requires PoP key verification for this session ({error}).");
                    error!("Open https://www.fanbox.cc/pop?mode=register in your browser while logged in, then retry.");
                });
                self.available.store(false, Ordering::SeqCst);
                RequestError::InvalidSession
            }
            "general_error" => {
                self.available.store(false, Ordering::SeqCst);
                RequestError::InvalidSession
            }
            _ => RequestError::UnexpectedStatus(status, body),
        }
    }

    pub async fn get_supporting_creators(&self) -> Result<APIListSupportingCreator> {
        let url = "https://api.fanbox.cc/plan.listSupporting";

        #[derive(Deserialize)]
        struct Response {
            plans: APIListSupportingCreator,
        }

        self.fetch::<Response>(url)
            .await
            .map(|response| response.plans)
    }

    pub async fn get_following_creators(&self) -> Result<APIListFollowingCreator> {
        let url = "https://api.fanbox.cc/creator.listFollowing";

        #[derive(Deserialize)]
        struct Response {
            creators: APIListFollowingCreator,
        }

        self.fetch::<Response>(url)
            .await
            .map(|response| response.creators)
    }

    pub async fn list_post_paninate(
        &self,
        creator: &CachedCreator,
        strategy: Strategy,
    ) -> Result<APIListCreatorPaginate> {
        let url = format!(
            "https://api.fanbox.cc/post.paginateCreator?creatorId={}",
            creator.creator_id
        );

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct PaginateResponse {
            page_urls: APIListCreatorPaginate,
        }

        let urls = self.fetch::<PaginateResponse>(&url).await?.page_urls;

        if matches!(strategy, Strategy::Full) {
            return Ok(urls);
        }

        let trimmed_urls = urls
            .into_iter()
            .filter(|url| match parse_url(url) {
                Some(date) => date > creator.last_updated,
                None => true,
            })
            .collect();

        fn parse_url(url: &str) -> Option<i64> {
            let url = Url::parse(url).ok()?;
            let date_str = url
                .query_pairs()
                .find(|(k, _)| k == "firstPublishedDatetime")
                .map(|(_, v)| v.to_string())?;

            NaiveDateTime::parse_from_str(&date_str, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|dt| dt.and_utc().timestamp())
        }

        Ok(trimmed_urls)
    }

    pub async fn list_post(
        &self,
        url: &str,
        last_updated: i64,
        strategy: Strategy,
    ) -> Result<APIListCreatorPost> {
        #[derive(Deserialize)]
        struct ListResponse {
            posts: APIListCreatorPost,
        }

        let posts = self
            .fetch::<ListResponse>(url)
            .await
            .map(|response| response.posts)?;

        if matches!(strategy, Strategy::Full) {
            return Ok(posts);
        }

        Ok(posts
            .into_iter()
            .filter(|post| post.published_datetime.timestamp() > last_updated)
            .collect())
    }

    pub async fn get_post(&self, post_id: &str) -> Result<APIPost> {
        let url = format!("https://api.fanbox.cc/post.info?postId={post_id}");

        #[derive(Deserialize)]
        struct Response {
            post: APIPost,
        }

        self.fetch::<Response>(&url)
            .await
            .map(|response| response.post)
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
            comments.extend(response.items);
        }
        Ok(comments)
    }
}

#[derive(Debug, Clone)]
pub struct FanboxDownloader {
    inner: ArchiveClient,
    pb: ProgressBar,
}

impl FanboxDownloader {
    pub fn new(pb: ProgressBar) -> Self {
        Self {
            inner: ArchiveClient::new(
                Client::new(),
                Limit {
                    per_min: NonZeroU32::new(90).unwrap(),
                    burst: NonZeroU32::new(4).unwrap(),
                    ..Default::default()
                },
            ),
            pb,
        }
    }

    pub async fn download(&self, with_in: &Path, url: &str) -> Result<PathBuf> {
        struct Guard<'a>(&'a ProgressBar);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.0.inc(1);
            }
        }

        self.pb.inc_length(1);
        let _guard = Guard(&self.pb);
        Ok(self.inner.download(with_in, url).await?)
    }
}

#[derive(Deserialize, Debug)]
pub struct FanboxAPIResponse<T> {
    body: Option<T>,
    #[serde(default)]
    error: String,
}
