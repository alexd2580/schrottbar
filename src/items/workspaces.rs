use std::sync::Arc;

use log::{debug, error};
use tokio::{sync::Mutex, task::JoinHandle};

use crate::{
    error::Error,
    section_writer::{ACCENT, DARK_GRAY, GRAY, LIGHT_GRAY, SectionWriter, WHITE},
    state_item::{ItemAction, ItemActionReceiver, MainAction, MainActionSender, StateItem},
    types::{ClickHandler, PowerlineDirection, PowerlineStyle},
};

use super::niri;

type SharedState = Arc<Mutex<Vec<niri_ipc::Workspace>>>;

pub struct Workspaces(SharedState);

impl Workspaces {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
}

#[async_trait::async_trait]
impl StateItem for Workspaces {
    async fn print(&self, writer: &mut SectionWriter, output: &str) -> Result<(), Error> {
        let state = self.0.lock().await;
        let mut workspaces: Vec<&niri_ipc::Workspace> = state
            .iter()
            .filter(|ws| ws.output.as_deref() == Some(output))
            .collect();
        workspaces.sort_by_key(|ws| ws.idx);

        if workspaces.is_empty() {
            return Ok(());
        }

        writer.set_style(PowerlineStyle::Circle);
        writer.set_direction(PowerlineDirection::Left);
        writer.open(DARK_GRAY, GRAY);

        for (i, ws) in workspaces.iter().enumerate() {
            if i > 0 {
                writer.write_hspace(4);
            }
            let ws_id = ws.id;
            writer.set_on_click(Arc::new(move |_button| {
                let request = niri_ipc::Request::Action(niri_ipc::Action::FocusWorkspace {
                    reference: niri_ipc::WorkspaceReferenceArg::Id(ws_id),
                });
                tokio::spawn(async move {
                    if let Err(e) = niri::niri_request(request).await {
                        error!("Failed to focus workspace: {e}");
                    }
                });
            }) as ClickHandler);
            let (circle_color, fg) = if ws.is_focused {
                (ACCENT, WHITE)
            } else if ws.is_active {
                (GRAY, WHITE)
            } else {
                (DARK_GRAY, LIGHT_GRAY)
            };
            writer.set_fg(fg);
            writer.write_circled(format!("{}", ws.idx), circle_color);
            writer.clear_on_click();
        }
        writer.set_direction(PowerlineDirection::Right);
        writer.close();
        Ok(())
    }

    fn start_coroutine(
        &self,
        main_action_sender: MainActionSender,
        item_action_receiver: ItemActionReceiver,
    ) -> JoinHandle<()> {
        tokio::spawn(workspace_coroutine(
            self.0.clone(),
            main_action_sender,
            item_action_receiver,
        ))
    }
}

async fn workspace_coroutine(
    state: SharedState,
    main_action_sender: MainActionSender,
    mut item_action_receiver: ItemActionReceiver,
) {
    match niri::niri_request(niri_ipc::Request::Workspaces).await {
        Ok(niri_ipc::Response::Workspaces(workspaces)) => {
            *state.lock().await = workspaces;
            let _ = main_action_sender.enqueue(MainAction::Redraw).await;
        }
        Ok(other) => {
            error!("Unexpected niri response: {other:?}");
            return;
        }
        Err(err) => {
            error!("Failed to get initial workspaces: {err}");
            return;
        }
    }

    let mut lines = match niri::open_event_stream().await {
        Ok(l) => l,
        Err(err) => {
            error!("{err}");
            return;
        }
    };

    loop {
        tokio::select! {
            event = niri::next_event(&mut lines) => {
                match event {
                    Some(event) => {
                        let changed = handle_event(&state, event).await;
                        if changed && !main_action_sender.enqueue(MainAction::Redraw).await {
                            break;
                        }
                    }
                    None => {
                        debug!("niri event stream ended");
                        break;
                    }
                }
            }
            message = item_action_receiver.next() => {
                match message {
                    None | Some(ItemAction::Update) => {}
                    Some(ItemAction::Terminate) => break,
                }
            }
        }
    }
    debug!("workspace coroutine exiting");
}

async fn handle_event(state: &SharedState, event: niri_ipc::Event) -> bool {
    match event {
        niri_ipc::Event::WorkspacesChanged { workspaces } => {
            *state.lock().await = workspaces;
            true
        }
        niri_ipc::Event::WorkspaceActivated { id, focused } => {
            let mut state = state.lock().await;
            let output = state
                .iter()
                .find(|w| w.id == id)
                .and_then(|w| w.output.clone());
            for ws in state.iter_mut() {
                if focused {
                    ws.is_focused = ws.id == id;
                }
                if ws.output == output {
                    ws.is_active = ws.id == id;
                }
            }
            true
        }
        _ => false,
    }
}
