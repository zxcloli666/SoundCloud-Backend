#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Login,
    LoginStatus,
    LinkCreate,
    LinkClaim,
    LinkStatus,
}

impl Kind {
    pub fn parse(path: &str) -> Option<Self> {
        match path {
            "/auth/login" => Some(Kind::Login),
            "/auth/login/status" => Some(Kind::LoginStatus),
            "/auth/link/create" => Some(Kind::LinkCreate),
            "/auth/link/claim" => Some(Kind::LinkClaim),
            "/auth/link/status" => Some(Kind::LinkStatus),
            _ => None,
        }
    }

    pub fn path(self) -> &'static str {
        match self {
            Kind::Login => "/auth/login",
            Kind::LoginStatus => "/auth/login/status",
            Kind::LinkCreate => "/auth/link/create",
            Kind::LinkClaim => "/auth/link/claim",
            Kind::LinkStatus => "/auth/link/status",
        }
    }

    pub fn starts(self) -> bool {
        matches!(self, Kind::Login | Kind::LinkCreate)
    }
}
