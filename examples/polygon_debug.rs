//! Renders powerline separators between narrow bars to a PNG for debugging.
//!
//! Run: cargo run --example polygon_debug
//! Output: /tmp/polygon_debug.png

use tiny_skia::Pixmap;

use schrottbar::renderer::{self, Renderer};
use schrottbar::types::{PowerlineDirection, PowerlineFill, PowerlineStyle, RGBA};

const BLACK: RGBA = (0, 0, 0, 255);
const BG1: RGBA = (40, 80, 120, 255);
const BG2: RGBA = (120, 40, 80, 255);

fn main() {
    let bar_height: u32 = 10;
    let pw = renderer::powerline_width(bar_height, PowerlineStyle::Powerline);
    let bar_w = 2u32;

    // [2px BG1] [Right Full sep] [2px BG2] [Left Full sep] [2px BG3]
    let total_w = bar_w * 5 + pw * 4;

    let mut pixmap = Pixmap::new(total_w, bar_height).unwrap();
    let mut pm = pixmap.as_mut();

    let mut x = 0u32;

    // Bar 1
    Renderer::fill_rect(&mut pm, x, 0, bar_w, bar_height, BLACK);
    x += bar_w;

    // Forward slash: Right Full — BG1 arrow on black background
    let polys = renderer::shape_powerline(
        bar_height,
        x,
        PowerlineDirection::Right,
        PowerlineFill::Full,
    );
    Renderer::fill_polys(&mut pm, &polys, BG1);
    x += pw;

    // Bar 2
    Renderer::fill_rect(&mut pm, x, 0, bar_w, bar_height, BLACK);
    x += bar_w;

    // Backslash: Left Full — BG2 arrow on black background
    let polys =
        renderer::shape_powerline(bar_height, x, PowerlineDirection::Left, PowerlineFill::Full);
    Renderer::fill_polys(&mut pm, &polys, BG2);
    x += pw;

    // Bar 3
    Renderer::fill_rect(&mut pm, x, 0, bar_w, bar_height, BLACK);
    x += bar_w;

    // Forward slash: Right Sparse — BG1 arrow on black background
    let polys =
        renderer::shape_powerline(bar_height, x, PowerlineDirection::Right, PowerlineFill::No);
    Renderer::fill_polys(&mut pm, &polys, BG1);
    x += pw;

    // Bar 2
    Renderer::fill_rect(&mut pm, x, 0, bar_w, bar_height, BLACK);
    x += bar_w;

    // Backslash: Left Sparse — BG2 arrow on black background
    let polys =
        renderer::shape_powerline(bar_height, x, PowerlineDirection::Left, PowerlineFill::No);
    Renderer::fill_polys(&mut pm, &polys, BG2);
    x += pw;

    // Bar 3
    Renderer::fill_rect(&mut pm, x, 0, bar_w, bar_height, BLACK);

    let path = "/tmp/polygon_debug.png";
    pixmap.save_png(path).unwrap();
    println!("Saved to {path} ({total_w}x{bar_height})");
}
