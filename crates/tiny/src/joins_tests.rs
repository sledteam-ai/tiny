use super::*;
use crate::sledteam_session::SledteamSession;
use libtiny_common::MsgSource;
use libtiny_tui::TUI;
use libtiny_wire::{Cmd, Msg, Pfx};
use std::cell::Cell;
use tokio::sync::mpsc;
use tokio_stream::{StreamExt, wrappers::ReceiverStream};

const SERV: &str = "irc";
const ULID: &str = "01J00000000000000000000000";

struct TestClient(Cell<usize>);
impl Client for TestClient {
    fn get_serv_name(&self) -> &str {
        SERV
    }
    fn get_nick(&self) -> String {
        "musher".to_owned()
    }
    fn is_nick_accepted(&self) -> bool {
        true
    }
    fn join(&self, _: &ChanNameRef) {
        self.0.set(self.0.get() + 1);
    }
}

fn run(test: impl FnOnce(UI, TUI, TestClient)) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    tokio::task::LocalSet::new().block_on(&runtime, async {
        let (_input, receiver) = mpsc::channel::<term_input::Event>(10);
        let (tui, _events) = TUI::run_test(80, 12, ReceiverStream::new(receiver).map(Ok));
        tui.new_server_tab(SERV, None);
        let ui = UI::new(tui.clone(), None, SledteamSession::default());
        test(ui, tui, TestClient(Cell::new(0)));
    });
}

fn channel() -> ChanName {
    ChanName::new(format!("#{ULID}"))
}
fn pending(ui: &UI) -> usize {
    ui.joins
        .0
        .borrow()
        .get(SERV)
        .map_or(0, |connection| connection.pending.len())
}
fn receive(ui: &UI, client: &TestClient, cmd: Cmd) {
    // Use the production event dispatcher, including the self-JOIN focus decision.
    crate::conn::handle_conn_ev(
        ui,
        client,
        libtiny_client::Event::Msg(Msg {
            tags: vec![],
            pfx: Some(Pfx::User {
                nick: client.get_nick(),
                user: "u@host".to_owned(),
            }),
            cmd,
        }),
    );
}

#[test]
fn background_join_creates_tab_preserving_conversation_after_insertion() {
    run(|ui, tui, client| {
        // Inserting into irc shifts the selected conversation's numeric index.
        tui.new_server_tab("other", None);
        let origin = ChanName::new("#current".to_owned());
        tui.new_chan_tab("other", &origin);
        tui.focus_chan_tab("other", &origin);
        let focused = tui.current_tab();
        join_background(&client, &ui, ULID);
        receive(&ui, &client, Cmd::JOIN { chan: channel() });
        assert_eq!(tui.current_tab(), focused);
        assert_eq!(pending(&ui), 0);
        tui.focus_chan_tab(SERV, &channel());
        assert_eq!(
            tui.current_tab(),
            Some(MsgSource::Chan {
                serv: SERV.to_owned(),
                chan: channel()
            })
        );
    });
}

#[test]
fn explicit_join_still_focuses() {
    run(|ui, tui, client| {
        join_canonical(&client, &ui, SERV, ULID);
        receive(&ui, &client, Cmd::JOIN { chan: channel() });
        assert_eq!(
            tui.current_tab(),
            Some(MsgSource::Chan {
                serv: SERV.to_owned(),
                chan: channel()
            })
        );
    });
}

#[test]
fn foreground_overrides_background_and_cannot_be_downgraded() {
    run(|ui, tui, client| {
        join_background(&client, &ui, ULID);
        join_canonical(&client, &ui, SERV, ULID);
        join_background(&client, &ui, ULID);
        assert_eq!(client.0.get(), 2);
        receive(&ui, &client, Cmd::JOIN { chan: channel() });
        assert_eq!(
            tui.current_tab(),
            Some(MsgSource::Chan {
                serv: SERV.to_owned(),
                chan: channel()
            })
        );
    });
}

#[test]
fn repeated_background_requests_coalesce_and_already_joined_leaves_no_intent() {
    run(|ui, tui, client| {
        let focused = tui.current_tab();
        for _ in 0..3 {
            join_background(&client, &ui, ULID);
        }
        assert_eq!(client.0.get(), 1);
        assert_eq!(pending(&ui), 1);
        receive(&ui, &client, Cmd::JOIN { chan: channel() });
        for _ in 0..3 {
            join_background(&client, &ui, ULID);
        }
        assert_eq!(client.0.get(), 1);
        assert_eq!(pending(&ui), 0);
        assert_eq!(tui.current_tab(), focused);
        // A later unsolicited self JOIN must not inherit stale background intent.
        receive(&ui, &client, Cmd::JOIN { chan: channel() });
        assert_ne!(tui.current_tab(), focused);
    });
}

