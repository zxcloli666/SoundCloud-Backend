use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use super::support::{Bench, chosen, image, preset, refused, report, someone};
use crate::modules::chat::public_chat::ID;
use crate::modules::chat::public_chat::support::{OWNER, moderator, open, reset, serial};

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn public_staff_set_presets_free_and_images_with_star_and_three_reports_clear_an_image(
    pg: PgPool,
) -> anyhow::Result<()> {
    let _serial = serial().await;
    let bench = Bench::public(pg.clone());
    reset(&bench.chat).await;
    bench.forget_limits(ID).await?;
    let (staff, member, first, second) = (someone(), someone(), someone(), someone());
    for user in [OWNER, &staff, &member, &first, &second] {
        open(&bench.chat, user).await;
    }
    moderator(&bench.chat, &staff).await;

    assert_eq!(
        bench.put(&member, ID, preset("aurora")).await?,
        refused(StatusCode::FORBIDDEN, "chat_not_member")
    );
    let aurora = chosen(preset("aurora"), &staff);
    let (status, view) = bench.put(&staff, ID, preset("aurora")).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(
        (&view["kind"], &view["backdrop"]),
        (&json!("public"), &aurora)
    );
    assert_eq!(bench.show(&member, ID).await?["backdrop"], aurora);
    assert_eq!(bench.listed(&member, ID).await?["backdrop"], aurora);

    let media = bench.upload(&staff, ID, "backdrop", "uploaded").await?;
    assert_eq!(
        bench.put(&staff, ID, image(media)).await?,
        refused(StatusCode::FORBIDDEN, "chat_backdrop_star_required")
    );
    bench.star(&staff).await?;
    let (status, view) = bench.put(&staff, ID, image(media)).await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["backdrop"], chosen(image(media), &staff));
    let (status, sent) = bench.send_meta(&staff, ID, &[media]).await?;
    assert_eq!(status, StatusCode::OK, "{sent}");
    assert!(bench.show(&member, ID).await?["meta_ct"].is_string());
    let (status, view) = bench.put(OWNER, ID, json!(null)).await?;
    assert_eq!((status, &view["backdrop"]), (StatusCode::OK, &json!(null)));
    assert!(bench.refs_of(ID).await?.is_empty());
    assert_eq!(bench.status_of(media).await?, "deleting");

    bench.forget_limits(ID).await?;
    let again = bench.upload(&staff, ID, "backdrop", "uploaded").await?;
    assert_eq!(bench.put(&staff, ID, image(again)).await?.0, StatusCode::OK);
    assert_eq!(
        bench.send_meta(&staff, ID, &[again]).await?.0,
        StatusCode::OK
    );
    for reporter in [&first, &second] {
        let (status, filed) = bench.file(reporter, report(ID, again)).await?;
        assert_eq!(status, StatusCode::OK, "{filed}");
        assert_eq!(bench.stored(ID).await?, chosen(image(again), &staff));
    }
    let (status, filed) = bench.file(&member, report(ID, again)).await?;
    assert_eq!(status, StatusCode::OK, "{filed}");
    assert_eq!(bench.stored(ID).await?, json!(null));
    assert!(bench.refs_of(ID).await?.is_empty());
    assert_eq!(bench.show(&member, ID).await?["backdrop"], json!(null));

    bench.forget_limits(ID).await?;
    assert_eq!(
        bench.put(&staff, ID, preset("paper")).await?.0,
        StatusCode::OK
    );
    bench.unstar(&staff).await?;
    let lapsed = Bench::public(pg);
    assert_eq!(
        lapsed.show(&member, ID).await?["backdrop"],
        chosen(preset("paper"), &staff),
        "a public preset stays without STAR"
    );
    reset(&bench.chat).await;
    Ok(())
}
