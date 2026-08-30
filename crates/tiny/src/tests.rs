use crate::conn;
use crate::ui::UI;
use libtiny_common::{ChanName, MsgSource};
use libtiny_tui::TUI;
use libtiny_tui::test_utils::expect_screen;
use libtiny_wire::{Cmd, Msg, MsgTarget, Pfx};

use termbox_simple::CellBuf;

use libtiny_client as client;
use term_input as input;

use std::future::Future;
use std::panic::Location;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

struct TestClient {
    nick: String,
}

impl conn::Client for TestClient {
    fn get_serv_name(&self) -> &str {
        SERV_NAME
    }

    fn get_nick(&self) -> String {
        self.nick.clone()
    }

    fn is_nick_accepted(&self) -> bool {
        true
    }
}

static SERV_NAME: &str = "x.y.z";
const DEFAULT_TUI_WIDTH: u16 = 40;
const DEFAULT_TUI_HEIGHT: u16 = 5;

struct TestSetup {
    /// TUI test instance
    tui: TUI,
    /// Send input events to the TUI using this channel
    snd_input_ev: mpsc::Sender<input::Event>,
    /// Send connection events to connection handler (`conn::task`) using this channel
    snd_conn_ev: mpsc::Sender<client::Event>,
    /// Events emitted by editable input surfaces toward Tiny's IRC send path.
    rcv_tui_ev: mpsc::Receiver<libtiny_common::Event>,
    /// UI wrapper shared with the connection handler.
    ui: UI,
}

fn run_test<F, Fut>(nick: String, test: F)
where
    F: Fn(TestSetup) -> Fut,
    Fut: Future<Output = ()>,
{
    run_test_with_size(nick, DEFAULT_TUI_WIDTH, DEFAULT_TUI_HEIGHT, test)
}

fn run_test_with_size<F, Fut>(nick: String, width: u16, height: u16, test: F)
where
    F: Fn(TestSetup) -> Fut,
    Fut: Future<Output = ()>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&runtime, async move {
        // Create test TUI
        let (snd_input_ev, rcv_input_ev) = mpsc::channel::<term_input::Event>(100);
        let rcv_input_ev = ReceiverStream::new(rcv_input_ev);
        let (tui, rcv_tui_ev) = TUI::run_test(width, height, rcv_input_ev.map(Ok));

        let tiny_ui = UI::new(
            tui.clone(),
            None,
            crate::sledteam_session::SledteamSession::default(),
        );

        // Create test connection event channel
        let (snd_conn_ev, rcv_conn_ev) = mpsc::channel::<client::Event>(100);
        // Spawn connection event handler task
        tokio::task::spawn_local(conn::task_with_linger(
            rcv_conn_ev,
            tiny_ui.clone(),
            Box::new(TestClient { nick }),
            Duration::ZERO,
        ));

        tui.new_server_tab(SERV_NAME, None);
        tui.draw();

        test(TestSetup {
            tui,
            snd_input_ev,
            snd_conn_ev,
            rcv_tui_ev,
            ui: tiny_ui,
        })
        .await;
    });
}

#[test]
fn incoming_multiline_event_uses_one_sender_header() {
    const HEIGHT: u16 = 12;
    run_test_with_size(
        "osa1".to_owned(),
        DEFAULT_TUI_WIDTH,
        HEIGHT,
        |TestSetup {
             tui,
             snd_input_ev: _snd_input_ev,
             snd_conn_ev,
             ..
         }| async move {
            let chan = ChanName::new("#01J00000000000000000000000".to_owned());
            snd_conn_ev
                .send(client::Event::Msg(Msg {
                    tags: Vec::new(),
                    pfx: Some(Pfx::User {
                        nick: "osa1".to_owned(),
                        user: "a@b".to_owned(),
                    }),
                    cmd: Cmd::JOIN { chan: chan.clone() },
                }))
                .await
                .unwrap();
            snd_conn_ev
                .send(client::Event::MultilineMsg(Msg {
                    tags: Vec::new(),
                    pfx: Some(Pfx::User {
                        nick: "mushbot".to_owned(),
                        user: "bot@localhost".to_owned(),
                    }),
                    cmd: Cmd::PRIVMSG {
                        target: MsgTarget::Chan(chan),
                        msg: "first line\nsecond line\nthird line".to_owned(),
                        is_notice: false,
                        ctcp: None,
                    },
                }))
                .await
                .unwrap();
            yield_(5).await;
            tui.draw();
            assert!(!_snd_input_ev.is_closed());

            let screen = buffer_text(&tui.get_front_buffer(), DEFAULT_TUI_WIDTH, HEIGHT);
            assert_eq!(screen.matches("mushbot").count(), 1, "{screen}");
            assert!(
                screen.contains("first line\nsecond line\nthird line"),
                "{screen}"
            );
        },
    )
}

