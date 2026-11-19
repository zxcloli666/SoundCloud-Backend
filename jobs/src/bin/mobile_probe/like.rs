use sc_mobile::{Caller, MobileClient, likes};

use crate::actions::{LEFT, step};
use crate::journal::Journal;

pub async fn run(client: &MobileClient, caller: &Caller, journal: &mut Journal, track: &str) {
    let create = likes::create(track);
    let created = step(client, caller, journal, "like.create", &create, |_| {
        "liked".to_owned()
    })
    .await;
    if !created.ok {
        return;
    }
    journal.skip("like.read", "no read: the notes give no likes listing");
    let delete = likes::delete(track);
    let deleted = step(client, caller, journal, "like.delete", &delete, |_| {
        "unliked".to_owned()
    })
    .await;
    if deleted.blocked {
        journal.skip("like.left", LEFT);
    }
}
