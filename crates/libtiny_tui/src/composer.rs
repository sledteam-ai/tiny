//! Implements the deliberately small multiline editing pane used by `MessagingUI`.
//!
//! Input is routed here only while a tab is composing; submission returns logical lines to the
//! existing multiline send flow. Cursor movement is character-based and selection-free.

use crate::config::Colors;
use crate::key_map::KeyAction;
use crate::widget::WidgetRet;
use termbox_simple::{TB_BOLD, Termbox};

const MAX_HEIGHT: i32 = 8;

pub(crate) struct Composer {
    buffer: Vec<char>,
    cursor: usize,
    preferred_column: Option<usize>,
    scroll: usize,
    original_input: String,
    original_cursor: i32,
}

impl Composer {
    pub(crate) fn new(original_input: String, original_cursor: i32, pasted: &str) -> Self {
        let mut buffer: Vec<char> = original_input.chars().collect();
        let cursor = (original_cursor as usize).min(buffer.len());
        let pasted: Vec<char> = normalized_chars(pasted).collect();
        let pasted_len = pasted.len();
        buffer.splice(cursor..cursor, pasted);
        let cursor = cursor + pasted_len;

        Composer {
            buffer,
            cursor,
            preferred_column: None,
            scroll: 0,
            original_input,
            original_cursor,
        }
    }

    pub(crate) fn height(parent_height: i32) -> i32 {
        MAX_HEIGHT
            .min((parent_height / 2).max(3))
            .min(parent_height.max(0))
    }

    pub(crate) fn cancel(self) -> (String, i32) {
        (self.original_input, self.original_cursor)
    }

    pub(crate) fn keypressed(&mut self, action: &KeyAction) -> WidgetRet {
        match action {
            KeyAction::Cancel => WidgetRet::Remove,
            KeyAction::ComposerSend => {
                if self.buffer.is_empty() {
                    WidgetRet::KeyHandled
                } else {
                    let text: String = self.buffer.iter().collect();
                    WidgetRet::Lines(text.split('\n').map(str::to_owned).collect())
                }
            }
            KeyAction::InputSend => {
                self.insert('\n');
                WidgetRet::KeyHandled
            }
            KeyAction::InputDeletePrevChar => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.buffer.remove(self.cursor);
                    self.preferred_column = None;
                }
                WidgetRet::KeyHandled
            }
            KeyAction::InputDeleteNextChar => {
                if self.cursor < self.buffer.len() {
                    self.buffer.remove(self.cursor);
                    self.preferred_column = None;
                }
                WidgetRet::KeyHandled
            }
            KeyAction::InputMoveCursLeft => {
                self.cursor = self.cursor.saturating_sub(1);
                self.preferred_column = None;
                WidgetRet::KeyHandled
            }
            KeyAction::InputMoveCursRight => {
                self.cursor = (self.cursor + 1).min(self.buffer.len());
                self.preferred_column = None;
                WidgetRet::KeyHandled
            }
            KeyAction::InputPrevEntry => {
                self.move_vertical(false);
                WidgetRet::KeyHandled
            }
            KeyAction::InputNextEntry => {
                self.move_vertical(true);
                WidgetRet::KeyHandled
            }
            KeyAction::InputMoveCursStart => {
                self.cursor = self.line_start(self.cursor);
                self.preferred_column = None;
                WidgetRet::KeyHandled
            }
            KeyAction::InputMoveCursEnd => {
                self.cursor = self.line_end(self.cursor);
                self.preferred_column = None;
                WidgetRet::KeyHandled
            }
            KeyAction::Input(ch) => {
                self.insert(*ch);
                WidgetRet::KeyHandled
            }
            KeyAction::OpenComposer | KeyAction::InputAutoComplete => WidgetRet::KeyHandled,
            KeyAction::TabMoveLeft
            | KeyAction::TabMoveRight
            | KeyAction::TabNext
            | KeyAction::TabPrev
            | KeyAction::TabGoto(_)
            | KeyAction::Command(_) => WidgetRet::KeyIgnored,
            _ => WidgetRet::KeyHandled,
        }
    }

    pub(crate) fn insert_text(&mut self, text: &str) {
        let chars: Vec<char> = normalized_chars(text).collect();
        let len = chars.len();
        self.buffer.splice(self.cursor..self.cursor, chars);
        self.cursor += len;
        self.preferred_column = None;
    }

    pub(crate) fn draw(
        &mut self,
        tb: &mut Termbox,
        colors: &Colors,
        pos_x: i32,
        pos_y: i32,
        width: i32,
        height: i32,
    ) {
        if width <= 0 || height <= 0 {
            return;
        }

        let border_fg = colors.user_msg.fg | TB_BOLD;
        let border_bg = colors.user_msg.bg;
        if width == 1 || height == 1 {
            for x in 0..width {
                tb.change_cell(pos_x + x, pos_y, '━', border_fg, border_bg);
            }
            return;
        }

        tb.change_cell(pos_x, pos_y, '┏', border_fg, border_bg);
        tb.change_cell(pos_x + width - 1, pos_y, '┓', border_fg, border_bg);
        tb.change_cell(pos_x, pos_y + height - 1, '┗', border_fg, border_bg);
        tb.change_cell(
            pos_x + width - 1,
            pos_y + height - 1,
            '┛',
            border_fg,
            border_bg,
        );
        for x in 1..width - 1 {
            tb.change_cell(pos_x + x, pos_y, '━', border_fg, border_bg);
            tb.change_cell(pos_x + x, pos_y + height - 1, '━', border_fg, border_bg);
        }
        for y in 1..height - 1 {
            tb.change_cell(pos_x, pos_y + y, '┃', border_fg, border_bg);
            tb.change_cell(pos_x + width - 1, pos_y + y, '┃', border_fg, border_bg);
        }

        if width <= 2 || height <= 2 {
            return;
        }

        let content_width = (width - 2) as usize;
        let content_height = (height - 2) as usize;
        let (cursor_row, cursor_col) = self.visual_position(self.cursor, content_width);
        if cursor_row < self.scroll {
            self.scroll = cursor_row;
        } else if cursor_row >= self.scroll + content_height {
            self.scroll = cursor_row + 1 - content_height;
        }

        let mut row = 0;
        let mut col = 0;
        for ch in &self.buffer {
            if *ch == '\n' {
                row += 1;
                col = 0;
                continue;
            }
            if row >= self.scroll && row < self.scroll + content_height {
                tb.change_cell(
                    pos_x + 1 + col as i32,
                    pos_y + 1 + (row - self.scroll) as i32,
                    *ch,
                    colors.user_msg.fg,
                    colors.user_msg.bg,
                );
            }
            col += 1;
            if col == content_width {
                row += 1;
                col = 0;
            }
        }

        tb.set_cursor(Some((
            (pos_x + 1 + cursor_col as i32) as u16,
            (pos_y + 1 + (cursor_row - self.scroll) as i32) as u16,
        )));
    }

    fn insert(&mut self, ch: char) {
        self.buffer.insert(self.cursor, ch);
        self.cursor += 1;
        self.preferred_column = None;
    }

    fn line_start(&self, cursor: usize) -> usize {
        self.buffer[..cursor]
            .iter()
            .rposition(|ch| *ch == '\n')
            .map_or(0, |idx| idx + 1)
    }

    fn line_end(&self, cursor: usize) -> usize {
        self.buffer[cursor..]
            .iter()
            .position(|ch| *ch == '\n')
            .map_or(self.buffer.len(), |idx| cursor + idx)
    }

    fn move_vertical(&mut self, down: bool) {
        let start = self.line_start(self.cursor);
        let column = *self.preferred_column.get_or_insert(self.cursor - start);
        if down {
            let end = self.line_end(self.cursor);
            if end < self.buffer.len() {
                let next_start = end + 1;
                self.cursor = (next_start + column).min(self.line_end(next_start));
            }
        } else if start > 0 {
            let previous_end = start - 1;
            let previous_start = self.line_start(previous_end);
            self.cursor = (previous_start + column).min(previous_end);
        }
    }

    fn visual_position(&self, index: usize, width: usize) -> (usize, usize) {
        let mut row = 0;
        let mut col = 0;
        for ch in &self.buffer[..index] {
            if *ch == '\n' {
                row += 1;
                col = 0;
            } else {
                col += 1;
                if col == width {
                    row += 1;
                    col = 0;
                }
            }
        }
        (row, col)
    }
}

