#![allow(
    clippy::cognitive_complexity,
    clippy::new_without_default,
    clippy::too_many_arguments
)]
// https://github.com/rust-lang/rust-clippy/issues/7526
#![allow(clippy::needless_collect)]

use std::borrow::Borrow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::{self, SplitWhitespace};
use time::Tm;

use crate::config::{Colors, Config, TabConfig, TabConfigs, parse_config};
use crate::key_map::{KeyAction, KeyMap};
use crate::messaging::{MessagingUI, Timestamp};
use crate::msg_area::Layout;
use crate::notifier::Notifier;
use crate::tab::{Tab, tab_style};
use crate::widget::WidgetRet;

use libtiny_common::{ChanNameRef, CommandInfo, MsgSource, MsgTarget, TabStyle};
use term_input::{Event, Key};
use termbox_simple::{CellBuf, Termbox};

#[derive(Debug)]
pub(crate) enum TUIRet {
    /// A command was submitted, either directly, or via a key bound to a command.
    ///
    /// `cmd` won't have an initial '/'.
    KeyCommand { cmd: String, from: MsgSource },

    /// A message was sent. `msg` will have at least one character.
    Input { msg: Vec<char>, from: MsgSource },

    /// A multiline message was submitted from the internal composer.
    Lines { lines: Vec<String>, from: MsgSource },
}

const LEFT_ARROW: char = '<';
const RIGHT_ARROW: char = '>';

const QUIT_CMD: CommandInfo = CommandInfo::new("quit", "`/quit` or `/quit <reason>`", "Quit tiny");
const CLEAR_CMD: CommandInfo = CommandInfo::new("clear", "`/clear`", "Clears current tab");
const IGNORE_CMD: CommandInfo =
    CommandInfo::new("ignore", "`/ignore`", "Ignore join/quit messages");
const NOTIFY_CMD: CommandInfo = CommandInfo::new(
    "notify",
    "`/notify [off|mentions|messages]`",
    "Set channel notifications",
);
const SWITCH_CMD: CommandInfo =
    CommandInfo::new("switch", "`/switch <tab name>`", "Switches to tab");
const RELOAD_CMD: CommandInfo = CommandInfo::new("reload", "`/reload`", "Reloads config file");

const TUI_COMMANDS: [CommandInfo; 6] = [
    QUIT_CMD, CLEAR_CMD, IGNORE_CMD, NOTIFY_CMD, SWITCH_CMD, RELOAD_CMD,
];

pub fn command_infos() -> &'static [CommandInfo] {
    &TUI_COMMANDS
}

// Public for benchmarks
pub struct TUI {
    /// Termbox instance
    tb: Termbox,

    /// Color scheme
    colors: Colors,

    /// Max number of message lines
    scrollback: usize,

    /// Messaging area layout: aligned or compact
    msg_layout: Layout,

    tabs: Vec<Tab>,
    active_idx: usize,
    width: i32,
    height: i32,
    h_scroll: i32,

    key_map: KeyMap,

    /// Config file path
    config_path: Option<PathBuf>,

    /// TabConfig settings loaded from config file
    tab_configs: TabConfigs,

    #[cfg(test)]
    legacy_single_line_layout: bool,
}

pub(crate) enum CmdResult {
    /// Command executed successfully by the TUI.
    Handled,

    /// Pass command through to the caller.
    Pass,

    /// Quit command was executed, with the payload as the quit message.
    Quit(Option<String>),
}

impl TUI {
    pub(crate) fn new(config_path: PathBuf) -> TUI {
        let tb = Termbox::init().unwrap(); // TODO: check errors
        TUI::new_tb(Some(config_path), tb)
    }

    /// Create a test instance. Does not render to the screen, just updates the termbox buffer.
    /// Useful for testing rendering. See also [`get_front_buffer`](TUI::get_front_buffer).
    pub fn new_test(w: u16, h: u16) -> TUI {
        let tb = Termbox::init_test(w, h);
        TUI::new_tb(None, tb)
    }

    #[cfg(test)]
    pub(crate) fn use_single_line_input(&mut self) {
        self.legacy_single_line_layout = true;
        self.tabs[self.active_idx]
            .widget
            .use_legacy_single_line_layout();
    }

    /// Get termbox front buffer. Useful for testing rendering.
    pub(crate) fn get_front_buffer(&self) -> CellBuf {
        self.tb.get_front_buffer()
    }

    #[cfg(test)]
    pub(crate) fn set_layout(&mut self, layout: Layout) {
        self.msg_layout = layout
    }

    pub(crate) fn current_tab(&self) -> &MsgSource {
        &self.tabs[self.active_idx].src
    }

    #[cfg(test)]
    pub(crate) fn get_tabs(&self) -> &[Tab] {
        &self.tabs
    }

    fn new_tb(config_path: Option<PathBuf>, tb: Termbox) -> TUI {
        // This is now done in reload_config() below
        // tb.set_clear_attributes(colors.clear.fg as u8, colors.clear.bg as u8);

        let width = tb.width();
        let height = tb.height();

        let mut tui = TUI {
            tb,
            colors: Colors::default(),
            scrollback: usize::MAX,
            msg_layout: Layout::Compact,
            tabs: Vec::new(),
            active_idx: 0,
            width,
            height,
            h_scroll: 0,
            key_map: KeyMap::default(),
            config_path,
            tab_configs: TabConfigs::default(),
            #[cfg(test)]
            legacy_single_line_layout: false,
        };

        // Init "mentions" tab. This needs to happen right after creating the TUI to be able to
        // show any errors in TUI.
        tui.new_server_tab("mentions", None);
        tui.hide_server_tab("mentions");
        tui.add_client_msg(
            "Any mentions to you will be listed here.",
            &MsgTarget::Server { serv: "mentions" },
        );

        tui.reload_config();
        tui
    }

