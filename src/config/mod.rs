pub mod save_type;

use clap::{arg, Parser, ValueEnum};
use clap_verbosity_flag::{InfoLevel, Verbosity};
use dotenv::dotenv;
use fake_user_agent::get_rua;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use indicatif_log_bridge::LogWrapper;
use save_type::SaveType;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, ops::Deref, path::PathBuf};

use crate::fanbox::{Creator, PostListItem};

#[derive(Debug, Clone, Parser, Default)]
pub struct Config {
    /// Your `FANBOXSESSID` cookie
    #[clap(env = "FANBOXSESSID")]
    session: String,
    /// Which you path want to save
    #[arg(default_value = "./archive", env = "OUTPUT")]
    output: PathBuf,
    /// Which you type want to save
    #[arg(short, long, default_value = "supporting", env = "SAVE")]
    save: SaveType,
    /// Archiving strategy
    #[arg(long, default_value = "increment")]
    strategy: Strategy,
    /// Whitelist of creator IDs
    #[arg(short, long, num_args = 0..)]
    whitelist: Vec<String>,
    /// Blacklist of creator IDs
    #[arg(short, long, num_args = 0..)]
    blacklist: Vec<String>,
    /// Limit fetch number of posts per minute
    #[arg(long, default_value = "60")]
    limit: u32,
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
        let mut config = <Self as Parser>::parse();
        config.init_logger();

        if config.user_agent.is_none() {
            let random_user_agent = get_rua();
            config.user_agent = Some(random_user_agent.to_string());
        }

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
        self.user_agent.clone().unwrap_or_default()
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
    pub fn limit(&self) -> u32 {
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
        // is_restricted means the post is for supporters only
        accept &= !post.is_restricted;

        accept
    }

    pub fn strategy(&self) -> Strategy {
        self.strategy
    }

    pub fn progress(&self, prefix: &'static str) -> Progress {
        Progress::new(&self.multi, prefix)
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, Hash, ValueEnum, PartialEq, Eq, Default)]
pub enum Strategy {
    #[default]
    Increment,
    Full,
    Force,
}

impl Strategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Strategy::Increment => "increment",
            Strategy::Full => "full",
            Strategy::Force => "force",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Progress(ProgressBar);

impl Progress {
    pub fn new(multi: &MultiProgress, prefix: &'static str) -> Self {
        Self(
            multi.add(
                ProgressBar::new(0)
                    .with_style(Self::style())
                    .with_prefix(format!("[{prefix}]")),
            ),
        )
    }

    fn style() -> ProgressStyle {
        ProgressStyle::with_template("{prefix:.bold.dim} {wide_bar:.cyan/blue} {pos:>3}/{len:3}")
            .unwrap()
            .progress_chars("#>-")
    }
}

impl Deref for Progress {
    type Target = ProgressBar;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct ProgressSet {
    pub authors: Progress,
    pub posts: Progress,
    pub files: Progress,
}

impl ProgressSet {
    pub fn new(config: &Config) -> Self {
        Self {
            authors: config.progress("authors"),
            posts: config.progress("posts"),
            files: config.progress("files"),
        }
    }
}
