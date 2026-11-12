use sqlx::PgPool;

use super::terms::TermRow;

const SOUND_ALIKE: &[(&str, &str)] = &[
    ("психосиз", "psychosis"),
    ("плейбой", "playboi"),
    ("карти", "carti"),
    ("найткол", "nightcall"),
    ("уикенд", "weekend"),
    ("квин", "queen"),
    ("дрейк", "drake"),
    ("айлиш", "eilish"),
    ("тейлор", "taylor"),
    ("свифт", "swift"),
    ("хаус", "house"),
    ("фейс", "face"),
    ("кул", "cool"),
    ("лав", "love"),
    ("гёрл", "girl"),
    ("майкл", "michael"),
    ("джексон", "jackson"),
    ("рианна", "rihanna"),
    ("канье", "kanye"),
    ("джордж", "george"),
    ("еллоу", "yellow"),
    ("раммштайн", "rammstein"),
    ("металлика", "metallica"),
    ("имеджин", "imagine"),
    ("колдплей", "coldplay"),
    ("блэк", "black"),
    ("найк", "nike"),
    ("шон", "shawn"),
    ("сэвидж", "savage"),
    ("марли", "marley"),
];

const SOUND_APART: &[(&str, &str)] = &[
    ("кул", "call"),
    ("найк", "neck"),
    ("блэк", "block"),
    ("дрейк", "dreck"),
    ("лав", "live"),
    ("квин", "can"),
    ("хаус", "cause"),
    ("шайн", "john"),
    ("бэд", "bid"),
    ("психосиз", "psicosis"),
];

async fn sound(pool: &PgPool, word: &str) -> anyhow::Result<String> {
    Ok(sqlx::query_scalar("SELECT search_sound($1)")
        .bind(word)
        .fetch_one(pool)
        .await?)
}

async fn seed(pool: &PgPool, words: &[(&str, i32)]) -> anyhow::Result<()> {
    for (word, ndoc) in words {
        sqlx::query(
            "INSERT INTO search_terms (word, ndoc, latin) VALUES ($1, $2, search_latin($1))",
        )
        .bind(word)
        .bind(ndoc)
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn heard(pool: &PgPool, q: &str) -> anyhow::Result<Vec<(String, String)>> {
    let mut tx = pool.begin().await?;
    sqlx::raw_sql(include_str!("../../../queries/search/configure.sql"))
        .execute(&mut *tx)
        .await?;
    let rows = sqlx::query_file_as!(TermRow, "queries/search/terms.sql", q, 6_i64)
        .fetch_all(&mut *tx)
        .await?;
    Ok(rows
        .into_iter()
        .filter(|row| row.kind.as_deref() == Some("sound"))
        .filter_map(|row| Some((row.lexeme, row.word?)))
        .collect())
}

fn pairs(found: &[(String, String)]) -> Vec<(&str, &str)> {
    found
        .iter()
        .map(|(said, word)| (said.as_str(), word.as_str()))
        .collect()
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_said_name_sounds_like_its_spelling(pool: PgPool) -> anyhow::Result<()> {
    for (said, spelled) in SOUND_ALIKE {
        assert_eq!(
            sound(&pool, said).await?,
            sound(&pool, spelled).await?,
            "{said} {spelled}"
        );
    }
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn another_vowel_or_consonant_sounds_apart(pool: PgPool) -> anyhow::Result<()> {
    for (said, spelled) in SOUND_APART {
        assert_ne!(
            sound(&pool, said).await?,
            sound(&pool, spelled).await?,
            "{said} {spelled}"
        );
    }
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_said_name_brings_its_most_common_spelling(pool: PgPool) -> anyhow::Result<()> {
    seed(
        &pool,
        &[
            ("psihosis", 3),
            ("psyhosis", 6),
            ("psychosis", 2107),
            ("psychossis", 20),
            ("psicosis", 9000),
            ("psychotic", 300),
        ],
    )
    .await?;
    assert_eq!(
        pairs(&heard(&pool, "психосиз").await?),
        [("психосиз", "psychosis")]
    );
    assert!(heard(&pool, "psychosis").await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn every_said_word_of_a_name_brings_its_own(pool: PgPool) -> anyhow::Result<()> {
    seed(
        &pool,
        &[
            ("плейбой", 73),
            ("playboi", 17698),
            ("карти", 156),
            ("carti", 21025),
        ],
    )
    .await?;
    assert_eq!(
        pairs(&heard(&pool, "плейбой карти").await?),
        [("плейбой", "playboi"), ("карти", "carti")]
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_spelling_with_the_same_letters_is_not_repeated(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool, &[("стар", 100), ("star", 87101), ("starr", 4835)]).await?;
    assert_eq!(pairs(&heard(&pool, "стар").await?), [("стар", "starr")]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_common_word_said_alone_keeps_to_itself(pool: PgPool) -> anyhow::Result<()> {
    seed(
        &pool,
        &[
            ("кино", 5635),
            ("keeno", 414),
            ("пачка", 800),
            ("pushka", 106),
        ],
    )
    .await?;
    assert!(heard(&pool, "кино").await?.is_empty());
    assert_eq!(
        pairs(&heard(&pool, "кино пачка").await?),
        [("кино", "keeno"), ("пачка", "pushka")]
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_short_common_word_never_brings_sound_alikes(pool: PgPool) -> anyhow::Result<()> {
    seed(
        &pool,
        &[
            ("вверх", 4542),
            ("where", 282483),
            ("руки", 13220),
            ("rookie", 3245),
        ],
    )
    .await?;
    assert_eq!(
        pairs(&heard(&pool, "руки вверх").await?),
        [("руки", "rookie")]
    );
    Ok(())
}