#[test]
fn test_own_join_focuses_channel_tab() {
    run_test(
        "osa1".to_owned(),
        |TestSetup {
             tui,
             snd_input_ev,
             snd_conn_ev,
             ..
         }| async move {
            let chan = ChanName::new("#01J00000000000000000000000".to_owned());
            let join = Msg {
                tags: Vec::new(),
                pfx: Some(Pfx::User {
                    nick: "osa1".to_owned(),
                    user: "a@b".to_owned(),
                }),
                cmd: Cmd::JOIN { chan: chan.clone() },
            };

            snd_conn_ev.send(client::Event::Msg(join)).await.unwrap();
            yield_(5).await;

            assert!(!snd_input_ev.is_closed());
            assert_eq!(
                tui.current_tab(),
                Some(MsgSource::Chan {
                    serv: SERV_NAME.to_owned(),
                    chan,
                })
            );
        },
    )
}

#[test]
fn test_privmsg_from_user_without_user_or_host_part_issue_247() {
    run_test(
        "osa1".to_owned(),
        |TestSetup {
             tui,
             snd_input_ev,
             snd_conn_ev,
             ..
         }| async move {
            snd_conn_ev.send(client::Event::Connected).await.unwrap();
            snd_conn_ev
                .send(client::Event::NickChange {
                    new_nick: "osa1".to_owned(),
                })
                .await
                .unwrap();
            yield_(5).await;

            // Join a channel to test msg sent to channel
            let join = Msg {
                tags: Vec::new(),
                pfx: Some(Pfx::User {
                    nick: "osa1".to_owned(),
                    user: "a@b".to_owned(),
                }),
                cmd: Cmd::JOIN {
                    chan: ChanName::new("#chan".to_owned()),
                },
            };
            snd_conn_ev.send(client::Event::Msg(join)).await.unwrap();
            yield_(5).await;

            // Send a PRIVMSG to the channel
            let chan_msg = Msg {
                tags: Vec::new(),
                pfx: Some(Pfx::Ambiguous("tiny_test_user".to_owned())),
                cmd: Cmd::PRIVMSG {
                    target: MsgTarget::Chan(ChanName::new("#chan".to_owned())),
                    msg: "msg to chan".to_owned(),
                    is_notice: false,
                    ctcp: None,
                },
            };
            snd_conn_ev
                .send(client::Event::Msg(chan_msg))
                .await
                .unwrap();
            yield_(5).await;

            // Send a PRIVMSG to current nick
            let msg = Msg {
                tags: Vec::new(),
                pfx: Some(Pfx::Ambiguous("tiny_test_user".to_owned())),
                cmd: Cmd::PRIVMSG {
                    target: MsgTarget::User("osa1".to_owned()),
                    // This generates a notification when the test is run with
                    // desktop-notifications feature, so show a helpful message to not confuse
                    // users (#371)
                    msg: "this is a test in tiny IRC client -- please ignore".to_owned(),
                    is_notice: false,
                    ctcp: None,
                },
            };
            snd_conn_ev.send(client::Event::Msg(msg)).await.unwrap();
            yield_(5).await;

            // Check channel tab
            tui.draw();

            #[rustfmt::skip]
            let screen =
            "|                                        |
             |                                        |
             |00:00 tiny_test_user: msg to chan       |
             |osa1:                                   |
             |x.y.z #chan tiny_test_user              |";

            let mut front_buffer = tui.get_front_buffer();
            normalize_timestamps(&mut front_buffer, DEFAULT_TUI_WIDTH, DEFAULT_TUI_HEIGHT);
            expect_screen(
                screen,
                &front_buffer,
                DEFAULT_TUI_WIDTH,
                DEFAULT_TUI_HEIGHT,
                Location::caller(),
            );

            // Check privmsg tab
            next_tab(&snd_input_ev).await; // privmsg tab
            yield_(5).await;
            tui.draw();

            #[rustfmt::skip]
            let screen =
            "|                                        |
             |00:00 tiny_test_user: this is a test in |
             |tiny IRC client -- please ignore        |
             |osa1:                                   |
             |x.y.z #chan tiny_test_user              |";

            let mut front_buffer = tui.get_front_buffer();
            normalize_timestamps(&mut front_buffer, DEFAULT_TUI_WIDTH, DEFAULT_TUI_HEIGHT);
            expect_screen(
                screen,
                &front_buffer,
                DEFAULT_TUI_WIDTH,
                DEFAULT_TUI_HEIGHT,
                Location::caller(),
            );
        },
    )
}

