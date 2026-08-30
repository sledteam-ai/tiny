//! Draws Tiny's read-only navigation reference over the completed TUI frame.
//!
//! `TUI` owns visibility and suppresses input before routing; this module only paints a compact,
//! centered dialog and clips it to terminals smaller than the preferred dimensions.

use termbox_simple::{TB_BOLD, Termbox};

use crate::config::Colors;

const LINES: &[&str] = &[
    "Navigation",
    "=============",
    "",
    "F2              close this help",
    "Tab             open/send prompt composer",
    "Alt+Left        previous tab",
    "Alt+Right       next tab",
    "",
    "Alt+Up          scroll chat up",
    "Alt+Down        scroll chat down",
    "",
    "Composer",
    "--------",
    "Enter           newline",
    "Tab             send to chat",
    "Esc             cancel prompt",
    "Ctrl+a          go to start of text",
    "Ctrl+k          delete text from cursor onwards",
    "",
    "Command Line",
    "------------",
    "Enter           send / run command",
    "/               access Sledteam commands",
];

pub(crate) fn draw(tb: &mut Termbox, colors: &Colors, screen_width: i32, screen_height: i32) {
    if screen_width <= 0 || screen_height <= 0 {
        return;
    }

    tb.hide_cursor();
    let content_width = LINES.iter().map(|line| line.len()).max().unwrap_or(0) as i32;
    let width = (content_width + 4).min(screen_width);
    let height = (LINES.len() as i32 + 2).min(screen_height);
    let pos_x = (screen_width - width) / 2;
    let pos_y = (screen_height - height) / 2;
    let style = colors.exit_dialogue;

    for y in 0..height {
        for x in 0..width {
            let ch = if x == 0 && y == 0 {
                '┌'
            } else if x == width - 1 && y == 0 {
                '┐'
            } else if x == 0 && y == height - 1 {
                '└'
            } else if x == width - 1 && y == height - 1 {
                '┘'
            } else if y == 0 || y == height - 1 {
                '─'
            } else if x == 0 || x == width - 1 {
                '│'
            } else {
                ' '
            };
            tb.change_cell(pos_x + x, pos_y + y, ch, style.fg, style.bg);
        }
    }

    let text_width = (width - 4).max(0) as usize;
    for (row, line) in LINES.iter().take((height - 2).max(0) as usize).enumerate() {
        let fg = if row == 0 {
            style.fg | TB_BOLD
        } else {
            style.fg
        };
        for (col, ch) in line.chars().take(text_width).enumerate() {
            tb.change_cell(
                pos_x + 2 + col as i32,
                pos_y + 1 + row as i32,
                ch,
                fg,
                style.bg,
            );
        }
    }
}
