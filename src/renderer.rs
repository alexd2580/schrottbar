use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache};
use tiny_skia::{BlendMode, FillRule, Paint, PathBuilder, PixmapMut, Rect, Transform};

use crate::types::{IconData, Poly, Polys, PowerlineDirection, PowerlineFill, PowerlineStyle, RGBA};

pub struct Renderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    font_size: f32,
    font_family: String,
    ascent: u32,
    descent: u32,
}

/// Create a tiny-skia paint with R↔B swapped so that tiny-skia's RGBA output
/// becomes BGRA, matching wl_shm Argb8888 byte order on little-endian.
fn rgba_to_paint(rgba: RGBA) -> Paint<'static> {
    rgba_to_paint_aa(rgba, true)
}

fn rgba_to_paint_aa(rgba: RGBA, anti_alias: bool) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(rgba.2, rgba.1, rgba.0, rgba.3);
    paint.anti_alias = anti_alias;
    paint
}

fn shape_buffer(
    font_system: &mut FontSystem,
    font_family: &str,
    font_size: f32,
    text: &str,
) -> Buffer {
    let metrics = Metrics::new(font_size, font_size);
    let mut buffer = Buffer::new(font_system, metrics);
    let attrs = Attrs::new().family(Family::Name(font_family));
    buffer.set_text(font_system, text, attrs, Shaping::Advanced);
    buffer.shape_until_scroll(font_system, false);
    buffer
}

impl Renderer {
    pub fn new(font_family: &str, font_size: f32) -> Self {
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();

        let buffer = shape_buffer(&mut font_system, font_family, font_size, "M");
        let (ascent, descent) = buffer
            .layout_runs()
            .next()
            .map(|run| {
                // line_y is the baseline Y from the top of the line.
                let ascent = run.line_y as u32;
                // Ensure ascent + descent == ceil(font_size) to avoid truncation loss.
                let descent = font_size.ceil() as u32 - ascent;
                (ascent, descent)
            })
            .unwrap_or((font_size as u32, 0));

        log::info!(
            "Font metrics: ascent={ascent} descent={descent} font_size={font_size} bar_height={}",
            ascent + descent + Self::PADDING
        );
        assert!(ascent > 0, "ascent must be positive");
        assert!(descent > 0, "descent must be positive");
        assert!(
            ascent + descent <= font_size.ceil() as u32,
            "ascent({ascent}) + descent({descent}) exceeds font_size({font_size})"
        );

        Self {
            font_system,
            swash_cache,
            font_size,
            font_family: font_family.to_string(),
            ascent,
            descent,
        }
    }

    pub const PADDING: u32 = 6;

    pub fn height(&self) -> u32 {
        self.ascent + self.descent + Self::PADDING
    }

    pub fn ascent(&self) -> u32 {
        self.ascent
    }

    pub fn descent(&self) -> u32 {
        self.descent
    }

    fn shape_text(&mut self, text: &str) -> Buffer {
        shape_buffer(
            &mut self.font_system,
            &self.font_family,
            self.font_size,
            text,
        )
    }

    pub fn text_width(&mut self, text: &str) -> u32 {
        self.shape_text(text)
            .layout_runs()
            .map(|run| run.line_w)
            .sum::<f32>()
            .ceil() as u32
    }

    pub fn draw_text_outlined(
        &mut self,
        pixmap: &mut PixmapMut,
        text: &str,
        fg: RGBA,
        outline: RGBA,
        x: u32,
        canvas_height: u32,
    ) {
        self.draw_text_inner(pixmap, text, fg, x, canvas_height, Some(outline));
    }

