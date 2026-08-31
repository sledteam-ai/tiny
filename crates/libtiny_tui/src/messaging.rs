use termbox_simple::Termbox;

use std::convert::From;

use time::{self, Tm};

use crate::command_completion::CommandCompletion;
use crate::composer::Composer;
use crate::config::Colors;
use crate::exit_dialogue::ExitDialogue;
use crate::input_area::InputArea;
use crate::key_map::KeyAction;
use crate::msg_area::line::SegStyle;
use crate::msg_area::{Layout, MsgArea};
use crate::trie::Trie;
use crate::widget::WidgetRet;
use libtiny_common::CommandInfo;

/// An input field and an area for showing messages and activities of a tab (channel, server,
/// mentions tab).
pub(crate) struct MessagingUI {
    /// The area showing the messages and activities.
    msg_area: MsgArea,

    /// The single-line command and message field.
    input_field: InputArea,

    /// The transient auxiliary input pane. `None` means normal Tiny input mode.
    composer: Option<Composer>,

    /// Slash-command matches shown in the same auxiliary region as the composer.
    command_completion: Option<CommandCompletion>,

    command_infos: &'static [CommandInfo],

    exit_dialogue: Option<ExitDialogue>,

    /// Width of the UI, in characters.
    width: i32,

    /// Height of the UI, in lines.
    height: i32,

    /// All nicks in the channel. Used in autocompletion.
    nicks: Trie,

    /// The last line in `msg_area` that shows join, leave, disconnect activities.
    last_activity_line: Option<ActivityLine>,

    /// Last timestamp added to the UI.
    last_ts: Option<Timestamp>,
}

/// Length of ": " suffix of nicks in messages
pub(crate) const MSG_NICK_SUFFIX_LEN: usize = 2;

/// Like `time::Tm`, but we only care about hour and minute parts.
#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) struct Timestamp {
    hour: i32,
    min: i32,
}

// 80 characters. TODO: We need to make sure we don't need more whitespace than that. We should
// probably add an upper bound to max_nick_length config field?
static WHITESPACE: &str =
    "                                                                                ";

const SCROLLBACK_INDICATOR: char = '↑';

impl Timestamp {
    /// The width of a timestamp plus a space.
    pub(crate) const WIDTH: usize = 6;

    /// Spaces for a timestamp slot in aligned layout.
    pub(crate) const BLANK: &'static str = "      ";

    fn stamp(&self) -> String {
        format!("{:02}:{:02} ", self.hour, self.min)
    }
}

impl From<Tm> for Timestamp {
    fn from(tm: Tm) -> Timestamp {
        Timestamp {
            hour: tm.tm_hour,
            min: tm.tm_min,
        }
    }
}

/// A line showing joins, leaves, and disconnects.
struct ActivityLine {
    /// Timestamp of the line.
    ts: Timestamp,

    /// Index of the line in its `MsgArea`.
    line_idx: usize,
}

impl MessagingUI {
    pub(crate) fn new(
        width: i32,
        height: i32,
        scrollback: usize,
        msg_layout: Layout,
    ) -> MessagingUI {
        let mut input_field = InputArea::new(width, get_input_field_max_height(height));
        let input_height = input_field.get_height(width);
        MessagingUI {
            msg_area: MsgArea::new(width, height - input_height, scrollback, msg_layout),
            input_field,
            composer: None,
            command_completion: None,
            command_infos: &[],
            exit_dialogue: None,
            width,
            height,
            nicks: Trie::new(),
            last_activity_line: None,
            last_ts: None,
        }
    }

    pub(crate) fn set_nick(&mut self, nick: String) {
        let nick_color = self.get_nick_color(&nick);
        self.input_field.set_nick(nick, nick_color);
        // update text field size
        let w = self.width;
        let h = self.height;
        self.resize(w, h);
    }

    pub(crate) fn get_nick(&self) -> Option<String> {
        self.input_field.get_nick()
    }

    #[cfg(test)]
    pub(crate) fn lines_text(&self) -> Vec<String> {
        self.msg_area.lines_text()
    }

