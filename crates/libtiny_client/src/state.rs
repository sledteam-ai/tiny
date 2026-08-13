#![allow(clippy::get_first, clippy::zero_prefixed_literal)]

use crate::{Cmd, Event, ServerInfo};
use crate::{SASLAuth, utils};
use libtiny_common::{ChanName, ChanNameRef};
use libtiny_wire as wire;
use libtiny_wire::{Msg, Pfx};

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use tokio::sync::mpsc::{Receiver, Sender};
use tokio::time::{Duration, timeout};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

#[derive(Clone)]
pub struct State {
    inner: Rc<RefCell<StateInner>>,
}

impl State {
    pub(crate) fn new(server_info: ServerInfo) -> State {
        State {
            inner: Rc::new(RefCell::new(StateInner::new(server_info))),
        }
    }

    pub(crate) fn reset(&self) {
        self.inner.borrow_mut().reset()
    }

    pub(crate) fn send_ping(&self, snd_irc_msg: &mut Sender<String>) {
        self.inner.borrow_mut().send_ping(snd_irc_msg)
    }

    pub(crate) fn update(
        &self,
        msg: &mut Msg,
        snd_ev: &mut Sender<Event>,
        snd_irc_msg: &mut Sender<String>,
    ) {
        self.inner.borrow_mut().update(msg, snd_ev, snd_irc_msg);
    }

    pub(crate) fn introduce(&self, snd_irc_msg: &mut Sender<String>) {
        self.inner.borrow_mut().introduce(snd_irc_msg)
    }

    // FIXME: This allocates a new String
    pub(crate) fn get_nick(&self) -> String {
        self.inner.borrow().current_nick.clone()
    }

    // FIXME: Maybe use RwLock instead of Mutex
    pub(crate) fn is_nick_accepted(&self) -> bool {
        self.inner.borrow().nick_accepted
    }

    pub(crate) fn get_usermask(&self) -> Option<String> {
        self.inner.borrow().usermask.clone()
    }

    pub(crate) fn multiline_limits(&self) -> Option<(usize, usize)> {
        self.inner.borrow().multiline_limits()
    }

    pub(crate) fn next_batch_reference(&self) -> String {
        self.inner.borrow_mut().next_batch_reference()
    }

    pub(crate) fn set_away(&self, msg: Option<&str>) {
        self.inner.borrow_mut().away_status = msg.map(str::to_owned);
    }

    pub(crate) fn get_chan_nicks(&self, chan: &ChanNameRef) -> Vec<String> {
        self.inner.borrow().get_chan_nicks(chan)
    }

    pub(crate) fn leave_channel(
        &self,
        msg_chan: &mut Sender<Cmd>,
        chan: &ChanNameRef,
        reason: Option<String>,
    ) {
        self.inner
            .borrow_mut()
            .leave_channel(msg_chan, chan, reason)
    }

    pub(crate) fn kill_join_tasks(&self) {
        self.inner.borrow_mut().kill_join_tasks();
    }
}

struct StateInner {
    /// Nicks to try, in this order.
    nicks: Vec<String>,

    /// NickServ password
    nickserv_ident: Option<String>,

    /// An index to `nicks`. When out of range we add `current_nick_idx - nicks.length()`
    /// underscores to the last nick in `nicks`
    current_nick_idx: usize,

    /// A cache of current nick, to avoid allocating new nicks when inventing new nicks with
    /// underscores.
    current_nick: String,

    /// Currently joined channels. Every channel we join will be added here to be able to re-join
    /// automatically on reconnect and channels we leave will be removed.
    ///
    /// This would be a `HashMap<String, ..>` but we want to join channels in the order the user
    /// specified, so using a `Vec`.
    ///
    /// TODO: I'm not sure if this is necessary. Why not just create channel tabs in the specified
    /// order, in TUI?
    chans: Vec<Chan>,

    /// Away reason if away mode is on. `None` otherwise.
    away_status: Option<String>,

    /// servername to be used in PING messages. Read from 002 RPL_YOURHOST. `None` until 002.
    servername: Option<String>,

    /// Our usermask given by the server. Currently only parsed after a JOIN, reply 396.
    ///
    /// Note that RPL_USERHOST (302) does not take cloaks into account, so we don't parse USERHOST
    /// responses to set this field.
    usermask: Option<String>,

    /// Do we have a nick yet? Try another nick on ERR_NICKNAMEINUSE (433) until we've got a nick.
    nick_accepted: bool,

