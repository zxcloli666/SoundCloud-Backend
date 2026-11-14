use std::time::{Duration, UNIX_EPOCH};

use backend_contracts::pipeline::SayRequest;
use uuid::Uuid;

use super::*;

#[test]
fn a_line_goes_out_with_the_reply_subject_the_ticket_and_the_deadline_of_the_rpc_contract() {
    let ticket = Uuid::now_v7();
    let ask = Ask {
        request: SayRequest {
            voice: "loli".to_owned(),
            text: "Привет.".to_owned(),
        },
        reply_to: format!("speak.reply.98765.{ticket}"),
        ticket,
        deadline: UNIX_EPOCH + Duration::from_millis(1_700_000_090_250),
    };

    let headers = headers(&ask);
    let header = |name: &str| headers.get(name).map(|value| value.as_str().to_owned());

    assert_eq!(header(REPLY_TO_HEADER), Some(ask.reply_to.clone()));
    assert_eq!(header(MSG_ID_HEADER), Some(ticket.to_string()));
    assert_eq!(header(DEADLINE_HEADER), Some("1700000090250".to_owned()));
}
