use clap::{arg, Parser, ValueEnum};
use clap_verbosity_flag::{InfoLevel, Verbosity};
use dotenv::dotenv;
use env_logger::TimestampPrecision;
use indicatif::MultiProgress;
use indicatif_log_bridge::LogWrapper;
use log::info;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fmt::{self, Display},
    fs::File,
    io::{BufReader, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, Parser, Default)]
pub struct Config {
    /// Your `FANBOXSESSID` cookie
    #[clap(env = "FANBOXSESSID")]
    session: String,
    /// Which you path want to save
    #[arg(short, long, default_value = "./archive", env = "OUTPUT")]
    output: PathBuf,
    /// Which you type want to save
    #[arg(short, long, default_value = "supporting", env = "SAVE")]
    save: SaveType,
    /// Force download
    #[arg(short, long)]
    force: bool,
    /// Whitelist of creator IDs
    #[arg(short, long, num_args = 0..)]
    whitelist: Vec<String>,
    /// Blacklist of creator IDs
    #[arg(short, long, num_args = 0..)]
    blacklist: Vec<String>,
    /// Limit download concurrency
    #[arg(long, default_value = "5")]
    limit: usize,
    /// Cache directory
    #[arg(long, name = "cache-path", default_value = ".")]
    cache_path: Option<String>,
    /// Overwrite existing files
    #[arg(long, name = "no-cache")]
    no_cache: bool,
    /// Skip free post
    #[arg(long, name = "skip-free")]
    skip_free: bool,
    /// If fanbox has robot check, you need to provide `cf_clearance` cookie
    #[clap(long)]
    clearance: Option<String>,
    /// If fanbox has robot check, you need to provide `user-agent` header
    #[clap(long)]
    user_agent: Option<String>,
    #[command(flatten)]
    pub verbose: Verbosity<InfoLevel>,
    #[clap(skip)]
    pub multi_progress: MultiProgress,
    #[clap(skip)]
    cleanup: Arc<CacheCleanup>,
}

impl Config {
    pub fn parse() -> Self {
        dotenv().ok();
        let config = <Self as Parser>::parse();

        let debug = config.verbose.log_level().unwrap() > log::Level::Info;
        let logger = env_logger::Builder::new()
            .format_timestamp(if debug {
                Some(TimestampPrecision::Millis)
            } else {
                None
            })
            .format_target(debug)
            .filter_level(config.verbose.log_level_filter())
            .build();

        let multi = MultiProgress::new();

        LogWrapper::new(multi.clone(), logger).try_init().unwrap();

        config
    }
    pub fn session(&self) -> String {
        if self.session.starts_with("FANBOXSESSID=") {
            self.session.clone()
        } else {
            format!("FANBOXSESSID={}", self.session)
        }
    }
    pub fn clearance(&self) -> String {
        let clearance = self.clearance.clone().unwrap_or_default();
        if clearance.starts_with("cf_clearance=") {
            clearance
        } else {
            format!("cf_clearance={}", clearance)
        }
    }
    pub fn user_agent(&self) -> String {
        let default_user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36 Edg/126.0.0.0".to_string();
        self.user_agent.clone().unwrap_or(default_user_agent)
    }
    pub fn save_types(&self) -> SaveType {
        self.save
    }
    pub fn cache(&self) -> Option<PathBuf> {
        if self.no_cache || self.force {
            return None;
        };
        self.cache_path
            .clone()
            .or_else(|| Some(".".to_string()))
            .and_then(|s| Some(PathBuf::from(s)))
    }
    pub fn load_cache<T: DeserializeOwned>(&self, path: &str) -> Option<T> {
        let cache = self.cache()?;
        let path = cache.join(path);

        if path.exists() {
            info!("Loading cache {:?}", &path);
            let file = File::open(path).unwrap();
            let reader = BufReader::new(file);
            let data = serde_json::from_reader(reader).unwrap();
            Some(data)
        } else {
            None
        }
    }

    pub fn save_cache<T: Serialize>(&self, file: &str, data: &T) -> Option<()> {
        let cache = self.cache()?;
        let path = cache.join(file);
        let data = serde_json::to_vec(data).unwrap();
        self.cleanup.push(path, data);
        Some(())
    }

    pub fn output(&self) -> &PathBuf {
        &self.output
    }
    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn filter_creator(&self, creator_id: &str) -> bool {
        if !self.whitelist.is_empty() {
            return !self.whitelist.contains(&creator_id.to_string());
        }

        if !self.blacklist.is_empty() {
            return !self.blacklist.contains(&creator_id.to_string());
        }

        true
    }
    pub fn skip_free(&self) -> bool {
        self.skip_free
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, Hash, ValueEnum, PartialEq, Eq)]
pub enum SaveType {
    All,
    Following,
    Supporting,
}

impl Default for SaveType {
    fn default() -> Self {
        SaveType::Supporting
    }
}

impl Display for SaveType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SaveType::All => write!(f, "all"),
            SaveType::Following => write!(f, "following"),
            SaveType::Supporting => write!(f, "supporting"),
        }
    }
}

#[derive(Debug, Default)]
struct CacheCleanup(Mutex<Vec<(PathBuf, Vec<u8>)>>);

impl CacheCleanup {
    pub fn push(&self, path: PathBuf, data: Vec<u8>) {
        self.0.lock().unwrap().push((path, data));
    }
}

impl Drop for CacheCleanup {
    fn drop(&mut self) {
        let data = self.0.lock().unwrap();
        for (path, data) in data.iter() {
            info!("Saving cache {:?}", &path);
            let mut file = File::create(path).unwrap();
            file.write_all(data).unwrap();
        }
    }
}
