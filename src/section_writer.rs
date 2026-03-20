use crate::types::{
    ClickHandler, ContentItem, ContentShape, IconData, PowerlineDirection, PowerlineFill,
    PowerlineStyle, RGBA,
};

pub const WHITE: RGBA = (240, 240, 248, 255);
pub const LIGHT_GRAY: RGBA = (210, 212, 224, 255);
pub const GRAY: RGBA = (120, 120, 132, 255);
pub const DARK_GRAY: RGBA = (50, 54, 68, 220);
#[allow(dead_code)]
pub const DARKEST_GRAY: RGBA = (24, 26, 36, 230);
pub const BLACK: RGBA = (0, 0, 0, 0);
pub const OPAQUE_BLACK: RGBA = (20, 20, 28, 255);
pub const RED: RGBA = (240, 62, 62, 255);
#[allow(dead_code)]
pub const TOO_RED: RGBA = (255, 20, 20, 255);
pub const DARK_GREEN: RGBA = (6, 140, 80, 255);
#[allow(dead_code)]
pub const GREEN: RGBA = (16, 172, 100, 255);
pub const BLUE: RGBA = (56, 132, 244, 255);
pub const ACCENT: RGBA = (180, 80, 220, 255);
pub const ACCENT_DIM: RGBA = (120, 96, 230, 255);

pub const THIN_SPACE: &str = "\u{2009}";

pub const WHITE_ON_BLACK: (RGBA, RGBA) = (BLACK, WHITE);

// pub const GOOD: (RGBA, RGBA) = (GREEN, BLACK);
// pub const NEUTRAL: (RGBA, RGBA) = (DARK_GREEN, LIGHT_GRAY);
// pub const INFO: (RGBA, RGBA) = (INFO_YELLOW, BLACK);
// pub const WARN: (RGBA, RGBA) = (RED, LIGHT_GRAY);
// pub const CRITICAL: (RGBA, RGBA) = (TOO_RED, WHITE);

#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
pub fn mix_colors(value: f32, min: f32, max: f32, min_color: RGBA, max_color: RGBA) -> RGBA {
    if value <= min {
        return min_color;
    }
    if value >= max {
        return max_color;
    }

    let alpha = ((value - min) / (max - min)).clamp(0f32, 1f32);
    let inv_alpha = 1.0 - alpha;

    let (r1, g1, b1, a1) = min_color;
    let (r2, g2, b2, a2) = max_color;
    (
        (f32::from(r1) * inv_alpha + f32::from(r2) * alpha) as u8,
        (f32::from(g1) * inv_alpha + f32::from(g2) * alpha) as u8,
        (f32::from(b1) * inv_alpha + f32::from(b2) * alpha) as u8,
        (f32::from(a1) * inv_alpha + f32::from(a2) * alpha) as u8,
    )
}

pub fn mix_colors_multi(value: f32, reference_points: &[(f32, RGBA)]) -> RGBA {
    let (mut min, mut min_color) = reference_points[0];
    for &(max, max_color) in reference_points {
        if value < max {
            return mix_colors(value, min, max, min_color, max_color);
        }

        min = max;
        min_color = max_color;
    }
    min_color
}

fn font_color(bg: RGBA) -> RGBA {
    let luminance = 0.299 * f32::from(bg.0) + 0.587 * f32::from(bg.1) + 0.114 * f32::from(bg.2);
    if luminance > 128.0 {
        OPAQUE_BLACK
    } else {
        WHITE
    }
}

pub struct SectionWriter {
    texts: Vec<ContentItem>,

    style: PowerlineStyle,
    direction: PowerlineDirection,

    bg: RGBA,
    fg: RGBA,
    click_handler: Option<ClickHandler>,
}

impl SectionWriter {
    pub fn set_style(&mut self, style: PowerlineStyle) {
        self.style = style;
    }

    pub fn set_direction(&mut self, direction: PowerlineDirection) {
        self.direction = direction;
    }

    pub fn set_fg(&mut self, fg: RGBA) {
        self.fg = fg;
    }