    /// Server information
    server_info: ServerInfo,

    /// Connection-scoped IRCv3 capability negotiation state.
    cap_ls: Vec<String>,
    advertised_multiline_limits: Option<(usize, usize)>,
    batch_negotiated: bool,
    multiline_negotiated: bool,
    introduced: bool,

    /// Monotonically increasing on this client, including across reconnects.
    next_batch_reference: u64,
}

#[derive(Debug)]
struct Chan {
    /// Name of the channel
    name: ChanName,
    /// Set of nicknames in channel
    nicks: HashSet<String>,
    /// Channel joined state
    join_state: JoinState,
    /// Join attempts
    join_attempts: u8,
}

/// State transitions:
///    NotJoined -> Joining: When we get 477 for the channel
///    NotJoined -> Joined: When we get a JOIN message for the channel on first attempt
///    Joining -> Joined: When we get a JOIN message for the channel
///    Joining -> NotJoined: Connection reset
///    Joined -> NotJoined: Connection reset
///    Joined -> Joining: Unexpected/Invalid state
#[derive(Debug)]
enum JoinState {
    /// Initial state for Chan
    NotJoined,
    /// In the process of joining the channel
    Joining {
        /// Sender to kill the retry task if tab is closed
        stop_task: Sender<()>,
    },
    /// Successfully joined the channel
    Joined,
}

const MAX_JOIN_RETRIES: u8 = 3;

impl Chan {
    fn new(name: ChanName) -> Chan {
        Chan {
            name,
            nicks: HashSet::new(),
            join_state: JoinState::NotJoined,
            join_attempts: MAX_JOIN_RETRIES,
        }
    }

    fn with_nicks(name: ChanName, nicks: HashSet<String>) -> Chan {
        Chan {
            name,
            nicks,
            join_state: JoinState::NotJoined,
            join_attempts: MAX_JOIN_RETRIES,
        }
    }

    fn reset(&mut self) {
        self.nicks.clear();
        self.join_state = JoinState::NotJoined;
        self.join_attempts = MAX_JOIN_RETRIES;
    }

    fn set_joining(&mut self, stop_task: Sender<()>) {
        self.join_state = JoinState::Joining { stop_task }
    }

    /// Uses a retry.
    /// Returns number of retries left or None.
    fn retry_join(&mut self) -> Option<u8> {
        match self.join_attempts {
            0 => None,
            _ => {
                self.join_attempts -= 1;
                Some(self.join_attempts)
            }
        }
    }
}

impl StateInner {
    fn new(server_info: ServerInfo) -> StateInner {
        let current_nick = server_info.nicks[0].to_owned();
        let chans = server_info
            .auto_join
            .iter()
            .map(|s| Chan::new(s.to_owned()))
            .collect();
        StateInner {
            nicks: server_info.nicks.clone(),
            nickserv_ident: server_info.nickserv_ident.clone(),
            current_nick_idx: 0,
            current_nick,
            chans,
            away_status: None,
            servername: None,
            usermask: None,
            nick_accepted: false,
            cap_ls: Vec::new(),
            advertised_multiline_limits: None,
            batch_negotiated: false,
            multiline_negotiated: false,
            introduced: false,
            next_batch_reference: 1,
            server_info,
        }
    }

    fn reset(&mut self) {
        self.nick_accepted = false;
        self.nicks.clone_from(&self.server_info.nicks);
        self.current_nick_idx = 0;
        self.current_nick.clone_from(&self.nicks[0]);
        // Only reset the values here; the key set will be used to join channels
        for chan in &mut self.chans {
            chan.reset();
        }
        self.servername = None;
        self.usermask = None;
        self.cap_ls.clear();
        self.advertised_multiline_limits = None;
        self.batch_negotiated = false;
        self.multiline_negotiated = false;
        self.introduced = false;
    }

    fn send_ping(&mut self, snd_irc_msg: &mut Sender<String>) {
        if let Some(ref servername) = self.servername {
            snd_irc_msg.try_send(wire::ping(servername)).unwrap();
        }
    }

    fn introduce(&mut self, snd_irc_msg: &mut Sender<String>) {
        if self.introduced {
            return;
        }
        self.introduced = true;
        if let Some(ref pass) = self.server_info.pass {
            snd_irc_msg.try_send(wire::pass(pass)).unwrap();
        }
        snd_irc_msg
            .try_send(wire::nick(&self.current_nick))
            .unwrap();
        let user = self.server_info.user.as_ref().unwrap_or(&self.nicks[0]);
        snd_irc_msg
            .try_send(wire::user(user, &self.server_info.realname))
            .unwrap();
    }

