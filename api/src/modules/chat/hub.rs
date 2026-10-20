use tokio::sync::watch;

pub struct ChatHub;

impl ChatHub {
    pub fn new() -> Self {
        Self
    }

    pub fn wake(&self, _user_id: &str) {}

    pub fn subscribe(&self, _user_id: &str) -> watch::Receiver<u64> {
        watch::channel(0).1
    }
}

impl Default for ChatHub {
    fn default() -> Self {
        Self::new()
    }
}