    fn ignore(&mut self, src: &MsgSource) {
        let (target, ignore) = match src {
            MsgSource::Serv { serv } => {
                let mut config = self.get_tab_config(serv, None);
                let new_ignore = config.toggle_ignore();
                self.tab_configs.set_by_server(serv, config);
                (MsgTarget::AllServTabs { serv }, new_ignore)
            }
            MsgSource::Chan { serv, chan } => {
                let mut config = self.get_tab_config(serv, Some(chan));
                let new_ignore = config.toggle_ignore();
                self.tab_configs.set(serv, Some(chan), config);
                (MsgTarget::Chan { serv, chan }, new_ignore)
            }
            MsgSource::User { serv, nick } => {
                let nick_chan = ChanNameRef::new(nick);
                let mut config = self.get_tab_config(serv, Some(nick_chan));
                let new_ignore = config.toggle_ignore();
                self.tab_configs.set(serv, Some(nick_chan), config);
                (MsgTarget::User { serv, nick }, new_ignore)
            }
        };
        let msg = if ignore {
            "Ignore enabled"
        } else {
            "Ignore disabled"
        };
        self.add_client_notify_msg(msg, &target);
    }

    fn notify(&mut self, words: &mut SplitWhitespace, src: &MsgSource) {
        if !cfg!(feature = "desktop-notifications") {
            self.add_client_msg(
                "Desktop notification support is disabled in this build. \
                Please see https://github.com/osa1/tiny/#installation for \
                instructions on enabling desktop notifications or get a \
                pre-built binary with libdbus in https://github.com/osa1/tiny/releases.",
                &MsgTarget::CurrentTab,
            );
            return;
        }

        let words: Vec<&str> = words.collect();

        let mut show_usage = || {
            self.add_client_err_msg(
                &format!("Usage: {}", NOTIFY_CMD.usage),
                &MsgTarget::CurrentTab,
            )
        };

        if words.is_empty() {
            self.show_notify_mode(src);
        } else if words.len() != 1 {
            show_usage();
        } else {
            let notifier = match words[0] {
                "off" => {
                    self.add_client_notify_msg("Notifications turned off", &MsgTarget::CurrentTab);
                    Notifier::Off
                }
                "mentions" => {
                    self.add_client_notify_msg(
                        "Notifications enabled for mentions",
                        &MsgTarget::CurrentTab,
                    );
                    Notifier::Mentions
                }
                "messages" => {
                    self.add_client_notify_msg(
                        "Notifications enabled for all messages",
                        &MsgTarget::CurrentTab,
                    );
                    Notifier::Messages
                }
                _ => {
                    return show_usage();
                }
            };
            // can't use `MsgSource::to_target` here, `Serv` case is different
            let tab_target = match src {
                MsgSource::Serv { serv } => MsgTarget::AllServTabs { serv },
                MsgSource::Chan { serv, chan } => MsgTarget::Chan {
                    serv,
                    chan: chan.borrow(),
                },
                MsgSource::User { serv, nick } => MsgTarget::User { serv, nick },
            };
            self.set_notifier(notifier, &tab_target);
        }
    }

    pub(crate) fn try_handle_cmd(&mut self, cmd: &str, src: &MsgSource) -> CmdResult {
        let mut words = cmd.split_whitespace();
        match words.next() {
            Some("clear") => {
                self.clear(&src.to_target());
                CmdResult::Handled
            }
            Some("ignore") => {
                self.ignore(src);
                CmdResult::Handled
            }
            Some("notify") => {
                self.notify(&mut words, src);
                CmdResult::Handled
            }
            Some("switch") => {
                match words.next() {
                    Some(s) => self.switch(s),
                    None => self.add_client_err_msg(
                        &format!("Usage: {}", SWITCH_CMD.usage),
                        &MsgTarget::CurrentTab,
                    ),
                }
                CmdResult::Handled
            }
            Some("reload") => {
                self.reload_config();
                self.add_client_notify_msg("Reloaded config file.", &MsgTarget::CurrentTab);
                CmdResult::Handled
            }
            Some("help") => {
                self.add_client_msg("TUI Commands: ", &MsgTarget::CurrentTab);
                for cmd in TUI_COMMANDS.iter() {
                    self.add_client_msg(
                        &format!(
                            "/{:<10} - {:<25} - Usage: {}",
                            cmd.name, cmd.summary, cmd.usage
                        ),
                        &MsgTarget::CurrentTab,
                    );
                }
                // Fall through to print help for cmd.rs commands
                CmdResult::Pass
            }
            Some("quit") => {
                // Note: `SplitWhitespace::as_str` could be used here instead, when it gets stabilized.
                let reason: String = cmd.chars().skip("quit ".len()).collect();

                if reason.is_empty() {
                    CmdResult::Quit(None)
                } else {
                    CmdResult::Quit(Some(reason))
                }
            }
            _ => CmdResult::Pass,
        }
    }

    pub(crate) fn load_config(&mut self) -> Option<Config> {
        if let Some(ref config_path) = self.config_path {
            match parse_config(config_path) {
                Err(err) => {
                    self.add_client_err_msg(
                        &format!("Can't parse TUI config: {err}"),
                        &MsgTarget::CurrentTab,
                    );
                    None
                }
                Ok(config) => Some(config),
            }
        } else {
            None
        }
    }

    fn apply_config(&mut self, config: Option<Config>) {
        if let Some(config) = config {
            self.tab_configs = TabConfigs::from(&config);
            let Config {
                colors,
                scrollback,
                max_nick_length,
                key_map,
                layout,
                ..
            } = config;
            self.set_colors(colors);
            self.scrollback = scrollback.max(1);
            self.key_map.load(&key_map.unwrap_or_default());
            if let Some(layout) = layout {
                match layout {
                    crate::config::Layout::Compact => self.msg_layout = Layout::Compact,
                    crate::config::Layout::Aligned => {
                        self.msg_layout = Layout::Aligned {
                            max_nick_len: max_nick_length,
                        }
                    }
                }
            }
        }
    }

