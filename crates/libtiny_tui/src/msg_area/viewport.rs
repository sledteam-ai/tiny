use std::cmp::max;

use termbox_simple::Termbox;

use super::MsgArea;
use crate::config::Colors;

const SCROLL_CHUNK_LINES: i32 = 5;

impl MsgArea {
    pub(crate) fn resize(&mut self, width: i32, height: i32) {
        self.width = width;
        let old_height = self.height;
        self.height = height;
        let old_total_lines = self.lines_height;
        self.lines_height = None;

        self.update_total_visible_lines();
        self.recalculate_scroll(old_height, old_total_lines.unwrap());
    }

    pub(crate) fn draw(&mut self, tb: &mut Termbox, colors: &Colors, pos_x: i32, pos_y: i32) {
        // Where to render current line
        let mut row = pos_y + self.height - 1;

        // How many visible lines to skip
        let mut skip = self.scroll;

        // Draw lines in reverse order
        let mut line_idx = (self.lines.len() as i32) - 1;
        while line_idx >= 0 && row >= pos_y {
            let line = &mut self.lines[line_idx as usize];
            let line_height = line.rendered_height(self.width);
            debug_assert!(line_height > 0);

            if skip >= line_height {
                // skip the whole line
                line_idx -= 1;
                skip -= line_height;
                continue;
            }

            // Rendered line height
            let height = line_height - skip;

            // Where to start rendering this line?
            let line_row = row - height + 1;

            // How many lines to skip in the `Line` before rendering
            let render_from = max(0, pos_y - line_row);

            line.draw(tb, colors, pos_x, line_row, render_from, height);
            row = line_row - 1;
            line_idx -= 1;
            skip = 0;

            if line_row < pos_y {
                break;
            }
        }
    }

    /// The total number of visible lines if each Line was rendered at the current screen width
    pub(super) fn update_total_visible_lines(&mut self) -> i32 {
        match self.lines_height {
            Some(height) => height,
            None => {
                let mut total_height = 0;
                for line in &mut self.lines {
                    total_height += line.rendered_height(self.width);
                }
                self.lines_height = Some(total_height);
                total_height
            }
        }
    }

    pub(crate) fn scroll_up(&mut self) {
        if self.scroll < max(0, self.update_total_visible_lines() - self.height) {
            self.scroll += 1;
        }
    }

    pub(crate) fn scroll_down(&mut self) {
        if self.scroll > 0 {
            self.scroll -= 1;
        }
    }

    pub(crate) fn scroll_chunk_up(&mut self) {
        let max_scroll = max(0, self.update_total_visible_lines() - self.height);
        self.scroll = (self.scroll + SCROLL_CHUNK_LINES).min(max_scroll);
    }

    pub(crate) fn scroll_chunk_down(&mut self) {
        self.scroll = max(0, self.scroll - SCROLL_CHUNK_LINES);
    }

    pub(crate) fn is_scrolled(&self) -> bool {
        self.scroll > 0
    }

    #[cfg(test)]
    pub(crate) fn scroll_offset(&self) -> i32 {
        self.scroll
    }

    pub(crate) fn scroll_top(&mut self) {
        self.scroll = max(0, self.update_total_visible_lines() - self.height);
    }

    pub(crate) fn scroll_bottom(&mut self) {
        self.scroll = 0;
    }

    pub(crate) fn page_up(&mut self) {
        for _ in 0..10 {
            self.scroll_up();
        }
    }

    pub(crate) fn page_down(&mut self) {
        self.scroll = max(0, self.scroll - 10);
    }

    /// Recalculate the scroll offset due to resizing of the window
    fn recalculate_scroll(&mut self, old_height: i32, old_total_lines: i32) {
        if self.scroll > 0 {
            let ratio = (self.scroll as f32 + old_height as f32) / old_total_lines as f32;
            let total_lines = self.update_total_visible_lines();
            self.scroll = max(0, ((ratio * total_lines as f32) as i32) - self.height);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg_area::SegStyle;

    fn add_line(msg_area: &mut MsgArea, text: &str) {
        msg_area.add_text(text, SegStyle::UserMsg);
        msg_area.flush_line();
    }

    #[test]
    fn chunk_scrolling_preserves_position_and_returns_to_live_follow() {
        let mut msg_area = MsgArea::new(100, 3, usize::MAX, crate::msg_area::Layout::Compact);
        for line in 0..10 {
            msg_area.add_text(&format!("line{line}"), SegStyle::UserMsg);
            msg_area.flush_line();
        }

        msg_area.scroll_chunk_up();
        assert_eq!(msg_area.scroll, SCROLL_CHUNK_LINES);
        assert!(msg_area.is_scrolled());

        msg_area.add_text("new", SegStyle::UserMsg);
        msg_area.flush_line();
        assert_eq!(msg_area.scroll, SCROLL_CHUNK_LINES + 1);

        msg_area.scroll_chunk_down();
        assert_eq!(msg_area.scroll, 1);
        msg_area.scroll_chunk_down();
        assert_eq!(msg_area.scroll, 0);
        assert!(!msg_area.is_scrolled());

        msg_area.add_text("live", SegStyle::UserMsg);
        msg_area.flush_line();
        assert_eq!(msg_area.scroll, 0);
    }

    #[test]
    fn wrapped_recent_line_scrolls_by_rendered_rows() {
        let mut msg_area = MsgArea::new(5, 3, usize::MAX, crate::msg_area::Layout::Compact);
        add_line(&mut msg_area, "older");
        add_line(&mut msg_area, "ABCDEFGHIJKLMNOPQRST");

        assert_eq!(msg_area.num_lines(), 2);
        assert_eq!(msg_area.update_total_visible_lines(), 5);

        msg_area.scroll_up();
        assert_eq!(msg_area.scroll_offset(), 1);
        msg_area.scroll_up();
        assert_eq!(msg_area.scroll_offset(), 2);
        msg_area.scroll_up();
        assert_eq!(msg_area.scroll_offset(), 2);
    }

    #[test]
    fn resize_recalculates_wrapped_scrollback_range() {
        let mut msg_area = MsgArea::new(5, 3, usize::MAX, crate::msg_area::Layout::Compact);
        add_line(&mut msg_area, "older");
        add_line(&mut msg_area, "ABCDEFGHIJKLMNOPQRST");

        msg_area.scroll_top();
        assert_eq!(msg_area.scroll_offset(), 2);

        msg_area.resize(10, 3);
        assert_eq!(msg_area.update_total_visible_lines(), 3);
        assert_eq!(msg_area.scroll_offset(), 0);

        msg_area.resize(4, 3);
        assert_eq!(msg_area.update_total_visible_lines(), 7);
        msg_area.scroll_top();
        assert_eq!(msg_area.scroll_offset(), 4);
    }
}