    pub(crate) fn draw(&mut self, tb: &mut Termbox, colors: &Colors, pos_x: i32, pos_y: i32) {
        match &self.exit_dialogue {
            Some(exit_dialogue) => {
                exit_dialogue.draw(tb, colors, pos_x, self.height - 1);
            }
            None => {
                let auxiliary_height = self.auxiliary_height();
                let single_line_region_height = self.height - auxiliary_height;
                // Draw InputArea first because it can trigger a resize of MsgArea.
                self.input_field.draw(
                    tb,
                    colors,
                    pos_x,
                    pos_y,
                    single_line_region_height,
                    &mut self.msg_area,
                );
                if let Some(composer) = &mut self.composer {
                    composer.draw(
                        tb,
                        colors,
                        pos_x,
                        pos_y + single_line_region_height,
                        self.width,
                        auxiliary_height,
                        true,
                    );
                } else if let Some(completion) = &self.command_completion {
                    completion.draw(
                        tb,
                        colors,
                        pos_x,
                        pos_y + single_line_region_height,
                        self.width,
                        auxiliary_height,
                    );
                }
            }
        }
        self.msg_area.draw(tb, colors, pos_x, pos_y);
        if self.msg_area.is_scrolled() && self.width > 0 {
            tb.change_cell(
                pos_x + self.width - 1,
                pos_y,
                SCROLLBACK_INDICATOR,
                colors.tab_active.fg,
                colors.tab_active.bg,
            );
        }
    }

    pub(crate) fn keypressed(&mut self, key_action: &KeyAction) -> WidgetRet {
        match key_action {
            KeyAction::Exit => {
                self.toggle_exit_dialogue();
                WidgetRet::KeyHandled
            }
            KeyAction::MessagesPageUp => {
                self.msg_area.page_up();
                WidgetRet::KeyHandled
            }
            KeyAction::MessagesPageDown => {
                self.msg_area.page_down();
                WidgetRet::KeyHandled
            }
            KeyAction::MessagesScrollUp => {
                self.msg_area.scroll_up();
                WidgetRet::KeyHandled
            }
            KeyAction::MessagesScrollDown => {
                self.msg_area.scroll_down();
                WidgetRet::KeyHandled
            }
            KeyAction::MessagesScrollChunkUp => {
                self.msg_area.scroll_chunk_up();
                WidgetRet::KeyHandled
            }
            KeyAction::MessagesScrollChunkDown => {
                self.msg_area.scroll_chunk_down();
                WidgetRet::KeyHandled
            }
            KeyAction::MessagesScrollTop => {
                self.msg_area.scroll_top();
                WidgetRet::KeyHandled
            }
            KeyAction::MessagesScrollBottom => {
                self.msg_area.scroll_bottom();
                WidgetRet::KeyHandled
            }
            KeyAction::InputAutoComplete => {
                if self.exit_dialogue.is_none() && self.composer.is_none() {
                    self.input_field.autocomplete(&self.nicks);
                }
                WidgetRet::KeyHandled
            }
            KeyAction::InputFocusToggle if self.exit_dialogue.is_none() => {
                if self.composer.is_some() {
                    self.submit_composer()
                } else {
                    self.open_composer();
                    WidgetRet::KeyHandled
                }
            }
            KeyAction::OpenComposer if self.exit_dialogue.is_none() => {
                self.open_composer();
                WidgetRet::KeyHandled
            }
            KeyAction::ComposerSend if self.exit_dialogue.is_none() && self.composer.is_some() => {
                self.submit_composer()
            }
            KeyAction::InputPrevEntry
                if self.exit_dialogue.is_none() && self.composer.is_none() =>
            {
                if let Some(completion) = &mut self.command_completion {
                    completion.select_previous();
                    return WidgetRet::KeyHandled;
                }
                self.input_field.keypressed(key_action)
            }
            KeyAction::InputNextEntry
                if self.exit_dialogue.is_none() && self.composer.is_none() =>
            {
                if let Some(completion) = &mut self.command_completion {
                    completion.select_next();
                    return WidgetRet::KeyHandled;
                }
                self.input_field.keypressed(key_action)
            }
            KeyAction::InputSend if self.exit_dialogue.is_none() && self.composer.is_none() => {
                if let Some(completion) = &self.command_completion {
                    let command = completion.selected_command();
                    self.input_field
                        .replace_text(&format!("/{} ", command.name));
                    self.command_completion = None;
                    self.resize(self.width, self.height);
                    return WidgetRet::KeyHandled;
                }
                self.input_field.keypressed(key_action)
            }
            KeyAction::Cancel
                if self.exit_dialogue.is_none()
                    && self.composer.is_none()
                    && self.command_completion.is_some() =>
            {
                self.command_completion = None;
                self.resize(self.width, self.height);
                WidgetRet::KeyHandled
            }
            KeyAction::Cancel if self.exit_dialogue.is_none() && self.composer.is_some() => {
                self.close_composer();
                WidgetRet::KeyHandled
            }
            KeyAction::TabNext
            | KeyAction::TabPrev
            | KeyAction::TabMoveLeft
            | KeyAction::TabMoveRight
            | KeyAction::TabGoto(_)
            | KeyAction::Command(_)
                if self.exit_dialogue.is_none() && self.composer.is_some() =>
            {
                WidgetRet::KeyHandled
            }
            key_action => {
                let normal_input = self.composer.is_none() && self.exit_dialogue.is_none();
                let ret = {
                    if let Some(exit_dialogue) = self.exit_dialogue.as_ref() {
                        exit_dialogue.keypressed(key_action)
                    } else {
                        match &mut self.composer {
                            None => self.input_field.keypressed(key_action),
                            Some(composer) => composer.keypressed(key_action),
                        }
                    }
                };

                if normal_input {
                    self.update_command_completion();
                }

                if let WidgetRet::Remove = ret {
                    self.exit_dialogue = None;
                    WidgetRet::KeyHandled
                } else {
                    ret
                }
            }
        }
    }