    fn reload_config(&mut self) {
        let config = self.load_config();
        self.apply_config(config);
    }

    fn set_colors(&mut self, colors: Colors) {
        self.tb
            .set_clear_attributes(colors.clear.fg as u8, colors.clear.bg as u8);
        self.colors = colors;
    }

    fn new_tab(&mut self, idx: usize, src: MsgSource, alias: Option<String>) {
        let visible_name = alias.unwrap_or_else(|| match &src {
            MsgSource::Serv { serv } => serv.to_owned(),
            MsgSource::Chan { chan, .. } => chan.display().to_owned(),
            MsgSource::User { nick, .. } => nick.to_owned(),
        });

        let switch = {
            // Maps a switch key to number of times it's used
            let mut switch_keys: HashMap<char, u16> = HashMap::with_capacity(self.tabs.len());

            for tab in self.tabs.iter().filter(|tab| tab.is_visible()) {
                if let Some(key) = tab.switch {
                    *switch_keys.entry(key).or_default() += 1;
                }
            }

            // From the characters in tab name, find the one that is used the least
            let mut new_tab_switch_char: Option<(char, u16)> = None;
            for ch in visible_name.chars() {
                if !ch.is_alphabetic() {
                    continue;
                }
                match switch_keys.get(&ch).copied() {
                    None => {
                        new_tab_switch_char = Some((ch, 0));
                        break;
                    }
                    Some(n_uses) => match new_tab_switch_char {
                        None => {
                            new_tab_switch_char = Some((ch, n_uses));
                        }
                        Some((_, new_tab_switch_char_n_uses)) => {
                            if new_tab_switch_char_n_uses > n_uses {
                                new_tab_switch_char = Some((ch, n_uses));
                            }
                        }
                    },
                }
            }
            new_tab_switch_char.map(|(ch, _)| ch)
        };

        self.tabs.insert(
            idx,
            Tab {
                visible_name,
                widget: MessagingUI::new(
                    self.width,
                    self.height - 1,
                    self.scrollback,
                    self.msg_layout,
                ),
                src,
                style: TabStyle::Normal,
                hidden: false,
                switch,
            },
        );
        #[cfg(test)]
        if self.legacy_single_line_layout {
            self.tabs[idx].widget.use_legacy_single_line_layout();
        }
    }

    fn hide_server_tab(&mut self, serv: &str) {
        if let Some(tab_idx) = self.find_serv_tab_idx(serv) {
            self.tabs[tab_idx].hide();
        }
    }

    /// Returns index of the new tab if a new tab is created.
    pub fn new_server_tab(&mut self, serv: &str, alias: Option<String>) -> Option<usize> {
        match self.find_serv_tab_idx(serv) {
            None => {
                let tab_idx = self.tabs.len();
                self.new_tab(
                    tab_idx,
                    MsgSource::Serv {
                        serv: serv.to_owned(),
                    },
                    alias,
                );
                Some(tab_idx)
            }
            Some(_) => None,
        }
    }

    /// Closes a server tab and all associated channel tabs.
    pub(crate) fn close_server_tab(&mut self, serv: &str) {
        if let Some(tab_idx) = self.find_serv_tab_idx(serv) {
            self.tabs.retain(|tab: &Tab| tab.src.serv_name() != serv);
            if self.active_idx == tab_idx {
                self.select_tab(if tab_idx == 0 { 0 } else { tab_idx - 1 });
            }
        }
        self.fix_scroll_after_close();
    }

    /// Returns index of the new tab if a new tab is created.
    pub(crate) fn new_chan_tab(&mut self, serv: &str, chan: &ChanNameRef) -> Option<usize> {
        match self.find_chan_tab_idx(serv, chan) {
            None => match self.find_last_serv_tab_idx(serv) {
                None => {
                    self.new_server_tab(serv, None);
                    self.new_chan_tab(serv, chan)
                }
                Some(serv_tab_idx) => {
                    let tab_idx = serv_tab_idx + 1;
                    self.new_tab(
                        tab_idx,
                        MsgSource::Chan {
                            serv: serv.to_owned(),
                            chan: chan.to_owned(),
                        },
                        None,
                    );
                    if self.active_idx >= tab_idx {
                        self.next_tab();
                    }
                    if let Some(nick) = self.tabs[serv_tab_idx].widget.get_nick() {
                        self.tabs[tab_idx].widget.set_nick(nick);
                    }
                    Some(tab_idx)
                }
            },
            Some(_) => None,
        }
    }

    pub(crate) fn focus_chan_tab(&mut self, serv: &str, chan: &ChanNameRef) {
        if let Some(tab_idx) = self.find_chan_tab_idx(serv, chan) {
            self.select_tab(tab_idx);
        }
    }

    pub(crate) fn close_chan_tab(&mut self, serv: &str, chan: &ChanNameRef) {
        if let Some(tab_idx) = self.find_chan_tab_idx(serv, chan) {
            self.tabs.remove(tab_idx);
            if self.active_idx == tab_idx {
                self.select_tab(if tab_idx == 0 { 0 } else { tab_idx - 1 });
            }
        }
        self.fix_scroll_after_close();
    }

    /// Returns index of the new tab if a new tab is created.
    pub(crate) fn new_user_tab(&mut self, serv: &str, nick: &str) -> Option<usize> {
        match self.find_user_tab_idx(serv, nick) {
            None => match self.find_last_serv_tab_idx(serv) {
                None => {
                    self.new_server_tab(serv, None);
                    self.new_user_tab(serv, nick)
                }
                Some(tab_idx) => {
                    self.new_tab(
                        tab_idx + 1,
                        MsgSource::User {
                            serv: serv.to_owned(),
                            nick: nick.to_owned(),
                        },
                        None,
                    );
                    if let Some(nick) = self.tabs[tab_idx].widget.get_nick() {
                        self.tabs[tab_idx + 1].widget.set_nick(nick);
                    }
                    self.tabs[tab_idx + 1].widget.join(nick, None, false);
                    Some(tab_idx + 1)
                }
            },
            Some(_) => None,
        }
    }

