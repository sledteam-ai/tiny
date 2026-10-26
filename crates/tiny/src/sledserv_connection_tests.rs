use crate::{conn, sledserv, ui::UI};
use libtiny_common::{ChanName, ChanNameRef, MsgSource};
use libtiny_tui::TUI;
use libtiny_wire::{Cmd, Msg, MsgTarget, Pfx};
use std::{cell::Cell, rc::Rc, time::Duration};
use tokio::sync::mpsc;
use tokio_stream::{StreamExt, wrappers::ReceiverStream};

const SERV: &str = "irc";
const ID: &str = "01J00000000000000000000000";
struct Client(Rc<Cell<usize>>);
impl conn::Client for Client {
    fn get_serv_name(&self) -> &str {
        SERV
    }
    fn get_nick(&self) -> String {
        "musher".into()
    }
    fn is_nick_accepted(&self) -> bool {
        true
    }
    fn join(&self, _: &ChanNameRef) {
        self.0.set(self.0.get() + 1);
    }
}
fn origin() -> MsgSource {
    MsgSource::Serv { serv: SERV.into() }
}
fn run(test: impl AsyncFn(UI, TUI, Client)) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    tokio::task::LocalSet::new().block_on(&runtime, async {
        let (_input, receiver) = mpsc::channel::<term_input::Event>(10);
        let (tui, _events) = TUI::run_test(80, 12, ReceiverStream::new(receiver).map(Ok));
        let ui = UI::new(
            tui.clone(),
            None,
            crate::sledteam_session::SledteamSession::default(),
        );
        ui.new_server_tab(SERV, None);
        test(ui, tui, Client(Rc::new(Cell::new(0)))).await;
    });
}
fn receive(ui: &UI, client: &Client, cmd: Cmd) {
    conn::handle_conn_ev(
        ui,
        client,
        libtiny_client::Event::Msg(Msg {
            tags: vec![],
            pfx: Some(Pfx::User {
                nick: sledserv::NICK.into(),
                user: "u@h".into(),
            }),
            cmd,
        }),
    );
}

#[test]
fn correlated_label_resolution_preserves_foreground_join_and_ignores_duplicates() {
    run(async |ui, tui, client| {
        assert!(ui.record_sledserv_join(origin(), "channel resolve wheel", ID));
        let packet = serde_json::json!({"schema_version":1,"request_id":ID,"command":"channel.resolve","status":"ok","data":{"ulid":ID,"label":"wheel"}}).to_string();
        for _ in 0..2 {
            receive(
                &ui,
                &client,
                Cmd::PRIVMSG {
                    ctcp: None,
                    is_notice: false,
                    target: MsgTarget::User("musher".into()),
                    msg: packet.clone(),
                },
            );
        }
        assert_eq!(client.0.get(), 1);
        assert!(ui.pending_sledserv_origins().is_empty());
        assert_eq!(
            ui.display_channel(&ChanName::new(format!("#{ID}"))),
            "wheel"
        );
        conn::handle_conn_ev(
            &ui,
            &client,
            libtiny_client::Event::Msg(Msg {
                tags: vec![],
                pfx: Some(Pfx::User {
                    nick: "musher".into(),
                    user: "u@h".into(),
                }),
                cmd: Cmd::JOIN {
                    chan: ChanName::new(format!("#{ID}")),
                },
            }),
        );
        assert_eq!(
            tui.current_tab(),
            Some(MsgSource::Chan {
                serv: SERV.into(),
                chan: ChanName::new(format!("#{ID}"))
            })
        );
    });
}

#[test]
fn disconnect_and_remote_close_clear_pending_requests() {
    run(async |ui, _tui, client| {
        for event in [
            libtiny_client::Event::Disconnected,
            libtiny_client::Event::ConnectionClosed,
        ] {
            assert!(ui.record_sledserv_request(origin(), "ets", ID));
            conn::handle_conn_ev(&ui, &client, event);
            assert!(ui.pending_sledserv_origins().is_empty());
        }
    });
}