#[test]
fn test_bouncer_relay_issue_271() {
    run_test(
        "osa1-soju".to_owned(),
        |TestSetup {
             tui,
             snd_input_ev,
             snd_conn_ev,
             ..
         }| async move {
            snd_conn_ev.send(client::Event::Connected).await.unwrap();
            snd_conn_ev
                .send(client::Event::NickChange {
                    new_nick: "osa1-soju".to_owned(),
                })
                .await
                .unwrap();

            let msg = Msg {
                tags: Vec::new(),
                pfx: Some(Pfx::User {
                    nick: "osa1-soju".to_owned(),
                    user: "osa1-soju@127.0.0.1".to_owned(),
                }),
                cmd: Cmd::PRIVMSG {
                    target: MsgTarget::User("osa1/oftc".to_owned()),
                    msg: "blah blah".to_owned(),
                    is_notice: false,
                    ctcp: None,
                },
            };

            snd_conn_ev.send(client::Event::Msg(msg)).await.unwrap();

            yield_(5).await;
            tui.draw();

            next_tab(&snd_input_ev).await; // server tab
            next_tab(&snd_input_ev).await; // privmsg tab
            yield_(5).await;

            tui.draw();

            #[rustfmt::skip]
            let screen =
            "|                                        |
             |                                        |
             |00:00 osa1-soju: blah blah              |
             |osa1-soju:                              |
             |x.y.z osa1/oftc                         |";

            let mut front_buffer = tui.get_front_buffer();
            normalize_timestamps(&mut front_buffer, DEFAULT_TUI_WIDTH, DEFAULT_TUI_HEIGHT);
            expect_screen(
                screen,
                &front_buffer,
                DEFAULT_TUI_WIDTH,
                DEFAULT_TUI_HEIGHT,
                Location::caller(),
            );
        },
    )
}

