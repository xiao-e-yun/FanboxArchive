use std::{cell::RefCell, collections::HashMap, fs::read_to_string, path::Path};

use dashmap::{mapref::one::RefMut, DashMap};
use serde::{Deserialize, Serialize};

use crate::fanbox::PostListItem;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Context {
    pub creators: DashMap<String, CachedCreator>,
    pub failed_posts: RefCell<Vec<PostListItem>>,
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

    pub fn get_creator_mut(
        &self,
        user_id: &str,
        creator_id: &str,
    ) -> RefMut<'_, String, CachedCreator> {
        self.creators
            .entry(user_id.to_string())
            .and_modify(|creator| {
                if creator.creator_id != creator_id {
                    creator.old_creator_id = Some(creator.creator_id.clone());
                    creator.creator_id = creator_id.to_string();
                }
            })
            .or_insert_with(|| CachedCreator {
                creator_id: creator_id.to_string(),
                support_records: HashMap::new(),
                old_creator_id: None,
            })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CachedCreator {
    pub creator_id: String,
    /// fee: updated
    pub support_records: HashMap<u32, i64>,
    #[serde(skip)]
    pub old_creator_id: Option<String>,
}

impl CachedCreator {
    pub fn last_updated(&self, fee: u32) -> Option<i64> {
        // Find the maximum updated timestamp for all fees that are greater than or equal to the given fee.
        self.support_records
            .iter()
            .filter_map(|(f, u)| if *f >= fee { Some(*u) } else { None })
            .max()
    }
    pub fn update(&mut self, updated: i64, fee: u32) {
        self.support_records.insert(fee, updated);
    }
}