    fn multiline_limits(&self) -> Option<(usize, usize)> {
        if self.batch_negotiated && self.multiline_negotiated {
            self.advertised_multiline_limits
        } else {
            None
        }
    }

    fn next_batch_reference(&mut self) -> String {
        let reference = format!("tiny-{}", self.next_batch_reference);
        self.next_batch_reference = self.next_batch_reference.wrapping_add(1).max(1);
        reference
    }

    fn get_next_nick(&mut self) -> &str {
        self.current_nick_idx += 1;
        // debug!("current_nick_idx: {}", self.current_nick_idx);
        if self.current_nick_idx >= self.nicks.len() {
            let n_underscores = self.current_nick_idx - self.nicks.len() + 1;
            let mut new_nick = self.nicks.last().unwrap().to_string();
            for _ in 0..n_underscores {
                new_nick.push('_');
            }
            self.current_nick = new_nick;
        } else {
            self.current_nick
                .clone_from(&self.nicks[self.current_nick_idx]);
        }
        &self.current_nick
    }

    fn update(
        &mut self,
        msg: &mut Msg,
        snd_ev: &mut Sender<Event>,
        snd_irc_msg: &mut Sender<String>,
    ) {
        let Msg { pfx, cmd } = msg;

        use wire::Cmd::*;
        match cmd {
            // PING: Send PONG
            PING { server } => {
                snd_irc_msg.try_send(wire::pong(server)).unwrap();
            }

            // JOIN: If this is us then update usermask if possible, create the channel state. If
            // someone else add the nick to channel.
            JOIN { chan } => {
                match pfx {
                    Some(Pfx::User { nick, user }) if nick == &self.current_nick => {
                        // Set usermask
                        let usermask = format!("{nick}!{user}");
                        self.usermask = Some(usermask);
                    }
                    _ => {}
                }

                match pfx {
                    Some(Pfx::User { nick, .. }) | Some(Pfx::Ambiguous(nick)) => {
                        if nick == &self.current_nick {
                            // We joined a channel, initialize channel state
                            match utils::find_idx(&self.chans, |c| &c.name == chan) {
                                None => {
                                    let mut chan = Chan::new(chan.to_owned());
                                    // Since nick was found in the prefix, we are in the channel
                                    chan.join_state = JoinState::Joined;
                                    self.chans.push(chan);
                                }
                                Some(chan_idx) => {
                                    // This happens because we initialize channel states for channels
                                    // that we will join on connection when the client is first created
                                    let chan = &mut self.chans[chan_idx];
                                    chan.join_state = JoinState::Joined;
                                    chan.nicks.clear();
                                }
                            }
                        } else {
                            match utils::find_idx(&self.chans, |c| &c.name == chan) {
                                Some(chan_idx) => {
                                    self.chans[chan_idx]
                                        .nicks
                                        .insert(wire::drop_nick_prefix(nick).to_owned());
                                }
                                None => {
                                    debug!("Can't find channel state for JOIN: {cmd:?}");
                                }
                            }
                        }
                    }
                    Some(Pfx::Server(_)) | None => {}
                }
            }

            // PART: If this is us remove the channel state. Otherwise remove the nick from the
            // channel.
            PART { chan, .. } => match pfx {
                Some(Pfx::User { nick, .. }) | Some(Pfx::Ambiguous(nick)) => {
                    if nick == &self.current_nick {
                        match utils::find_idx(&self.chans, |c| &c.name == chan) {
                            None => {
                                debug!("Can't find channel state: {}", chan.display());
                            }
                            Some(chan_idx) => {
                                self.chans.remove(chan_idx);
                            }
                        }
                    } else {
                        match utils::find_idx(&self.chans, |c| &c.name == chan) {
                            Some(chan_idx) => {
                                self.chans[chan_idx]
                                    .nicks
                                    .remove(wire::drop_nick_prefix(nick));
                            }
                            None => {
                                debug!("Can't find channel state for PART: {cmd:?}");
                            }
                        }
                    }
                }
                Some(Pfx::Server(_)) | None => {}
            },

            // QUIT: Update the `chans` field for the channels that the user was in
            QUIT { chans, .. } => {
                let nick = match pfx {
                    Some(Pfx::User { nick, .. }) | Some(Pfx::Ambiguous(nick)) => nick,
                    Some(Pfx::Server(_)) | None => {
                        return;
                    }
                };
                for chan in self.chans.iter_mut() {
                    if chan.nicks.contains(nick) {
                        chans.push(chan.name.to_owned());
                        chan.nicks.remove(nick);
                    }
                }
            }

            // 396: Try to set usermask.
            Reply { num: 396, params } => {
                // :hobana.freenode.net 396 osa1 haskell/developer/osa1
                // :is now your hidden host (set by services.)
                if params.len() == 3 {
                    let usermask =
                        format!("{}!~{}@{}", self.current_nick, self.nicks[0], params[1]);
                    self.usermask = Some(usermask);
                }
            }

            // Reply 477 when user needs to be identified with NickServ to join a channel
            // ex. Reply { num: 477, params: ["<your_nick>", "<channel name>", "<Server reply message>"] }
            Reply { num: 477, params } => {
                // Only try to automatically rejoin if nickserv_ident is configured
                if let (Some(channel), Some(msg_477)) = (params.get(1), params.get(2)) {
                    let channel = ChanNameRef::new(channel);
                    snd_ev
                        .try_send(Event::Msg(wire::Msg {
                            pfx: pfx.clone(),
                            cmd: wire::Cmd::PRIVMSG {
                                ctcp: None,
                                is_notice: true,
                                msg: msg_477.clone(),
                                target: wire::MsgTarget::Chan(channel.to_owned()),
                            },
                        }))
                        .unwrap();
                    // Get channel name from params
                    if self.nickserv_ident.is_some() {
                        // Helper for creating an event
                        let create_message = |msg: String| Event::ChannelJoinError {
                            chan: channel.to_owned(),
                            msg,
                        };
                        // Find channel in self.chans
                        if let Some(idx) = utils::find_idx(&self.chans, |c| c.name == *channel) {
                            let chan = &mut self.chans[idx];
                            // Retry joining channel if retries are available
                            if let Some(retries) = chan.retry_join() {
                                let retry_msg = format!(
                                    "Attempting to rejoin {} in 10 seconds... ({}/{})",
                                    channel.display(),
                                    MAX_JOIN_RETRIES - retries,
                                    MAX_JOIN_RETRIES
                                );
                                snd_ev.try_send(create_message(retry_msg)).unwrap();
                                let snd_irc_msg = snd_irc_msg.clone();
                                // Spawn task and delay rejoin to give NickServ time to identify nick
                                let (snd_abort, rcv_abort) = tokio::sync::mpsc::channel(1);
                                match &mut chan.join_state {
                                    JoinState::NotJoined => chan.set_joining(snd_abort),
                                    JoinState::Joining { stop_task, .. } => *stop_task = snd_abort,
                                    JoinState::Joined => {
                                        error!("Unexpected JoinState for channel.");
                                        return;
                                    }
                                }
                                tokio::task::spawn_local(retry_channel_join(
                                    channel.to_owned(),
                                    snd_irc_msg,
                                    rcv_abort,
                                ));
                            } else {
                                // No more retries
                                let no_retries_msg =
                                    format!("Unable to join {}.", channel.display());
                                snd_ev.try_send(create_message(no_retries_msg)).unwrap();
                            }
                        } else {
                            warn!("Could not find channel in server state channel list.");
                        }
                    } else {
                        debug!("Received 477 reply but nickserv_ident is not configured.");
                    }
                } else {
                    warn!("Could not parse 477 reply: {cmd:?}");
                }
            }

            // 302: Try to set usermask.
            Reply { num: 302, params } => {
                // 302 RPL_USERHOST
                // :ircd.stealth.net 302 yournick :syrk=+syrk@millennium.stealth.net
                //
                // We know there will be only one nick because /userhost cmd sends
                // one parameter (our nick)
                //
                // Example args: ["osa1", "osa1=+omer@moz-s8a.9ac.93.91.IP "]

                let param = &params[1];
                match param.find('=') {
                    None => {
                        warn!("Could not parse 302 RPL_USERHOST to set usermask.");
                    }
                    Some(mut i) => {
                        if param.as_bytes().get(i + 1) == Some(&b'+')
                            || param.as_bytes().get(i + 1) == Some(&b'-')
                        {
                            i += 1;
                        }
                        let usermask = param[i..].trim();
                        self.usermask = Some(usermask.to_owned());
                    }
                }
            }

            // RPL_WELCOME: Start introduction sequence and NickServ authentication.
            Reply { num: 001, .. } => {
                snd_ev.try_send(Event::Connected).unwrap();
                snd_ev
                    .try_send(Event::NickChange {
                        new_nick: self.current_nick.clone(),
                    })
                    .unwrap();
                self.nick_accepted = true;
                if let Some(ref pwd) = self.nickserv_ident {
                    snd_irc_msg
                        .try_send(wire::privmsg("NickServ", &format!("identify {pwd}")))
                        .unwrap();
                }
            }

            // RPL_YOURHOST: Set servername
            Reply { num: 002, params } => {
                // 002    RPL_YOURHOST
                //        "Your host is <servername>, running version <ver>"

                // An example <servername>: cherryh.freenode.net[149.56.134.238/8001]

                match parse_servername(pfx.as_ref(), params) {
                    None => {
                        error!("Could not parse server name in 002 RPL_YOURHOST message.");
                    }
                    Some(servername) => {
                        self.servername = Some(servername);
                    }
                }
            }

            // ERR_NICKNAMEINUSE: Try another nick if we don't have a nick yet.
            Reply { num: 433, .. } => {
                if !self.nick_accepted {
                    let new_nick = self.get_next_nick();
                    // debug!("new nick: {}", new_nick);
                    snd_ev
                        .try_send(Event::NickChange {
                            new_nick: new_nick.to_owned(),
                        })
                        .unwrap();
                    snd_irc_msg.try_send(wire::nick(new_nick)).unwrap();
                }
            }

            // NICK message sent from the server when our nick change request was successful
            NICK {
                nick: new_nick,
                chans,
            } => {
                match pfx {
                    Some(Pfx::User { nick: old_nick, .. }) | Some(Pfx::Ambiguous(old_nick)) => {
                        if old_nick == &self.current_nick {
                            snd_ev
                                .try_send(Event::NickChange {
                                    new_nick: new_nick.to_owned(),
                                })
                                .unwrap();

                            match utils::find_idx(&self.nicks, |nick| nick == new_nick) {
                                None => {
                                    self.nicks.push(new_nick.to_owned());
                                    self.current_nick_idx = self.nicks.len() - 1;
                                }
                                Some(nick_idx) => {
                                    self.current_nick_idx = nick_idx;
                                }
                            }

                            self.current_nick.clone_from(new_nick);

                            if let Some(ref pwd) = self.nickserv_ident {
                                snd_irc_msg
                                    .try_send(wire::privmsg("NickServ", &format!("identify {pwd}")))
                                    .unwrap();
                            }
                        }

                        // Rename the nick in channel states, also populate the chan list
                        for chan in &mut self.chans {
                            if chan.nicks.remove(old_nick) {
                                chan.nicks.insert(new_nick.to_owned());
                                chans.push(chan.name.to_owned());
                            }
                        }
                    }
                    Some(Pfx::Server(_)) | None => {}
                }
            }

            // RPL_ENDOFMOTD: Join channels, set away status
            Reply { num: 376, .. } => {
                if !self.chans.is_empty() {
                    let chans = self.chans.iter().map(|c| c.name.as_ref());
                    snd_irc_msg.try_send(wire::join(chans)).unwrap();
                }
                if self.away_status.is_some() {
                    snd_irc_msg
                        .try_send(wire::away(self.away_status.as_deref()))
                        .unwrap();
                }
            }

            // RPL_NAMREPLY: Set users in a channel
            Reply { num: 353, params } => {
                let chan = ChanNameRef::new(&params[2]);
                match utils::find_idx(&self.chans, |c| &c.name == chan) {
                    None => self.chans.push(Chan::with_nicks(
                        chan.to_owned(),
                        params[3]
                            .split_whitespace()
                            .map(|s| wire::drop_nick_prefix(s).to_owned())
                            .collect(),
                    )),
                    Some(idx) => {
                        let nick_set = &mut self.chans[idx].nicks;
                        for nick in params[3].split_whitespace() {
                            nick_set.insert(wire::drop_nick_prefix(nick).to_owned());
                        }
                    }
                }
            }

            // SASL authentication
            CAP {
                client: _,
                subcommand,
                continuation,
                params,
            } => match subcommand.as_ref() {
                "ACK" => {
                    for cap in params.iter() {
                        match cap_name(cap) {
                            "batch" => self.batch_negotiated = true,
                            "draft/multiline" => self.multiline_negotiated = true,
                            _ => {}
                        }
                    }
                    if params.iter().any(|cap| cap_name(cap) == "sasl") {
                        if let Some(sasl) = &self.server_info.sasl_auth {
                            let msg = match sasl {
                                SASLAuth::Plain { .. } => "PLAIN",
                                SASLAuth::External { .. } => "EXTERNAL",
                            };
                            snd_irc_msg.try_send(wire::authenticate(msg)).unwrap();
                        } else {
                            warn!("SASL AUTH not set but got SASL ACK");
                            snd_irc_msg.try_send(wire::cap_end()).unwrap();
                        }
                    } else {
                        snd_irc_msg.try_send(wire::cap_end()).unwrap();
                    }
                }
                "NAK" => {
                    for cap in params.iter() {
                        match cap_name(cap) {
                            "batch" => self.batch_negotiated = false,
                            "draft/multiline" => self.multiline_negotiated = false,
                            _ => {}
                        }
                    }
                    snd_irc_msg.try_send(wire::cap_end()).unwrap();
                }
                "LS" => {
                    self.cap_ls.extend(params.iter().cloned());
                    if *continuation {
                        return;
                    }

                    self.introduce(snd_irc_msg);
                    self.advertised_multiline_limits = self
                        .cap_ls
                        .iter()
                        .find_map(|cap| parse_multiline_limits(cap));

                    let has_cap = |name| self.cap_ls.iter().any(|cap| cap_name(cap) == name);
                    let mut requested = Vec::new();
                    if has_cap("batch") {
                        requested.push("batch");
                    }
                    if self.advertised_multiline_limits.is_some() {
                        requested.push("draft/multiline");
                    }
                    if self.server_info.sasl_auth.is_some() && has_cap("sasl") {
                        requested.push("sasl");
                    }

                    if requested.is_empty() {
                        snd_irc_msg.try_send(wire::cap_end()).unwrap();
                    } else {
                        snd_irc_msg.try_send(wire::cap_req(&requested)).unwrap();
                    }
                }
                "DEL" => {
                    for cap in params.iter() {
                        match cap_name(cap) {
                            "batch" => self.batch_negotiated = false,
                            "draft/multiline" => self.multiline_negotiated = false,
                            _ => {}
                        }
                    }
                }
                _ => {}
            },

            // https://ircv3.net/specs/extensions/sasl-3.1.html
            AUTHENTICATE { param } => {
                if param.as_str() == "+" {
                    // Empty AUTHENTICATE response; server accepted the specified SASL mechanism
                    if let Some(ref auth) = self.server_info.sasl_auth {
                        let msg = match auth {
                            SASLAuth::Plain { username, password } => {
                                let msg = format!("{username}\x00{username}\x00{password}");
                                use base64::engine::Engine;
                                base64::engine::general_purpose::STANDARD.encode(msg)
                            }
                            // Reply with an empty response (Empty responses are sent as "AUTHENTICATE +")
                            SASLAuth::External { .. } => "+".to_string(),
                        };
                        snd_irc_msg.try_send(wire::authenticate(&msg)).unwrap();
                    }
                }
            }

            Reply { num: 903, .. } | Reply { num: 904, .. } => {
                // 903: RPL_SASLSUCCESS, 904: ERR_SASLFAIL
                snd_irc_msg.try_send(wire::cap_end()).unwrap();
            }

            // Ignore the rest
            _ => {}
        }
    }

