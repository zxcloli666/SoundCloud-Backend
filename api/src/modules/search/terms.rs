use sqlx::{Postgres, Transaction};

use crate::error::AppResult;

const CATALOG_GROUPS: usize = 6;
const LYRICS_TOKENS: i64 = 8;
const LYRICS_GROUPS: usize = 4;

const NOISE: &[&str] = &[
    "official",
    "video",
    "audio",
    "lyrics",
    "lyric",
    "music",
    "clip",
    "hd",
    "hq",
    "mp3",
    "full",
    "ft",
    "feat",
    "prod",
    "remastered",
    "текст",
    "клип",
    "песня",
    "слова",
];

const VARIANTS: &[&str] = &[
    "remix",
    "sped up",
    "slowed",
    "nightcore",
    "cover",
    "live",
    "8d",
    "reverb",
    "bass boosted",
    "instrumental",
    "karaoke",
    "mashup",
    "edit",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Tracks,
    Entities,
    Lyrics,
}

impl Shape {
    fn token_cap(self) -> i64 {
        match self {
            Self::Tracks | Self::Entities => CATALOG_GROUPS as i64,
            Self::Lyrics => LYRICS_TOKENS,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TermRow {
    pub ord: i16,
    pub lexeme: String,
    pub word: Option<String>,
    pub ndoc: Option<i32>,
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terms {
    pub strict: String,
    pub loose: Option<String>,
    pub variant: bool,
}

#[derive(Debug)]
struct Group {
    lexeme: String,
    alternatives: Vec<String>,
    known: bool,
    corrected: bool,
    rarity: i64,
}

pub async fn resolve(
    tx: &mut Transaction<'_, Postgres>,
    q_norm: &str,
    shape: Shape,
) -> AppResult<Option<Terms>> {
    let rows = sqlx::query_file_as!(
        TermRow,
        "queries/search/terms.sql",
        q_norm,
        shape.token_cap()
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(build(&rows, shape))
}

pub fn build(rows: &[TermRow], shape: Shape) -> Option<Terms> {
    let groups = groups(rows);
    if groups.is_empty() {
        return None;
    }
    let spoken = groups
        .iter()
        .map(|group| group.lexeme.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let variant = VARIANTS
        .iter()
        .any(|word| format!(" {spoken} ").contains(&format!(" {word} ")));
    let backed = groups
        .iter()
        .any(|group| is_word(group) && is_listed(group));
    let mut required: Vec<&Group> = groups
        .iter()
        .filter(|group| {
            is_word(group) && (is_listed(group) || (shape == Shape::Entities && backed))
        })
        .collect();
    if required.is_empty() {
        let longest = groups
            .iter()
            .max_by_key(|group| group.lexeme.chars().count())?;
        required.push(longest);
    }
    let limit = match shape {
        Shape::Tracks | Shape::Entities => CATALOG_GROUPS,
        Shape::Lyrics => LYRICS_GROUPS,
    };
    if shape == Shape::Lyrics {
        let mut by_rarity: Vec<(usize, &Group)> = required.into_iter().enumerate().collect();
        by_rarity.sort_by_key(|(position, group)| (group.rarity, *position));
        by_rarity.truncate(limit);
        by_rarity.sort_by_key(|(position, _)| *position);
        required = by_rarity.into_iter().map(|(_, group)| group).collect();
    }
    required.truncate(limit);
    let clauses: Vec<String> = required.iter().map(|group| clause(group)).collect();
    let loose = match (shape, clauses.len()) {
        (_, 0 | 1) => None,
        (Shape::Tracks | Shape::Entities, 2) => Some(clauses.join(" | ")),
        (Shape::Lyrics, 2) => None,
        _ => Some(all_but_one(&clauses)),
    };
    Some(Terms {
        strict: clauses.join(" & "),
        loose,
        variant,
    })
}

fn groups(rows: &[TermRow]) -> Vec<Group> {
    let mut groups: Vec<(i16, Group)> = Vec::new();
    for row in rows {
        if groups.last().is_none_or(|(ord, _)| *ord != row.ord) {
            groups.push((
                row.ord,
                Group {
                    lexeme: row.lexeme.clone(),
                    alternatives: vec![row.lexeme.clone()],
                    known: false,
                    corrected: false,
                    rarity: i64::MAX,
                },
            ));
        }
        let Some((_, group)) = groups.last_mut() else {
            continue;
        };
        let (Some(word), Some(kind)) = (row.word.as_deref(), row.kind.as_deref()) else {
            continue;
        };
        match kind {
            "same" => {
                group.known = true;
                group.rarity = group.rarity.min(i64::from(row.ndoc.unwrap_or(0)));
            }
            _ => group.corrected = true,
        }
        if !group.alternatives.iter().any(|known| known == word) {
            group.alternatives.push(word.to_owned());
        }
    }
    groups
        .into_iter()
        .map(|(_, mut group)| {
            if !group.known {
                group.rarity = 0;
            }
            group
        })
        .collect()
}

fn is_word(group: &Group) -> bool {
    group.lexeme.chars().count() > 1 && !NOISE.contains(&group.lexeme.as_str())
}

fn is_listed(group: &Group) -> bool {
    group.known || group.corrected
}

fn clause(group: &Group) -> String {
    let lexemes: Vec<String> = group.alternatives.iter().map(|word| quote(word)).collect();
    match lexemes.as_slice() {
        [only] => only.clone(),
        _ => format!("({})", lexemes.join(" | ")),
    }
}

fn all_but_one(clauses: &[String]) -> String {
    (0..clauses.len())
        .map(|skipped| {
            let kept: Vec<&str> = clauses
                .iter()
                .enumerate()
                .filter(|(position, _)| *position != skipped)
                .map(|(_, clause)| clause.as_str())
                .collect();
            format!("({})", kept.join(" & "))
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

pub fn quote(lexeme: &str) -> String {
    format!("'{}'", lexeme.replace('\\', "\\\\").replace('\'', "''"))
}
