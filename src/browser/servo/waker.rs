use std::sync::mpsc;

use servo::EventLoopWaker;

use super::events::ServoCommand;

#[derive(Clone)]
pub struct ServoWaker {
    tx: mpsc::SyncSender<ServoCommand>,
}

impl ServoWaker {
    pub fn new(tx: mpsc::SyncSender<ServoCommand>) -> Self {
        Self { tx }
    }
}

impl EventLoopWaker for ServoWaker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        let _ = self.tx.try_send(ServoCommand::Paint);
    }
}
