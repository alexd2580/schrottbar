use super::{CompositorEvent, CompositorModule, WorkspaceInfo};
use crate::error::Error;
use tokio::sync::mpsc;

pub struct NiriCompositor {
    socket_path: String,
}

impl NiriCompositor {
    pub fn new() -> Result<Self, Error> {
        let socket_path = std::env::var("NIRI_SOCKET")
            .map_err(|_| Error::Local("NIRI_SOCKET not set".to_string()))?;
        Ok(Self { socket_path })
    }
}

#[async_trait::async_trait]
impl CompositorModule for NiriCompositor {
    async fn get_workspaces(&self) -> Result<Vec<WorkspaceInfo>, Error> {
        // TODO: Implement via niri-ipc
        let _ = &self.socket_path;
        Ok(Vec::new())
    }

    async fn subscribe_events(&self) -> Result<mpsc::Receiver<CompositorEvent>, Error> {
        // TODO: Implement via niri-ipc event stream
        let (_tx, rx) = mpsc::channel(32);
        Ok(rx)
    }
}
