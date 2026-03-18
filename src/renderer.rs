use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache};
use tiny_skia::{BlendMode, FillRule, Paint, PathBuilder, PixmapMut, Rect, Transform};

use crate::types::{Poly, Polys, PowerlineDirection, PowerlineFill, PowerlineStyle, RGBA};

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

    pub fn draw_text(
        &mut self,
        pixmap: &mut PixmapMut,
        text: &str,
        fg: RGBA,
        x: u32,
        canvas_height: u32,
    ) {
        let buffer = self.shape_text(text);

        let text_height = self.ascent + self.descent;
        assert!(
            canvas_height >= text_height,
            "canvas_height({canvas_height}) < text_height({text_height})"
        );
        let baseline_offset = (canvas_height - text_height) / 2 + self.ascent;

        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((x as f32, 0.0), 1.0);

                if let Some(image) = self
                    .swash_cache
                    .get_image(&mut self.font_system, physical.cache_key)
                {
                    let gx = physical.x + image.placement.left;
                    let gy = baseline_offset as i32 - image.placement.top;

                    for iy in 0..image.placement.height as i32 {
                        for ix in 0..image.placement.width as i32 {
                            let px = gx + ix;
                            let py = gy + iy;

                            if px < 0
                                || py < 0
                                || px >= pixmap.width() as i32
                                || py >= pixmap.height() as i32
                            {
                                continue;
                            }

                            let idx = (iy * image.placement.width as i32 + ix) as usize;
                            let alpha = match image.content {
                                cosmic_text::SwashContent::Mask => {
                                    image.data.get(idx).copied().unwrap_or(0)
                                }
                                cosmic_text::SwashContent::Color => {
                                    // RGBA data, take the alpha channel.
                                    image.data.get(idx * 4 + 3).copied().unwrap_or(0)
                                }
                                cosmic_text::SwashContent::SubpixelMask => {
                                    // Use the green channel as alpha approximation.
                                    image.data.get(idx * 3 + 1).copied().unwrap_or(0)
                                }
                            };

                            if alpha == 0 {
                                continue;
                            }

                            let pixel_offset =
                                (py as u32 * pixmap.width() + px as u32) as usize * 4;
                            let pixels = pixmap.data_mut();

                            if image.content == cosmic_text::SwashContent::Color {
                                // Use the glyph's own color (e.g., color emoji).
                                let base = idx * 4;
                                let sr = image.data[base];
                                let sg = image.data[base + 1];
                                let sb = image.data[base + 2];
                                blend_pixel(pixels, pixel_offset, sr, sg, sb, alpha);
                            } else {
                                blend_pixel(pixels, pixel_offset, fg.0, fg.1, fg.2, alpha);
                            }
                        }
                    }
                }
            }
        }
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
    let w = ((height + 1) / 2) as f32;

    let xl = xl as f32;
    let xr = xl + w;
    let yt = 0.0f32;
    let yb = h;

    match (direction, fill) {
        (PowerlineDirection::Left, PowerlineFill::Full) => {
            vec![vec![
                (xr, yb),
                (xl, yt + h / 2.0),
                (xr, yt),
            ]]
        }
        (PowerlineDirection::Right, PowerlineFill::Full) => {
            vec![vec![
                (xl, yb),
                (xr, yt + h / 2.0),
                (xl, yt),
            ]]
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
            vec![vec![
                (xl, yb),
                (xr, yb - h_4),
                (xr, yt + h_4),
                (xl, yt),
            ]]
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
            vec![vec![
                (xl, yt + h_4),
                (xl, yb - h_4),
                (xr, yb),
                (xr, yt),
            ]]
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
    let w = ((height + 1) / 2) as f32;

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
    }
}

pub fn powerline_width(height: u32, style: PowerlineStyle) -> u32 {
    match style {
        PowerlineStyle::Powerline => (height + 1) / 2,
        PowerlineStyle::Octagon => (height + 1) / 4,
        PowerlineStyle::Circle => (height + 1) / 2,
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
