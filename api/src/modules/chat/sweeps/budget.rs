use crate::modules::chat::ChatState;

pub struct Budget {
    batches: u32,
}

impl Budget {
    pub fn new(batches: u32) -> Self {
        Self { batches }
    }

    pub fn left(&self, chat: &ChatState) -> bool {
        self.batches > 0 && !chat.shutdown.is_cancelled()
    }

    pub fn spend(&mut self, chat: &ChatState) -> bool {
        if !self.left(chat) {
            return false;
        }
        self.batches -= 1;
        true
    }
}