    pub(crate) fn close_user_tab(&mut self, serv: &str, nick: &str) {
        if let Some(tab_idx) = self.find_user_tab_idx(serv, nick) {
            self.tabs.remove(tab_idx);
            if self.active_idx == tab_idx {
                self.select_tab(if tab_idx == 0 { 0 } else { tab_idx - 1 });
            }
        }
        self.fix_scroll_after_close();
    }

    pub(crate) fn handle_input_event(&mut self, ev: Event) -> Option<TUIRet> {
        match ev {
            Event::Key(key) => self.keypressed(key),

            Event::String(str) => {
                // For some reason on my terminal newlines in text are
                // translated to carriage returns when pasting so we check for
                // both just to make sure
                if str.contains('\n') || str.contains('\r') {
                    self.tabs[self.active_idx].widget.paste_into_composer(&str);
                } else {
                    // TODO this may be too slow for pasting long single lines
                    for ch in str.chars() {
                        self.handle_input_event(Event::Key(Key::Char(ch)));
                    }
                }
                None
            }

            Event::Unknown(_) => None,
        }
    }

    fn keypressed(&mut self, key: Key) -> Option<TUIRet> {
        let key_action = self.key_map.get(&key).or(match key {
            Key::Char(c) => Some(KeyAction::Input(c)),
            Key::AltChar(c) => Some(KeyAction::TabGoto(c)),
            _ => None,
        });

        let key_action = key_action?;

        match self.tabs[self.active_idx].widget.keypressed(&key_action) {
            WidgetRet::KeyHandled => None,

            WidgetRet::KeyIgnored => {
                self.handle_keypress(key_action);
                None
            }

            WidgetRet::Command(cmd) => Some(TUIRet::KeyCommand {
                cmd,
                from: self.tabs[self.active_idx].src.clone(),
            }),

            WidgetRet::Input(input) => Some(TUIRet::Input {
                msg: input,
                from: self.tabs[self.active_idx].src.clone(),
            }),

            WidgetRet::Lines(lines) => Some(TUIRet::Lines {
                lines,
                from: self.tabs[self.active_idx].src.clone(),
            }),

            WidgetRet::Remove => unimplemented!(),
        }
    }

    fn handle_keypress(&mut self, key_action: KeyAction) {
        match key_action {
            KeyAction::TabNext => {
                self.next_tab();
            }

            KeyAction::TabPrev => {
                self.prev_tab();
            }

            KeyAction::TabMoveLeft => {
                self.move_tab_left();
            }

            KeyAction::TabMoveRight => {
                self.move_tab_right();
            }

            KeyAction::TabGoto(c) => self.go_to_tab(c),

            _ => {}
        }
    }

    /// Handles resize events. Call on SIGWINCH.
    pub(crate) fn resize(&mut self) {
        self.tb.resize();
        self.tb.clear();

        self.width = self.tb.width();
        self.height = self.tb.height();

        self.resize_();
    }

    /// Set terminal size. Useful when testing resizing.
    pub fn set_size(&mut self, w: u16, h: u16) {
        self.tb.set_buffer_size(w, h);

        self.width = i32::from(w);
        self.height = i32::from(h);

        self.resize_();
    }

    fn resize_(&mut self) {
        for tab in &mut self.tabs {
            tab.widget.resize(self.width, self.height - 1);
        }
        let max_scroll = (self.visible_tabs_width() - self.width).max(0);
        self.h_scroll = self.h_scroll.min(max_scroll);
        self.ensure_tab_visible(self.active_idx);

        // redraw after resize
        self.draw()
    }

    pub(crate) fn get_tab_config(
        &self,
        serv_name: &str,
        chan_name: Option<&ChanNameRef>,
    ) -> TabConfig {
        let configs = &self.tab_configs;
        configs
            .get(serv_name, chan_name) // tries to get
            .or_else(|| configs.get(serv_name, None))
            .or_else(|| configs.get("_defaults", None))
            .unwrap_or_default()
    }

    pub(crate) fn set_tab_config(
        &mut self,
        serv_name: &str,
        chan_name: Option<&ChanNameRef>,
        config: TabConfig,
    ) {
        self.tab_configs.set(serv_name, chan_name, config)
    }
}

////////////////////////////////////////////////////////////////////////////////
// Rendering

impl TUI {
    fn visible_tab_idxs(&self) -> Vec<usize> {
        self.tabs
            .iter()
            .enumerate()
            .filter_map(|(idx, tab)| tab.is_visible().then_some(idx))
            .collect()
    }

    fn visible_tabs_width(&self) -> i32 {
        let mut width = 0;
        let mut first = true;
        for tab_idx in self.visible_tab_idxs() {
            if first {
                first = false;
            } else {
                width += 1;
            }
            width += self.tabs[tab_idx].width();
        }
        width
    }

    fn visible_tab_offset(&self, tab_idx: usize) -> Option<i32> {
        let mut offset = 0;
        for visible_idx in self.visible_tab_idxs() {
            if visible_idx == tab_idx {
                return Some(offset);
            }
            offset += self.tabs[visible_idx].width() + 1;
        }
        None
    }

    fn draw_left_arrow(&self) -> bool {
        self.h_scroll > 0
    }

    fn draw_right_arrow(&self) -> bool {
        let visible_width = self.visible_tabs_width();
        let available_width = self.width - if self.draw_left_arrow() { 2 } else { 0 };
        visible_width > self.h_scroll + available_width
    }

