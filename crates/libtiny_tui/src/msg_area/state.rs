use std::collections::VecDeque;

use super::Line;
use crate::messaging::{MSG_NICK_SUFFIX_LEN, Timestamp};

pub(crate) struct MsgArea {
    pub(super) lines: VecDeque<Line>,
    pub(super) scrollback: usize,

    // Rendering related
    pub(super) width: i32,
    pub(super) height: i32,

    /// Vertical scroll: An offset from the last visible line.
    /// E.g. when this is 0, `self.lines[self.lines.len() - 1]` is drawn at the
    /// bottom of screen.
    pub(super) scroll: i32,

    pub(super) line_buf: Line,

    /// Cached total rendered height of all lines. Invalidate on resize, update
    /// when adding new lines.
    pub(super) lines_height: Option<i32>,

    pub(super) layout: Layout,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Layout {
    Compact,
    Aligned { max_nick_len: usize },
}

impl Layout {
    pub(crate) fn is_aligned(&self) -> bool {
        matches!(self, Layout::Aligned { .. })
    }

    pub(super) fn msg_padding(&self) -> usize {
        match self {
            Layout::Compact => 0,
            Layout::Aligned { max_nick_len } => {
                Timestamp::WIDTH + max_nick_len + MSG_NICK_SUFFIX_LEN
            }
        }
    }
}

impl MsgArea {
    pub(crate) fn new(width: i32, height: i32, scrollback: usize, layout: Layout) -> MsgArea {
        MsgArea {
            lines: VecDeque::with_capacity(512.min(scrollback)),
            scrollback,
            width,
            height,
            scroll: 0,
            line_buf: Line::new(),
            lines_height: Some(0),
            layout,
        }
    }

    pub(crate) fn get_height(&self) -> i32 {
        self.height
    }

    pub(crate) fn num_lines(&self) -> usize {
        self.lines.len()
    }

    #[cfg(test)]
    pub(crate) fn lines_text(&self) -> Vec<String> {
        self.lines.iter().map(Line::text).collect()
    }

    pub(crate) fn layout(&self) -> Layout {
        self.layout
    }
}