fn normalized_chars(text: &str) -> impl Iterator<Item = char> + '_ {
    let mut previous_was_cr = false;
    text.chars().filter_map(move |ch| {
        if ch == '\n' && previous_was_cr {
            previous_was_cr = false;
            None
        } else if ch == '\r' {
            previous_was_cr = true;
            Some('\n')
        } else {
            previous_was_cr = false;
            Some(ch)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_inserts_newline_and_submit_returns_all_lines() {
        let mut composer = Composer::new(String::new(), 0, "one");
        assert!(matches!(
            composer.keypressed(&KeyAction::InputSend),
            WidgetRet::KeyHandled
        ));
        composer.keypressed(&KeyAction::Input('t'));
        composer.keypressed(&KeyAction::Input('w'));
        composer.keypressed(&KeyAction::Input('o'));
        assert!(matches!(
            composer.keypressed(&KeyAction::ComposerSend),
            WidgetRet::Lines(lines) if lines == ["one", "two"]
        ));
    }

    #[test]
    fn vertical_movement_preserves_column_across_short_lines() {
        let mut composer = Composer::new(String::new(), 0, "abcd\nx\nwxyz");
        composer.cursor = 3;
        composer.move_vertical(true);
        assert_eq!(composer.cursor, 6);
        composer.move_vertical(true);
        assert_eq!(composer.cursor, 10);
        composer.move_vertical(false);
        assert_eq!(composer.cursor, 6);
        composer.move_vertical(false);
        assert_eq!(composer.cursor, 3);
    }

    #[test]
    fn paste_is_normalized_and_inserted_at_the_single_line_cursor() {
        let composer = Composer::new("ac".to_owned(), 1, "b\r\nc\rd");
        assert_eq!(composer.buffer.iter().collect::<String>(), "ab\nc\ndc");
        assert_eq!(composer.cursor, 6);
    }
}