    fn visible_tabs_before_offset(&self, offset: i32) -> Vec<usize> {
        let mut current_offset = 0;
        let mut ret = vec![];
        for tab_idx in self.visible_tab_idxs() {
            let tab_width = self.tabs[tab_idx].width() + 1;
            if current_offset + tab_width > offset {
                break;
            }
            ret.push(tab_idx);
            current_offset += tab_width;
        }
        ret
    }

    fn rendered_tabs(&self) -> Vec<usize> {
        let visible_idxs = self.visible_tab_idxs();
        if visible_idxs.is_empty() {
            return vec![];
        }

        let mut start = 0;
        let mut offset = 0;
        while start < visible_idxs.len() - 1 {
            let tab_width = self.tabs[visible_idxs[start]].width() + 1;
            if offset + tab_width > self.h_scroll {
                break;
            }
            offset += tab_width;
            start += 1;
        }

        let mut width_left = self.width;
        if self.draw_left_arrow() {
            width_left -= 2;
        }
        if self.draw_right_arrow() {
            width_left -= 2;
        }

        let mut rendered = vec![];
        for (idx, tab_idx) in visible_idxs[start..].iter().copied().enumerate() {
            let width = self.tabs[tab_idx].width();
            if width > width_left {
                break;
            }
            rendered.push(tab_idx);
            width_left -= width;
            if idx != visible_idxs.len() - start - 1 {
                width_left -= 1;
            }
        }

        rendered
    }

    fn ensure_tab_visible(&mut self, tab_idx: usize) {
        if !self.tabs[tab_idx].is_visible() {
            return;
        }

        let Some(tab_offset) = self.visible_tab_offset(tab_idx) else {
            return;
        };

        let tab_width = self.tabs[tab_idx].width();
        let left_arrow_width = if self.h_scroll > 0 { 2 } else { 0 };
        let right_arrow_width = if self.draw_right_arrow() { 2 } else { 0 };
        let visible_width = self.width - left_arrow_width - right_arrow_width;
        if tab_offset < self.h_scroll {
            self.h_scroll = tab_offset;
        } else if tab_offset + tab_width > self.h_scroll + visible_width {
            self.h_scroll = tab_offset;
        }
    }

    pub fn draw(&mut self) {
        self.tb.clear();

        if self.height < 2 {
            return;
        }

        self.tabs[self.active_idx]
            .widget
            .draw(&mut self.tb, &self.colors, 0, 0);

        // decide whether we need to draw left/right arrows in tab bar
        let left_arr = self.draw_left_arrow();
        let right_arr = self.draw_right_arrow();

        let rendered_tabs = self.rendered_tabs();

        let mut pos_x: i32 = 0;
        if left_arr {
            let hidden_left = self.visible_tabs_before_offset(self.h_scroll);
            let tabs = hidden_left
                .iter()
                .map(|idx| &self.tabs[*idx])
                .collect::<Vec<&Tab>>();
            let style = tabs
                .iter()
                .map(|tab| tab.style)
                .max()
                .map(|style| tab_style(style, &self.colors))
                .unwrap_or(self.colors.tab_normal);
            self.tb
                .change_cell(pos_x, self.height - 1, LEFT_ARROW, style.fg, style.bg);
            pos_x += 2;
        }

        // Debugging
        // debug!("number of tabs to draw: {}", tab_right - tab_left);
        // debug!("left_arr: {}, right_arr: {}", left_arr, right_arr);

        // finally draw the tabs
        for tab_idx in rendered_tabs.iter().copied() {
            let tab = &self.tabs[tab_idx];
            tab.draw(
                &mut self.tb,
                &self.colors,
                pos_x,
                self.height - 1,
                self.active_idx == tab_idx,
            );
            pos_x += tab.width() + 1; // +1 for margin
        }

        if right_arr {
            let rendered_last = rendered_tabs.last().copied();
            let after_rendered = match rendered_last {
                None => vec![],
                Some(last) => {
                    let mut seen_last = false;
                    self.visible_tab_idxs()
                        .into_iter()
                        .filter(|idx| {
                            if seen_last {
                                true
                            } else {
                                seen_last = *idx == last;
                                false
                            }
                        })
                        .collect::<Vec<usize>>()
                }
            };
            let tabs = after_rendered
                .iter()
                .map(|idx| &self.tabs[*idx])
                .collect::<Vec<&Tab>>();
            let style = tabs
                .iter()
                .map(|tab| tab.style)
                .max()
                .map(|style| tab_style(style, &self.colors))
                .unwrap_or(self.colors.tab_normal);
            self.tb
                .change_cell(pos_x, self.height - 1, RIGHT_ARROW, style.fg, style.bg);
        }

        self.tb.present();
    }

    ////////////////////////////////////////////////////////////////////////////
    // Moving between tabs, horizontal scroll updates

    fn select_tab(&mut self, tab_idx: usize) {
        self.active_idx = tab_idx;
        self.ensure_tab_visible(tab_idx);
        self.tabs[self.active_idx].set_style(TabStyle::Normal);
    }

    pub(crate) fn next_tab(&mut self) {
        self.next_tab_();
        self.tabs[self.active_idx].set_style(TabStyle::Normal);
    }

    pub(crate) fn prev_tab(&mut self) {
        self.prev_tab_();
        self.tabs[self.active_idx].set_style(TabStyle::Normal);
    }

    /// After closing a tab scroll left if there is space on the right and we can fit more tabs
    /// from the left into the visible part of the tab bar.
    fn fix_scroll_after_close(&mut self) {
        if self.visible_tab_idxs().is_empty() {
            self.h_scroll = 0;
            return;
        }

        let max_scroll = (self.visible_tabs_width() - self.width).max(0);
        self.h_scroll = self.h_scroll.min(max_scroll);
        self.ensure_tab_visible(self.active_idx);
    }