    pub(crate) fn resize(&mut self, width: i32, height: i32) {
        self.width = width;
        self.height = height;

        self.input_field
            .resize(width, get_input_field_max_height(height));
        let auxiliary_height = self.auxiliary_height();
        let input_height = self.input_field.get_height(width) + auxiliary_height;
        let msg_area_height = height - input_height;
        self.msg_area.resize(width, msg_area_height);

        // We don't show the nick in exit dialogue, so it has the full width
        if let Some(exit_dialogue) = &mut self.exit_dialogue {
            exit_dialogue.resize(width);
        }
    }

    #[cfg(test)]
    pub(crate) fn scroll_offset(&self) -> i32 {
        self.msg_area.scroll_offset()
    }

    pub(crate) fn paste_into_composer(&mut self, pasted: &str) {
        self.open_composer();
        self.composer.as_mut().unwrap().insert_text(pasted);
    }

    #[cfg(test)]
    pub(crate) fn input_focus(&self) -> &'static str {
        match self.composer {
            None => "single_line",
            Some(_) => "composer",
        }
    }

    #[cfg(test)]
    pub(crate) fn msg_area_height(&self) -> i32 {
        self.msg_area.get_height()
    }

    fn composer_height(&self) -> i32 {
        if self.composer.is_some() {
            Composer::height(self.height)
        } else {
            0
        }
    }

    fn auxiliary_height(&self) -> i32 {
        if let Some(composer) = &self.command_completion {
            composer.height(self.height)
        } else {
            self.composer_height()
        }
    }

    pub(crate) fn set_command_completions(&mut self, commands: &'static [CommandInfo]) {
        self.command_infos = commands;
        self.update_command_completion();
    }

    fn update_command_completion(&mut self) {
        let text = self.input_field.text();
        let prefix = text
            .strip_prefix('/')
            .filter(|prefix| !prefix.chars().any(char::is_whitespace));
        self.command_completion = match prefix {
            Some(prefix) => match self.command_completion.take() {
                Some(mut completion) => completion.update(prefix).then_some(completion),
                None => CommandCompletion::new(self.command_infos, prefix),
            },
            None => None,
        };
        self.resize(self.width, self.height);
    }

    #[cfg(test)]
    pub(crate) fn command_completion_matches(&self) -> Option<Vec<CommandInfo>> {
        self.command_completion
            .as_ref()
            .map(CommandCompletion::matches)
    }

    #[cfg(test)]
    pub(crate) fn selected_command_completion(&self) -> Option<CommandInfo> {
        self.command_completion
            .as_ref()
            .map(CommandCompletion::selected)
    }

    #[cfg(test)]
    pub(crate) fn input_text(&self) -> String {
        self.input_field.text()
    }

    fn open_composer(&mut self) {
        if self.composer.is_none() {
            self.command_completion = None;
            self.composer = Some(Composer::new());
            self.resize(self.width, self.height);
        }
    }

    fn close_composer(&mut self) {
        self.composer = None;
        self.update_command_completion();
    }

    fn submit_composer(&mut self) -> WidgetRet {
        let ret = self
            .composer
            .as_mut()
            .expect("composer submission requires an open composer")
            .keypressed(&KeyAction::ComposerSend);
        self.close_composer();
        ret
    }

    fn toggle_exit_dialogue(&mut self) {
        if self.exit_dialogue.take().is_none() {
            // We don't show the nick in exit dialogue, so it has the full width
            self.exit_dialogue = Some(ExitDialogue::new(self.width));
        }
    }
}

