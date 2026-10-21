use sqlx::PgPool;
use uuid::Uuid;

use super::{FileReport, file_with, load, reporter_weight, validate};
use crate::modules::chat::ChatError;
use crate::modules::chat::moderation::evidence;
use crate::modules::chat::moderation::testing::{
    TestKeys, aged_user, conversation, conversation_key, post, state,
};

fn input(conversation: Uuid, seq_from: i64, seq_to: i64) -> FileReport {
    FileReport {
        conversation_id: conversation,
        seq_from,
        seq_to,
        reason: "harassment".to_owned(),
        comment: None,
    }
}

#[test]
fn a_report_names_a_known_reason_and_a_short_range() {
    let conversation = Uuid::now_v7();
    assert!(validate(&input(conversation, 3, 3)).is_ok());
    assert!(validate(&input(conversation, 3, 52)).is_ok());
    assert_eq!(
        validate(&input(conversation, 3, 53)),
        Err(ChatError::Invalid)
    );
    assert_eq!(
        validate(&input(conversation, 4, 3)),
        Err(ChatError::Invalid)
    );
    assert_eq!(
        validate(&input(conversation, 0, 1)),
        Err(ChatError::Invalid)
    );
    let mut odd = input(conversation, 1, 1);
    odd.reason = "politics".to_owned();
    assert_eq!(validate(&odd), Err(ChatError::Invalid));
    let mut long = input(conversation, 1, 1);
    long.comment = Some("x".repeat(501));
    assert_eq!(validate(&long), Err(ChatError::Invalid));
}

#[sqlx::test(migrations = "./migrations")]
async fn a_report_seals_the_range_and_ten_messages_of_context(pool: PgPool) {
    let keys = TestKeys::default();
    let chat = state(pool.clone(), None);
    let dm = conversation(&pool, "direct", &[("11", true), ("22", true)]).await;
    let ck = conversation_key(&pool, &keys, dm).await;
    for index in 0..12 {
        let sender = if index % 2 == 0 { "11" } else { "22" };
        post(&pool, dm, &ck, sender, &format!("context {index}")).await;
    }
    let first = post(&pool, dm, &ck, "22", "you are a clown").await;
    let mine = post(&pool, dm, &ck, "11", "stop it").await;
    let last = post(&pool, dm, &ck, "22", "I WILL FIND YOU").await;
    post(&pool, dm, &ck, "22", "after the range").await;

    let filed = file_with(&chat, &keys, "11", &input(dm, first, last))
        .await
        .expect("a report");
    assert!(!filed.duplicate);
    let row = load(&chat, filed.id).await.expect("read").expect("a row");
    assert_eq!(row.target_user, "22");
    assert_eq!(row.state, "open");
    assert_eq!(row.weight, 0.25);
    assert_eq!(row.evidence_kek_version, Some(1));

    let sealed = row.evidence.expect("evidence");
    assert!(!String::from_utf8_lossy(&sealed).contains("clown"));
    let bundle = evidence::open(&keys, 1, &sealed).expect("evidence opens");
    let lines = evidence::lines(&bundle);
    assert_eq!(lines.len(), 13);
    assert_eq!(lines.iter().filter(|line| !line.reported).count(), 10);
    assert_eq!(lines[0].text, "context 2");
    let reported: Vec<(&str, &str)> = lines
        .iter()
        .filter(|line| line.reported)
        .map(|line| (line.sender.as_str(), line.text.as_str()))
        .collect();
    assert_eq!(
        reported,
        vec![
            ("22", "you are a clown"),
            ("11", "stop it"),
            ("22", "I WILL FIND YOU")
        ]
    );
    assert_eq!(
        lines.iter().find(|l| l.seq == mine).map(|l| l.reported),
        Some(true)
    );

    let again = file_with(&chat, &keys, "11", &input(dm, last, last))
        .await
        .expect("a duplicate");
    assert_eq!((again.id, again.duplicate), (filed.id, true));
}

