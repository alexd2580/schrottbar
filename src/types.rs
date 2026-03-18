pub type RGBA = (u8, u8, u8, u8);
pub type Poly = Vec<(f32, f32)>;
pub type Polys = Vec<Poly>;

#[derive(Debug, Copy, Clone)]
pub enum Alignment {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PowerlineStyle {
    Powerline,
    Octagon,
    Circle,
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

#[derive(Clone, PartialEq)]
pub enum ContentShape {
    Text(String),
    Powerline(PowerlineStyle, PowerlineFill, PowerlineDirection),
    /// A spinning arc. The f32 is the rotation angle in radians.
    Spinner(f32),
}

#[derive(Clone, PartialEq)]
pub struct ContentItem {
    pub fg: RGBA,
    pub bg: RGBA,
    pub shape: ContentShape,
}
