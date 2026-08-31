use libtiny_common::CommandInfo;
use termbox_simple::{TB_BOLD, Termbox};

use crate::config::Colors;
use crate::termbox;

/// Filtering and presentation state for the command completion auxiliary pane.
pub(crate) struct CommandCompletion {
    commands: &'static [CommandInfo],
    matching: Vec<usize>,
    selected: usize,
}

impl CommandCompletion {
    pub(crate) fn new(commands: &'static [CommandInfo], prefix: &str) -> Option<Self> {
        let matching = commands
            .iter()
            .enumerate()
            .filter_map(|(idx, command)| command.name.starts_with(prefix).then_some(idx))
            .collect::<Vec<_>>();
        (!matching.is_empty()).then_some(Self {
            commands,
            matching,
            selected: 0,
        })
    }

    pub(crate) fn update(&mut self, prefix: &str) -> bool {
        let selected_command = self.matching.get(self.selected).copied();
        self.matching = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(idx, command)| command.name.starts_with(prefix).then_some(idx))
            .collect();

        if self.matching.is_empty() {
            return false;
        }

        self.selected = selected_command
            .and_then(|selected| self.matching.iter().position(|idx| *idx == selected))
            .unwrap_or_else(|| self.selected.min(self.matching.len() - 1));
        true
    }

    pub(crate) fn select_previous(&mut self) {
        self.selected = if self.selected == 0 {
            self.matching.len() - 1
        } else {
            self.selected - 1
        };
    }

    pub(crate) fn select_next(&mut self) {
        self.selected = (self.selected + 1) % self.matching.len();
    }

    pub(crate) fn selected_command(&self) -> CommandInfo {
        self.commands[self.matching[self.selected]]
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

    #[cfg(test)]
    pub(crate) fn selected(&self) -> CommandInfo {
        self.selected_command()
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
        let first_visible = self.selected.saturating_sub(visible_rows.saturating_sub(1));
        for (row, (match_idx, idx)) in self
            .matching
            .iter()
            .enumerate()
            .skip(first_visible)
            .take(visible_rows)
            .enumerate()
        {
            let command = self.commands[*idx];
            let label = format!("/{:<command_width$} ", command.name);
            let selected = match_idx == self.selected;
            let mut command_style = colors.user_msg;
            let mut summary_style = colors.faded;
            if selected {
                command_style.fg |= TB_BOLD;
                summary_style.fg |= TB_BOLD;
            }
            let x = termbox::print_chars(
                tb,
                pos_x + 1,
                pos_y + 1 + row as i32,
                command_style,
                label.chars().take(content_width as usize),
            );
            if x < pos_x + width - 1 {
                termbox::print_chars(
                    tb,
                    x,
                    pos_y + 1 + row as i32,
                    summary_style,
                    command
                        .summary
                        .chars()
                        .take((pos_x + width - 1 - x) as usize),
                );
            }
        }
    }
}
