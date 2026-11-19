use crate::request::MobileRequest;

pub fn me() -> MobileRequest {
    MobileRequest::get(&["me"])
}