#[test]
fn task_end_and_cancellation_clear_requests_without_clearing_replacement() {
    run(async |ui, _tui, client| {
        let (sender, receiver) = mpsc::channel(1);
        let task = tokio::task::spawn_local(conn::task_with_linger(
            receiver,
            ui.clone(),
            Box::new(Client(client.0.clone())),
            Duration::ZERO,
        ));
        tokio::task::yield_now().await;
        assert!(ui.record_sledserv_request(origin(), "ets", ID));
        drop(sender);
        task.await.unwrap();
        assert!(ui.pending_sledserv_origins().is_empty());
        let (_sender, receiver) = mpsc::channel(1);
        let task = tokio::task::spawn_local(conn::task(receiver, ui.clone(), Box::new(client)));
        tokio::task::yield_now().await;
        assert!(ui.record_sledserv_request(origin(), "ets", ID));
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(ui.pending_sledserv_origins().is_empty());
        let old = sledserv::ConnectionLifetime::begin(&ui, SERV);
        let replacement = sledserv::ConnectionLifetime::begin(&ui, SERV);
        assert!(ui.record_sledserv_request(origin(), "ets", ID));
        drop(old);
        assert_eq!(ui.pending_sledserv_origins(), [origin()]);
        drop(replacement);
        assert!(ui.pending_sledserv_origins().is_empty());
    });
}

#[test]
fn correlated_committed_spans_join_in_background_once_without_focus_change() {
    run(async |ui, tui, client| {
        let second = "01J00000000000000000000001";
        let current = ChanName::new("#current".into());
        tui.new_server_tab("other", None);
        tui.new_chan_tab("other", &current);
        tui.focus_chan_tab("other", &current);
        let focus = tui.current_tab();
        for id in [ID, second] {
            assert!(ui.record_sledserv_request(origin(), "span add Test", id));
        }
        for (id, outcome) in [(second, "degraded"), (ID, "completed")] {
            let packet = serde_json::json!({"schema_version":1,"request_id":id,"command":"travel.span.add","status":"ok","data":{"ulid":id,"label":format!("Span {id}"),"outcome":outcome,"effects":"workbench unavailable"}}).to_string();
            for _ in 0..2 {
                receive(
                    &ui,
                    &client,
                    Cmd::PRIVMSG {
                        ctcp: None,
                        is_notice: false,
                        target: MsgTarget::User("musher".into()),
                        msg: packet.clone(),
                    },
                );
            }
            assert_eq!(
                tui.current_tab(),
                focus,
                "reply delivery must preserve conversation"
            );
            conn::handle_conn_ev(
                &ui,
                &client,
                libtiny_client::Event::Msg(Msg {
                    tags: vec![],
                    pfx: Some(Pfx::User {
                        nick: "musher".into(),
                        user: "u@h".into(),
                    }),
                    cmd: Cmd::JOIN {
                        chan: ChanName::new(format!("#{id}")),
                    },
                }),
            );
            assert_eq!(tui.current_tab(), focus);
        }
        for id in [ID, second] {
            tui.focus_chan_tab(SERV, &ChanName::new(format!("#{id}")));
            assert_eq!(
                tui.current_tab(),
                Some(MsgSource::Chan {
                    serv: SERV.into(),
                    chan: ChanName::new(format!("#{id}"))
                })
            );
        }
        assert_eq!(client.0.get(), 2);
        assert!(ui.pending_sledserv_origins().is_empty());
    });
}

#[test]
fn rejected_or_invalid_span_creation_never_joins() {
    run(async |ui, _tui, client| {
        for data in [
            serde_json::json!(null),
            serde_json::json!({"ulid":"invalid","label":"Span","outcome":"completed"}),
            serde_json::json!({"ulid":ID,"label":"Span","outcome":"rejected"}),
            serde_json::json!({"ulid":ID,"label":"Span"}),
        ] {
            assert!(ui.record_sledserv_request(origin(), "span add Test", ID));
            let packet = if data.is_null() {
                serde_json::json!({"schema_version":1,"request_id":ID,"command":"travel.span.add","status":"error","error":{"code":"span_already_exists","message":"Exists"}})
            } else {
                serde_json::json!({"schema_version":1,"request_id":ID,"command":"travel.span.add","status":"ok","data":data})
            };
            receive(
                &ui,
                &client,
                Cmd::PRIVMSG {
                    ctcp: None,
                    is_notice: false,
                    target: MsgTarget::User("musher".into()),
                    msg: packet.to_string(),
                },
            );
        }
        assert_eq!(client.0.get(), 0);
    });
}
