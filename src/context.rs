use std::{
    collections::{HashMap, HashSet},
    fs::read_to_string,
    path::Path,
};

use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Context {
    /// user_id: CachedCreator
    pub creators: HashMap<String, CachedCreator>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CachedCreator {
    pub last_updated: i64,
    pub creator_id: String,
    /// fee: unimported post.
    pub unimports: HashMap<u32, HashSet<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnimportedPost {
    pub creator: String,
    pub user: String,
    pub id: String,
    pub fee: u32,
}
