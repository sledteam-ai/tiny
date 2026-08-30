use std::cmp::max;

use super::{Line, MsgArea};

impl MsgArea {
    pub(crate) fn modify_line<F>(&mut self, idx: usize, f: F)
    where
        F: Fn(&mut Line),
    {
        let old_height = self.lines[idx].rendered_height(self.width);
        f(&mut self.lines[idx]);
        let new_height = self.lines[idx].rendered_height(self.width);

        if let Some(ref mut total_height) = self.lines_height {
            *total_height += new_height - old_height;
        }

        // All current callers extend the newest activity line. Preserve an
        // anchored scrollback viewport only when that line is already entirely
        // below it. If the viewport still intersects the growing line, keep
        // following live output instead of moving upward once per new wrap.
        if self.scroll != 0 && idx + 1 == self.lines.len() {
            if self.scroll < old_height {
                self.scroll = 0;
            } else {
                self.scroll = max(0, self.scroll + new_height - old_height);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Colors;
    use crate::msg_area::{Layout, SegStyle};
    use termbox_simple::Termbox;

    fn add_line(msg_area: &mut MsgArea, text: &str) {
        msg_area.add_text(text, SegStyle::UserMsg);
        msg_area.flush_line();
    }

    fn rendered_rows(msg_area: &mut MsgArea) -> Vec<String> {
        let width = msg_area.width as u16;
        let height = msg_area.height as u16;
        let mut tb = Termbox::init_test(width, height);
        msg_area.draw(&mut tb, &Colors::default(), 0, 0);
        tb.present();
        let buf = tb.get_front_buffer();

        buf.cells
            .chunks(usize::from(width))
            .map(|row| row.iter().map(|cell| cell.ch).collect::<String>())
            .collect()
    }

    #[test]
    fn modifying_a_line_updates_its_wrapped_scrollback_range() {
        let mut msg_area = MsgArea::new(5, 3, usize::MAX, Layout::Compact);
        add_line(&mut msg_area, "older");
        add_line(&mut msg_area, "ABCDE");

        // Prime the aggregate height cache while the recent line occupies one row.
        assert_eq!(msg_area.update_total_visible_lines(), 2);
        msg_area.modify_line(1, |line| {
            line.add_text("FGHIJKLMNOPQRST", SegStyle::UserMsg);
        });

        assert_eq!(msg_area.update_total_visible_lines(), 5);
        msg_area.scroll_top();
        assert_eq!(msg_area.scroll_offset(), 2);
        assert_eq!(rendered_rows(&mut msg_area), ["older", "ABCDE", "FGHIJ"]);
    }

    #[test]
    fn growing_newest_line_preserves_anchored_scrollback() {
        let mut msg_area = MsgArea::new(4, 3, usize::MAX, Layout::Compact);
        for old_line in 0..6 {
            add_line(&mut msg_area, &format!("o{old_line:03}"));
        }
        add_line(&mut msg_area, "r000r001r002");
        let recent_idx = msg_area.num_lines() - 1;

        // The newest line occupies three rendered rows and is entirely below
        // this viewport, so growing it should retain the same older content.
        for _ in 0..3 {
            msg_area.scroll_up();
        }
        assert_eq!(msg_area.scroll_offset(), 3);

        msg_area.modify_line(recent_idx, |line| {
            line.add_text("r003r004", SegStyle::UserMsg);
        });

        assert_eq!(msg_area.update_total_visible_lines(), 11);
        assert_eq!(msg_area.scroll_offset(), 5);
    }

    #[test]
    fn incrementally_growing_recent_line_does_not_skip_rows() {
        let mut msg_area = MsgArea::new(4, 3, usize::MAX, Layout::Compact);
        for old_line in 0..6 {
            add_line(&mut msg_area, &format!("o{old_line:03}"));
        }

        add_line(&mut msg_area, "r000r001r002");
        let recent_idx = msg_area.num_lines() - 1;

        // Model a viewport near live output: it is scrolled slightly, but still
        // intersects the newest logical line while incremental chunks arrive.
        msg_area.scroll_up();
        assert_eq!(msg_area.scroll_offset(), 1);

        for recent_row in 3..10 {
            msg_area.modify_line(recent_idx, |line| {
                line.add_text(&format!("r{recent_row:03}"), SegStyle::UserMsg);
            });
        }

        assert_eq!(msg_area.update_total_visible_lines(), 16);
        assert_eq!(msg_area.scroll_offset(), 0);

        let expected_recent = (0..10).map(|row| format!("r{row:03}")).collect::<Vec<_>>();
        let mut reached_recent = Vec::new();

        loop {
            let rows = rendered_rows(&mut msg_area);
            if rows.iter().any(|row| row.starts_with('o')) {
                break;
            }
            for row in rows {
                if row.starts_with('r') && !reached_recent.contains(&row) {
                    reached_recent.push(row);
                }
            }
            msg_area.scroll_up();
        }

        reached_recent.sort();
        assert_eq!(reached_recent, expected_recent);
        msg_area.scroll_top();
        assert_eq!(msg_area.scroll_offset(), 13);
    }
}
