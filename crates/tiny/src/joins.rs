//! Local JOIN focus intent. Server names identify Tiny's single live client per server;
//! transport transitions and the connection task's lifetime bound every entry.
use crate::{conn::Client, ui::UI};
use libtiny_common::{ChanName, ChanNameRef};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JoinFocus {
    Foreground,
    Background,
}

#[derive(Default)]
struct Connection {
    token: Rc<()>,
    pending: HashMap<ChanName, JoinFocus>,
    joined: HashSet<ChanName>,
    ended: bool,
    retrying: HashSet<ChanName>,
}

#[derive(Clone, Default)]
pub(crate) struct JoinIntents(Rc<RefCell<HashMap<String, Connection>>>);

impl JoinIntents {
    pub(crate) fn end_connection(&self, serv: &str) {
        self.reset(serv, true);
    }

    fn reset(&self, serv: &str, ended: bool) {
        let mut connections = self.0.borrow_mut();
        let connection = connections.entry(serv.to_owned()).or_default();
        *connection = Connection {
            token: connection.token.clone(),
            ended,
            ..Connection::default()
        };
    }

    pub(crate) fn cancel(&self, serv: &str, chan: &ChanNameRef) {
        if let Some(connection) = self.0.borrow_mut().get_mut(serv) {
            connection.pending.remove(chan);
            connection.retrying.remove(chan);
        }
    }

    fn request(&self, serv: &str, chan: &ChanNameRef, focus: JoinFocus) -> bool {
        let mut connections = self.0.borrow_mut();
        let connection = connections.entry(serv.to_owned()).or_default();
        if connection.joined.contains(chan) {
            connection.pending.remove(chan);
            // Preserve the existing foreground path, including its repeated JOIN send.
            return focus == JoinFocus::Foreground;
        }
        if connection.ended {
            // Foreground commands retain their existing send behaviour while offline.
            return focus == JoinFocus::Foreground;
        }
        if let Some(pending) = connection.pending.get_mut(chan) {
            if focus == JoinFocus::Foreground {
                *pending = focus;
            }
            // Explicit commands still send JOIN on every invocation, as before.
            return focus == JoinFocus::Foreground;
        }
        connection.pending.insert(chan.to_owned(), focus);
        true
    }
}

/// A task token prevents a retiring client for the same server from clearing or
/// consuming the replacement client's intent after /close followed by /connect.
pub(crate) struct ConnectionLifetime<'a> {
    intents: &'a JoinIntents,
    serv: &'a str,
    token: Rc<()>,
}
impl<'a> ConnectionLifetime<'a> {
    pub(crate) fn begin(intents: &'a JoinIntents, serv: &'a str) -> Self {
        let connection = Connection::default();
        let token = connection.token.clone();
        intents.0.borrow_mut().insert(serv.to_owned(), connection);
        Self {
            intents,
            serv,
            token,
        }
    }

    pub(crate) fn is_current(&self) -> bool {
        self.intents
            .0
            .borrow()
            .get(self.serv)
            .is_some_and(|connection| Rc::ptr_eq(&connection.token, &self.token))
    }
}
impl Drop for ConnectionLifetime<'_> {
    fn drop(&mut self) {
        if self.is_current() {
            self.intents.end_connection(self.serv);
        }
    }
}

pub(crate) fn join_canonical(client: &dyn Client, ui: &UI, serv: &str, ulid: &str) {
    join(client, ui, serv, ulid, JoinFocus::Foreground);
}

/// Explicit opt-in for callers that already have a canonical channel ULID.
/// Structured committed-creation replies use this without changing focus.
pub(crate) fn join_background(client: &dyn Client, ui: &UI, ulid: &str) {
    if client.is_nick_accepted() {
        join(
            client,
            ui,
            client.get_serv_name(),
            ulid,
            JoinFocus::Background,
        );
    }
}

