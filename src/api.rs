use chrono::NaiveDateTime;
use log::debug;
use post_archiver_utils::{ArchiveClient, Error, Result};
use reqwest::{
    Client, Url, header::{self, HeaderMap}
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
        let user_agent = config.user_agent();

        let mut default_headers = Self::generate_user_headers(&user_agent);
        debug!("Using headers: {default_headers:#?} (without cookies)");

        default_headers.insert(header::COOKIE, config.cookies().parse().unwrap());

        let inner = ArchiveClient::builder(
            Client::builder()
                .default_headers(default_headers)
                .build()
                .unwrap(),
            config.limit(),
        )
        .pre_sec_limit(2)
        .build();

        Self { inner }
    }

    pub fn generate_user_headers(user_agent: &str) -> HeaderMap {
        let platform = if user_agent.contains("Windows") || user_agent.contains("Win64") {
            "\"Windows\""
        } else if user_agent.contains("Macintosh") || user_agent.contains("Mac OS X") {
            "\"macOS\""
        } else if user_agent.contains("Linux") || user_agent.contains("X11") {
            "\"Linux\""
        } else {
            "\"Unknown\""
        };

        let mobile = if user_agent.contains("Mobile") {
            "?1"
        } else {
            "?0"
        };

        let ua = if user_agent.contains("Edg/") {
            "Edg"
        } else if user_agent.contains("Chrome/") {
            "Chromium"
        } else if user_agent.contains("Firefox/") {
            "Firefox"
        } else if user_agent.contains("Safari/") && !user_agent.contains("Chrome/") {
            "Safari"
        } else {
            "Unknown"
        };

        let version = user_agent
            .split_whitespace()
            .find(|part| part.starts_with(ua))
            .unwrap_or("Unknown/12")
            .split('/')
            .nth(1)
            .unwrap_or("12")
            .split('.')
            .next()
            .unwrap_or("12");

        let ua = format!("\"Chromium\";v=\"{version}\",") + &match ua {
                "Edg" => format!("\"Microsoft Edge\";v=\"{version}\""),
                "Chromium" => format!("\"Google Chrome\";v=\"{version}\""),
                "Firefox" => format!("\"Firefox\";v=\"{version}\""),
                "Safari" => format!("\"Safari\";v=\"{version}\""),
                _ => format!("\"{ua}\";v=\"{version}\""),
            }
            + ", \"Not_A Brand\";v=\"99\"";

        HeaderMap::from_iter([
            (header::ORIGIN, "https://www.fanbox.cc".parse().unwrap()),
            (header::REFERER, "https://www.fanbox.cc/".parse().unwrap()),
            (header::USER_AGENT, user_agent.parse().unwrap()),
            (
                header::DNT,
                "1".parse().unwrap(),
            ),
            (
                header::ACCEPT_LANGUAGE,
                "ja-JP,ja;q=0.9,en-US;q=0.8,en;q=0.7".parse().unwrap(),
            ),
            (
                header::ACCEPT,
                "application/json, text/plain, */*".parse().unwrap(),
            ),
            (
                header::HeaderName::from_static("sec-ch-ua"),
                ua.parse().unwrap(),
            ),
            (
                header::HeaderName::from_static("sec-ch-ua-platform"),
                platform.parse().unwrap(),
            ),
            (
                header::HeaderName::from_static("sec-ch-ua-mobile"),
                mobile.parse().unwrap(),
            ),
            (
                header::HeaderName::from_static("sec-fetch-dest"),
                "empty".parse().unwrap(),
            ),
            (
                header::HeaderName::from_static("sec-fetch-mode"),
                "cors".parse().unwrap(),
            ),
            (
                header::HeaderName::from_static("sec-fetch-site"),
                "same-site".parse().unwrap(),
            ),
        ])
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

    pub async fn get_posts(
        &self,
        creator: &str,
        updated: Option<i64>,
    ) -> Result<(APIListCreatorPost, i64)> {
        let url = format!("https://api.fanbox.cc/post.paginateCreator?creatorId={creator}",);
        let urls: APIListCreatorPaginate = self.fetch(&url).await?;

        let mut tasks = JoinSet::new();
        let mut skip = false;
        let mut last_date = None;
        for url in urls {
            skip |= {
                let url = Url::parse(&url).unwrap();
                let date = url
                    .query_pairs()
                    .find(|(k, _)| k == "firstPublishedDatetime")
                    .map(|(_, v)| {
                        NaiveDateTime::parse_from_str(&v, "%Y-%m-%d %H:%M:%S")
                            .unwrap()
                            .and_utc()
                            .timestamp()
                    });
                last_date = last_date.or(date);
                matches!((date, updated), (Some(date), Some(updated)) if date <= updated)
            };

            if skip {
                debug!("Skipping remaining posts for {creator}");
                break;
            }

            let client = self.clone();
            tasks.spawn(async move { client.fetch::<APIListCreatorPost>(&url).await });
        }

        tasks
            .join_all()
            .await
            .into_iter()
            .try_collect::<Vec<APIListCreatorPost>>()
            .map(|posts| posts.into_iter().flatten().collect::<APIListCreatorPost>())
            .map(|posts| (posts, last_date.unwrap_or_default()))
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
