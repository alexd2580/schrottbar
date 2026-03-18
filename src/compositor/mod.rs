use crate::error::Error;

pub mod niri;
pub mod window_title;

#[derive(Debug, Clone)]
pub struct WorkspaceInfo {
    pub index: u32,
    pub name: Option<String>,
    pub is_active: bool,
    pub is_focused: bool,
    pub output: String,
    pub windows: Vec<WindowInfo>,
}

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub app_id: String,
    pub is_focused: bool,
}

#[derive(Debug)]
pub enum CompositorEvent {
    WorkspaceChanged,
    WindowChanged,
    OutputChanged,
}

#[async_trait::async_trait]
pub trait CompositorModule: Send + Sync {
    async fn get_workspaces(&self) -> Result<Vec<WorkspaceInfo>, Error>;
    async fn subscribe_events(&self) -> Result<tokio::sync::mpsc::Receiver<CompositorEvent>, Error>;
}