/// Calculation for input field's maximum height
fn get_input_field_max_height(window_height: i32) -> i32 {
    window_height / 2
}

////////////////////////////////////////////////////////////////////////////////
// Adding new messages

impl MessagingUI {
    /// Add a new line with the given timestamp (`ts`) if we're not already showing the timestamp.
    ///
    /// In compact layout this adds the indentation for the timestamp column if we're already
    /// showing the timestamp.
    fn add_timestamp(&mut self, ts: Timestamp) {
        if let Some(ts_) = self.last_ts {
            if ts_ != ts {
                self.msg_area.add_text(&ts.stamp(), SegStyle::Timestamp);
            } else if self.msg_area.layout().is_aligned() {
                self.msg_area
                    .add_text(Timestamp::BLANK, SegStyle::Timestamp);
            }
        } else {
            self.msg_area.add_text(&ts.stamp(), SegStyle::Timestamp);
        }
        self.last_ts = Some(ts);
    }

    pub(crate) fn show_topic(&mut self, topic: &str, ts: Timestamp) {
        self.add_timestamp(ts);

        self.msg_area.add_text(topic, SegStyle::Topic);

        self.msg_area.flush_line();
    }

    pub(crate) fn add_client_err_msg(&mut self, msg: &str) {
        self.msg_area.add_text(msg, SegStyle::ErrMsg);
        self.msg_area.flush_line();
    }

    pub(crate) fn add_client_notify_msg(&mut self, msg: &str) {
        self.msg_area.add_text(msg, SegStyle::Faded);
        self.msg_area.flush_line();
    }

    pub(crate) fn add_client_msg(&mut self, msg: &str) {
        self.msg_area.add_text(msg, SegStyle::UserMsg);
        self.msg_area.flush_line();
    }

    pub(crate) fn add_privmsg(
        &mut self,
        sender: &str,
        msg: &str,
        ts: Timestamp,
        highlight: bool,
        is_action: bool,
    ) {
        // HACK: Some servers (bridges) don't send RPL_NAMREPLY and JOIN/PART messages but we still
        // want to support tab completion on those servers, so when we see a message from someone
        // we add the user to the nick list so that tab completion will complete their nick. See
        // #253 for details.
        self.nicks.insert(sender);

        self.add_timestamp(ts);

        let nick_color = self.get_nick_color(sender);
        let nick_col_style = SegStyle::NickColor(nick_color);

        // actions are /me msgs so they don't show the nick in the nick column, but in the msg
        if is_action {
            let formatted_action = self.format_nick("**");
            self.msg_area.add_text(&formatted_action, SegStyle::UserMsg);
            // separator between nick and msg
            self.msg_area.add_text("  ", SegStyle::Faded);
            self.msg_area.add_text(sender, nick_col_style);
            // a space replacing the usual ':'
            self.msg_area.add_text(" ", SegStyle::UserMsg);
        } else {
            let formatted_nick = self.format_nick(sender);
            self.msg_area.add_text(&formatted_nick, nick_col_style);
            // separator between nick and msg
            self.msg_area.add_text(": ", SegStyle::Faded);
        }

        let msg_style = if highlight {
            SegStyle::Highlight
        } else {
            SegStyle::UserMsg
        };

        self.msg_area.add_text(msg, msg_style);
        self.msg_area.set_current_line_alignment();
        self.msg_area.flush_line();
    }

    pub(crate) fn add_multiline_privmsg(
        &mut self,
        sender: &str,
        lines: &[String],
        ts: Timestamp,
        highlight: bool,
    ) {
        self.nicks.insert(sender);
        self.add_timestamp(ts);

        let nick_color = self.get_nick_color(sender);
        let formatted_nick = self.format_nick(sender);
        self.msg_area
            .add_text(&formatted_nick, SegStyle::NickColor(nick_color));
        self.msg_area.add_text(":", SegStyle::Faded);
        self.msg_area.flush_line();

        let msg_style = if highlight {
            SegStyle::Highlight
        } else {
            SegStyle::UserMsg
        };
        for line in lines {
            self.msg_area.add_text(line, msg_style);
            self.msg_area.flush_line();
        }
    }

