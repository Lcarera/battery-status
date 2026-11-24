//! Tray icon pixels: device symbol (left) + vertical battery bar (right). No Windows code.

use crate::config::BarColors;
use crate::device::DeviceKind;

pub const GREEN: u32 = 0xFF2E_B82E;
pub const YELLOW: u32 = 0xFFE6_B800;
pub const RED: u32 = 0xFFE5_3935;
const WHITE: u32 = 0xFFFF_FFFF;
const CLEAR: u32 = 0x0000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarColor {
    Green,
    Yellow,
    Red,
}

impl BarColor {
    fn argb(self) -> u32 {
        match self {
            BarColor::Green => GREEN,
            BarColor::Yellow => YELLOW,
            BarColor::Red => RED,
        }
    }
}

pub fn bar_color(percent: u8, colors: &BarColors) -> BarColor {
    if percent > colors.green_above {
        BarColor::Green
    } else if percent > colors.red_at_or_below {
        BarColor::Yellow
    } else {
        BarColor::Red
    }
}

// Symbols on a 16-row x 10-column grid ('#' = white pixel).
const HEADPHONES: [&str; 16] = [
    "..........", "..........", "...####...", "..#....#..", ".#......#.", ".#......#.",
    ".#......#.", ".#......#.", "###....###", "###....###", "###....###", "###....###",
    "###....###", "..........", "..........", "..........",
];
const PHONE: [&str; 16] = [
    "..........", "..######..", "..#....#..", "..#....#..", "..#....#..", "..#....#..",
    "..#....#..", "..#....#..", "..#....#..", "..#....#..", "..#....#..", "..#....#..",
    "..#....#..", "..#.##.#..", "..######..", "..........",
];
const GAMEPAD: [&str; 16] = [
    "..........", "..........", "..........", "..........", "..........", ".########.",
    "##########", "#.######.#", "##########", "####..####", "###....###", ".#......#.",
    "..........", "..........", "..........", "..........",
];
const GENERIC: [&str; 16] = [
    "..........", "..........", "..........", "..........", "..######..", "..#....#..",
    "..#....#..", "..#....#..", "..#....#..", "..#....#..", "..#....#..", "..######..",
    "..........", "..........", "..........", "..........",
];

// Bar on the 16x16 grid: outline columns 11 and 15, rows 1 and 15; fill columns 12..=14, rows 2..=14.
const BAR_ROWS: u32 = 13;

fn symbol(kind: DeviceKind) -> &'static [&'static str; 16] {
    match kind {
        DeviceKind::Headphones => &HEADPHONES,
        DeviceKind::Phone => &PHONE,
        DeviceKind::Gamepad => &GAMEPAD,
        DeviceKind::Generic => &GENERIC,
    }
}

fn filled_rows(percent: u8) -> u32 {
    let p = percent.min(100) as u32;
    if p == 0 { 0 } else { ((p * BAR_ROWS + 50) / 100).max(1) }
}

fn grid_pixel(kind: DeviceKind, filled: u32, fill: u32, gx: u32, gy: u32) -> u32 {
    if gx < 10 {
        return if symbol(kind)[gy as usize].as_bytes()[gx as usize] == b'#' { WHITE } else { CLEAR };
    }
    let in_bar_rows = (1..=15).contains(&gy);
    let outline = in_bar_rows && (gx == 11 || gx == 15 || gy == 1 || gy == 15) && gx >= 11;
    if outline {
        return WHITE;
    }
    if (12..=14).contains(&gx) && (2..=14).contains(&gy) && gy > 14 - filled {
        return fill;
    }
    CLEAR
}

/// ARGB pixels, `size * size`, row-major.
pub fn render(kind: DeviceKind, percent: u8, color: BarColor, size: u32, dimmed: bool) -> Vec<u32> {
    let filled = if dimmed { 0 } else { filled_rows(percent) };
    let fill = color.argb();
    let mut px = Vec::with_capacity((size * size) as usize);
    for y in 0..size {
        for x in 0..size {
            px.push(grid_pixel(kind, filled, fill, x * 16 / size, y * 16 / size));
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(px: &[u32], c: u32) -> usize {
        px.iter().filter(|&&p| p == c).count()
    }

    #[test]
    fn color_limits() {
        let c = BarColors::default();
        assert_eq!(bar_color(51, &c), BarColor::Green);
        assert_eq!(bar_color(50, &c), BarColor::Yellow);
        assert_eq!(bar_color(21, &c), BarColor::Yellow);
        assert_eq!(bar_color(20, &c), BarColor::Red);
        assert_eq!(bar_color(0, &c), BarColor::Red);
    }

    #[test]
    fn render_size() {
        assert_eq!(render(DeviceKind::Phone, 50, BarColor::Green, 16, false).len(), 256);
        assert_eq!(render(DeviceKind::Phone, 50, BarColor::Green, 24, false).len(), 576);
    }

    #[test]
    fn bar_height_follows_percent() {
        let g = |p| count(&render(DeviceKind::Headphones, p, BarColor::Green, 16, false), GREEN);
        assert!(g(100) > g(50));
        assert_eq!(g(0), 0);
    }

    #[test]
    fn bar_uses_color() {
        let px = render(DeviceKind::Gamepad, 15, BarColor::Red, 16, false);
        assert!(count(&px, RED) > 0);
        assert_eq!(count(&px, GREEN), 0);
    }

    #[test]
    fn dimmed_has_no_bar_pixels() {
        let px = render(DeviceKind::Gamepad, 15, BarColor::Red, 16, true);
        assert_eq!(count(&px, RED), 0);
    }

    #[test]
    fn kinds_render_differently() {
        assert_ne!(
            render(DeviceKind::Headphones, 50, BarColor::Yellow, 16, false),
            render(DeviceKind::Phone, 50, BarColor::Yellow, 16, false)
        );
    }
}