#[test]
fn disconnect_and_reconnect_clear_intent_and_membership() {
    run(|ui, _, client| {
        join_background(&client, &ui, ULID);
        for event in [
            libtiny_client::Event::Disconnected,
            libtiny_client::Event::Connected,
        ] {
            crate::conn::handle_conn_ev(&ui, &client, event);
            assert_eq!(pending(&ui), 0);
        }
        join_background(&client, &ui, ULID);
        receive(&ui, &client, Cmd::JOIN { chan: channel() });
        crate::conn::handle_conn_ev(&ui, &client, libtiny_client::Event::Disconnected);
        join_background(&client, &ui, ULID); // Offline: no command, no intent.
        assert_eq!(pending(&ui), 0);
        crate::conn::handle_conn_ev(&ui, &client, libtiny_client::Event::Connected);
        join_background(&client, &ui, ULID);
        assert_eq!(pending(&ui), 1);
        assert_eq!(client.0.get(), 3);
        let lifetime = ConnectionLifetime::begin(&ui.joins, SERV);
        join_background(&client, &ui, ULID);
        drop(lifetime);
        assert_eq!(pending(&ui), 0);
    });
}

#[test]
fn terminal_join_failure_clears_intent_but_retry_progress_retains_it() {
    run(|ui, _, client| {
        for num in [403, 405, 471, 473, 474, 475, 477] {
            join_background(&client, &ui, ULID);
            receive(
                &ui,
                &client,
                Cmd::Reply {
                    num,
                    params: vec!["musher".into(), channel().display().into(), "denied".into()],
                },
            );
            assert_eq!(pending(&ui), 0, "numeric {num}");
        }
        join_background(&client, &ui, ULID);
        crate::conn::handle_conn_ev(
            &ui,
            &client,
            libtiny_client::Event::ChannelJoinError {
                chan: channel(),
                msg: "Attempting to rejoin...".into(),
            },
        );
        receive(
            &ui,
            &client,
            Cmd::Reply {
                num: 477,
                params: vec![
                    "musher".into(),
                    channel().display().into(),
                    "identify".into(),
                ],
            },
        );
        assert_eq!(pending(&ui), 1);
        crate::conn::handle_conn_ev(
            &ui,
            &client,
            libtiny_client::Event::ChannelJoinError {
                chan: channel(),
                msg: format!("Unable to join {}.", channel().display()),
            },
        );
        assert_eq!(pending(&ui), 0);
    });
}

#[test]
fn other_users_join_does_not_consume_intent_and_channels_are_case_insensitive() {
    run(|ui, tui, client| {
        join_background(&client, &ui, ULID);
        crate::conn::handle_conn_ev(
            &ui,
            &client,
            libtiny_client::Event::Msg(Msg {
                tags: vec![],
                pfx: Some(Pfx::Ambiguous("someone".into())),
                cmd: Cmd::JOIN { chan: channel() },
            }),
        );
        assert_eq!(pending(&ui), 1);
        let focused = tui.current_tab();
        receive(
            &ui,
            &client,
            Cmd::JOIN {
                chan: ChanName::new(channel().display().to_ascii_lowercase()),
            },
        );
        assert_eq!(pending(&ui), 0);
        assert_eq!(tui.current_tab(), focused);
    });
}

#[test]
fn retiring_connection_cannot_clear_replacement_intent() {
    run(|ui, _, client| {
        let retired = ConnectionLifetime::begin(&ui.joins, SERV);
        let current = ConnectionLifetime::begin(&ui.joins, SERV);
        join_background(&client, &ui, ULID);
        assert!(!retired.is_current());
        assert!(current.is_current());
        drop(retired);
        assert_eq!(pending(&ui), 1);
        drop(current);
        assert_eq!(pending(&ui), 0);
    });
}

#[test]
fn part_and_kick_allow_a_fresh_background_join() {
    run(|ui, _, client| {
        receive(&ui, &client, Cmd::JOIN { chan: channel() });
        receive(
            &ui,
            &client,
            Cmd::PART {
                chan: channel(),
                msg: None,
            },
        );
        join_background(&client, &ui, ULID);
        assert_eq!(pending(&ui), 1);
        receive(&ui, &client, Cmd::JOIN { chan: channel() });
        receive(
            &ui,
            &client,
            Cmd::Other {
                cmd: "KICK".into(),
                params: vec![channel().display().into(), client.get_nick(), "bye".into()],
            },
        );
        join_background(&client, &ui, ULID);
        assert_eq!(pending(&ui), 1);
        assert_eq!(client.0.get(), 2);
    });
}
