use tokio::task::JoinHandle;

use crate::{
    error::Error,
    section_writer::{DARK_GREEN, SectionWriter, WHITE},
    state_item::{ItemAction, ItemActionReceiver, MainAction, MainActionSender, StateItem},
    utils::spinner,
};

#[allow(dead_code)]
pub struct TestDisplay;

impl TestDisplay {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl StateItem for TestDisplay {
    async fn print(&self, writer: &mut SectionWriter, _output: &str) -> Result<(), Error> {
        // Loading spinner demo
        writer.open(DARK_GREEN, WHITE);
        writer.write_spinner(spinner::angle());
        writer.write(" Loading".to_string());
        writer.close();

        Ok(())
    }

    fn start_coroutine(
        &self,
        main_action_sender: MainActionSender,
        mut item_action_receiver: ItemActionReceiver,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                if !main_action_sender.enqueue(MainAction::Redraw).await {
                    break;
                }
                tokio::select! {
                    message = item_action_receiver.next() => {
                        match message {
                            None | Some(ItemAction::Update) => {}
                            Some(ItemAction::Terminate) => break,
                        }
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(crate::utils::spinner::TICK_MS)) => {}
                }
            }
        })
    }
}
