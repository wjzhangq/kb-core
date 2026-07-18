pub mod embed;
pub mod parse;

use std::sync::Arc;
use anyhow::Result;
use tokio::sync::mpsc;

use crate::config::KBConfig;
use crate::db::DbConn;
use crate::tantivy_idx::TantivyIndex;

pub type ParseQueueSender = mpsc::Sender<i64>;
pub type EmbedQueueSender = mpsc::Sender<i64>;

pub struct ParseQueue {
    pub tx: ParseQueueSender,
}

pub struct EmbedQueue {
    pub tx: EmbedQueueSender,
}