    pub(crate) fn switch(&mut self, string: &str) {
        let mut next_idx = self.active_idx;
        for (tab_idx, tab) in self.tabs.iter().enumerate() {
            if !tab.is_visible() {
                continue;
            }
            match tab.src {
                MsgSource::Serv { ref serv } => {
                    if serv.contains(string) {
                        next_idx = tab_idx;
                        break;
                    }
                }
                MsgSource::Chan { ref chan, .. } => {
                    // TODO: Case sensitive matching here is not ideal
                    if chan.display().contains(string) {
                        next_idx = tab_idx;
                        break;
                    }
                }
                MsgSource::User { ref nick, .. } => {
                    if nick.contains(string) {
                        next_idx = tab_idx;
                        break;
                    }
                }
            }
        }
        if next_idx != self.active_idx {
            self.select_tab(next_idx);
        }
    }

    fn next_tab_(&mut self) {
        let visible_idxs = self.visible_tab_idxs();
        if visible_idxs.is_empty() {
            return;
        }
        let next_idx = match visible_idxs.iter().position(|idx| *idx == self.active_idx) {
            Some(pos) if pos == visible_idxs.len() - 1 => {
                self.h_scroll = 0;
                visible_idxs[0]
            }
            Some(pos) => visible_idxs[pos + 1],
            None => visible_idxs
                .iter()
                .copied()
                .find(|idx| *idx > self.active_idx)
                .unwrap_or_else(|| {
                    self.h_scroll = 0;
                    visible_idxs[0]
                }),
        };
        self.active_idx = next_idx;
        self.ensure_tab_visible(next_idx);
    }

    fn prev_tab_(&mut self) {
        let visible_idxs = self.visible_tab_idxs();
        if visible_idxs.is_empty() {
            return;
        }
        let prev_idx = match visible_idxs.iter().position(|idx| *idx == self.active_idx) {
            Some(0) => visible_idxs[visible_idxs.len() - 1],
            Some(pos) => visible_idxs[pos - 1],
            None => visible_idxs
                .iter()
                .rev()
                .copied()
                .find(|idx| *idx < self.active_idx)
                .unwrap_or(visible_idxs[visible_idxs.len() - 1]),
        };
        if prev_idx == visible_idxs[0] {
            self.h_scroll = 0;
        }
        self.active_idx = prev_idx;
        self.ensure_tab_visible(prev_idx);
    }

    fn move_tab_left(&mut self) {
        if self.active_idx == 0 {
            return;
        }
        if self.is_server_tab(self.active_idx) {
            // move all server tabs
            let (left, right) = self.server_tab_range(self.active_idx);
            if left > 0 {
                let mut insert_idx = left - 1;
                while insert_idx > 0 && !self.is_server_tab(insert_idx) {
                    insert_idx -= 1;
                }
                let to_move: Vec<Tab> = self.tabs.drain(left..right).collect();
                self.tabs.splice(insert_idx..insert_idx, to_move);
                self.select_tab(insert_idx);
            }
        } else if !self.is_server_tab(self.active_idx - 1) {
            let tab = self.tabs.remove(self.active_idx);
            self.tabs.insert(self.active_idx - 1, tab);
            let active_idx = self.active_idx - 1;
            self.select_tab(active_idx);
        }
    }

    fn move_tab_right(&mut self) {
        if self.active_idx == self.tabs.len() - 1 {
            return;
        }
        if self.is_server_tab(self.active_idx) {
            // move all server tabs
            let (left, right) = self.server_tab_range(self.active_idx);
            if right < self.tabs.len() {
                let right_next = self.server_tab_range(right).1;
                let insert_idx = right_next - (right - left);
                let to_move: Vec<Tab> = self.tabs.drain(left..right).collect();
                self.tabs.splice(insert_idx..insert_idx, to_move);
                self.select_tab(insert_idx);
            }
        } else if !self.is_server_tab(self.active_idx + 1) {
            let tab = self.tabs.remove(self.active_idx);
            self.tabs.insert(self.active_idx + 1, tab);
            let active_idx = self.active_idx + 1;
            self.select_tab(active_idx);
        }
    }

    fn go_to_tab(&mut self, c: char) {
        let visible_idxs = self.visible_tab_idxs();
        if visible_idxs.is_empty() {
            return;
        }
        match c.to_digit(10) {
            Some(i) => {
                let new_tab_idx: usize = if i as usize > visible_idxs.len() || i == 0 {
                    visible_idxs[visible_idxs.len() - 1]
                } else {
                    visible_idxs[i as usize - 1]
                };
                self.select_tab(new_tab_idx);
                self.tabs[self.active_idx].set_style(TabStyle::Normal);
            }
            None => {
                // multiple tabs can have same switch character so scan
                // forwards instead of starting from the first tab
                let start = visible_idxs
                    .iter()
                    .position(|idx| *idx == self.active_idx)
                    .unwrap_or(0);
                for i in 1..=visible_idxs.len() {
                    let idx = visible_idxs[(start + i) % visible_idxs.len()];
                    if self.tabs[idx].switch == Some(c) {
                        self.select_tab(idx);
                        break;
                    }
                }
            }
        }
    }

    ////////////////////////////////////////////////////////////////////////////
    // Interfacing with tabs