    fn draw_text_inner(
        &mut self,
        pixmap: &mut PixmapMut,
        text: &str,
        fg: RGBA,
        x: u32,
        canvas_height: u32,
        outline: Option<RGBA>,
    ) {
        let buffer = self.shape_text(text);

        let text_height = self.ascent + self.descent;
        assert!(
            canvas_height >= text_height,
            "canvas_height({canvas_height}) < text_height({text_height})"
        );
        let baseline_offset = (canvas_height - text_height) / 2 + self.ascent;

        let pw = pixmap.width() as i32;
        let ph = pixmap.height() as i32;

        // Collect glyph images so we can do multiple passes without re-rasterizing.
        struct GlyphData {
            gx: i32,
            gy: i32,
            width: i32,
            height: i32,
            content: cosmic_text::SwashContent,
            data: Vec<u8>,
        }

        let mut glyphs = Vec::new();
        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((x as f32, 0.0), 1.0);
                if let Some(image) = self
                    .swash_cache
                    .get_image(&mut self.font_system, physical.cache_key)
                {
                    glyphs.push(GlyphData {
                        gx: physical.x + image.placement.left,
                        gy: baseline_offset as i32 - image.placement.top,
                        width: image.placement.width as i32,
                        height: image.placement.height as i32,
                        content: image.content,
                        data: image.data.clone(),
                    });
                }
            }
        }

        let render_glyphs = |pixmap: &mut PixmapMut, color: RGBA, dx: i32, dy: i32| {
            for g in &glyphs {
                for iy in 0..g.height {
                    for ix in 0..g.width {
                        let px = g.gx + ix + dx;
                        let py = g.gy + iy + dy;
                        if px < 0 || py < 0 || px >= pw || py >= ph {
                            continue;
                        }

                        let idx = (iy * g.width + ix) as usize;
                        let alpha = match g.content {
                            cosmic_text::SwashContent::Mask => {
                                g.data.get(idx).copied().unwrap_or(0)
                            }
                            cosmic_text::SwashContent::Color => {
                                g.data.get(idx * 4 + 3).copied().unwrap_or(0)
                            }
                            cosmic_text::SwashContent::SubpixelMask => {
                                g.data.get(idx * 3 + 1).copied().unwrap_or(0)
                            }
                        };
                        if alpha == 0 {
                            continue;
                        }

                        let pixel_offset = (py as u32 * pw as u32 + px as u32) as usize * 4;
                        let pixels = pixmap.data_mut();

                        if g.content == cosmic_text::SwashContent::Color {
                            let base = idx * 4;
                            blend_pixel(
                                pixels,
                                pixel_offset,
                                g.data[base],
                                g.data[base + 1],
                                g.data[base + 2],
                                alpha,
                            );
                        } else {
                            blend_pixel(pixels, pixel_offset, color.0, color.1, color.2, alpha);
                        }
                    }
                }
            }
        };

        // Soft shadow: single pass offset down-right
        if let Some(shadow_color) = outline {
            render_glyphs(pixmap, shadow_color, 2, 1);
        }

        // Foreground pass
        render_glyphs(pixmap, fg, 0, 0);
    }

    pub fn fill_rect(pixmap: &mut PixmapMut, x: u32, y: u32, w: u32, h: u32, color: RGBA) {
        // Clamp to pixmap bounds — content may overflow the screen.
        let w = w.min(pixmap.width().saturating_sub(x));
        let h = h.min(pixmap.height().saturating_sub(y));
        if w == 0 || h == 0 {
            return;
        }
        if let Some(rect) = Rect::from_xywh(x as f32, y as f32, w as f32, h as f32) {
            let mut paint = rgba_to_paint(color);
            paint.blend_mode = BlendMode::Source;
            pixmap.fill_rect(rect, &paint, Transform::identity(), None);
        }
    }

    /// Blit an RGBA icon onto the BGRA pixmap, centered vertically.
    pub fn draw_icon(pixmap: &mut PixmapMut, icon: &IconData, x: u32, canvas_height: u32) {
        if icon.width == 0 || icon.height == 0 {
            return;
        }
        let y_offset = canvas_height.saturating_sub(icon.height) / 2;
        let pw = pixmap.width() as i32;
        let ph = pixmap.height() as i32;

        for iy in 0..icon.height {
            for ix in 0..icon.width {
                let px = x as i32 + ix as i32;
                let py = y_offset as i32 + iy as i32;
                if px < 0 || py < 0 || px >= pw || py >= ph {
                    continue;
                }

                let src_idx = ((iy * icon.width + ix) * 4) as usize;
                let r = icon.pixels[src_idx];
                let g = icon.pixels[src_idx + 1];
                let b = icon.pixels[src_idx + 2];
                let a = icon.pixels[src_idx + 3];
                if a == 0 {
                    continue;
                }

                let dst_offset = (py as u32 * pw as u32 + px as u32) as usize * 4;
                blend_pixel(pixmap.data_mut(), dst_offset, r, g, b, a);
            }
        }
    }

    pub fn fill_polys(pixmap: &mut PixmapMut, polys: &[Poly], color: RGBA) {
        let mut pb = PathBuilder::new();
        for points in polys {
            if points.len() < 3 {
                continue;
            }
            // Skip polygons that are entirely off-screen.
            let max_x = pixmap.width() as f32;
            let max_y = pixmap.height() as f32;
            if points.iter().all(|&(x, _)| x > max_x) || points.iter().all(|&(_, y)| y > max_y) {
                continue;
            }
            pb.move_to(points[0].0, points[0].1);
            for &(x, y) in &points[1..] {
                pb.line_to(x, y);
            }
            pb.close();
        }

        if let Some(path) = pb.finish() {
            let mut paint = rgba_to_paint_aa(color, true);
            paint.blend_mode = BlendMode::Source;
            pixmap.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }
}