    fn get_chan_nicks(&self, chan: &ChanNameRef) -> Vec<String> {
        match utils::find_idx(&self.chans, |c| c.name == *chan) {
            None => {
                error!("Could not find channel index in get_chan_nicks.");
                vec![]
            }
            Some(chan_idx) => {
                let mut nicks = self.chans[chan_idx]
                    .nicks
                    .iter()
                    .cloned()
                    .collect::<Vec<String>>();
                nicks.sort_unstable_by(|a, b| {
                    a.to_lowercase().partial_cmp(&b.to_lowercase()).unwrap()
                });
                nicks
            }
        }
    }

    /// If channel is in Joining state cancel Joining task, otherwise sent part message
    fn leave_channel(
        &mut self,
        msg_chan: &mut Sender<Cmd>,
        chan: &ChanNameRef,
        reason: Option<String>,
    ) {
        if let Some(idx) = utils::find_idx(&self.chans, |c| c.name == *chan) {
            match &mut self.chans[idx].join_state {
                JoinState::NotJoined => {}
                JoinState::Joining { stop_task, .. } => {
                    debug!("Aborting task to retry joining {}", chan.display());
                    let _ = stop_task.try_send(());
                }
                JoinState::Joined => msg_chan
                    .try_send(Cmd::Msg(wire::part(chan, reason)))
                    .unwrap(),
            }
        }
    }

