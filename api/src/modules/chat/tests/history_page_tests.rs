use chrono::Utc;
use uuid::Uuid;

use super::*;

fn row(seq: i64, body: usize) -> MessageRow {
    MessageRow {
        conversation_id: Uuid::from_u128(1),
        seq,
        client_id: Uuid::from_u128(u128::try_from(seq).expect("a positive seq")),
        sender_user: "101".to_owned(),
        sender_device: Some(Uuid::from_u128(5)),
        class: "post".to_owned(),
        epoch: 0,
        target_seq: None,
        body: Some(vec![3; body]),
        created_at: Utc::now(),
        expires_at: None,
    }
}

fn encoded_len(page: &HistoryPage) -> usize {
    serde_json::to_vec(page).expect("page json").len()
}

#[test]
fn a_page_never_exceeds_its_byte_cap() {
    for (cap, body) in [(24 * 1024, 2_000), (256 * 1024, 9_000), (4_096, 700)] {
        let rows: Vec<MessageRow> = (1..=200).map(|seq| row(seq, body)).collect();
        let page = fit(rows, 200, cap);
        assert!(page.more, "cap {cap}");
        assert!(!page.messages.is_empty());
        assert!(
            encoded_len(&page) <= cap,
            "cap {cap}: {}",
            encoded_len(&page)
        );
    }
}

#[test]
fn a_page_fills_close_to_the_cap() {
    let rows: Vec<MessageRow> = (1..=200).map(|seq| row(seq, 1_000)).collect();
    let page = fit(rows, 200, 24 * 1024);
    let one = serde_json::to_vec(&page.messages[0])
        .expect("row json")
        .len();
    assert!(encoded_len(&page) + one + 1 > 24 * 1024);
}

#[test]
fn the_row_limit_sets_more() {
    let rows: Vec<MessageRow> = (1..=6).map(|seq| row(seq, 10)).collect();
    let page = fit(rows.clone(), 5, PAGE_BYTES_MAX);
    assert_eq!(page.messages.len(), 5);
    assert!(page.more);
    let page = fit(rows, 6, PAGE_BYTES_MAX);
    assert_eq!(page.messages.len(), 6);
    assert!(!page.more);
}

#[test]
fn one_huge_row_still_comes_alone_so_paging_never_stalls() {
    let rows = vec![row(1, 40 * 1024), row(2, 10)];
    let page = fit(rows, 50, PAGE_BYTES_MAX_PRO);
    assert_eq!(page.messages.len(), 1);
    assert!(page.more);
}

#[test]
fn pro_pages_are_capped_at_24_kb_and_by_the_client_budget() {
    assert_eq!(byte_cap(Tier::Pro, 0), PAGE_BYTES_MAX_PRO);
    assert_eq!(byte_cap(Tier::Direct, 0), PAGE_BYTES_MAX);
    assert_eq!(byte_cap(Tier::Relay, 1 << 30), PAGE_BYTES_MAX);
    assert_eq!(byte_cap(Tier::Direct, 10_000), 10_000);
    assert_eq!(byte_cap(Tier::Pro, 100_000), PAGE_BYTES_MAX_PRO);
}

#[test]
fn the_row_limit_defaults_to_50_and_tops_out_at_200() {
    assert_eq!(rows_limit(None), PAGE_ROWS_DEFAULT);
    assert_eq!(rows_limit(Some(0)), PAGE_ROWS_DEFAULT);
    assert_eq!(rows_limit(Some(7)), 7);
    assert_eq!(rows_limit(Some(5_000)), PAGE_ROWS_MAX);
}

mod stored {
    use sqlx::PgPool;

    use super::super::*;
    use crate::modules::chat::ChatResult;
    use crate::modules::chat::conversations::fixtures::{access_of, dm, exec, group, post};
    use crate::modules::chat::conversations::remove;
    use crate::modules::chat::messages::HistoryQuery;
    use crate::modules::chat::model::MemberState;

    fn query(after: Option<i64>, before: Option<i64>, limit: u32) -> HistoryQuery {
        HistoryQuery {
            after,
            before,
            limit: Some(limit),
        }
    }