/// Powerline arrow shape geometry.
pub fn shape_powerline(
    height: u32,
    xl: u32,
    direction: PowerlineDirection,
    fill: PowerlineFill,
) -> Polys {
    let h = height as f32;
    let w = height.div_ceil(2) as f32;

    let xl = xl as f32;
    let xr = xl + w;
    let yt = 0.0f32;
    let yb = h;

    match (direction, fill) {
        (PowerlineDirection::Left, PowerlineFill::Full) => {
            vec![vec![(xr, yb), (xl, yt + h / 2.0), (xr, yt)]]
        }
        (PowerlineDirection::Right, PowerlineFill::Full) => {
            vec![vec![(xl, yb), (xr, yt + h / 2.0), (xl, yt)]]
        }
        (PowerlineDirection::Left, PowerlineFill::No) => {
            let t = 1.5f32;
            vec![vec![
                (xl, yt + h / 2.0 + t / 2.0),
                (xr - t, yb),
                (xr, yb),
                (xr, yb - t),
                (xl + t, yt + h / 2.0),
                (xr, yt + t),
                (xr, yt),
                (xr - t, yt),
                (xl, yt + h / 2.0 - t / 2.0),
            ]]
        }
        (PowerlineDirection::Right, PowerlineFill::No) => {
            let t = 1.5f32;
            vec![vec![
                (xr, yt + h / 2.0 - t / 2.0),
                (xl + t, yt),
                (xl, yt),
                (xl, yt + t),
                (xr - t, yt + h / 2.0),
                (xl, yb - t),
                (xl, yb),
                (xl + t, yb),
                (xr, yt + h / 2.0 + t / 2.0),
            ]]
        }
    }
}

pub fn shape_octagon(
    height: u32,
    xl: u32,
    direction: PowerlineDirection,
    fill: PowerlineFill,
) -> Polys {
    let h = height as f32;
    let h_4 = h / 4.0;
    let w = ((height + 1) / 4) as f32;

    let xl = xl as f32;
    let xr = xl + w;
    let yt = 0.0f32;
    let yb = h;

    match (direction, fill) {
        // Octagon Right: flat left edge, two angled edges meeting a vertical right side.
        //   top-left → top-right(angled) → right-side(vertical) → bottom-right(angled) → bottom-left
        (PowerlineDirection::Right, PowerlineFill::Full) => {
            vec![vec![(xl, yb), (xr, yb - h_4), (xr, yt + h_4), (xl, yt)]]
        }
        (PowerlineDirection::Right, PowerlineFill::No) => {
            let t = 2.5f32;
            vec![vec![
                // outer
                (xl, yt),
                (xr, yt + h_4),
                (xr, yb - h_4),
                (xl, yb),
                // inner
                (xl, yb - t),
                (xr - t, yb - h_4),
                (xr - t, yt + h_4),
                (xl, yt + t),
            ]]
        }
        // Octagon Left: flat right edge, two angled edges meeting a vertical left side.
        (PowerlineDirection::Left, PowerlineFill::Full) => {
            vec![vec![(xl, yt + h_4), (xl, yb - h_4), (xr, yb), (xr, yt)]]
        }
        (PowerlineDirection::Left, PowerlineFill::No) => {
            let t = 2.5f32;
            vec![vec![
                // outer
                (xr, yt),
                (xl, yt + h_4),
                (xl, yb - h_4),
                (xr, yb),
                // inner
                (xr, yb - t),
                (xl + t, yb - h_4),
                (xl + t, yt + h_4),
                (xr, yt + t),
            ]]
        }
    }
}

