//! The tray icon is the chart: one small vertical bar per provider, filled to
//! how much of that provider's tightest quota is left.

pub const HEIGHT: u32 = 44; // 22pt at 2x, the macOS menubar size
const BAR: u32 = 8;
const GAP: u32 = 4;
const PAD: u32 = 3;
const MAX_BARS: usize = 6;

/// `levels`: remaining fraction per provider, `None` when unknown.
/// `template`: draw in black + alpha only, for macOS to tint to the menubar.
/// Returns RGBA pixels with their width and height.
pub fn render(levels: &[Option<f64>], template: bool) -> (Vec<u8>, u32, u32) {
    let placeholder = [None, None, None];
    let levels = if levels.is_empty() { &placeholder[..] } else { levels };
    let levels = &levels[..levels.len().min(MAX_BARS)];

    let n = levels.len() as u32;
    let width = PAD * 2 + n * BAR + (n - 1) * GAP;
    let mut rgba = vec![0u8; (width * HEIGHT * 4) as usize];
    let (top, bottom) = (PAD, HEIGHT - PAD);
    let span = (bottom - top) as f64;

    for (i, level) in levels.iter().enumerate() {
        let x0 = PAD + i as u32 * (BAR + GAP);
        // Never draw a known-but-tiny quota as nothing at all.
        let fill_top = match level {
            Some(l) => bottom - ((span * l.clamp(0.0, 1.0)).round() as u32).max(2),
            None => bottom,
        };
        let fill = match (template, level) {
            (true, _) => [0, 0, 0, 255],
            (false, Some(l)) if *l < 0.2 => [236, 104, 94, 255],
            (false, Some(l)) if *l < 0.5 => [238, 184, 72, 255],
            (false, _) => [84, 194, 138, 255],
        };
        let track = if template { [0, 0, 0, 70] } else { [140, 140, 140, 110] };

        for y in top..bottom {
            for x in x0..x0 + BAR {
                // Knock the four corner pixels off each bar to round it.
                let edge_x = x == x0 || x == x0 + BAR - 1;
                let edge_y = y == top || y == bottom - 1;
                if edge_x && edge_y {
                    continue;
                }
                let colour = if y >= fill_top { fill } else { track };
                let at = ((y * width + x) * 4) as usize;
                rgba[at..at + 4].copy_from_slice(&colour);
            }
        }
    }
    (rgba, width, HEIGHT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled_rows(rgba: &[u8], width: u32, x: u32) -> u32 {
        (0..HEIGHT)
            .filter(|y| rgba[((y * width + x) * 4 + 3) as usize] == 255)
            .count() as u32
    }

    #[test]
    fn bars_track_remaining_quota() {
        let (rgba, width, height) = render(&[Some(1.0), Some(0.5), Some(0.0), None], true);
        assert_eq!((width, height), (PAD * 2 + 4 * BAR + 3 * GAP, HEIGHT));
        assert_eq!(rgba.len() as u32, width * height * 4);
        let column = |i: u32| filled_rows(&rgba, width, PAD + i * (BAR + GAP) + BAR / 2);
        assert_eq!(column(0), HEIGHT - 2 * PAD); // full
        assert_eq!(column(1), (HEIGHT - 2 * PAD) / 2); // half
        assert_eq!(column(2), 2); // empty but known: a sliver
        assert_eq!(column(3), 0); // unknown: track only
    }

    #[test]
    fn no_providers_still_draws_an_icon() {
        let (rgba, width, _) = render(&[], false);
        assert!(width > 0 && rgba.iter().any(|&b| b != 0));
    }
}
