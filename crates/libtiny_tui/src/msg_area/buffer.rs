use std::mem;

use super::{Line, MsgArea, SegStyle};
use crate::line_split::LineType;

impl MsgArea {
    /// Used to force a line to be aligned.
    pub(crate) fn set_current_line_alignment(&mut self) {
        let msg_padding = self.layout.msg_padding();
        self.line_buf.set_type(LineType::AlignedMsg { msg_padding });
    }

    pub(crate) fn add_text(&mut self, str: &str, style: SegStyle) {
        self.line_buf.add_text(str, style);
    }

    pub(crate) fn flush_line(&mut self) -> usize {
        let line_height = self.line_buf.rendered_height(self.width);
        // Check if we're about to overflow
        let mut removed_line_height = 0;
        if self.lines.len() == self.scrollback {
            // Remove oldest line
            if let Some(mut removed) = self.lines.pop_front() {
                removed_line_height = removed.rendered_height(self.width);
            }
        }
        self.lines
            .push_back(mem::replace(&mut self.line_buf, Line::new()));
        if self.scroll != 0 {
            self.scroll += line_height;
        }
        if let Some(ref mut total_height) = self.lines_height {
            *total_height += line_height - removed_line_height;
        }
        self.lines.len() - 1
    }

    pub(crate) fn clear(&mut self) {
        self.lines.clear();
        self.scroll = 0;
        self.lines_height = Some(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg_area::Layout;

    #[test]
    fn newline_scrolling() {
        let mut msg_area = MsgArea::new(100, 1, usize::MAX, Layout::Compact);
        // Adding a new line when scroll is 0 should not change it
        assert_eq!(msg_area.scroll, 0);
        msg_area.add_text("line1", SegStyle::UserMsg);
        msg_area.flush_line();
        assert_eq!(msg_area.scroll, 0);

        msg_area.add_text("line2", SegStyle::UserMsg);
        msg_area.flush_line();
        assert_eq!(msg_area.scroll, 0);

        msg_area.scroll_up();
        assert_eq!(msg_area.scroll, 1);
        msg_area.add_text("line3", SegStyle::UserMsg);
        msg_area.flush_line();
        assert_eq!(msg_area.scroll, 2);
    }

    #[test]
    fn test_max_lines() {
        // Can't show more than 3 lines.
        let mut msg_area = MsgArea::new(100, 1, 3, Layout::Compact);
        msg_area.add_text("first", SegStyle::UserMsg);
        msg_area.flush_line();
        msg_area.add_text("second", SegStyle::UserMsg);
        msg_area.flush_line();
        msg_area.add_text("third", SegStyle::UserMsg);
        msg_area.flush_line();
        assert_eq!(msg_area.lines.len(), 3);
        msg_area.add_text("fourth", SegStyle::UserMsg);
        // Will pop out "first" line
        msg_area.flush_line();
        assert_eq!(msg_area.lines.len(), 3);
        assert_eq!(msg_area.update_total_visible_lines(), 3);
    }
}
