use std::collections::HashMap;

use dashmap::DashMap;
use post_archiver::manager::PostArchiverManager;
use serde::{Deserialize, Serialize};

const FANBOX_ARCHIVE_FEATURE: &str = "fanbox-archive";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Context {
    pub creators: DashMap<String, CachedCreators>,
}

impl Context {
    pub fn load(manager: &PostArchiverManager) -> Self {
        let (_, extra) = manager
            .get_feature_with_extra(FANBOX_ARCHIVE_FEATURE)
            .unwrap_or_default();

        let json = serde_json::to_value(&extra).unwrap();
        serde_json::from_value(json).unwrap_or_default()
    }

    pub fn save(&self, manager: &PostArchiverManager) {
        let extras = HashMap::from([(
            "creators".to_string(),
            serde_json::to_value(&self.creators).unwrap(),
        )]);
        manager.set_feature_with_extra(FANBOX_ARCHIVE_FEATURE, 1, extras);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CachedCreators {
    pub updated: i64,
    pub fee: u32,
}

impl CachedCreators {
    pub fn last_updated(&self, fee: u32) -> Option<i64> {
        let fee_unchanged = fee <= self.fee;
        fee_unchanged.then_some(self.updated)
    }
    pub fn update(&mut self, updated: i64, fee: u32) {
        if updated > self.updated {
            self.updated = updated;
        }
        if fee > self.fee {
            self.fee = fee;
        }
    }
}