    /// Kills all tasks that are trying to join channels
    fn kill_join_tasks(&mut self) {
        for chan in &mut self.chans {
            if let JoinState::Joining { stop_task } = &mut chan.join_state {
                let _ = stop_task.try_send(());
            }
        }
    }
}

async fn retry_channel_join(
    channel: ChanName,
    snd_irc_msg: Sender<String>,
    rcv_abort: Receiver<()>,
) {
    debug!("Attempting to re-join channel {}", channel.display());

    let mut rcv_abort = ReceiverStream::new(rcv_abort).fuse();

    // Send join message after timeout. Ok means channel tab was closed.
    if timeout(Duration::from_secs(10), rcv_abort.next())
        .await
        .is_err()
    {
        // Send join message
        snd_irc_msg
            .try_send(wire::join(std::iter::once(channel.as_ref())))
            .unwrap();
    }
}

const SERVERNAME_PREFIX: &str = "Your host is ";
const SERVERNAME_PREFIX_LEN: usize = SERVERNAME_PREFIX.len();

/// Parse server name from RPL_YOURHOST reply or fallback to using the server name inside
/// Pfx::Server. See https://www.irc.com/dev/docs/refs/numerics/002.html for more info.
fn parse_servername(pfx: Option<&Pfx>, params: &[String]) -> Option<String> {
    parse_yourhost_msg(params).or_else(|| parse_server_pfx(pfx))
}

