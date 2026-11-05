use sqlx::PgPool;
use uuid::Uuid;

use super::super::Cause;
use super::super::perks::tests::{Bench, profile, someone};
use crate::modules::rooms::model::{Backdrop, Perks};

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_handoff_takes_the_perks_of_the_new_host(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg)?;
    let (first, free, star) = (someone(), someone(), someone());
    bench.star(&first, true).await?;
    bench.star(&star, true).await?;
    let code = bench
        .rooms()
        .create(&first, &profile("First"), false, None)
        .await?
        .code;
    bench
        .rooms()
        .join(&code, &free, &profile("Free"), &bench.pg)
        .await?;
    let preset = Backdrop::Preset {
        id: "dunes".to_owned(),
    };
    bench
        .rooms()
        .apply(&code, Cause::Quiet, |room| {
            room.backdrop = Some(preset.clone());
            Ok(())
        })
        .await?;
    bench.rooms().leave(&code, &first).await?;
    let view = bench.rooms().view(&code, &free).await?;
    assert_eq!(
        (view.host_id.as_str(), view.perks),
        (free.as_str(), Perks::free())
    );
    assert_eq!(view.backdrop, None);
    assert_eq!(
        bench.rooms().room(&code).await?.backdrop,
        Some(preset.clone())
    );

    bench
        .rooms()
        .join(&code, &star, &profile("Star"), &bench.pg)
        .await?;
    bench.rooms().leave(&code, &free).await?;
    let view = bench.rooms().view(&code, &star).await?;
    assert_eq!(
        (view.host_id.as_str(), view.perks),
        (star.as_str(), Perks::star())
    );
    assert_eq!(view.backdrop, Some(preset));

    let next = someone();
    bench.star(&next, true).await?;
    bench
        .rooms()
        .join(&code, &next, &profile("Next"), &bench.pg)
        .await?;
    let media = Uuid::new_v4();
    bench
        .rooms()
        .apply(&code, Cause::Quiet, |room| {
            room.backdrop = Some(Backdrop::Image { media_id: media });
            Ok(())
        })
        .await?;
    assert_eq!(
        bench
            .rooms()
            .view(&code, &star)
            .await?
            .backdrop
            .and_then(|b| b.image()),
        Some(media)
    );
    bench.rooms().leave(&code, &star).await?;
    let view = bench.rooms().view(&code, &next).await?;
    assert_eq!(
        (view.host_id.as_str(), view.perks),
        (next.as_str(), Perks::star())
    );
    assert_eq!(view.backdrop, None);
    assert_eq!(bench.rooms().room(&code).await?.backdrop, None);
    bench.rooms().end(&code).await?;
    Ok(())
}
