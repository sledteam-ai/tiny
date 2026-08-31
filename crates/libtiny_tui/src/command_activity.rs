use termbox_simple::Termbox;

use crate::command_completion::CommandCompletion;
use crate::config::Colors;
use crate::termbox;

/// Transient command UI shown in the auxiliary region below the normal input.
pub(crate) enum CommandActivity {
    Completion(CommandCompletion),
    Feedback(CommandFeedback),
}

impl CommandActivity {
    pub(crate) fn height(&self, parent_height: i32) -> i32 {
        match self {
            CommandActivity::Completion(completion) => completion.height(parent_height),
            CommandActivity::Feedback(feedback) => feedback.height(parent_height),
        }
    }

    pub(crate) fn draw(
        &self,
        tb: &mut Termbox,
        colors: &Colors,
        pos_x: i32,
        pos_y: i32,
        width: i32,
        height: i32,
    ) {
        match self {
            CommandActivity::Completion(completion) => {
                completion.draw(tb, colors, pos_x, pos_y, width, height)
            }
            CommandActivity::Feedback(feedback) => {
                feedback.draw(tb, colors, pos_x, pos_y, width, height)
            }
        }
    }
}

pub(crate) struct CommandFeedback {
    lines: Vec<String>,
}

impl CommandFeedback {
    pub(crate) fn new(lines: &[String]) -> Option<Self> {
        (!lines.is_empty()).then(|| Self {
            lines: lines.to_vec(),
        })
    }

    fn height(&self, parent_height: i32) -> i32 {
        (self.lines.len() as i32 + 2).min((parent_height - 3).max(0))
    }

    fn draw(
        &self,
        tb: &mut Termbox,
        colors: &Colors,
        pos_x: i32,
        pos_y: i32,
        width: i32,
        height: i32,
    ) {
        if width <= 2 || height <= 2 {
            return;
        }

        let content_width = (width - 2) as usize;
        for (row, line) in self.lines.iter().take((height - 2) as usize).enumerate() {
            termbox::print_chars(
                tb,
                pos_x + 1,
                pos_y + 1 + row as i32,
                colors.err_msg,
                line.chars().take(content_width),
            );
        }
    }

    #[cfg(test)]
    pub(crate) fn lines(&self) -> Vec<String> {
        self.lines.clone()
    }
}