/// Try to parse servername in a 002 RPL_YOURHOST reply params.
fn parse_yourhost_msg(params: &[String]) -> Option<String> {
    let msg = params.get(1).or_else(|| params.get(0))?;
    if msg.len() >= SERVERNAME_PREFIX_LEN && &msg[..SERVERNAME_PREFIX_LEN] == SERVERNAME_PREFIX {
        let slice1 = &msg[SERVERNAME_PREFIX_LEN..];
        let servername_ends = slice1.find('[').or_else(|| slice1.find(','))?;
        Some(slice1[..servername_ends].to_owned())
    } else {
        None
    }
}

/// Get the server name from a prefix.
fn parse_server_pfx(pfx: Option<&Pfx>) -> Option<String> {
    match pfx {
        Some(Pfx::Server(server_name)) | Some(Pfx::Ambiguous(server_name)) => {
            Some(server_name.to_owned())
        }
        Some(Pfx::User { .. }) | None => None,
    }
}

fn cap_name(cap: &str) -> &str {
    cap.trim_start_matches(['-', '~', '='])
        .split_once('=')
        .map_or(cap.trim_start_matches(['-', '~', '=']), |(name, _)| name)
}

fn parse_multiline_limits(cap: &str) -> Option<(usize, usize)> {
    let value = cap.strip_prefix("draft/multiline=")?;
    let mut max_bytes = None;
    let mut max_lines = usize::MAX;
    for token in value.split(',') {
        if let Some(value) = token.strip_prefix("max-bytes=") {
            max_bytes = value.parse::<usize>().ok().filter(|limit| *limit > 0);
        } else if let Some(value) = token.strip_prefix("max-lines=") {
            max_lines = value.parse::<usize>().ok().filter(|limit| *limit > 0)?;
        }
    }
    Some((max_bytes?, max_lines))
}

