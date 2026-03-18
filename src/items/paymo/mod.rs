use std::sync::Arc;

use chrono::{DateTime, Utc};
use log::{debug, info, warn};
use crate::types::{PowerlineDirection, PowerlineStyle};
use tokio::{sync::Mutex, task::JoinHandle};

use crate::{
    section_writer::{SectionWriter, ACCENT_DIM},
    error::Error,
    state_item::{
        wait_seconds, ItemAction, ItemActionReceiver, MainAction, MainActionSender, StateItem,
    },
    utils::time::split_duration,
};

use self::paymo::{query_running_paymo_task, PaymoData};

mod paymo;

type SharedData = Arc<Mutex<Option<PaymoData>>>;
pub struct Paymo(SharedData);

impl Default for Paymo {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }
}

// fn duration_since_midnight() -> chrono::Duration {
//     let secs_from_midnight = Local::now().num_seconds_from_midnight();
//     chrono::Duration::seconds(i64::from(secs_from_midnight))
// }

#[async_trait::async_trait]
impl StateItem for Paymo {
    #[allow(clippy::cast_precision_loss)]
    async fn print(&self, writer: &mut SectionWriter, _output: &str) -> Result<(), Error> {
        let state = self.0.lock().await;
        if let Some(PaymoData {
            running_task: Some((name, started)),
            ..
        }) = &*state
        {
            let utc: DateTime<Utc> = Utc::now();
            let diff = utc - started.to_utc();
            let (hours, minutes) = split_duration(diff);

            writer.set_style(PowerlineStyle::Powerline);
            writer.set_direction(PowerlineDirection::Left);

            writer.with_bg(ACCENT_DIM, &|writer| {
                writer.write(format!("\u{f10eb} {name}: {hours:0>2}:{minutes:0>2}"));
                writer.set_direction(PowerlineDirection::Right);
            });
        }
        Ok(())
    }

    fn start_coroutine(
        &self,
        main_action_sender: MainActionSender,
        item_action_receiver: ItemActionReceiver,
    ) -> JoinHandle<()> {
        tokio::spawn(paymo_coroutine(
            self.0.clone(),
            main_action_sender,
            item_action_receiver,
        ))
    }
}

async fn paymo_coroutine(
    state: SharedData,
    main_action_sender: MainActionSender,
    mut item_action_receiver: ItemActionReceiver,
) {
    let mut initialized = false;
    loop {
        {
            let mut state_lock = state.lock().await;
            let new_state = query_running_paymo_task(&*state_lock).await;
            if !initialized {
                initialized = true;
                if new_state.is_none() {
                    break;
                }
            }
            *state_lock = new_state;
            if !main_action_sender.enqueue(MainAction::Redraw).await {
                break;
            }
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
