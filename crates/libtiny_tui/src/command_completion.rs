use libtiny_common::CommandInfo;
use termbox_simple::Termbox;

use crate::config::Colors;
use crate::termbox;

/// Filtering and presentation state for the command completion auxiliary pane.
pub(crate) struct CommandCompletion {
    commands: &'static [CommandInfo],
    matching: Vec<usize>,
}

impl CommandCompletion {
    pub(crate) fn new(commands: &'static [CommandInfo], prefix: &str) -> Option<Self> {
        let matching = commands
            .iter()
            .enumerate()
            .filter_map(|(idx, command)| command.name.starts_with(prefix).then_some(idx))
            .collect::<Vec<_>>();
        (!matching.is_empty()).then_some(Self { commands, matching })
    }

    pub(crate) fn height(&self, parent_height: i32) -> i32 {
        (self.matching.len() as i32 + 2).min((parent_height - 3).max(0))
    }

    #[cfg(test)]
    pub(crate) fn matches(&self) -> Vec<CommandInfo> {
        self.matching
            .iter()
            .map(|idx| self.commands[*idx])
            .collect()
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
        if width <= 0 || height <= 0 {
            return;
        }

        if width <= 2 || height <= 2 {
            return;
        }

        let visible_rows = (height - 2) as usize;
        let content_width = width - 2;
        let command_width = self
            .matching
            .iter()
            .map(|idx| self.commands[*idx].name.len() + 1)
            .max()
            .unwrap_or(0)
            .min((content_width as usize) / 2);
        for (row, idx) in self.matching.iter().take(visible_rows).enumerate() {
            let command = self.commands[*idx];
            let label = format!("/{:<command_width$} ", command.name);
            let x = termbox::print_chars(
                tb,
                pos_x + 1,
                pos_y + 1 + row as i32,
                colors.user_msg,
                label.chars().take(content_width as usize),
            );
            if x < pos_x + width - 1 {
                termbox::print_chars(
                    tb,
                    x,
                    pos_y + 1 + row as i32,
                    colors.faded,
                    command
                        .summary
                        .chars()
                        .take((pos_x + width - 1 - x) as usize),
                );
            }
        }
    }
}
