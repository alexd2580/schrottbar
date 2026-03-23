use crate::types::{PowerlineDirection, PowerlineStyle};
use chrono::Local;
use log::debug;
use tokio::task::JoinHandle;

use crate::{
    error::Error,
    section_writer::{ACCENT_DIM, SectionWriter, WHITE},
    state_item::{
        ItemAction, ItemActionReceiver, MainAction, MainActionSender, StateItem, wait_seconds,
    },
};

pub struct Time {
    format: String,
}

impl Time {
    pub fn new() -> Self {
        Self {
            format: "%a %d.%m %R".to_owned(),
        }
    }
}

#[async_trait::async_trait]
impl StateItem for Time {
    async fn print(&self, writer: &mut SectionWriter, _output: &str) -> Result<(), Error> {
        let now = Local::now();
        writer.set_style(PowerlineStyle::Powerline);
        writer.set_direction(PowerlineDirection::Left);
        writer.open(ACCENT_DIM, WHITE);
        writer.write(format!("󰥔 {}", now.format(&self.format)));
        writer.close();
        Ok(())
    }

    fn start_coroutine(
        &self,
        main_action_sender: MainActionSender,
        item_action_receiver: ItemActionReceiver,
    ) -> JoinHandle<()> {
        tokio::spawn(time_coroutine(main_action_sender, item_action_receiver))
    }
}

async fn time_coroutine(
    main_action_sender: MainActionSender,
    mut item_action_receiver: ItemActionReceiver,
) {
    loop {
        if !main_action_sender.enqueue(MainAction::Redraw).await {
            break;
        }

        tokio::select! {
            message = item_action_receiver.next() => {
                match message {
                    None | Some(ItemAction::Update)  => {},
                    Some(ItemAction::Terminate) => break,
                }
            }
            _ = wait_seconds(30) => {}
        }
    }
    debug!("coroutine exiting");
}
