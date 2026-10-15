//! Status icons: a round badge in the status color with the symbol cut
//! out (an arrow, a clock, bars, a cross, a check, "!"), drawn as
//! anti-aliased images at the cell size: the grid font has no symbols that
//! look alike everywhere, and emoji break the grid.
//!
//! An icon's image id holds the icon, its size and its color, so whoever
//! renders a frame can make the images it hasn't got yet
//! ([`upload_missing`]); a theme change simply asks for other ids.

use std::collections::HashSet;

use orchard_grid::{Frame, ImageId, Rgba};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// Downloading.
    Running,
    /// Waiting in the queue.
    Queued,
    Paused,
    Done,
    Failed,
    /// Done, but the checksum doesn't match.
    Damaged,
}

impl Icon {
    const ALL: [Icon; 6] = [Icon::Running, Icon::Queued, Icon::Paused, Icon::Done, Icon::Failed, Icon::Damaged];
}

/// Marks icon ids among the app's external image ids.
const TAG: u64 = 0x1C << 56;

/// The image id of `icon`, `size` pixels square, in `color`.
pub fn image_id(icon: Icon, size: u32, color: Rgba) -> ImageId {
    let i = Icon::ALL.iter().position(|&x| x == icon).unwrap_or(0) as u64;
    let rgba = (color.r as u64) << 24 | (color.g as u64) << 16 | (color.b as u64) << 8 | color.a as u64;
    ImageId::external(TAG | i << 48 | (size.min(0xffff) as u64) << 32 | rgba)
}

/// The icon, size and color an id stands for (`None`: not an icon).
pub fn decode(id: ImageId) -> Option<(Icon, u32, Rgba)> {
    let k = id.external_key()?;
    if k & (0xff << 56) != TAG {
        return None;
    }
    let icon = *Icon::ALL.get((k >> 48 & 0xff) as usize)?;
    let size = (k >> 32 & 0xffff) as u32;
    let c = |s: u32| (k >> s & 0xff) as u8;
    Some((icon, size, Rgba::new(c(24), c(16), c(8), c(0))))
}

/// Make the icon images the frame shows that `have` hasn't got (`set`
/// stores one: id, width, height, RGBA pixels).
pub fn upload_missing(frame: &Frame, have: &mut HashSet<ImageId>, mut set: impl FnMut(ImageId, u32, u32, &[u8])) {
    for p in &frame.images {
        if have.contains(&p.image) {
            continue;
        }
        if let Some((icon, size, color)) = decode(p.image) {
            set(p.image, size, size, &rasterize(icon, size, color));
            have.insert(p.image);
        }
    }
}

/// The icon as RGBA pixels, `size` × `size`.
pub fn rasterize(icon: Icon, size: u32, color: Rgba) -> Vec<u8> {
    const SS: u32 = 4;
    let n = size.max(1);
    let mut out = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let mut hits = 0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let u = (x as f32 + (sx as f32 + 0.5) / SS as f32) / n as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / SS as f32) / n as f32;
                    if in_badge(u, v) && !in_symbol(icon, u, v) {
                        hits += 1;
                    }
                }
            }
            let a = (color.a as u32 * hits / (SS * SS)) as u8;
            out.extend_from_slice(&[color.r, color.g, color.b, a]);
        }
    }
    out
}

fn in_badge(u: f32, v: f32) -> bool {
    let (dx, dy) = (u - 0.5, v - 0.5);
    dx * dx + dy * dy <= 0.48 * 0.48
}

/// Whether (u, v) is on a segment from a to b, `r` thick on each side.
fn capsule(u: f32, v: f32, a: (f32, f32), b: (f32, f32), r: f32) -> bool {
    let (px, py) = (u - a.0, v - a.1);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = ((px * dx + py * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    let (ex, ey) = (px - dx * t, py - dy * t);
    ex * ex + ey * ey <= r * r
}

fn rect(u: f32, v: f32, x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
    (x0..=x1).contains(&u) && (y0..=y1).contains(&v)
}

/// Inside the triangle a, b, c.
fn triangle(u: f32, v: f32, a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let side = |p: (f32, f32), q: (f32, f32)| (q.0 - p.0) * (v - p.1) - (q.1 - p.1) * (u - p.0);
    let (d1, d2, d3) = (side(a, b), side(b, c), side(c, a));
    !((d1 < 0.0 || d2 < 0.0 || d3 < 0.0) && (d1 > 0.0 || d2 > 0.0 || d3 > 0.0))
}

fn in_symbol(icon: Icon, u: f32, v: f32) -> bool {
    match icon {
        Icon::Running => rect(u, v, 0.425, 0.2, 0.575, 0.52) || triangle(u, v, (0.25, 0.48), (0.75, 0.48), (0.5, 0.8)),
        Icon::Queued => capsule(u, v, (0.5, 0.5), (0.5, 0.24), 0.06) || capsule(u, v, (0.5, 0.5), (0.7, 0.62), 0.06),
        Icon::Paused => rect(u, v, 0.32, 0.27, 0.445, 0.73) || rect(u, v, 0.555, 0.27, 0.68, 0.73),
        Icon::Done => capsule(u, v, (0.27, 0.52), (0.43, 0.68), 0.07) || capsule(u, v, (0.43, 0.68), (0.73, 0.34), 0.07),
        Icon::Failed => capsule(u, v, (0.33, 0.33), (0.67, 0.67), 0.07) || capsule(u, v, (0.67, 0.33), (0.33, 0.67), 0.07),
        Icon::Damaged => {
            capsule(u, v, (0.5, 0.23), (0.5, 0.57), 0.07) || {
                let (dx, dy) = (u - 0.5, v - 0.75);
                dx * dx + dy * dy <= 0.075 * 0.075
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        let c = Rgba::new(166, 226, 46, 255);
        for icon in Icon::ALL {
            assert_eq!(decode(image_id(icon, 28, c)), Some((icon, 28, c)));
        }
        assert_eq!(decode(ImageId::external(7)), None);
    }

    #[test]
    fn a_badge_with_the_symbol_cut_out() {
        let n = 32;
        let px = rasterize(Icon::Paused, n, Rgba::new(255, 0, 0, 255));
        let alpha = |x: u32, y: u32| px[((y * n + x) * 4 + 3) as usize];
        assert_eq!(alpha(0, 0), 0, "outside the circle");
        assert_eq!(alpha(16, 4), 255, "the badge");
        assert_eq!(alpha(12, 16), 0, "the left bar is cut out");
        assert_eq!(alpha(16, 16), 255, "between the bars");
    }
}