    pub fn set_on_click(&mut self, handler: ClickHandler) {
        self.click_handler = Some(handler);
    }

    pub fn clear_on_click(&mut self) {
        self.click_handler = None;
    }

    pub fn write(&mut self, text: String) {
        self.texts.push(ContentItem {
            shape: ContentShape::Text(text),
            fg: self.fg,
            bg: self.bg,
            on_click: self.click_handler.clone(),
        });
    }

    pub fn write_icon(&mut self, icon: IconData) {
        self.texts.push(ContentItem {
            shape: ContentShape::Icon(icon),
            fg: self.fg,
            bg: self.bg,
            on_click: self.click_handler.clone(),
        });
    }

    pub fn push_raw(&mut self, item: ContentItem) {
        self.texts.push(item);
    }

    pub fn write_hspace(&mut self, width: u32) {
        self.texts.push(ContentItem {
            shape: ContentShape::HSpace(width),
            fg: self.bg,
            bg: self.bg,
            on_click: None,
        });
    }

    pub fn write_circled(&mut self, text: String, circle_color: RGBA) {
        self.texts.push(ContentItem {
            shape: ContentShape::CircledText(text, circle_color),
            fg: self.fg,
            bg: self.bg,
            on_click: self.click_handler.clone(),
        });
    }

    pub fn write_ringed(&mut self, text: String, ring_color: RGBA) {
        self.texts.push(ContentItem {
            shape: ContentShape::RingedText(text, ring_color),
            fg: self.fg,
            bg: self.bg,
            on_click: self.click_handler.clone(),
        });
    }

    pub fn write_spinner(&mut self, angle: f32) {
        self.texts.push(ContentItem {
            shape: ContentShape::Spinner(angle),
            fg: self.fg,
            bg: self.bg,
            on_click: self.click_handler.clone(),
        });
    }

    fn write_powerline(&mut self, fill: PowerlineFill) {
        self.texts.push(ContentItem {
            shape: ContentShape::Powerline(self.style, fill, self.direction),
            fg: self.fg,
            bg: self.bg,
            on_click: None, // powerline separators are not clickable
        });
    }

    fn separate(&mut self, next_background: RGBA, next_foreground: RGBA) {
        if next_background == self.bg {
            self.fg = BLACK;
            self.write_powerline(PowerlineFill::No);
        } else {
            match self.direction {
                PowerlineDirection::Left => {
                    self.fg = next_background;
                    self.write_powerline(PowerlineFill::Full);
                    self.bg = next_background;
                }
                PowerlineDirection::Right => {
                    self.fg = self.bg;
                    self.bg = next_background;
                    self.write_powerline(PowerlineFill::Full);
                }
            }
        }

        self.fg = next_foreground;
    }

    #[allow(dead_code)]
    pub fn open_(&mut self, next_colors: (RGBA, RGBA)) {
        self.open(next_colors.0, next_colors.1);
    }

    pub fn open(&mut self, next_background: RGBA, next_foreground: RGBA) {
        self.separate(next_background, next_foreground);
    }

    pub fn open_bg(&mut self, next_bg: RGBA) {
        self.separate(next_bg, font_color(next_bg));
    }

    pub fn split(&mut self) {
        self.separate(self.bg, self.fg);
    }

    pub fn close(&mut self) {
        self.separate(WHITE_ON_BLACK.0, WHITE_ON_BLACK.1);
    }

    pub fn with_bg(&mut self, next_bg: RGBA, body: &impl Fn(&mut Self)) {
        self.open_bg(next_bg);
        body(self);
        self.close();
    }

    pub fn unwrap(self) -> Vec<ContentItem> {
        self.texts
    }
}

impl Default for SectionWriter {
    fn default() -> Self {
        Self {
            texts: Vec::new(),
            style: PowerlineStyle::Powerline,
            direction: PowerlineDirection::Right,
            bg: BLACK,
            fg: WHITE,
            click_handler: None,
        }
    }
}