    fn seqs(page: &HistoryPage) -> Vec<i64> {
        page.messages.iter().map(|message| message.seq).collect()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn pages_walk_both_ways_in_seq_order(pool: PgPool) -> ChatResult<()> {
        let id = dm(&pool, "101", "202", MemberState::Active).await;
        for _ in 0..7 {
            post(&pool, id, "101").await;
        }
        let access = access_of(&pool, id, "202").await;
        let latest = read(&pool, &access, &query(None, None, 3), PAGE_BYTES_MAX).await?;
        assert_eq!((seqs(&latest), latest.more), (vec![5, 6, 7], true));
        let older = read(&pool, &access, &query(None, Some(5), 3), PAGE_BYTES_MAX).await?;
        assert_eq!((seqs(&older), older.more), (vec![2, 3, 4], true));
        let oldest = read(&pool, &access, &query(None, Some(2), 3), PAGE_BYTES_MAX).await?;
        assert_eq!((seqs(&oldest), oldest.more), (vec![1], false));
        let newer = read(&pool, &access, &query(Some(4), None, 2), PAGE_BYTES_MAX).await?;
        assert_eq!((seqs(&newer), newer.more), (vec![5, 6], true));
        let tail = read(&pool, &access, &query(Some(6), None, 5), PAGE_BYTES_MAX).await?;
        assert_eq!((seqs(&tail), tail.more), (vec![7], false));
        assert_eq!(
            read(&pool, &access, &query(Some(1), Some(5), 5), PAGE_BYTES_MAX)
                .await
                .err(),
            Some(ChatError::Invalid)
        );
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn history_never_serves_an_expired_row(pool: PgPool) -> ChatResult<()> {
        let id = dm(&pool, "101", "202", MemberState::Active).await;
        for _ in 0..3 {
            post(&pool, id, "101").await;
        }
        exec(
            &pool,
            "UPDATE chat.messages SET expires_at = now() - interval '1 second' WHERE conversation_id = $1 AND seq = 2",
            id,
        )
        .await;
        let access = access_of(&pool, id, "101").await;
        let page = read(&pool, &access, &query(None, None, 50), PAGE_BYTES_MAX).await?;
        assert_eq!(seqs(&page), vec![1, 3]);
        let after = read(&pool, &access, &query(Some(1), None, 50), PAGE_BYTES_MAX).await?;
        assert_eq!(seqs(&after), vec![3]);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_cleared_chat_starts_after_the_clear(pool: PgPool) -> ChatResult<()> {
        let id = dm(&pool, "101", "202", MemberState::Active).await;
        post(&pool, id, "101").await;
        post(&pool, id, "202").await;
        remove::hide(&pool, &access_of(&pool, id, "101").await).await?;
        post(&pool, id, "202").await;
        let mine = read(
            &pool,
            &access_of(&pool, id, "101").await,
            &query(None, None, 50),
            PAGE_BYTES_MAX,
        )
        .await?;
        assert_eq!(seqs(&mine), vec![3]);
        let theirs = read(
            &pool,
            &access_of(&pool, id, "202").await,
            &query(Some(0), None, 50),
            PAGE_BYTES_MAX,
        )
        .await?;
        assert_eq!(seqs(&theirs), vec![1, 2, 3]);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn hidden_group_history_starts_at_the_join(pool: PgPool) -> ChatResult<()> {
        let id = group(&pool, "101", &[("202", MemberState::Active)]).await;
        post(&pool, id, "101").await;
        exec(
            &pool,
            "UPDATE chat.conversations SET settings = '{\"history_visible\": false}' WHERE id = $1",
            id,
        )
        .await;
        exec(
            &pool,
            "UPDATE chat.members SET joined_seq = 2 WHERE conversation_id = $1 AND user_id = '202'",
            id,
        )
        .await;
        post(&pool, id, "101").await;
        let page = read(
            &pool,
            &access_of(&pool, id, "202").await,
            &query(None, None, 50),
            PAGE_BYTES_MAX,
        )
        .await?;
        assert_eq!(seqs(&page), vec![3]);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_stored_page_stays_under_the_pro_cap(pool: PgPool) -> ChatResult<()> {
        let id = dm(&pool, "101", "202", MemberState::Active).await;
        for _ in 0..120 {
            post(&pool, id, "101").await;
        }
        let access = access_of(&pool, id, "101").await;
        let page = read(&pool, &access, &query(None, None, 200), PAGE_BYTES_MAX_PRO).await?;
        assert!(page.more);
        assert!(serde_json::to_vec(&page).expect("json").len() <= PAGE_BYTES_MAX_PRO);
        assert_eq!(page.messages.last().map(|message| message.seq), Some(120));
        Ok(())
    }
}
