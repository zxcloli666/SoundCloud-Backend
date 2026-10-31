use backend_contracts::admin_catalog::{AdminCatalogReply, AdminCatalogRequest, AdminCommand};
use sqlx::{PgConnection, PgPool};

use crate::error::{Done, Fail, Refusal};
use crate::ledger::{self, Claim};
use crate::{artists, credits, enrich, playlists, tracks, wanted};

pub async fn execute(
    pool: &PgPool,
    request: &AdminCatalogRequest,
) -> Result<AdminCatalogReply, sqlx::Error> {
    match wt::retry(|| attempt(pool, request)).await {
        Ok(reply) => Ok(reply),
        Err(Fail::Refused(refusal)) => Ok(refusal.into_reply()),
        Err(Fail::Database(error)) => Err(error),
    }
}

async fn attempt(pool: &PgPool, request: &AdminCatalogRequest) -> Result<AdminCatalogReply, Fail> {
    let mut transaction = pool.begin().await?;
    let command = match ledger::claim(&mut transaction, request).await? {
        Claim::Fresh(command) => command,
        Claim::Replay(reply) => return Ok(reply),
        Claim::Reused => {
            return Err(Refusal::conflict("request id was used for a different edit").into());
        }
    };
    let body = run(&mut transaction, &request.command).await?;
    ledger::store(&mut transaction, request.request_id, &command, &body).await?;
    transaction.commit().await?;
    Ok(AdminCatalogReply { status: 200, body })
}

async fn run(connection: &mut PgConnection, command: &AdminCommand) -> Done {
    match command {
        AdminCommand::ArtistCreate(body) => artists::create(connection, body).await,
        AdminCommand::ArtistUpdate { artist_id, body } => {
            artists::update(connection, *artist_id, body).await
        }
        AdminCommand::ArtistMerge { src_id, dst_id } => {
            artists::merge(connection, *src_id, *dst_id).await
        }
        AdminCommand::ArtistRetryCrawl { artist_id } => {
            artists::retry_crawl(connection, *artist_id).await
        }
        AdminCommand::ScAccountUpsert { artist_id, body } => {
            enrich::upsert_account(connection, *artist_id, body).await
        }
        AdminCommand::ScAccountDelete {
            artist_id,
            sc_user_id,
        } => enrich::delete_account(connection, *artist_id, sc_user_id).await,
        AdminCommand::ScAccountDetachTracks {
            artist_id,
            sc_user_id,
            body,
        } => tracks::detach_account_tracks(connection, *artist_id, sc_user_id, body).await,
        AdminCommand::TrackSetPrimaryArtist { track_id, body } => {
            tracks::set_primary_artist(connection, *track_id, body).await
        }
        AdminCommand::TrackSetAlbum { track_id, body } => {
            tracks::set_album(connection, *track_id, body).await
        }
        AdminCommand::TrackAddCredit { track_id, body } => {
            tracks::add_credit(connection, *track_id, body).await
        }
        AdminCommand::TrackRemoveCredit {
            track_id,
            artist_id,
            role,
        } => tracks::remove_credit(connection, *track_id, *artist_id, role).await,
        AdminCommand::TrackDetachArtist { track_id, body } => {
            tracks::detach_artist(connection, *track_id, body).await
        }
        AdminCommand::TrackUnblockArtist {
            track_id,
            artist_id,
        } => tracks::unblock_artist(connection, *track_id, *artist_id).await,
        AdminCommand::CreditAccept(credit) => credits::accept(connection, credit).await,
        AdminCommand::CreditReject(credit) => credits::reject(connection, credit).await,
        AdminCommand::WantedLink { wanted_id, body } => {
            wanted::link(connection, *wanted_id, body).await
        }
        AdminCommand::WantedSetStatus { wanted_id, body } => {
            wanted::set_status(connection, *wanted_id, body).await
        }
        AdminCommand::LegacyPlaylistAbandon { archive_id } => {
            playlists::abandon_legacy(connection, *archive_id).await
        }
        AdminCommand::EnrichRetry(body) => enrich::retry(connection, body).await,
    }
}
