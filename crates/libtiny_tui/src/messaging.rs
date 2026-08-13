use termbox_simple::{TB_BOLD, Termbox};

use std::convert::From;

use time::{self, Tm};

use crate::composer::Composer;
use crate::config::Colors;
use crate::exit_dialogue::ExitDialogue;
use crate::input_area::InputArea;
use crate::key_map::KeyAction;
use crate::msg_area::line::SegStyle;
use crate::msg_area::{Layout, MsgArea};
use crate::trie::Trie;
use crate::widget::WidgetRet;

/// An input field and an area for showing messages and activities of a tab (channel, server,
/// mentions tab).
pub(crate) struct MessagingUI {
    /// The area showing the messages and activities.
    msg_area: MsgArea,

    /// The single-line command and message field.
    input_field: InputArea,

    composer: Composer,

    input_focus: InputFocus,

    #[cfg(test)]
    legacy_single_line_layout: bool,

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputFocus {
    SingleLine,
    Composer,
}

impl MessagingUI {
    pub(crate) fn new(
        width: i32,
        height: i32,
        scrollback: usize,
        msg_layout: Layout,
    ) -> MessagingUI {
        let composer_height = Composer::height(height);
        MessagingUI {
            msg_area: MsgArea::new(width, height - composer_height, scrollback, msg_layout),
            input_field: InputArea::new(width, get_input_field_max_height(height)),
            composer: Composer::new(),
            input_focus: InputFocus::Composer,
            #[cfg(test)]
            legacy_single_line_layout: false,
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
                #[cfg(test)]
                if self.legacy_single_line_layout {
                    self.input_field.draw(
                        tb,
                        colors,
                        pos_x,
                        pos_y,
                        self.height,
                        &mut self.msg_area,
                    );
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
                    return;
                }
                let composer_height = Composer::height(self.height);
                let single_line_region_height = self.height - composer_height;
                let focus_line_y = pos_y + single_line_region_height - 1;
                // Draw InputArea first because it can trigger a resize of MsgArea.
                self.input_field.draw(
                    tb,
                    colors,
                    pos_x,
                    pos_y,
                    single_line_region_height - 1,
                    &mut self.msg_area,
                );
                if self.input_focus == InputFocus::SingleLine {
                    for x in pos_x..pos_x + self.width {
                        tb.change_cell(
                            x,
                            focus_line_y,
                            '━',
                            colors.user_msg.fg | TB_BOLD,
                            colors.user_msg.bg,
                        );
                    }
                }
                self.composer.draw(
                    tb,
                    colors,
                    pos_x,
                    pos_y + single_line_region_height,
                    self.width,
                    composer_height,
                    self.input_focus == InputFocus::Composer,
                );
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
                if self.exit_dialogue.is_none() && self.input_focus == InputFocus::SingleLine {
                    self.input_field.autocomplete(&self.nicks);
                }
                WidgetRet::KeyHandled
            }
            KeyAction::InputFocusToggle if self.exit_dialogue.is_none() => {
                self.input_focus = match self.input_focus {
                    InputFocus::SingleLine => InputFocus::Composer,
                    InputFocus::Composer => InputFocus::SingleLine,
                };
                WidgetRet::KeyHandled
            }
            KeyAction::OpenComposer => {
                // Retain the old configurable action as a compatibility focus shortcut.
                self.input_focus = InputFocus::Composer;
                WidgetRet::KeyHandled
            }
            key_action => {
                let ret = {
                    if let Some(exit_dialogue) = self.exit_dialogue.as_ref() {
                        exit_dialogue.keypressed(key_action)
                    } else {
                        match self.input_focus {
                            InputFocus::SingleLine => self.input_field.keypressed(key_action),
                            InputFocus::Composer => self.composer.keypressed(key_action),
                        }
                    }
                };

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
        #[cfg(test)]
        let composer_height = if self.legacy_single_line_layout {
            0
        } else {
            Composer::height(height)
        };
        #[cfg(not(test))]
        let composer_height = Composer::height(height);
        #[cfg(test)]
        let focus_line_height = i32::from(!self.legacy_single_line_layout);
        #[cfg(not(test))]
        let focus_line_height = 1;
        let input_height = self.input_field.get_height(width) + focus_line_height + composer_height;
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
        self.composer.insert_text(pasted);
    }

    pub(crate) fn has_exit_dialogue(&self) -> bool {
        self.exit_dialogue.is_some()
    }

    #[cfg(test)]
    pub(crate) fn input_focus(&self) -> &'static str {
        match self.input_focus {
            InputFocus::SingleLine => "single_line",
            InputFocus::Composer => "composer",
        }
    }

    #[cfg(test)]
    pub(crate) fn use_legacy_single_line_layout(&mut self) {
        self.legacy_single_line_layout = true;
        self.input_focus = InputFocus::SingleLine;
        self.resize(self.width, self.height);
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
        let layout = self.msg_area.layout();
        let format_nick = |s: &str| -> String {
            if let Layout::Aligned { max_nick_len, .. } = layout {
                let mut aligned = format!("{s:>max_nick_len$.max_nick_len$}");
                if s.len() > max_nick_len {
                    aligned.pop();
                    aligned.push('…');
                }
                aligned
            } else {
                s.to_string()
            }
        };
        if is_action {
            self.msg_area
                .add_text(&format_nick("**"), SegStyle::UserMsg);
            // separator between nick and msg
            self.msg_area.add_text("  ", SegStyle::Faded);
            self.msg_area.add_text(sender, nick_col_style);
            // a space replacing the usual ':'
            self.msg_area.add_text(" ", SegStyle::UserMsg);
        } else {
            self.msg_area.add_text(&format_nick(sender), nick_col_style);
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
