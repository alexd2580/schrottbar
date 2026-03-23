use tokio::task::JoinHandle;

use crate::{
    error::Error,
    section_writer::SectionWriter,
    state_item::{ItemActionReceiver, MainActionSender, StateItem},
    types::{ContentItem, ContentShape},
};

pub struct HSpace(u32);

impl HSpace {
    pub fn new(width: u32) -> Self {
        Self(width)
    }
}

#[async_trait::async_trait]
impl StateItem for HSpace {
    async fn print(&self, writer: &mut SectionWriter, _output: &str) -> Result<(), Error> {
        writer.push_raw(ContentItem {
            fg: (0, 0, 0, 0),
            bg: (0, 0, 0, 0),
            shape: ContentShape::HSpace(self.0),
            on_click: None,
            hover_flag: None,
        });
        Ok(())
    }

    fn start_coroutine(
        &self,
        _main_action_sender: MainActionSender,
        _item_action_receiver: ItemActionReceiver,
    ) -> JoinHandle<()> {
        tokio::spawn(async {})
    }
}
