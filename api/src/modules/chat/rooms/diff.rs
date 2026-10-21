use crate::modules::chat::rooms::system::{Authored, SystemRow};
use crate::modules::rooms::listing::PublicTrack;
use crate::modules::rooms::model::{Member, Room};
use crate::modules::rooms::service::Cause;

pub fn departed(before: &Room, after: &Room) -> Vec<String> {
    before
        .members
        .iter()
        .filter(|m| !after.is_member(&m.user_id))
        .map(|m| m.user_id.clone())
        .collect()
}

fn actor(cause: Cause<'_>, room: &Room) -> String {
    match cause {
        Cause::Moderation { by, .. } => by.to_owned(),
        Cause::Member | Cause::Quiet => room.host_id.clone(),
    }
}

fn name_of(before: &Room, after: &Room, user_id: &str) -> String {
    after
        .member(user_id)
        .or_else(|| before.member(user_id))
        .map_or_else(|| user_id.to_owned(), |m| m.name.clone())
}

fn departures(before: &Room, after: &Room, cause: Cause<'_>, rows: &mut Vec<Authored>) {
    for member in before
        .members
        .iter()
        .filter(|m| !after.is_member(&m.user_id))
    {
        let row = match cause {
            Cause::Moderation { by, ban } => SystemRow::Kick {
                user_id: member.user_id.clone(),
                name: member.name.clone(),
                by: by.to_owned(),
                ban,
            },
            Cause::Member | Cause::Quiet => SystemRow::Leave {
                user_id: member.user_id.clone(),
                name: member.name.clone(),
            },
        };
        let sender = match cause {
            Cause::Moderation { by, .. } => by.to_owned(),
            Cause::Member | Cause::Quiet => member.user_id.clone(),
        };
        rows.push(Authored { sender, row });
    }
}

fn arrivals(before: &Room, after: &Room, rows: &mut Vec<Authored>) {
    let joined = after
        .members
        .iter()
        .filter(|m| !before.is_member(&m.user_id));
    rows.extend(joined.map(|member: &Member| Authored {
        sender: member.user_id.clone(),
        row: SystemRow::Join {
            user_id: member.user_id.clone(),
            name: member.name.clone(),
        },
    }));
}

fn host_change(before: &Room, after: &Room, rows: &mut Vec<Authored>) {
    if before.host_id == after.host_id {
        return;
    }
    if let Some(host) = after.member(&after.host_id) {
        rows.push(Authored {
            sender: host.user_id.clone(),
            row: SystemRow::Host {
                user_id: host.user_id.clone(),
                name: host.name.clone(),
            },
        });
    }
}

fn moderator_changes(before: &Room, after: &Room, cause: Cause<'_>, rows: &mut Vec<Authored>) {
    let by = actor(cause, after);
    let named = after
        .moderators
        .iter()
        .filter(|id| !before.is_moderator(id))
        .map(|id| (id, true));
    let dropped = before
        .moderators
        .iter()
        .filter(|id| !after.is_moderator(id) && after.is_member(id) && !after.is_host(id))
        .map(|id| (id, false));
    for (user_id, on) in named.chain(dropped) {
        rows.push(Authored {
            sender: by.clone(),
            row: SystemRow::Moderator {
                user_id: user_id.clone(),
                name: name_of(before, after, user_id),
                by: by.clone(),
                on,
            },
        });
    }
}

fn mute_changes(before: &Room, after: &Room, cause: Cause<'_>, now: i64, rows: &mut Vec<Authored>) {
    let by = actor(cause, after);
    for member in &after.members {
        let was = before.muted_until(&member.user_id, now);
        let is = after.muted_until(&member.user_id, now);
        if was == is || (is.is_none() && !before.is_member(&member.user_id)) {
            continue;
        }
        rows.push(Authored {
            sender: by.clone(),
            row: SystemRow::Mute {
                user_id: member.user_id.clone(),
                name: member.name.clone(),
                by: by.clone(),
                until_ms: is,
            },
        });
    }
}

fn title_change(before: &Room, after: &Room, cause: Cause<'_>, rows: &mut Vec<Authored>) {
    if before.title == after.title {
        return;
    }
    let by = actor(cause, after);
    rows.push(Authored {
        sender: by.clone(),
        row: SystemRow::Title {
            by,
            title: after.title.clone(),
        },
    });
}

fn track_change(before: &Room, after: &Room, rows: &mut Vec<Authored>) {
    let urn = match &after.playback.track_urn {
        Some(urn) if Some(urn) != before.playback.track_urn.as_ref() => urn.clone(),
        _ => return,
    };
    let card = after.playback.track.as_ref().and_then(PublicTrack::of);
    rows.push(Authored {
        sender: after.host_id.clone(),
        row: SystemRow::Track {
            urn,
            title: card.as_ref().map(|card| card.title.clone()),
            artist: card.as_ref().and_then(|card| card.artist.clone()),
            artwork_url: card.and_then(|card| card.artwork_url),
        },
    });
}

pub fn rows(before: &Room, after: &Room, cause: Cause<'_>, now: i64) -> Vec<Authored> {
    let mut rows = Vec::new();
    departures(before, after, cause, &mut rows);
    arrivals(before, after, &mut rows);
    host_change(before, after, &mut rows);
    moderator_changes(before, after, cause, &mut rows);
    mute_changes(before, after, cause, now, &mut rows);
    title_change(before, after, cause, &mut rows);
    track_change(before, after, &mut rows);
    rows
}
