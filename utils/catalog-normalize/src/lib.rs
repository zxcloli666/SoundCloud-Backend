mod names;
mod title;
mod translit;
mod work;

pub const NORMALIZER_VERSION: i16 = 1;

pub use names::{
    RawMetaMatch, compact_key, compare_with_meta, fold_chars, is_invisible, is_junk_artist_name,
    meta_artist_names, name_in, name_similarity, same_artist, split_artist_list,
    unescape_json_unicode,
};
pub use title::{
    ParsedTitle, clean_artist_name, compact_title, normalize_name, normalize_title, parse_sc_title,
    strip_translit_parens, title_marks_cover,
};
pub use work::{TitleForms, VersionMarker, title_forms, works_match};