fn join(client: &dyn Client, ui: &UI, serv: &str, ulid: &str, focus: JoinFocus) {
    let channel = ChanName::new(format!("#{ulid}"));
    let already_joined = ui
        .joins
        .0
        .borrow()
        .get(serv)
        .is_some_and(|connection| connection.joined.contains(&channel));
    if already_joined && focus == JoinFocus::Background {
        ui.new_chan_tab(serv, &channel);
    }
    let config = ui.get_tab_config(serv, Some(channel.as_ref()));
    ui.set_tab_config(serv, Some(&channel), config);
    if ui.joins.request(serv, &channel, focus) {
        client.join(&channel);
    }
}

pub(crate) fn own_join(ui: &UI, serv: &str, chan: &ChanNameRef) {
    let focus = {
        let mut connections = ui.joins.0.borrow_mut();
        let connection = connections.entry(serv.to_owned()).or_default();
        connection.joined.insert(chan.to_owned());
        connection.retrying.remove(chan);
        connection.pending.remove(chan)
    };
    ui.new_chan_tab(serv, chan);
    if focus != Some(JoinFocus::Background) {
        ui.focus_chan_tab(serv, chan);
    }
}

pub(crate) fn observe_event(ui: &UI, client: &dyn Client, event: &libtiny_client::Event) {
    use libtiny_client::Event;
    use libtiny_wire::{Cmd, Pfx};
    let serv = client.get_serv_name();
    match event {
        Event::ResolvingHost | Event::Disconnected | Event::ConnectionClosed => {
            ui.joins.end_connection(serv);
        }
        Event::Connected => {
            ui.joins.reset(serv, false);
        }
        // This event also reports retry progress. Only the exhausted-retries message is final.
        Event::ChannelJoinError { chan, msg }
            if msg == &format!("Unable to join {}.", chan.display()) =>
        {
            ui.joins.cancel(serv, chan);
        }
        Event::ChannelJoinError { chan, .. } => {
            ui.joins
                .0
                .borrow_mut()
                .entry(serv.to_owned())
                .or_default()
                .retrying
                .insert(chan.clone());
        }
        Event::Msg(msg) => match &msg.cmd {
            Cmd::Reply {
                num: 403 | 405 | 407 | 437 | 471 | 473 | 474 | 475 | 476 | 479 | 489,
                params,
            } if params.len() >= 3 => {
                ui.joins.cancel(serv, ChanNameRef::new(&params[1]));
            }
            // Retry progress is emitted before the original numeric. A dynamic JOIN with
            // no retry scheduled is terminal; an active NickServ retry retains its intent.
            Cmd::Reply { num: 477, params } if params.len() >= 3 => {
                let chan = ChanNameRef::new(&params[1]);
                let retrying = ui
                    .joins
                    .0
                    .borrow()
                    .get(serv)
                    .is_some_and(|connection| connection.retrying.contains(chan));
                if !retrying {
                    ui.joins.cancel(serv, chan);
                }
            }
            Cmd::Other { cmd, params }
                if cmd.eq_ignore_ascii_case("KICK")
                    && params.len() >= 2
                    && params[1] == client.get_nick() =>
            {
                if let Some(connection) = ui.joins.0.borrow_mut().get_mut(serv) {
                    let chan = ChanNameRef::new(&params[0]);
                    connection.joined.remove(chan);
                    connection.pending.remove(chan);
                    connection.retrying.remove(chan);
                }
            }
            Cmd::PART { chan, .. }
                if matches!(&msg.pfx,
                Some(Pfx::User { nick, .. }) | Some(Pfx::Ambiguous(nick))
                    if nick == &client.get_nick()) =>
            {
                if let Some(connection) = ui.joins.0.borrow_mut().get_mut(serv) {
                    connection.joined.remove(chan.as_ref());
                    connection.pending.remove(chan.as_ref());
                }
            }
            Cmd::ERROR { .. } => ui.joins.end_connection(serv),
            _ => {}
        },
        _ => {}
    }
}

#[cfg(test)]
#[path = "joins_tests.rs"]
mod tests;
