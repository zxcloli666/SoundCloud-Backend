use sqlx::PgPool;

use super::{RelatedWindow, project_page, window};

#[test]
fn the_related_window_is_bounded_and_never_asks_for_an_unreachable_offset() {
    assert_eq!(
        window(0, 30),
        Some(RelatedWindow {
            offset: 0,
            wanted: 31
        })
    );
    assert_eq!(
        window(6, 30),
        Some(RelatedWindow {
            offset: 180,
            wanted: 200
        })
    );
    assert_eq!(window(7, 30), None);
    assert_eq!(window(1, 200), None);
    assert_eq!(window(100, 200), None);
    assert_eq!(
        window(0, 200),
        Some(RelatedWindow {
            offset: 0,
            wanted: 200
        })
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn related_serves_public_neighbours_only_and_marks_the_viewers_likes(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing, deleted_at) VALUES
         ('1', 'soundcloud:tracks:1', 'Public', 'public', 1000, 'public', NULL),
         ('2', 'soundcloud:tracks:2', 'Private', 'private', 1000, 'private', NULL),
         ('3', 'soundcloud:tracks:3', 'Gone', 'gone', 1000, 'public', now());
         INSERT INTO user_likes_tracks (user_id, sc_track_id) VALUES ('42', '1');",
    )
    .execute(&pool)
    .await?;
    let ids = ["1".to_owned(), "2".to_owned(), "3".to_owned()];
    let page = project_page(&pool, "42", &ids, 0, 30, true).await?;
    assert_eq!(page.collection.len(), 1);
    assert_eq!(page.collection[0]["urn"], "soundcloud:tracks:1");
    assert_eq!(page.collection[0]["user_favorite"], true);
    assert!(page.has_more);
    assert_eq!(page.page_size, 30);
    let other = project_page(&pool, "43", &ids, 0, 30, false).await?;
    assert!(other.collection[0].get("user_favorite").is_none());
    assert!(!other.has_more);
    Ok(())
}