#[test]
fn test_sledserv_response_is_local_to_command_origin() {
    run_test(
        "osa1".to_owned(),
        |TestSetup {
             tui,
             snd_input_ev,
             snd_conn_ev,
             mut rcv_tui_ev,
             ui,
         }| async move {
            let origin_chan = ChanName::new("#origin".to_owned());
            tui.new_chan_tab(SERV_NAME, &origin_chan);
            let origin = MsgSource::Chan {
                serv: SERV_NAME.to_owned(),
                chan: origin_chan.clone(),
            };
            ui.record_sledserv_request(origin.clone(), "ets");

            // Move away while the request is in flight; receipt must not retarget the result.
            next_tab(&snd_input_ev).await;
            yield_(3).await;
            assert_ne!(tui.current_tab(), Some(origin.clone()));

            snd_conn_ev
                .send(client::Event::Msg(Msg {
                    tags: Vec::new(),
                    pfx: Some(Pfx::User {
                        nick: "SledServ".to_owned(),
                        user: "sledserv-service@localhost".to_owned(),
                    }),
                    cmd: Cmd::PRIVMSG {
                        target: MsgTarget::User("osa1".to_owned()),
                        msg: r#"{"schema_version":1,"command":"travel.ets","status":"ok","data":{"expedition":{"ulid":"E1","name":"Moonshot"},"trail":{"ulid":"T1","name":"North"},"spans":[]}}"#.to_owned(),
                        is_notice: false,
                        ctcp: None,
                    },
                }))
                .await
                .unwrap();
            yield_(5).await;

            assert!(!ui.user_tab_exists(SERV_NAME, "SledServ"));
            assert!(ui.pending_sledserv_origins().is_empty());
            assert!(!ui.intentional_shutdown_pending(SERV_NAME));
            assert_eq!(
                tui.current_tab(),
                Some(MsgSource::Serv {
                    serv: SERV_NAME.to_owned()
                })
            );
            tui.focus_chan_tab(SERV_NAME, &origin_chan);
            tui.draw();
            let output = libtiny_tui::test_utils::buffer_str(
                &tui.get_front_buffer(),
                DEFAULT_TUI_WIDTH,
                DEFAULT_TUI_HEIGHT,
            );
            assert!(output.contains("└── North"), "{output:?}");
            assert!(matches!(
                rcv_tui_ev.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));

            ui.record_sledserv_request(origin, "shutdown sledteam");
            snd_conn_ev
                .send(client::Event::Msg(Msg {
                    tags: Vec::new(),
                    pfx: Some(Pfx::User {
                        nick: "SledServ".to_owned(),
                        user: "sledserv-service@localhost".to_owned(),
                    }),
                    cmd: Cmd::PRIVMSG {
                        target: MsgTarget::User("osa1".to_owned()),
                        msg: r#"{"schema_version":1,"command":"runtime.shutdown","status":"ok","data":{"message":"Sledteam shutdown request sent."}}"#.to_owned(),
                        is_notice: false,
                        ctcp: None,
                    },
                }))
                .await
                .unwrap();
            yield_(5).await;

            assert!(!ui.user_tab_exists(SERV_NAME, "SledServ"));
            assert!(ui.pending_sledserv_origins().is_empty());
            assert!(ui.intentional_shutdown_pending(SERV_NAME));
            tui.draw();
            let output = libtiny_tui::test_utils::buffer_str(
                &tui.get_front_buffer(),
                DEFAULT_TUI_WIDTH,
                DEFAULT_TUI_HEIGHT,
            );
            assert!(output.contains("Shutting down Sledteam…"), "{output:?}");
            assert!(matches!(
                rcv_tui_ev.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));

            snd_conn_ev
                .send(client::Event::ConnectionClosed)
                .await
                .unwrap();
            snd_conn_ev.send(client::Event::Disconnected).await.unwrap();
            yield_(5).await;

            assert!(!ui.intentional_shutdown_pending(SERV_NAME));
            assert!(matches!(
                rcv_tui_ev.recv().await,
                Some(libtiny_common::Event::Quit { msg: None })
            ));
        },
    )
}

#[test]
fn test_unexpected_disconnect_and_shutdown_error_keep_reconnect_behavior() {
    const HEIGHT: u16 = 12;
    run_test_with_size(
        "osa1".to_owned(),
        DEFAULT_TUI_WIDTH,
        HEIGHT,
        |TestSetup {
             tui,
             snd_input_ev,
             snd_conn_ev,
             ui,
             ..
         }| async move {
            next_tab(&snd_input_ev).await;
            yield_(3).await;

            snd_conn_ev
                .send(client::Event::ConnectionClosed)
                .await
                .unwrap();
            snd_conn_ev.send(client::Event::Disconnected).await.unwrap();
            yield_(5).await;

            tui.draw();
            let output = libtiny_tui::test_utils::buffer_str(
                &tui.get_front_buffer(),
                DEFAULT_TUI_WIDTH,
                HEIGHT,
            );
            assert!(output.contains("Connection closed"), "{output:?}");
            assert!(
                output.contains("Disconnected. Will try to reconnect in")
                    && output.contains("30 seconds."),
                "{output:?}"
            );

            ui.record_sledserv_request(
                MsgSource::Serv {
                    serv: SERV_NAME.to_owned(),
                },
                "shutdown sledteam",
            );
            snd_conn_ev
                .send(client::Event::Msg(Msg {
                    tags: Vec::new(),
                    pfx: Some(Pfx::User {
                        nick: "SledServ".to_owned(),
                        user: "sledserv-service@localhost".to_owned(),
                    }),
                    cmd: Cmd::PRIVMSG {
                        target: MsgTarget::User("osa1".to_owned()),
                        msg: r#"{"schema_version":1,"command":"runtime.shutdown","status":"error","error":{"code":"shutdown_request_failed","message":"Could not contact the runtime."}}"#.to_owned(),
                        is_notice: false,
                        ctcp: None,
                    },
                }))
                .await
                .unwrap();
            yield_(5).await;

            assert!(!ui.intentional_shutdown_pending(SERV_NAME));
            snd_conn_ev
                .send(client::Event::ConnectionClosed)
                .await
                .unwrap();
            snd_conn_ev.send(client::Event::Disconnected).await.unwrap();
            yield_(5).await;

            tui.draw();
            let output = libtiny_tui::test_utils::buffer_str(
                &tui.get_front_buffer(),
                DEFAULT_TUI_WIDTH,
                HEIGHT,
            );
            assert!(output.contains("shutdown_request_failed"));
            assert!(output.contains("Connection closed"));
            assert!(output.contains("Disconnected. Will try to reconnect in"));
            assert!(output.contains("30 seconds."));
            drop(snd_input_ev);
        },
    )
}