/// Semicircle separator approximated as a polygon.
/// The flat edge is vertical, the curved edge bulges in the given direction.
pub fn shape_circle(
    height: u32,
    xl: u32,
    direction: PowerlineDirection,
    fill: PowerlineFill,
) -> Polys {
    let h = height as f32;
    let r = h / 2.0;
    let w = height.div_ceil(2) as f32;

    let xl = xl as f32;
    let xr = xl + w;
    let cy = h / 2.0;

    // Number of segments for the arc — more = smoother.
    let segments = 24u32;

    let arc_points = |radius: f32| -> Vec<(f32, f32)> {
        (0..=segments)
            .map(|i| {
                // Angle from -π/2 to π/2 (right-bulging semicircle)
                let t = -std::f32::consts::FRAC_PI_2
                    + std::f32::consts::PI * i as f32 / segments as f32;
                let x = radius * t.cos();
                let y = radius * t.sin();
                (x, cy + y)
            })
            .collect()
    };

    match (direction, fill) {
        (PowerlineDirection::Right, PowerlineFill::Full) => {
            // Flat left edge, curved right edge
            let mut points: Vec<(f32, f32)> = arc_points(r)
                .into_iter()
                .map(|(x, y)| (xl + x, y))
                .collect();
            // Close with flat left edge
            points.push((xl, h));
            points.push((xl, 0.0));
            vec![points]
        }
        (PowerlineDirection::Left, PowerlineFill::Full) => {
            // Curved left edge, flat right edge
            let mut points: Vec<(f32, f32)> = arc_points(r)
                .into_iter()
                .rev()
                .map(|(x, y)| (xr - x, y))
                .collect();
            points.push((xr, 0.0));
            points.push((xr, h));
            vec![points]
        }
        (PowerlineDirection::Right, PowerlineFill::No) => {
            let t = 1.5f32;
            let outer: Vec<(f32, f32)> = arc_points(r)
                .into_iter()
                .map(|(x, y)| (xl + x, y))
                .collect();
            let inner: Vec<(f32, f32)> = arc_points(r - t)
                .into_iter()
                .rev()
                .map(|(x, y)| (xl + x, y))
                .collect();
            let mut points = outer;
            points.extend(inner);
            vec![points]
        }
        (PowerlineDirection::Left, PowerlineFill::No) => {
            let t = 1.5f32;
            let outer: Vec<(f32, f32)> = arc_points(r)
                .into_iter()
                .rev()
                .map(|(x, y)| (xr - x, y))
                .collect();
            let inner: Vec<(f32, f32)> = arc_points(r - t)
                .into_iter()
                .map(|(x, y)| (xr - x, y))
                .collect();
            let mut points = outer;
            points.extend(inner);
            vec![points]
        }
    }
}

pub fn shape_polys(
    height: u32,
    xl: u32,
    style: PowerlineStyle,
    direction: PowerlineDirection,
    fill: PowerlineFill,
) -> Polys {
    match style {
        PowerlineStyle::Powerline => shape_powerline(height, xl, direction, fill),
        PowerlineStyle::Octagon => shape_octagon(height, xl, direction, fill),
        PowerlineStyle::Circle => shape_circle(height, xl, direction, fill),
        PowerlineStyle::Block => vec![], // No shape — just a transparent gap
        PowerlineStyle::Fade => vec![],  // Handled separately as gradient
    }
}

pub fn powerline_width(height: u32, style: PowerlineStyle) -> u32 {
    match style {
        PowerlineStyle::Powerline => height.div_ceil(2),
        PowerlineStyle::Octagon => (height + 1) / 4,
        PowerlineStyle::Circle => height.div_ceil(2),
        PowerlineStyle::Block => 3,
        PowerlineStyle::Fade => height / 2,
    }
}

/// Width (and height) of the spinner shape — a square cell.
pub fn spinner_size(height: u32) -> u32 {
    height
}