////////////////////////////////////////////////////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;

    fn server_info() -> ServerInfo {
        ServerInfo {
            addr: "irc.example".to_owned(),
            port: 6667,
            tls: false,
            pass: None,
            user: None,
            realname: "Tiny User".to_owned(),
            nicks: vec!["tiny".to_owned()],
            auto_join: Vec::new(),
            nickserv_ident: None,
            sasl_auth: None,
        }
    }

    fn cap(subcommand: &str, continuation: bool, params: &[&str]) -> Msg {
        Msg {
            pfx: Some(Pfx::Server("irc.example".to_owned())),
            cmd: wire::Cmd::CAP {
                client: "*".to_owned(),
                subcommand: subcommand.to_owned(),
                continuation,
                params: params.iter().map(|param| (*param).to_owned()).collect(),
            },
        }
    }

    fn update_and_drain(state: &State, mut msg: Msg) -> Vec<String> {
        let (mut snd_ev, _rcv_ev) = tokio::sync::mpsc::channel(10);
        let (mut snd_wire, mut rcv_wire) = tokio::sync::mpsc::channel(10);
        state.update(&mut msg, &mut snd_ev, &mut snd_wire);
        let mut messages = Vec::new();
        while let Ok(message) = rcv_wire.try_recv() {
            messages.push(message);
        }
        messages
    }

    #[test]
    fn cap_ls_req_ack_negotiates_multiline_and_ends() {
        let state = State::new(server_info());
        assert!(update_and_drain(&state, cap("LS", true, &["batch"])).is_empty());
        assert_eq!(
            update_and_drain(
                &state,
                cap(
                    "LS",
                    false,
                    &["draft/multiline=max-bytes=16384,max-lines=64"],
                ),
            ),
            vec![
                "NICK tiny\r\n".to_owned(),
                "USER tiny 8 * :Tiny User\r\n".to_owned(),
                "CAP REQ :batch draft/multiline\r\n".to_owned(),
            ]
        );
        assert_eq!(state.multiline_limits(), None);

        assert_eq!(
            update_and_drain(&state, cap("ACK", false, &["batch", "draft/multiline"])),
            vec!["CAP END\r\n".to_owned()]
        );
        assert_eq!(state.multiline_limits(), Some((16384, 64)));
    }

    #[test]
    fn unsupported_and_nak_leave_multiline_disabled() {
        let unsupported = State::new(server_info());
        assert_eq!(
            update_and_drain(&unsupported, cap("LS", false, &[])),
            vec![
                "NICK tiny\r\n".to_owned(),
                "USER tiny 8 * :Tiny User\r\n".to_owned(),
                "CAP END\r\n".to_owned(),
            ]
        );
        assert_eq!(unsupported.multiline_limits(), None);

        let nak = State::new(server_info());
        update_and_drain(
            &nak,
            cap(
                "LS",
                false,
                &["batch", "draft/multiline=max-bytes=16384,max-lines=64"],
            ),
        );
        assert_eq!(
            update_and_drain(&nak, cap("NAK", false, &["batch", "draft/multiline"])),
            vec!["CAP END\r\n".to_owned()]
        );
        assert_eq!(nak.multiline_limits(), None);
    }

    #[test]
    fn batch_references_are_valid_and_unique() {
        let state = State::new(server_info());
        assert_eq!(state.next_batch_reference(), "tiny-1");
        assert_eq!(state.next_batch_reference(), "tiny-2");
    }

    #[test]
    fn test_parse_servername_1() {
        // IRC standard
        let prefix = Some(Pfx::Server("card.freenode.net".to_string()));
        let params = vec![
            "nickname".to_string(),
            "Your host is card.freenode.net[38.229.70.22/6697], running version ircd-seven-1.1.9"
                .to_string(),
        ];
        assert_eq!(
            parse_servername(prefix.as_ref(), &params),
            Some("card.freenode.net".to_owned())
        );

        let prefix = Some(Pfx::Server("coulomb.oftc.net".to_string()));
        let params = vec![
            "nickname".to_string(),
            "Your host is coulomb.oftc.net[109.74.200.93/6697], running version hybrid-7.2.2+oftc1.7.3".to_string(),
        ];
        assert_eq!(
            parse_servername(prefix.as_ref(), &params),
            Some("coulomb.oftc.net".to_owned())
        );

        let prefix = Some(Pfx::Server("irc.eagle.y.se".to_string()));
        let params = vec![
            "nickname".to_string(),
            "Your host is irc.eagle.y.se, running version UnrealIRCd-4.0.18".to_string(),
        ];
        assert_eq!(
            parse_servername(prefix.as_ref(), &params),
            Some("irc.eagle.y.se".to_owned())
        );
    }

    #[test]
    fn test_parse_servername_2() {
        // Gitter variation
        // Msg { pfx: Some(Server("irc.gitter.im")), cmd: Reply { num: 2, params: ["nickname", " 1.10.0"] } }
        let prefix = Some(Pfx::Server("irc.gitter.im".to_string()));
        let params = vec!["nickname".to_string(), " 1.10.0".to_string()];
        assert_eq!(
            parse_servername(prefix.as_ref(), &params),
            Some("irc.gitter.im".to_owned())
        );
    }
}
