mod accounts;
mod indexed;
mod score;
mod wanted;

pub use accounts::{
    AccountRole, claims_another_artist, delete_account, extract_sc_user_id, is_soundcloud_url,
    upsert_account,
};
pub use indexed::{
    IndexedMatch, WORK_TITLE_THRESHOLD, attach_genius_song, best_indexed_for_artist_title,
};
pub use score::{
    DurationMatch, TrackMatch, artist_score, duration_match, evaluate_sc_candidate,
    sc_track_id_from_urn, title_score,
};
pub use wanted::link_wanted_to_sc;