/// A spinning arc: a 270° thick arc ring, rotated by `angle` radians.
/// Centered in a square cell of `height x height` at x offset `xl`.
pub fn shape_spinner(height: u32, xl: u32, angle: f32) -> Polys {
    let h = height as f32;
    let cx = xl as f32 + h / 2.0;
    let cy = h / 2.0;

    let r_outer = h * 0.38;
    let r_inner = h * 0.22;
    let segments = 32u32;

    // 270° arc = 3/4 of a full circle
    let arc_len = std::f32::consts::TAU * 0.75;

    let mut points = Vec::with_capacity((segments as usize + 1) * 2);

    // Outer arc
    for i in 0..=segments {
        let t = angle + arc_len * i as f32 / segments as f32;
        points.push((cx + r_outer * t.cos(), cy + r_outer * t.sin()));
    }

    // Inner arc (reversed to close the shape)
    for i in (0..=segments).rev() {
        let t = angle + arc_len * i as f32 / segments as f32;
        points.push((cx + r_inner * t.cos(), cy + r_inner * t.sin()));
    }

    vec![points]
}

/// Draw a filled circle centered at (cx, cy) with given radius.
pub fn draw_filled_circle(
    pixmap: &mut PixmapMut,
    cx: f32,
    cy: f32,
    radius: f32,
    color: RGBA,
) {
    use std::f32::consts::PI;
    const SEGMENTS: u32 = 24;

    let mut pb = PathBuilder::new();
    pb.move_to(cx + radius, cy);
    for i in 1..=SEGMENTS {
        let angle = 2.0 * PI * i as f32 / SEGMENTS as f32;
        pb.line_to(cx + radius * angle.cos(), cy + radius * angle.sin());
    }
    pb.close();

    if let Some(path) = pb.finish() {
        let paint = rgba_to_paint(color);
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// Draw a pill (stadium shape): rectangle with semicircle end caps.
pub fn draw_pill(
    pixmap: &mut PixmapMut,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: RGBA,
) {
    use std::f32::consts::{FRAC_PI_2, PI};
    const SEGMENTS: u32 = 12;

    let r = h / 2.0;
    let mut pb = PathBuilder::new();

    // Top edge (left to right)
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);

    // Right semicircle
    for i in 1..=SEGMENTS {
        let angle = -FRAC_PI_2 + PI * i as f32 / SEGMENTS as f32;
        pb.line_to(x + w - r + r * angle.cos(), y + r + r * angle.sin());
    }

    // Bottom edge (right to left)
    pb.line_to(x + r, y + h);

    // Left semicircle
    for i in 1..=SEGMENTS {
        let angle = FRAC_PI_2 + PI * i as f32 / SEGMENTS as f32;
        pb.line_to(x + r + r * angle.cos(), y + r + r * angle.sin());
    }
    pb.close();

    if let Some(path) = pb.finish() {
        let paint = rgba_to_paint(color);
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// Draw a circle outline (ring) centered at (cx, cy).
pub fn draw_ring(
    pixmap: &mut PixmapMut,
    cx: f32,
    cy: f32,
    radius: f32,
    thickness: f32,
    color: RGBA,
) {
    use std::f32::consts::PI;
    const SEGMENTS: u32 = 24;

    let r_outer = radius;
    let r_inner = radius - thickness;
    if r_inner <= 0.0 {
        return draw_filled_circle(pixmap, cx, cy, radius, color);
    }

    let mut pb = PathBuilder::new();

    // Outer circle
    pb.move_to(cx + r_outer, cy);
    for i in 1..=SEGMENTS {
        let angle = 2.0 * PI * i as f32 / SEGMENTS as f32;
        pb.line_to(cx + r_outer * angle.cos(), cy + r_outer * angle.sin());
    }
    pb.close();

    // Inner circle (winding creates a hole)
    pb.move_to(cx + r_inner, cy);
    for i in (0..SEGMENTS).rev() {
        let angle = 2.0 * PI * i as f32 / SEGMENTS as f32;
        pb.line_to(cx + r_inner * angle.cos(), cy + r_inner * angle.sin());
    }
    pb.close();

    if let Some(path) = pb.finish() {
        let paint = rgba_to_paint(color);
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::EvenOdd,
            Transform::identity(),
            None,
        );
    }
}

/// Draw a pill outline (stadium ring shape).
pub fn draw_pill_ring(
    pixmap: &mut PixmapMut,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    thickness: f32,
    color: RGBA,
) {
    use std::f32::consts::{FRAC_PI_2, PI};
    const SEGMENTS: u32 = 12;

    let r = h / 2.0;
    let ri = r - thickness;

    let mut pb = PathBuilder::new();

    // Outer pill
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    for i in 1..=SEGMENTS {
        let angle = -FRAC_PI_2 + PI * i as f32 / SEGMENTS as f32;
        pb.line_to(x + w - r + r * angle.cos(), y + r + r * angle.sin());
    }
    pb.line_to(x + r, y + h);
    for i in 1..=SEGMENTS {
        let angle = FRAC_PI_2 + PI * i as f32 / SEGMENTS as f32;
        pb.line_to(x + r + r * angle.cos(), y + r + r * angle.sin());
    }
    pb.close();

    // Inner pill (creates the hole)
    let ix = x + thickness;
    let iy = y + thickness;
    let iw = w - 2.0 * thickness;
    let ih = h - 2.0 * thickness;
    let ir = ih / 2.0;

    if ir > 0.0 && iw > 0.0 {
        pb.move_to(ix + ir, iy);
        for i in (0..SEGMENTS).rev() {
            let angle = -FRAC_PI_2 + PI * i as f32 / SEGMENTS as f32;
            pb.line_to(ix + ir + ri * angle.cos(), iy + ir + ri * angle.sin());
        }
        pb.line_to(ix + iw - ir, iy + ih);
        for i in (0..SEGMENTS).rev() {
            let angle = FRAC_PI_2 + PI * i as f32 / SEGMENTS as f32;
            pb.line_to(
                ix + iw - ir + ri * angle.cos(),
                iy + ir + ri * angle.sin(),
            );
        }
        pb.line_to(ix + ir, iy);
        pb.close();
    }

    if let Some(path) = pb.finish() {
        let paint = rgba_to_paint(color);
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::EvenOdd,
            Transform::identity(),
            None,
        );
    }
}

/// Draw a horizontal gradient rectangle from `left_color` to `right_color`.
#[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
pub fn fill_gradient_rect(
    pixmap: &mut PixmapMut,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    left_color: RGBA,
    right_color: RGBA,
) {
    let pw = pixmap.width() as i32;
    let ph = pixmap.height() as i32;
    let pixels = pixmap.data_mut();

    for col in 0..w {
        let t = if w <= 1 {
            0.0
        } else {
            col as f32 / (w - 1) as f32
        };
        let inv = 1.0 - t;
        let r = (f32::from(left_color.0) * inv + f32::from(right_color.0) * t) as u8;
        let g = (f32::from(left_color.1) * inv + f32::from(right_color.1) * t) as u8;
        let b = (f32::from(left_color.2) * inv + f32::from(right_color.2) * t) as u8;
        let a = (f32::from(left_color.3) * inv + f32::from(right_color.3) * t) as u8;

        let px = x as i32 + col as i32;
        if px < 0 || px >= pw {
            continue;
        }
        for row in 0..h {
            let py = y as i32 + row as i32;
            if py < 0 || py >= ph {
                continue;
            }
            let offset = (py * pw + px) as usize * 4;
            blend_pixel(pixels, offset, r, g, b, a);
        }
    }
}

/// Alpha-composite a single pixel.
/// Buffer is in BGRA byte order (wl_shm Argb8888 on little-endian).
fn blend_pixel(pixels: &mut [u8], offset: usize, r: u8, g: u8, b: u8, a: u8) {
    let alpha = a as f32 / 255.0;
    let inv = 1.0 - alpha;

    // BGRA byte order: [B, G, R, A]
    let db = pixels[offset];
    let dg = pixels[offset + 1];
    let dr = pixels[offset + 2];
    let da = pixels[offset + 3];

    pixels[offset] = (b as f32 * alpha + db as f32 * inv) as u8;
    pixels[offset + 1] = (g as f32 * alpha + dg as f32 * inv) as u8;
    pixels[offset + 2] = (r as f32 * alpha + dr as f32 * inv) as u8;
    pixels[offset + 3] = (a as f32 + da as f32 * inv) as u8;
}