#[sqlx::test(migrations = "./migrations")]
async fn only_members_report_and_only_what_one_other_person_sent(pool: PgPool) {
    let keys = TestKeys::default();
    let chat = state(pool.clone(), None);
    let group = conversation(&pool, "group", &[("11", true), ("22", true), ("33", true)]).await;
    let ck = conversation_key(&pool, &keys, group).await;
    let from_22 = post(&pool, group, &ck, "22", "hey").await;
    let from_33 = post(&pool, group, &ck, "33", "hoy").await;
    let from_11 = post(&pool, group, &ck, "11", "me").await;

    assert_eq!(
        file_with(&chat, &keys, "44", &input(group, from_22, from_22))
            .await
            .err(),
        Some(ChatError::NotMember)
    );
    assert_eq!(
        file_with(&chat, &keys, "11", &input(group, from_22, from_33))
            .await
            .err(),
        Some(ChatError::Invalid)
    );
    assert_eq!(
        file_with(&chat, &keys, "11", &input(group, from_11, from_11))
            .await
            .err(),
        Some(ChatError::Invalid)
    );
    assert_eq!(
        file_with(&chat, &keys, "11", &input(Uuid::now_v7(), 1, 1))
            .await
            .err(),
        Some(ChatError::NotFound)
    );

    sqlx::query(
        "UPDATE chat.members SET joined_seq = $2 WHERE conversation_id = $1 AND user_id = '11'",
    )
    .bind(group)
    .bind(from_33)
    .execute(&pool)
    .await
    .expect("a later join");
    assert_eq!(
        file_with(&chat, &keys, "11", &input(group, from_22, from_22))
            .await
            .err(),
        Some(ChatError::Invalid)
    );
    let saved = conversation(&pool, "self", &[("11", true)]).await;
    assert_eq!(
        file_with(&chat, &keys, "11", &input(saved, 1, 1))
            .await
            .err(),
        Some(ChatError::Invalid)
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_reporter_weighs_by_age_and_loses_weight_after_false_reports(pool: PgPool) {
    let chat = state(pool.clone(), None);
    assert_eq!(reporter_weight(&chat, "51").await.expect("a weight"), 0.25);
    aged_user(&pool, "51", 3).await;
    assert_eq!(reporter_weight(&chat, "51").await.expect("a weight"), 0.25);
    aged_user(&pool, "51", 8).await;
    assert_eq!(reporter_weight(&chat, "51").await.expect("a weight"), 1.0);

    for _ in 0..2 {
        sqlx::query(
            "INSERT INTO chat.reports (id, reporter, target_user, conversation_id, seq_from, seq_to, reason, weight, state, handled_at)
             VALUES ($1, '51', '52', $2, 1, 1, 'spam', 1, 'rejected', now())",
        )
        .bind(Uuid::now_v7())
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await
        .expect("a rejected report");
    }
    assert_eq!(reporter_weight(&chat, "51").await.expect("a weight"), 0.0);
}

#[sqlx::test(migrations = "./migrations")]
async fn the_recipient_of_a_message_request_reports_it_before_accepting(pool: PgPool) {
    let keys = TestKeys::default();
    let chat = state(pool.clone(), None);
    let request = conversation(&pool, "direct", &[("61", true), ("62", false)]).await;
    let ck = conversation_key(&pool, &keys, request).await;
    let spam = post(&pool, request, &ck, "61", "cheap followers").await;

    let filed = file_with(&chat, &keys, "62", &input(request, spam, spam))
        .await
        .expect("a filed report");
    let row = load(&chat, filed.id).await.expect("a load").expect("a row");
    assert_eq!(
        (row.reporter.as_str(), row.target_user.as_str()),
        ("62", "61")
    );

    sqlx::query(
        "UPDATE chat.members SET state = 'left' WHERE conversation_id = $1 AND user_id = '62'",
    )
    .bind(request)
    .execute(&pool)
    .await
    .expect("a decline");
    assert_eq!(
        file_with(&chat, &keys, "62", &input(request, spam, spam))
            .await
            .err(),
        Some(ChatError::NotMember)
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn evidence_seals_and_opens_with_the_real_kek_ring(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let dm = conversation(&pool, "direct", &[("91", true), ("92", true)]).await;
    let ck: [u8; 32] = chat_wire::random_bytes().expect("a ck");
    sqlx::query(
        "INSERT INTO chat.conversation_keys (conversation_id, epoch, wrapped, kek_version)
         VALUES ($1, 0, $2, $3)",
    )
    .bind(dm)
    .bind(chat.keks.wrap(dm, 0, &ck).expect("a wrapped ck"))
    .bind(chat.keks.current() as i16)
    .execute(&pool)
    .await
    .expect("a conversation key");
    let seq = post(&pool, dm, &ck, "92", "sealed by the ring").await;

    let filed = file_with(&chat, &chat.keks, "91", &input(dm, seq, seq))
        .await
        .expect("a report");
    let row = load(&chat, filed.id).await.expect("read").expect("a row");
    let version = row.evidence_kek_version.expect("a version") as u16;
    assert_eq!(version, chat.keks.reports_current());
    let sealed = row.evidence.expect("evidence");
    let bundle = evidence::open(&chat.keks, version, &sealed).expect("evidence opens");
    assert_eq!(
        evidence::lines(&bundle)
            .last()
            .map(|line| line.text.as_str()),
        Some("sealed by the ring")
    );
    assert!(evidence::open(&TestKeys::default(), version, &sealed).is_err());
}
