pub mod embed;
pub mod parse;

use tokio::sync::mpsc;

pub type ParseQueueSender = mpsc::Sender<i64>;
pub type EmbedQueueSender = mpsc::Sender<i64>;

pub struct ParseQueue {
    pub tx: ParseQueueSender,
}

pub struct EmbedQueue {
    pub tx: EmbedQueueSender,
}
