use crate::modules::chat::model::MessageView;

pub const CONVERSATION: usize = 160;
pub const GONE: usize = 48;
pub const GONE_MESSAGE: usize = 64;
const MESSAGE: usize = 300;

pub fn message(view: &MessageView) -> usize {
    let body = view
        .body
        .as_ref()
        .map_or(0, |body| body.len().div_ceil(3) * 4);
    MESSAGE + view.sender.len() + body
}
