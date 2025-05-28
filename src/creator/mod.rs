use std::{collections::HashSet, error::Error};

use log::info;
use post_archiver::{importer::UnsyncAuthor, manager::PostArchiverManager, Author, Link};
use rusqlite::Connection;

use crate::{api::fanbox::FanboxClient, config::Config, fanbox::Creator};

pub async fn get_creators(config: &Config) -> Result<Vec<Creator>, Box<dyn Error>> {
    let accepts = config.accepts();
    info!("Accepts:");
    for accept in accepts.list() {
        info!(" + {}", accept);
    }
    info!("");

    let client = FanboxClient::new(config);
    let mut creators: HashSet<Creator> = HashSet::new();
    info!("Checking creators");
    if accepts.accept_following() {
        let following = client.get_following_creators().await?;
        info!(" + Following: {} found", following.len());
        creators.extend(following.into_iter().map(|f| f.into()));
    }

    if accepts.accept_supporting() {
        let supporting = client.get_supporting_creators().await?;
        info!(" + Supporting: {} found", supporting.len());
        creators.extend(supporting.into_iter().map(|f| f.into()));
    }
    info!("");

    let total = creators.len();
    info!("Total: {} creators", total);
    creators.retain(|c| config.filter_creator(c));
    let filtered = creators.len();
    info!("Excluded: {} creators", total - filtered);
    info!("Included: {} creators", filtered);
    info!("");
    Ok(creators.into_iter().collect())
}

pub fn display_creators(creators: &[Creator]) {
    if log::log_enabled!(log::Level::Info) {
        let mut creators = creators.to_vec();
        creators.sort_by(|a, b| a.creator_id.cmp(&b.creator_id));

        let (mut id_width, mut fee_width) = (11_usize, 5_usize);
        for creator in creators.iter() {
            id_width = creator.creator_id.len().max(id_width);
            fee_width = creator.fee.to_string().len().max(fee_width);
        }

        info!(
            "+-{:-<id_width$}-+-{:-<fee_width$}--+-{}------- - -",
            " CreatorId ", " Fee ", " Name "
        );
        for creator in creators.iter() {
            info!(
                "| {:id_width$} | {:fee_width$}$ | {}",
                creator.creator_id, creator.fee, creator.name
            );
        }
        info!(
            "+-{}-+-{}--+------------ - -",
            "-".to_string().repeat(id_width),
            "-".to_string().repeat(fee_width)
        );
        info!("");
    }
}

pub fn sync_creators(
    manager: &mut PostArchiverManager<Connection>,
    creators: Vec<Creator>,
) -> Result<Vec<(Author, String)>, Box<dyn Error>> {
    let mut list = vec![];
    let manager = manager.transaction()?;

    for creator in creators.into_iter() {
        let fanbox_alias = format!("fanbox:{}", creator.creator_id);
        let fanbox_link = Link::new(
            "fanbox",
            &format!("https://{}.fanbox.cc/", creator.creator_id),
        );

        let pixiv_alias = format!("pixiv:{}", creator.user.user_id);
        let pixiv_link = Link::new(
            "pixiv",
            &format!("https://www.pixiv.net/users/{}", creator.user.user_id),
        );
        let author = UnsyncAuthor::new(creator.name.to_string())
            .alias(vec![pixiv_alias, fanbox_alias])
            .links(vec![fanbox_link, pixiv_link])
            .sync(&manager)?;

        list.push((author, creator.creator_id));
    }

    manager.commit()?;
    Ok(list)
}
