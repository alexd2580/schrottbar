use log::error;

use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
};

use crate::error::Error;
use crate::section_writer::SectionWriter;

#[derive(Debug)]
#[allow(dead_code)]
pub enum MainAction {
    Terminate,
    Redraw,
    Reinit,
}
pub struct MainActionSender(pub mpsc::Sender<MainAction>);
pub struct MainActionReceiver(pub mpsc::Receiver<MainAction>);

pub fn new_main_action_channel() -> (MainActionSender, MainActionReceiver) {
    let (sender, receiver) = mpsc::channel(32);
    (MainActionSender(sender), MainActionReceiver(receiver))
}

impl MainActionSender {
    pub async fn enqueue(&self, msg: MainAction) -> bool {
        match self.0.send(msg).await {
            Err(err) => {
                error!("{err}");
                false
            }
            Ok(()) => true,
        }
    }
}

impl Clone for MainActionSender {
    fn clone(&self) -> Self {
        MainActionSender(self.0.clone())
    }
}

impl MainActionReceiver {
    pub async fn next(&mut self) -> Option<MainAction> {
        self.0.recv().await
    }

    pub fn try_next(&mut self) -> Option<MainAction> {
        self.0.try_recv().ok()
    }
}

#[derive(Clone)]
#[allow(dead_code)]
pub enum ItemAction {
    Update,
    Terminate,
}
pub struct ItemActionSender(pub broadcast::Sender<ItemAction>);
pub struct ItemActionReceiver(pub broadcast::Receiver<ItemAction>);

pub fn new_item_action_channel() -> (ItemActionSender, ItemActionReceiver) {
    let (sender, receiver) = broadcast::channel(32);
    (ItemActionSender(sender), ItemActionReceiver(receiver))
}

impl ItemActionSender {
    pub fn enqueue(&self, msg: ItemAction) -> bool {
        match self.0.send(msg) {
            Err(err) => {
                error!("{err}");
                false
            }
            Ok(_) => true,
        }
    }

    pub fn listen(&self) -> ItemActionReceiver {
        ItemActionReceiver(self.0.subscribe())
    }
}

impl ItemActionReceiver {
    pub async fn next(&mut self) -> Option<ItemAction> {
        self.0.recv().await.map_or_else(
            |err| {
                error!("{err}");
                None
            },
            Some,
        )
    }
}

#[async_trait::async_trait]
pub trait StateItem {
    async fn print(&self, writer: &mut SectionWriter, output: &str) -> Result<(), Error>;
    fn start_coroutine(
        &self,
        main_action_sender: MainActionSender,
        item_action_receiver: ItemActionReceiver,
    ) -> JoinHandle<()>;
}

pub async fn wait_seconds(num_seconds: u64) {
    tokio::time::sleep(tokio::time::Duration::from_secs(num_seconds)).await;
}
