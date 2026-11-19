mod account;
mod apiv1;
mod args;
mod comment;
mod comment_apiv1;
mod datadome;
mod journal;
mod reaction;
mod scrub;
mod sign_in;
mod streams;

use std::process::ExitCode;

use sc_mobile::{Egress, MobileClient};

use crate::args::{Args, USAGE};
use crate::journal::Journal;
use crate::scrub::Scrubber;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let args = match args::parse(&arguments) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let app = match args::load_profile(&args.profile) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let scrubber = Scrubber::new(&app);
    let egress = match &args.proxy {
        Some(proxy) => Egress::Proxy(proxy.clone()),
        None => Egress::Direct,
    };
    let client = match MobileClient::new(app, egress) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let mut journal = match Journal::open(&args.out, scrubber) {
        Ok(journal) => journal,
        Err(error) => {
            eprintln!(
                "the output directory {} cannot be used: {error}",
                args.out.display()
            );
            return ExitCode::FAILURE;
        }
    };

    run(&client, &args, &mut journal).await;

    journal.print_summary();
    if journal.failed() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

async fn run(client: &MobileClient, args: &Args, journal: &mut Journal) {
    let Some(mut session) = sign_in::sign_in(client, journal).await else {
        return;
    };
    let own_urn = account::read_me(client, &session.caller(), journal, "me").await;
    if let Some(tokens) = account::refresh(client, &session, journal).await {
        session.tokens = tokens;
        account::read_me(client, &session.caller(), journal, "me_after_refresh").await;
    }
    let caller = session.caller();

    match &args.track {
        Some(track) => streams::probe(client, &caller, journal, "track", track).await,
        None => journal.skip("track", "no --track"),
    }
    match &args.goplus_track {
        Some(track) => streams::probe(client, &caller, journal, "goplus", track).await,
        None => journal.skip("goplus", "no --goplus-track"),
    }
    match &args.reaction_track {
        Some(track) => reaction::probe(client, &caller, journal, track).await,
        None => journal.skip("reaction", "no --reaction-track"),
    }
    comment::run(client, args, &caller, journal, own_urn.as_deref()).await;
}