    fn apply_to_target<F>(&mut self, target: &MsgTarget, can_create_tab: bool, f: &mut F)
    where
        F: FnMut(&mut Tab, bool),
    {
        // Creating a vector just to make borrow checker happy (I can't have a Vec<&mut Tab>)
        // I need to collect tabs here because of the "create if not exists" logic.
        // (see `target_idxs.is_empty()` below)
        let mut target_idxs: Vec<usize> = Vec::with_capacity(1);

        match *target {
            MsgTarget::Server { serv } => {
                for (tab_idx, tab) in self.tabs.iter().enumerate() {
                    if let MsgSource::Serv { serv: ref serv_ } = tab.src
                        && serv == serv_
                    {
                        target_idxs.push(tab_idx);
                        break;
                    }
                }
            }

            MsgTarget::Chan { serv, chan } => {
                for (tab_idx, tab) in self.tabs.iter().enumerate() {
                    if let MsgSource::Chan {
                        serv: ref serv_,
                        chan: ref chan_,
                    } = tab.src
                        && serv == serv_
                        && chan == chan_
                    {
                        target_idxs.push(tab_idx);
                        break;
                    }
                }
            }

            MsgTarget::User { serv, nick } => {
                for (tab_idx, tab) in self.tabs.iter().enumerate() {
                    if let MsgSource::User {
                        serv: ref serv_,
                        nick: ref nick_,
                    } = tab.src
                        && serv == serv_
                        && nick == nick_
                    {
                        target_idxs.push(tab_idx);
                        break;
                    }
                }
            }

            MsgTarget::AllServTabs { serv } => {
                for (tab_idx, tab) in self.tabs.iter().enumerate() {
                    if tab.src.serv_name() == serv {
                        target_idxs.push(tab_idx);
                    }
                }
            }

            MsgTarget::CurrentTab => {
                target_idxs.push(self.active_idx);
            }
        }

        // Create server/chan/user tab when necessary
        if target_idxs.is_empty()
            && can_create_tab
            && let Some(idx) = self.maybe_create_tab(target)
        {
            target_idxs.push(idx);
        }

        for tab_idx in target_idxs {
            f(&mut self.tabs[tab_idx], self.active_idx == tab_idx);
        }
    }

    fn maybe_create_tab(&mut self, target: &MsgTarget) -> Option<usize> {
        match *target {
            MsgTarget::Server { serv } | MsgTarget::AllServTabs { serv } => {
                self.new_server_tab(serv, None)
            }

            MsgTarget::Chan { serv, chan } => self.new_chan_tab(serv, chan),

            MsgTarget::User { serv, nick } => self.new_user_tab(serv, nick),

            _ => None,
        }
    }

    pub(crate) fn set_tab_style(&mut self, style: TabStyle, target: &MsgTarget) {
        let ignore = self
            .get_tab_config(
                target.serv_name().unwrap_or_default(),
                target.chan_or_user_name(),
            )
            .ignore
            .unwrap_or_default();
        self.apply_to_target(target, false, &mut |tab: &mut Tab, is_active: bool| {
            if (!ignore || style != TabStyle::JoinOrPart) && tab.style < style && !is_active {
                tab.set_style(style);
            }
        });
    }

    /// An error message coming from Tiny, probably because of a command error
    /// etc. Those are not timestamped and not logged.
    pub(crate) fn add_client_err_msg(&mut self, msg: &str, target: &MsgTarget) {
        self.apply_to_target(target, true, &mut |tab: &mut Tab, _| {
            tab.widget.add_client_err_msg(msg);
        });
    }

    /// A notify message coming from tiny, usually shows a response of a command
    /// e.g. "Notifications enabled".
    pub(crate) fn add_client_notify_msg(&mut self, msg: &str, target: &MsgTarget) {
        self.apply_to_target(target, false, &mut |tab: &mut Tab, _| {
            tab.widget.add_client_notify_msg(msg);
        });
    }

    /// A message from client, usually just to indidate progress, e.g.
    /// "Connecting...". Not timestamed and not logged.
    pub(crate) fn add_client_msg(&mut self, msg: &str, target: &MsgTarget) {
        self.apply_to_target(target, false, &mut |tab: &mut Tab, _| {
            tab.widget.add_client_msg(msg);
        });
    }

    /// privmsg is a message coming from a server or client. Shown with sender's
    /// nick/name and receive time and logged.
    pub(crate) fn add_privmsg(
        &mut self,
        sender: &str,
        msg: &str,
        ts: Tm,
        target: &MsgTarget,
        highlight: bool,
        is_action: bool,
    ) {
        let mut notifier = if let Some(serv) = target.serv_name() {
            self.get_tab_config(serv, target.chan_or_user_name())
                .notify
                .unwrap_or_default()
        } else {
            Notifier::default()
        };
        self.apply_to_target(target, true, &mut |tab: &mut Tab, _| {
            tab.widget
                .add_privmsg(sender, msg, Timestamp::from(ts), highlight, is_action);
            let nick = tab.widget.get_nick();
            if let Some(nick_) = nick {
                notifier.notify_privmsg(sender, msg, target, &nick_, highlight);
            }
        });
    }

    /// A message without any explicit sender info. Useful for e.g. in server
    /// and debug log tabs. Timestamped and logged.
    pub fn add_msg(&mut self, msg: &str, ts: Tm, target: &MsgTarget) {
        self.apply_to_target(target, true, &mut |tab: &mut Tab, _| {
            tab.widget.add_msg(msg, Timestamp::from(ts));
        });
    }

    /// Error messages related with the protocol - e.g. can't join a channel,
    /// nickname is in use etc. Timestamped and logged.
    pub(crate) fn add_err_msg(&mut self, msg: &str, ts: Tm, target: &MsgTarget) {
        self.apply_to_target(target, true, &mut |tab: &mut Tab, _| {
            tab.widget.add_err_msg(msg, Timestamp::from(ts));
        });
    }

    pub(crate) fn set_topic(&mut self, title: &str, ts: Tm, serv: &str, chan: &ChanNameRef) {
        let target = MsgTarget::Chan { serv, chan };
        self.apply_to_target(&target, false, &mut |tab: &mut Tab, _| {
            tab.widget.show_topic(title, Timestamp::from(ts));
        });
    }

    pub(crate) fn clear_nicks(&mut self, serv: &str) {
        let target = MsgTarget::AllServTabs { serv };
        self.apply_to_target(&target, false, &mut |tab: &mut Tab, _| {
            tab.widget.clear_nicks();
        });
    }

