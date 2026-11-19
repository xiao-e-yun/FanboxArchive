use std::{
    collections::{HashMap, HashSet},
    fs::read_to_string,
    mem,
    path::Path,
};

use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Context {
    /// user_id: CachedCreator
    pub creators: HashMap<String, CachedCreator>,
    pub failed_posts: Vec<String>,
}

impl Context {
    pub const RELATION_PATH: &'static str = "configs/fanbox-archive.json";

    pub fn load(path: &Path) -> Self {
        let path = path.join(Self::RELATION_PATH);
        let json = read_to_string(path).unwrap_or_default();
        serde_json::from_str(&json).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        let path = path.join(Self::RELATION_PATH);
        std::fs::create_dir_all(path.parent().unwrap()).expect("Failed to create context folder");
        let json = serde_json::to_string(self).expect("Failed to serialize context");
        std::fs::write(path, json).expect("Failed to save context");
    }

    pub fn get_creator(&self, user_id: &str, creator_id: &str) -> CachedCreator {
        self.creators
            .get(user_id)
            .cloned()
            .unwrap_or(CachedCreator {
                last_updated: 0,
                creator_id: creator_id.to_string(),
                unimports: HashMap::new(),
            })
    }

    pub fn creator(&mut self, user_id: &str) -> Option<CachedCreator> {
        self.creators.remove(user_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CachedCreator {
    pub last_updated: i64,
    pub creator_id: String,
    /// fee: unimported post.
    pub unimports: HashMap<u32, HashSet<String>>,
}

impl CachedCreator {
    pub fn take_unimported_posts(&mut self, fee: u32) -> Vec<String> {
        let mut posts = vec![];
        for (f, unimporteds) in self.unimports.iter_mut() {
            if *f > fee {
                posts.extend(mem::take(unimporteds));
            }
        }
        posts
    }

    pub fn push_unimport(&mut self, fee: u32, post_id: String) {
        self.unimports
            .entry(fee)
            .or_default()
            .insert(post_id);
    }
}