#[test]
fn test_privmsg_targetmask_issue_278() {
    run_test(
        "osa1".to_owned(),
        |TestSetup {
             tui,
             snd_input_ev,
             snd_conn_ev,
             ..
         }| async move {
            next_tab(&snd_input_ev).await;
            snd_conn_ev.send(client::Event::Connected).await.unwrap();
            snd_conn_ev
                .send(client::Event::NickChange {
                    new_nick: "osa1".to_owned(),
                })
                .await
                .unwrap();

            snd_conn_ev
                .send(client::Event::Msg(Msg {
                    tags: Vec::new(),
                    pfx: Some(Pfx::User {
                        nick: "tiny_test_user".to_owned(),
                        user: "e@a/b/c.d".to_owned(),
                    }),
                    cmd: Cmd::PRIVMSG {
                        target: MsgTarget::User("$$*".to_owned()),
                        // This generates a notification when the test is run with
                        // desktop-notifications feature, so show a helpful message to not confuse
                        // users (#371)
                        msg: "this is a test in tiny IRC client -- please ignore".to_owned(),
                        is_notice: true,
                        ctcp: None,
                    },
                }))
                .await
                .unwrap();

            yield_(3).await;

            next_tab(&snd_input_ev).await;

            tui.draw();

            yield_(3).await;

            #[rustfmt::skip]
            let screen =
            "|                                        |
             |00:00 tiny_test_user: this is a test in |
             |tiny IRC client -- please ignore        |
             |osa1:                                   |
             |x.y.z tiny_test_user                    |";

            let mut front_buffer = tui.get_front_buffer();
            normalize_timestamps(&mut front_buffer, DEFAULT_TUI_WIDTH, DEFAULT_TUI_HEIGHT);
            expect_screen(
                screen,
                &front_buffer,
                DEFAULT_TUI_WIDTH,
                DEFAULT_TUI_HEIGHT,
                Location::caller(),
            );
        },
    )
}

async fn next_tab(snd_input_ev: &mpsc::Sender<input::Event>) {
    snd_input_ev
        .send(term_input::Event::Key(term_input::Key::Ctrl('n')))
        .await
        .unwrap();
}

async fn yield_(n: usize) {
    for _ in 0..n {
        tokio::task::yield_now().await;
    }
}

/// Makes all timestamps 00:00
fn normalize_timestamps(cells: &mut CellBuf, w: u16, h: u16) {
    let cells = &mut cells.cells;
    for y in 0..h {
        let x = (w * y) as usize;
        if cells[x].ch.is_ascii_digit()
            && cells[x + 1].ch.is_ascii_digit()
            && cells[x + 2].ch == ':'
            && cells[x + 3].ch.is_ascii_digit()
            && cells[x + 4].ch.is_ascii_digit()
        {
            cells[x].ch = '0';
            cells[x + 1].ch = '0';
            cells[x + 2].ch = ':';
            cells[x + 3].ch = '0';
            cells[x + 4].ch = '0';
        }
    }
}

fn buffer_text(buffer: &CellBuf, width: u16, height: u16) -> String {
    (0..height)
        .map(|row| {
            let start = (row * width) as usize;
            buffer.cells[start..start + width as usize]
                .iter()
                .map(|cell| cell.ch)
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