    pub(crate) fn add_nick(&mut self, nick: &str, ts: Option<Tm>, target: &MsgTarget) {
        let ignore = self
            .get_tab_config(
                target.serv_name().unwrap_or_default(),
                target.chan_or_user_name(),
            )
            .ignore
            .unwrap_or_default();

        self.apply_to_target(target, false, &mut |tab: &mut Tab, _| {
            tab.widget.join(nick, ts.map(Timestamp::from), ignore);
        });
    }

    pub(crate) fn remove_nick(&mut self, nick: &str, ts: Option<Tm>, target: &MsgTarget) {
        let ignore = self
            .get_tab_config(
                target.serv_name().unwrap_or_default(),
                target.chan_or_user_name(),
            )
            .ignore
            .unwrap_or_default();

        self.apply_to_target(target, false, &mut |tab: &mut Tab, _| {
            tab.widget.part(nick, ts.map(Timestamp::from), ignore);
        });
    }

    pub(crate) fn rename_nick(
        &mut self,
        old_nick: &str,
        new_nick: &str,
        ts: Tm,
        target: &MsgTarget,
    ) {
        self.apply_to_target(target, false, &mut |tab: &mut Tab, _| {
            tab.widget.nick(old_nick, new_nick, Timestamp::from(ts));
            // TODO: Does this actually rename the tab?
            tab.update_source(&|src: &mut MsgSource| {
                if let MsgSource::User { ref mut nick, .. } = *src {
                    nick.clear();
                    nick.push_str(new_nick);
                }
            });
        });
    }

    pub(crate) fn set_nick(&mut self, serv: &str, new_nick: &str) {
        let target = MsgTarget::AllServTabs { serv };
        self.apply_to_target(&target, false, &mut |tab: &mut Tab, _| {
            tab.widget.set_nick(new_nick.to_owned())
        });
    }

    pub(crate) fn clear(&mut self, target: &MsgTarget) {
        self.apply_to_target(target, false, &mut |tab: &mut Tab, _| tab.widget.clear());
    }

    // TODO: Maybe remove this and add a `create: bool` field to MsgTarget::User
    pub(crate) fn user_tab_exists(&self, serv_: &str, nick_: &str) -> bool {
        for tab in &self.tabs {
            if let MsgSource::User { ref serv, ref nick } = tab.src
                && serv_ == serv
                && nick_ == nick
            {
                return true;
            }
        }
        false
    }

    #[cfg(test)]
    pub(crate) fn tab_lines_text(&self, target: &MsgTarget) -> Option<Vec<String>> {
        let mut target_idx = None;
        match *target {
            MsgTarget::Server { serv } => target_idx = self.find_serv_tab_idx(serv),
            MsgTarget::Chan { serv, chan } => target_idx = self.find_chan_tab_idx(serv, chan),
            MsgTarget::User { serv, nick } => target_idx = self.find_user_tab_idx(serv, nick),
            MsgTarget::AllServTabs { .. } | MsgTarget::CurrentTab => {}
        }
        target_idx.map(|idx| self.tabs[idx].widget.lines_text())
    }

    pub(crate) fn set_notifier(&mut self, notifier: Notifier, target: &MsgTarget) {
        if let Some(serv) = target.serv_name()
            && let Some(config) = self.tab_configs.get_mut(serv, target.chan_or_user_name())
        {
            config.notify = Some(notifier);
        }
    }

    pub(crate) fn show_notify_mode(&mut self, src: &MsgSource) {
        let notifier = self
            .get_tab_config(src.serv_name(), src.chan_name())
            .notify
            .unwrap_or_default();
        self.apply_to_target(&src.to_target(), false, &mut |tab: &mut Tab, _| {
            let msg = match notifier {
                Notifier::Off => "Notifications are off",
                Notifier::Mentions => "Notifications enabled for mentions",
                Notifier::Messages => "Notifications enabled for all messages",
            };
            tab.widget.add_client_notify_msg(msg);
        });
    }

    ////////////////////////////////////////////////////////////////////////////
    // Helpers

    fn find_serv_tab_idx(&self, serv_: &str) -> Option<usize> {
        for (tab_idx, tab) in self.tabs.iter().enumerate() {
            if let MsgSource::Serv { ref serv } = tab.src
                && serv_ == serv
            {
                return Some(tab_idx);
            }
        }
        None
    }

    fn find_chan_tab_idx(&self, serv_: &str, chan_: &ChanNameRef) -> Option<usize> {
        for (tab_idx, tab) in self.tabs.iter().enumerate() {
            if let MsgSource::Chan { ref serv, ref chan } = tab.src
                && serv_ == serv
                && chan_ == chan
            {
                return Some(tab_idx);
            }
        }
        None
    }

    fn find_user_tab_idx(&self, serv_: &str, nick_: &str) -> Option<usize> {
        for (tab_idx, tab) in self.tabs.iter().enumerate() {
            if let MsgSource::User { ref serv, ref nick } = tab.src
                && serv_ == serv
                && nick_ == nick
            {
                return Some(tab_idx);
            }
        }
        None
    }

    /// Index of the last tab with the given server name.
    fn find_last_serv_tab_idx(&self, serv: &str) -> Option<usize> {
        for (tab_idx, tab) in self.tabs.iter().enumerate().rev() {
            if tab.src.serv_name() == serv {
                return Some(tab_idx);
            }
        }
        None
    }

    fn is_server_tab(&self, idx: usize) -> bool {
        match self.tabs[idx].src {
            MsgSource::Serv { .. } => true,
            MsgSource::Chan { .. } | MsgSource::User { .. } => false,
        }
    }

    /// Given a tab index return range of tabs for the server of this tab.
    fn server_tab_range(&self, idx: usize) -> (usize, usize) {
        debug_assert!(idx < self.tabs.len());
        let mut left = idx;
        while !self.is_server_tab(left) {
            left -= 1;
        }
        let mut right = idx + 1;
        while right < self.tabs.len() && !self.is_server_tab(right) {
            right += 1;
        }
        (left, right)
    }
}
