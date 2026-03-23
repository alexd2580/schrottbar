use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[allow(clippy::upper_case_acronyms)]
pub type RGBA = (u8, u8, u8, u8);
pub type Poly = Vec<(f32, f32)>;
pub type Polys = Vec<Poly>;

/// Click handler closure. The `u32` parameter is the mouse button code.
pub type ClickHandler = Arc<dyn Fn(u32) + Send + Sync>;

/// Shared flag set by the bar when the pointer is over this item's zone.
pub type HoverFlag = Arc<AtomicBool>;

#[derive(Debug, Copy, Clone)]
pub enum Alignment {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum PowerlineStyle {
    Powerline,
    Octagon,
    Circle,
    /// Rectangular blocks with a thin gap between them (no shaped separator).
    Block,
    /// Horizontal gradient fade between two section backgrounds.
    Fade,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PowerlineFill {
    Full,
    No,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PowerlineDirection {
    Left,
    Right,
}

/// Pre-scaled RGBA pixel data for an icon.
#[derive(Clone, PartialEq)]
pub struct IconData {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA, 4 bytes per pixel.
    pub pixels: Vec<u8>,
}

#[derive(Clone, PartialEq)]
pub enum ContentShape {
    Text(String),
    /// Text centered inside a filled circle. The RGBA is the circle color.
    CircledText(String, RGBA),
    /// Text centered inside a circle outline (ring). The RGBA is the ring color.
    RingedText(String, RGBA),
    Powerline(PowerlineStyle, PowerlineFill, PowerlineDirection),
    /// A spinning arc. The f32 is the rotation angle in radians.
    Spinner(f32),
    /// A raster icon (e.g. system tray).
    Icon(IconData),
    /// Horizontal space of a fixed pixel width.
    HSpace(u32),
}

pub struct ContentItem {
    pub fg: RGBA,
    pub bg: RGBA,
    pub shape: ContentShape,
    pub on_click: Option<ClickHandler>,
    /// When set, the bar updates this flag based on pointer position.
    pub hover_flag: Option<HoverFlag>,
}

impl Clone for ContentItem {
    fn clone(&self) -> Self {
        Self {
            fg: self.fg,
            bg: self.bg,
            shape: self.shape.clone(),
            on_click: self.on_click.clone(),
            hover_flag: self.hover_flag.clone(),
        }
    }
}

impl PartialEq for ContentItem {
    fn eq(&self, other: &Self) -> bool {
        self.fg == other.fg && self.bg == other.bg && self.shape == other.shape
    }
}
