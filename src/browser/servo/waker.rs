use std::sync::mpsc;

use servo::EventLoopWaker;

use super::events::ServoCommand;

#[derive(Clone)]
pub struct ServoWaker {
    pub tx: mpsc::SyncSender<ServoCommand>,
}

impl EventLoopWaker for ServoWaker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        let _ = self.tx.try_send(ServoCommand::Paint);
    }
}
