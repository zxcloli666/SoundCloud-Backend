use std::sync::Arc;

use axum::http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

use super::super::RoomsService;
use crate::error::AppError;
use crate::modules::chat::ChatState;
use crate::modules::chat::test_support::chat_state;
use crate::modules::rooms::listing::PublicRoom;
use crate::modules::rooms::model::{
    Cover, FREE_CAPACITY, Perks, Profile, STAR_CAPACITY, SettingsPatch,
};
use crate::modules::subscriptions::SubscriptionsService;

const FAR: i64 = 4_000_000_000;

pub struct Bench {
    pub chat: Arc<ChatState>,
    pub pg: PgPool,
}

impl Bench {
    pub fn new(pg: PgPool) -> anyhow::Result<Self> {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        let chat = chat_state(pg.clone(), crate::redis::connect_url(&url)?, true);
        chat.rooms.install_chat(&chat);
        Ok(Self { chat, pg })
    }

    pub fn rooms(&self) -> &RoomsService {
        &self.chat.rooms
    }

    pub async fn star(&self, user: &str, on: bool) -> anyhow::Result<()> {
        let service = SubscriptionsService::new(self.pg.clone(), false);
        if on {
            service.upsert(user, FAR, None).await?;
        } else {
            service.remove(user).await?;
        }
        Ok(())
    }

    pub async fn fill(&self, code: &str, guests: usize) -> anyhow::Result<Vec<String>> {
        let mut joined = Vec::with_capacity(guests);
        for _ in 0..guests {
            let guest = someone();
            self.rooms()
                .join(code, &guest, &profile("Guest"), &self.pg)
                .await?;
            joined.push(guest);
        }
        Ok(joined)
    }
}

pub fn someone() -> String {
    format!("6{:011}", Uuid::now_v7().as_u128() % 100_000_000_000)
}

pub fn profile(name: &str) -> Profile {
    Profile {
        name: name.to_owned(),
        avatar_url: None,
    }
}

fn code_of<T: std::fmt::Debug>(outcome: Result<T, AppError>) -> (StatusCode, &'static str) {
    match outcome.unwrap_err() {
        AppError::Coded { status, code, .. } => (StatusCode::from_u16(status).unwrap(), code),
        other => panic!("an uncoded refusal: {other:?}"),
    }
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_star_host_opens_a_room_of_twenty_five_and_a_free_host_one_of_ten(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (star, free) = (someone(), someone());
    bench.star(&star, true).await?;
    let room = bench
        .rooms()
        .create(&star, &profile("Star"), true, None)
        .await?;
    assert_eq!(room.perks, Perks::star());
    bench.fill(&room.code, STAR_CAPACITY - 2).await?;
    let late = someone();
    let seated = bench
        .rooms()
        .join_public(&star, &late, &profile("Late"), &bench.pg)
        .await?;
    assert_eq!(seated.members.len(), STAR_CAPACITY);
    assert_eq!(
        code_of(
            bench
                .rooms()
                .join_public(&star, &someone(), &profile("x"), &bench.pg)
                .await
        ),
        (StatusCode::CONFLICT, "room_full")
    );
    assert_eq!(
        code_of(
            bench
                .rooms()
                .join(&room.code, &someone(), &profile("x"), &bench.pg)
                .await
        ),
        (StatusCode::CONFLICT, "room_full")
    );
    let stored = bench.rooms().room(&seated.code).await?;
    let card = PublicRoom::of(&stored, &[]).expect("a card");
    assert_eq!((card.capacity, card.star_host), (STAR_CAPACITY, true));

    let room = bench
        .rooms()
        .create(&free, &profile("Free"), true, None)
        .await?;
    assert_eq!(room.perks, Perks::free());
    bench.fill(&room.code, FREE_CAPACITY - 2).await?;
    bench
        .rooms()
        .join_public(&free, &someone(), &profile("Late"), &bench.pg)
        .await?;
    assert_eq!(
        code_of(
            bench
                .rooms()
                .join_public(&free, &someone(), &profile("x"), &bench.pg)
                .await
        ),
        (StatusCode::CONFLICT, "room_full")
    );
    let preview = bench.rooms().preview(&room.code).await?.expect("a preview");
    assert_eq!(
        (preview.capacity, preview.star_host),
        (FREE_CAPACITY, false)
    );
    bench.rooms().end(&room.code).await?;
    bench.rooms().end(&seated.code).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_lapse_found_by_the_recheck_removes_nobody(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg.clone())?;
    let host = someone();
    bench.star(&host, true).await?;
    let code = bench
        .rooms()
        .create(&host, &profile("Host"), true, None)
        .await?
        .code;
    let guests = bench.fill(&code, 11).await?;
    for guest in &guests[..5] {
        bench
            .rooms()
            .set_moderator(&code, &host, guest, true)
            .await?;
    }
    let patch: SettingsPatch = serde_json::from_value(serde_json::json!({
        "cover": {"url": "https://i1.sndcdn.com/artworks-9.jpg", "accent": 5},
        "backdrop": {"kind": "preset", "id": "snow-lamp"}
    }))?;
    bench.rooms().reconfigure(&code, &host, &patch).await?;
    assert!(bench.rooms().live_since(0).await?.contains(&code));
    assert_eq!(
        bench
            .rooms()
            .recheck_perks(std::slice::from_ref(&code))
            .await?,
        0
    );

    bench.star(&host, false).await?;
    let later = Bench::new(pg.clone())?;
    assert_eq!(
        later
            .rooms()
            .recheck_perks(std::slice::from_ref(&code))
            .await?,
        1
    );
    assert_eq!(
        later
            .rooms()
            .recheck_perks(std::slice::from_ref(&code))
            .await?,
        0
    );
    let view = later.rooms().view(&code, &host).await?;
    assert_eq!(view.perks, Perks::free());
    assert_eq!(view.members.len(), 12);
    assert_eq!(view.moderators.len(), 5);
    assert_eq!((view.cover, view.backdrop), (None, None));
    let stored = later.rooms().room(&code).await?;
    assert_eq!(
        stored.cover,
        Some(Cover {
            url: "https://i1.sndcdn.com/artworks-9.jpg".to_owned(),
            accent: 5
        })
    );
    assert!(stored.backdrop.is_some());
    assert_eq!(
        code_of(
            later
                .rooms()
                .join(&code, &someone(), &profile("x"), &later.pg)
                .await
        ),
        (StatusCode::CONFLICT, "room_full")
    );
    assert_eq!(
        code_of(
            later
                .rooms()
                .set_moderator(&code, &host, &guests[6], true)
                .await
        ),
        (StatusCode::CONFLICT, "room_moderators_full")
    );
    assert_eq!(
        code_of(later.rooms().reconfigure(&code, &host, &patch).await),
        (StatusCode::FORBIDDEN, "room_star_required")
    );
    let preview = later.rooms().preview(&code).await?.expect("a preview");
    assert_eq!(
        (preview.star_host, preview.capacity),
        (false, FREE_CAPACITY)
    );
    assert_eq!(
        (preview.cover.clone(), preview.backdrop.clone()),
        (None, None)
    );

    bench.star(&host, true).await?;
    let renewed = Bench::new(pg)?;
    assert_eq!(
        renewed
            .rooms()
            .recheck_perks(std::slice::from_ref(&code))
            .await?,
        1
    );
    let view = renewed.rooms().view(&code, &host).await?;
    assert_eq!(view.perks, Perks::star());
    assert!(view.cover.is_some() && view.backdrop.is_some());
    renewed.rooms().end(&code).await?;
    Ok(())
}
