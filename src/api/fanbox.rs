use std::{path::PathBuf, process::exit};

use log::{debug, error};
use reqwest::header;
use reqwest_middleware::RequestBuilder;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{
    config::Config,
    fanbox::{Comment, FollowingCreator, Post, PostComments, PostListItem, SupportingCreator},
};

use super::ArchiveClient;

pub type APIPost = Post;
pub type APIPostComments = Vec<Comment>;
pub type APIListCreatorPost = Vec<PostListItem>;
pub type APIListSupportingCreator = Vec<SupportingCreator>;
pub type APIListFollowingCreator = Vec<FollowingCreator>;
pub type APIListCreatorPaginate = Vec<String>;

#[derive(Debug, Clone)]
pub struct FanboxClient {
    inner: ArchiveClient,
    user_agent: String,
    session: String,
    overwrite: bool,
}

impl FanboxClient {
    pub fn new(config: &Config) -> Self {
        let inner = ArchiveClient::new(config);
        let session = config.cookies();
        let overwrite = config.overwrite();
        let user_agent = config.user_agent();

        Self {
            inner,
            session,
            overwrite,
            user_agent,
        }
    }

    fn wrap_request(&self, builder: RequestBuilder) -> RequestBuilder {

        builder
            .header(header::COOKIE, &self.session)
            .header(header::ORIGIN, "https://www.fanbox.cc")
            .header(header::USER_AGENT, &self.user_agent)
    }

    pub async fn fetch<T: DeserializeOwned>(&self, url: &str) -> Result<T, FanboxAPIResponseError> {
        let (client, _semaphore) = self.inner.client().await;

        debug!("Fetching {}", url);
        let request = client.get(url);
        let request = self.wrap_request(request);
        let response = request.send().await.expect("Failed to send request");
        let response = response.bytes().await.expect("Failed to get response body");

        match serde_json::from_slice::<FanboxAPIResponse<T>>(&response) {
            Ok(value) => Ok(value.body),
            Err(error) => {
                // try to parse as error
                match serde_json::from_slice::<FanboxAPIResponseError>(&response) {
                    Ok(response) => {
                        if response.error == "general_error" {
                            error!("The session is invalid or expired");
                            error!("Or the API has changed");
                        }
                        Err(response)
                    }
                    Err(_) => {
                        let response = String::from_utf8_lossy(&response);
                        error!("Failed to parse response: {}", error);

                        error!("Response URL: {}", url);
                        error!("Response cookies: {}",  &self.session);
                        error!("Response user-agent: {}", &self.user_agent);

                        error!("Response body: {}", response);
                        exit(1);
                    },
                }
            }
        }
    }

    pub async fn download(&self, url: &str, path: PathBuf) -> Result<(), reqwest::Error> {
        if !self.overwrite && path.exists() {
            debug!("Download was skip ({})", path.display());
            return Ok(());
        }

        let (client, _semaphore) = self.inner.client().await;
        let request = client.get(url);
        let request = self.wrap_request(request);
        let response = request.send().await.expect("Failed to send request");

        debug!("Downloading {} to {}", url, path.display());
        let mut file = tokio::fs::File::create(path).await.unwrap();
        self.inner.download(response, &mut file).await?;

        Ok(())
    }

    pub async fn get_supporting_creators(
        &self,
    ) -> Result<APIListSupportingCreator, Box<dyn std::error::Error>> {
        let url = "https://api.fanbox.cc/plan.listSupporting";
        let list: APIListSupportingCreator = self
            .fetch(url)
            .await
            .expect("Failed to get supporting authors");
        Ok(list)
    }

    pub async fn get_following_creators(
        &self,
    ) -> Result<APIListFollowingCreator, Box<dyn std::error::Error>> {
        let url = "https://api.fanbox.cc/creator.listFollowing";
        let list: APIListFollowingCreator = self
            .fetch(url)
            .await
            .expect("Failed to get following authors");
        Ok(list)
    }

    pub async fn get_posts(
        &self,
        creator: &str,
    ) -> Result<APIListCreatorPost, Box<dyn std::error::Error>> {
        let url = format!(
            "https://api.fanbox.cc/post.paginateCreator?creatorId={}",
            creator
        );
        let urls: APIListCreatorPaginate = self.fetch(&url).await.expect("Failed to get post list");

        let mut tasks = Vec::new();
        for url in urls {
            let client = self.clone();
            let future = async move {
                client
                    .fetch::<APIListCreatorPost>(&url)
                    .await
                    .expect("Failed to get post")
            };
            tasks.push(tokio::spawn(future));
        }

        let mut posts = Vec::new();
        for task in tasks {
            posts.extend(task.await?.into_iter());
        }

        Ok(posts)
    }

    pub async fn get_post(&self, post_id: &str) -> Result<APIPost, Box<dyn std::error::Error>> {
        let url = format!("https://api.fanbox.cc/post.info?postId={}", post_id);
        let post: APIPost = self.fetch(&url).await.expect("Failed to get post");
        Ok(post)
    }

    pub async fn get_post_comments(&self, post_id: &str, comment_count: u32) -> Result<APIPostComments, Box<dyn std::error::Error>> {

        if comment_count == 0 {
            return Ok(vec![])
        };

        let mut next_url = Some(format!("https://api.fanbox.cc/post.getComments?postId={}&limit=10", post_id));
        let mut comments = vec![];
        while let Some(url) = next_url {
            let response: PostComments = self.fetch(&url).await.expect("Failed to get post comments");
            let Some(response) = response.comment_list else { break };
            next_url = response.next_url;
            comments.extend(response.items.into_iter());
        };
        Ok(comments)
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub struct FanboxAPIResponse<T> {
    pub body: T,
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub struct FanboxAPIResponseError {
    error: String,
}
