use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::support::{Bench, chosen, missing, preset, refused, someone};

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn either_direct_member_with_star_sets_a_preset_that_both_see(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg.clone());
    let (first, second) = (someone(), someone());
    bench.star(&first).await?;
    let dm = bench.dm(&first, &second).await;

    assert_eq!(
        bench.put(&second, dm, preset("aurora")).await?,
        refused(StatusCode::FORBIDDEN, "chat_backdrop_star_required")
    );
    assert_eq!(
        bench.put(&first, dm, preset("lava")).await?,
        refused(StatusCode::BAD_REQUEST, "chat_invalid")
    );
    let aurora = chosen(preset("aurora"), &first);
    let (status, view) = bench.put(&first, dm, preset("aurora")).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["backdrop"], aurora);
    assert_eq!(bench.show(&second, dm).await?["backdrop"], aurora);
    assert_eq!(bench.listed(&second, dm).await?["backdrop"], aurora);
    assert_eq!(bench.stored(dm).await?, aurora);
    for _ in 0..7 {
        let (status, view) = bench.put(&first, dm, preset("aurora")).await?;
        assert_eq!(status, StatusCode::OK, "an unchanged value is free: {view}");
    }

    let (status, view) = bench.put(&second, dm, json!(null)).await?;
    assert_eq!((status, &view["backdrop"]), (StatusCode::OK, &json!(null)));
    assert_eq!(bench.stored(dm).await?, json!(null));

    let satin = chosen(preset("satin"), &first);
    assert_eq!(
        bench.put(&first, dm, preset("satin")).await?.1["backdrop"],
        satin
    );
    bench.unstar(&first).await?;
    let lapsed = Bench::new(pg.clone());
    assert_eq!(lapsed.show(&second, dm).await?["backdrop"], json!(null));
    assert_eq!(lapsed.listed(&second, dm).await?["backdrop"], json!(null));
    assert_eq!(lapsed.stored(dm).await?, satin, "the choice waits for STAR");
    bench.star(&first).await?;
    let back = Bench::new(pg);
    assert_eq!(back.show(&second, dm).await?["backdrop"], satin);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_group_backdrop_is_for_its_admins_and_saved_for_its_owner(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg);
    let (owner, admin, member, outsider) = (someone(), someone(), someone(), someone());
    for user in [&owner, &admin, &member] {
        bench.star(user).await?;
    }
    let group = bench.group(&owner, &[&admin], &[&member]).await?;
    for refused_user in [&member, &outsider] {
        assert_eq!(
            bench.put(refused_user, group, preset("dunes")).await?,
            refused(StatusCode::FORBIDDEN, "chat_not_member")
        );
    }
    let (status, view) = bench.put(&admin, group, preset("dunes")).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["backdrop"], chosen(preset("dunes"), &admin));
    assert_eq!(
        bench.show(&member, group).await?["backdrop"],
        chosen(preset("dunes"), &admin)
    );
    let (status, view) = bench.put(&owner, group, json!(null)).await?;
    assert_eq!((status, &view["backdrop"]), (StatusCode::OK, &json!(null)));

    let saved = bench.saved(&owner).await?;
    let (status, view) = bench.put(&owner, saved, preset("paper")).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["kind"], "self");
    assert_eq!(view["backdrop"], chosen(preset("paper"), &owner));
    let free = someone();
    let mine = bench.saved(&free).await?;
    assert_eq!(
        bench.put(&free, mine, preset("paper")).await?,
        refused(StatusCode::FORBIDDEN, "chat_backdrop_star_required")
    );
    assert_eq!(
        bench.put(&owner, Uuid::now_v7(), preset("paper")).await?,
        refused(StatusCode::FORBIDDEN, "chat_not_member")
    );

    let stranger = someone();
    let pending = bench.request(&owner, &stranger).await;
    assert_eq!(
        bench.put(&owner, pending, preset("paper")).await?,
        refused(StatusCode::FORBIDDEN, "chat_unavailable")
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn the_seventh_change_in_ten_minutes_is_refused_and_a_refused_change_costs_nothing(
    pg: PgPool,
) -> anyhow::Result<()> {
    let bench = Bench::new(pg);
    let user = someone();
    bench.star(&user).await?;
    let saved = bench.saved(&user).await?;
    for id in ["aurora", "vinyl", "dunes", "satin", "paper"] {
        let (status, view) = bench.put(&user, saved, preset(id)).await?;
        assert_eq!(status, StatusCode::OK, "{view}");
    }
    let wrong = Uuid::new_v4();
    assert_eq!(
        bench
            .put(&user, saved, json!({"kind": "image", "mediaId": wrong}))
            .await?,
        missing(wrong)
    );
    let (status, view) = bench.put(&user, saved, preset("candy-sky")).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(
        bench.put(&user, saved, preset("watercolor")).await?,
        refused(StatusCode::TOO_MANY_REQUESTS, "chat_request_limit")
    );
    let (status, view) = bench.put(&user, saved, preset("candy-sky")).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(
        bench.stored(saved).await?,
        chosen(preset("candy-sky"), &user)
    );
    bench.forget_limits(saved).await?;
    let (status, view) = bench.put(&user, saved, preset("watercolor")).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    Ok(())
}
