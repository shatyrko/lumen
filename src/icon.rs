//! The tray icon, drawn by code (a light bulb) so that it needs no icon theme. When any light
//! is on, the bulb is filled.

fn sd_capsule(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32, r: f32) -> f32 {
    let (pax, pay) = (px - ax, py - ay);
    let (bax, bay) = (bx - ax, by - ay);
    let h = ((pax * bax + pay * bay) / (bax * bax + bay * bay).max(1e-9)).clamp(0.0, 1.0);
    let (dx, dy) = (pax - bax * h, pay - bay * h);
    (dx * dx + dy * dy).sqrt() - r
}

/// Distance (negative inside) to the bulb glyph in the unit square.
fn glyph(x: f32, y: f32, filled: bool) -> f32 {
    let (cx, cy, r) = (0.5, 0.38, 0.27);
    let globe = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r;
    // the glass narrows into the neck
    let neck = {
        let t = ((y - 0.55) / 0.15).clamp(0.0, 1.0);
        let half = 0.20 - 0.07 * t;
        if (0.55..=0.70).contains(&y) { (x - 0.5).abs() - half } else { 1.0 }
    };
    let body = globe.min(neck);
    let stroke = 0.045;
    let outline = if filled { body } else { body.abs() - stroke };
    let base1 = sd_capsule(x, y, 0.39, 0.78, 0.61, 0.78, stroke);
    let base2 = sd_capsule(x, y, 0.42, 0.88, 0.58, 0.88, stroke);
    outline.min(base1).min(base2)
}

/// ARGB32 pixels in network byte order, as StatusNotifierItem wants them.
pub fn argb(size: u32, rgb: (u8, u8, u8), filled: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let ss = 4;
    for y in 0..size {
        for x in 0..size {
            let mut cover = 0.0;
            for sy in 0..ss {
                for sx in 0..ss {
                    let fx = (x as f32 + (sx as f32 + 0.5) / ss as f32) / size as f32;
                    let fy = (y as f32 + (sy as f32 + 0.5) / ss as f32) / size as f32;
                    if glyph(fx, fy, filled) < 0.0 {
                        cover += 1.0;
                    }
                }
            }
            let a = (cover / (ss * ss) as f32 * 255.0).round() as u8;
            out.extend_from_slice(&[a, rgb.0, rgb.1, rgb.2]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filled_has_more_ink() {
        let ink = |f| argb(32, (255, 255, 255), f).chunks(4).filter(|c| c[0] > 200).count();
        assert!(ink(false) > 60);
        assert!(ink(true) > ink(false));
        assert_eq!(argb(32, (1, 2, 3), true)[0], 0); // corner is empty
    }
}
