pub mod save_type;

use clap::Parser;
use clap_verbosity_flag::{InfoLevel, Verbosity};
use dotenv::dotenv;
use fake_user_agent::get_rua;
use indicatif::{MultiProgress, ProgressBar};
use indicatif_log_bridge::LogWrapper;
use post_archiver_utils::{define_progress, Strategy};
use save_type::SaveType;
use std::{collections::HashMap, num::NonZeroU32, path::PathBuf};

use crate::fanbox::{Creator, PostListItem};

#[derive(Debug, Clone, Parser)]
pub struct Config {
    /// Your `FANBOXSESSID` cookie
    #[clap(env = "FANBOXSESSID", hide_env_values = true)]
    session: String,
    /// Which you path want to save
    #[arg(default_value = "./archive", env = "OUTPUT")]
    output: PathBuf,
    /// Which you type want to save
    #[arg(short, long, default_value = "supporting", env = "SAVE")]
    save: SaveType,
    /// Archiving strategy
    #[arg(long, default_value_t)]
    strategy: Strategy,
    /// Whitelist of creator IDs
    #[arg(short, long, num_args = 0..)]
    whitelist: Vec<String>,
    /// Blacklist of creator IDs
    #[arg(short, long, num_args = 0..)]
    blacklist: Vec<String>,
    /// Limit fetch number of posts per minute
    #[arg(long, default_value = "60")]
    limit: NonZeroU32,
    /// Skip free post
    #[arg(long, name = "skip-free")]
    skip_free: bool,
    /// User agent when blocking
    #[arg(long, name = "user-agent")]
    user_agent: Option<String>,
    /// Custom cookies.  Exapmle: `name=value; name2=value2; ...`  (cf_clearance is required for blocking)
    #[arg(long, name = "cookies")]
    cookies: Option<String>,

    #[command(flatten)]
    pub verbose: Verbosity<InfoLevel>,

    #[clap(skip)]
    pub multi: MultiProgress,
}

impl Config {
    /// Parse the configuration from the environment and command line arguments
    pub fn parse() -> Self {
        dotenv().ok();
        let config = <Self as Parser>::parse();
        config.init_logger();
        config
    }
    /// Create a logger with the configured verbosity level
    pub fn init_logger(&self) {
        let mut logger = env_logger::Builder::new();
        logger
            .filter_level(self.verbose.log_level_filter())
            .format_target(false);
        LogWrapper::new(self.multi.clone(), logger.build())
            .try_init()
            .unwrap();
    }
    /// Get the cookies
    pub fn cookies(&self) -> String {
        let session = (
            "FANBOXSESSID",
            self.session
                .trim_start_matches("FANBOXSESSID=")
                .trim_end_matches(';')
                .trim(),
        );

        self.cookies
            .clone()
            .unwrap_or_default()
            .split(';')
            .filter_map(|cookie| {
                let trimmed = cookie.trim();
                (!trimmed.is_empty())
                    .then(|| trimmed.split_once('='))
                    .flatten()
            })
            .chain(std::iter::once(session))
            .collect::<HashMap<_, _>>()
            .into_iter()
            .map(|(name, value)| format!("{}={}", name.trim(), value.trim()))
            .collect::<Vec<_>>()
            .join(";")
    }
    /// Get the user agent for blocking
    pub fn user_agent(&self) -> String {
        self.user_agent
            .clone()
            .unwrap_or_else(|| get_rua().to_string()) // NOTE get rua never choose IE
    }
    pub fn accepts(&self) -> SaveType {
        self.save
    }
    pub fn skip_free(&self) -> bool {
        self.skip_free
    }

    pub fn whitelist(&self) -> &[String] {
        &self.whitelist
    }

    pub fn blacklist(&self) -> &[String] {
        &self.blacklist
    }

    pub fn output(&self) -> &PathBuf {
        &self.output
    }
    pub fn limit(&self) -> NonZeroU32 {
        self.limit
    }

    pub fn filter_creator(&self, creator: &Creator) -> bool {
        let creator_id = creator.creator_id.to_string();
        let mut accept = true;

        accept &= !(self.skip_free && creator.fee == 0);
        accept &= self.whitelist.is_empty() || self.whitelist.contains(&creator_id);
        accept &= !self.blacklist.contains(&creator_id);

        accept
    }

    pub fn filter_post(&self, post: &PostListItem) -> bool {
        let mut accept = true;

        // skip_free is true and the post is free
        accept &= !(self.skip_free && post.fee_required == 0);

        accept
    }

    pub fn strategy(&self) -> Strategy {
        self.strategy
    }
}

define_progress!(ProgressSet [
    creators,
    posts,
    files
]);

/// Increments the progress bar by one when dropped, so early returns are still counted.
pub struct IncOnDrop(pub ProgressBar);

impl Drop for IncOnDrop {
    fn drop(&mut self) {
        self.0.inc(1);
    }
}
