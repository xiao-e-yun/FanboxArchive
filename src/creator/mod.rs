use std::{collections::HashSet, error::Error};

use log::info;
use post_archiver::{importer::{UnsyncAlias, UnsyncAuthor}, manager::PostArchiverManager, AuthorId};
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
) -> Result<Vec<(AuthorId, String)>, Box<dyn Error>> {
    let manager = manager.transaction()?;

    let fanbox_platform = manager.import_platform("fanbox".to_string())?;
    let pixiv_platform = manager.import_platform("pixiv".to_string())?;

    let authors = creators.into_iter()
        .map(|creator| {
            let author = UnsyncAuthor::new(creator.name.to_string())
                .aliases(vec![
                    UnsyncAlias::new(fanbox_platform, creator.creator_id.clone()).link(format!("https://{}.fanbox.cc/", creator.creator_id)),
                    UnsyncAlias::new(pixiv_platform, creator.user.user_id.clone()).link(format!("https://www.pixiv.net/users/{}", creator.user.user_id)),
                ])
                .sync(&manager)?;

            Ok((author, creator.creator_id))
        }).collect::<Result<Vec<_>,rusqlite::Error>>()?;

    manager.commit()?;
    Ok(authors)
}
