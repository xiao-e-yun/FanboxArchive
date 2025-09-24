use std::fmt;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone, Copy, Hash, ValueEnum, PartialEq, Eq, Default)]
pub enum SaveType {
    All,
    Following,
    #[default]
    Supporting,
}

impl SaveType {
    pub fn accept_all(&self) -> bool {
        *self == SaveType::All
    }
    pub fn accept_following(&self) -> bool {
        *self == SaveType::Following || self.accept_all()
    }
    pub fn accept_supporting(&self) -> bool {
        *self == SaveType::Supporting || self.accept_all()
    }
    pub fn list(&self) -> Vec<&'static str> {
        match self {
            SaveType::All => vec!["following", "supporting"],
            SaveType::Following => vec!["following"],
            SaveType::Supporting => vec!["supporting"],
        }
    }
}

impl fmt::Display for SaveType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SaveType::All => write!(f, "all"),
            SaveType::Following => write!(f, "following"),
            SaveType::Supporting => write!(f, "supporting"),
        }
    }
}