    fn format_nick(&self, nick: &str) -> String {
        if let Layout::Aligned { max_nick_len, .. } = self.msg_area.layout() {
            let mut aligned = format!("{nick:>max_nick_len$.max_nick_len$}");
            if nick.len() > max_nick_len {
                aligned.pop();
                aligned.push('…');
            }
            aligned
        } else {
            nick.to_owned()
        }
    }

    pub(crate) fn add_msg(&mut self, msg: &str, ts: Timestamp) {
        self.add_timestamp(ts);
        self.msg_area.add_text(msg, SegStyle::UserMsg);
        self.msg_area.flush_line();
    }

    pub(crate) fn add_err_msg(&mut self, msg: &str, ts: Timestamp) {
        self.add_timestamp(ts);
        self.msg_area.add_text(msg, SegStyle::ErrMsg);
        self.msg_area.flush_line();
    }

    pub(crate) fn clear(&mut self) {
        self.msg_area.clear();
        self.last_activity_line = None;
        self.last_ts = None;
    }

    fn get_nick_color(&self, sender: &str) -> usize {
        // Anything works as long as it's fast
        let mut hash: usize = 5381;
        for c in sender.chars() {
            hash = hash.wrapping_mul(33).wrapping_add(c as usize);
        }
        hash
    }
}

////////////////////////////////////////////////////////////////////////////////
// Keeping nick list up-to-date

impl MessagingUI {
    pub(crate) fn clear_nicks(&mut self) {
        self.nicks.clear();
    }

    pub(crate) fn join(&mut self, nick: &str, ts: Option<Timestamp>, ignore: bool) {
        self.nicks.insert(nick);

        if !ignore && let Some(ts) = ts {
            let line_idx = self.get_activity_line_idx(ts);
            self.msg_area.modify_line(line_idx, |line| {
                line.add_char('+', SegStyle::Join);
                line.add_text(nick, SegStyle::Faded);
            });
        }
    }

    pub(crate) fn part(&mut self, nick: &str, ts: Option<Timestamp>, ignore: bool) {
        self.nicks.remove(nick);

        if !ignore && let Some(ts) = ts {
            let line_idx = self.get_activity_line_idx(ts);
            self.msg_area.modify_line(line_idx, |line| {
                line.add_char('-', SegStyle::Part);
                line.add_text(nick, SegStyle::Faded);
            });
        }
    }

    pub(crate) fn nick(&mut self, old_nick: &str, new_nick: &str, ts: Timestamp) {
        self.nicks.remove(old_nick);
        self.nicks.insert(new_nick);

        let line_idx = self.get_activity_line_idx(ts);
        self.msg_area.modify_line(line_idx, |line| {
            line.add_text(old_nick, SegStyle::Faded);
            line.add_char('>', SegStyle::Nick);
            line.add_text(new_nick, SegStyle::Faded);
        });
    }

    fn get_activity_line_idx(&mut self, ts: Timestamp) -> usize {
        match &self.last_activity_line {
            Some(l)
                if l.ts == ts && Some(l.line_idx) == self.msg_area.num_lines().checked_sub(1) =>
            {
                let line_idx = l.line_idx;
                // FIXME: It's a bit hacky to add a space in this function which from the name
                // looks like a getter.
                // The idea is that we want to add a space *before* adding new stuff, not *after*,
                // to avoid adding redundant spaces. The test `small_screen_1` breaks if we don't
                // get this right.
                self.msg_area
                    .modify_line(line_idx, |line| line.add_char(' ', SegStyle::UserMsg));
                line_idx
            }
            _ => {
                self.add_timestamp(ts);
                if let Layout::Aligned { max_nick_len, .. } = self.msg_area.layout() {
                    self.msg_area.add_text(
                        &WHITESPACE[..max_nick_len + MSG_NICK_SUFFIX_LEN],
                        SegStyle::UserMsg,
                    )
                }
                self.msg_area.set_current_line_alignment();
                let line_idx = self.msg_area.flush_line();
                self.last_activity_line = Some(ActivityLine { ts, line_idx });
                line_idx
            }
        }
    }
}
