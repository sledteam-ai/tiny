use super::*;

#[test]
fn test_sledserv_aliases_send_origin_and_preserve_local_placement() {
    use libtiny_common::ChanName;
    use libtiny_tui::TUI;
    use libtiny_tui::test_utils::buffer_str;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;
    use tokio::runtime::Builder;
    use tokio::sync::mpsc;
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::ReceiverStream;

    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&runtime, async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::task::spawn_local(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read_half, mut write_half) = tokio::io::split(stream);
            let mut lines = BufReader::new(read_half).lines();
            let mut privmsgs = Vec::new();

            while let Some(line) = lines.next_line().await.unwrap() {
                if line.starts_with("USER ") {
                    write_half
                        .write_all(b":test 001 tiny-test :Welcome\r\n")
                        .await
                        .unwrap();
                } else if line.starts_with("PRIVMSG ") {
                    privmsgs.push(line);
                    if privmsgs.len() == 11 {
                        break;
                    }
                }
            }

            privmsgs
        });

        let server_info = ServerInfo {
            addr: "127.0.0.1".to_owned(),
            port,
            tls: false,
            pass: None,
            user: None,
            realname: "Tiny test".to_owned(),
            nicks: vec!["tiny-test".to_owned()],
            auto_join: Vec::new(),
            nickserv_ident: None,
            sasl_auth: None,
        };
        let (client, mut client_events) = Client::new(server_info);

        while !matches!(
            client_events.recv().await,
            Some(libtiny_client::Event::Connected)
        ) {}

        let (snd_input, rcv_input) = mpsc::channel(1);
        let input = ReceiverStream::new(rcv_input).map(Ok);
        const TUI_HEIGHT: u16 = 10;
        let (tui, _tui_events) = TUI::run_test(40, TUI_HEIGHT, input);
        let ui = UI::new(
            tui.clone(),
            None,
            crate::sledteam_session::SledteamSession::default(),
        );
        ui.new_server_tab("127.0.0.1", None);
        let chan = ChanName::new("#01J00000000000000000000000".to_owned());
        ui.new_chan_tab("127.0.0.1", &chan);
        let source = MsgSource::Chan {
            serv: "127.0.0.1".to_owned(),
            chan,
        };
        let defaults = Defaults {
            nicks: vec!["tiny-test".to_owned()],
            realname: "Tiny test".to_owned(),
            join: Vec::new(),
            tls: false,
        };
        let mut clients = vec![client];

        run_cmd(
            "ets unexpected",
            source.clone(),
            &defaults,
            &ui,
            &mut clients,
        );
        ui.draw();
        assert!(buffer_str(&tui.get_front_buffer(), 40, TUI_HEIGHT).contains("Usage: /ets"));

        run_cmd(
            "shutdown sledteam",
            source.clone(),
            &defaults,
            &ui,
            &mut clients,
        );
        ui.draw();
        assert!(buffer_str(&tui.get_front_buffer(), 40, TUI_HEIGHT).contains("Usage: /shutdown"));

        run_cmd(&format!("trail add {}", "x".repeat(600)), source.clone(), &defaults, &ui, &mut clients);
        assert!(ui.pending_sledserv_origins().is_empty());
        run_cmd("ets", source.clone(), &defaults, &ui, &mut clients);
        assert!(!ui.user_tab_exists("127.0.0.1", crate::sledserv::NICK));
        run_cmd(
            "msg SledServ ets",
            source.clone(),
            &defaults,
            &ui,
            &mut clients,
        );
        run_cmd(
            "expedition add Find Magnetic North --project-root /tmp/fmn",
            source.clone(),
            &defaults,
            &ui,
            &mut clients,
        );
        run_cmd(
            "msg SledServ expedition add Find Magnetic North --project-root /tmp/fmn",
            source.clone(),
            &defaults,
            &ui,
            &mut clients,
        );
        run_cmd(
            "trail add buy supplies",
            source.clone(),
            &defaults,
            &ui,
            &mut clients,
        );
        run_cmd(
            "msg SledServ trail add buy supplies",
            source.clone(),
            &defaults,
            &ui,
            &mut clients,
        );
        run_cmd(
            "span add write schema",
            source.clone(),
            &defaults,
            &ui,
            &mut clients,
        );
        run_cmd(
            "msg SledServ span add write schema",
            source.clone(),
            &defaults,
            &ui,
            &mut clients,
        );
        run_cmd("shutdown", source.clone(), &defaults, &ui, &mut clients);
        run_cmd(
            "msg SledServ shutdown",
            source.clone(),
            &defaults,
            &ui,
            &mut clients,
        );

        run_cmd("join wheel", source.clone(), &defaults, &ui, &mut clients);
        assert_eq!(
            ui.pending_sledserv_origins(),
            [
                source.clone(),
                source.clone(),
                source.clone(),
                source.clone(),
                source.clone(),
                source
            ]
        );

        let mut ids = std::collections::HashSet::new();
        let messages: Vec<_> = server.await.unwrap().into_iter().map(|line| {
            let body = line.strip_prefix("PRIVMSG SledServ :").unwrap();
            if !body.starts_with('{') { return line; }
            let mut envelope: serde_json::Value = serde_json::from_str(body).unwrap();
            let id = envelope.as_object_mut().unwrap().remove("request_id").unwrap();
            let id = id.as_str().unwrap();
            assert!(id.parse::<ulid::Ulid>().is_ok());
            assert!(ids.insert(id.to_owned()));
            format!("PRIVMSG SledServ :{envelope}")
        }).collect();
        assert_eq!(ids.len(), 6);
        assert_eq!(
            messages,
            vec![
                format!("PRIVMSG SledServ :{}", serde_json::json!({"schema_version":1,"command":"ets","origin":"01J00000000000000000000000"})).to_owned(),
                "PRIVMSG SledServ :ets".to_owned(),
                format!("PRIVMSG SledServ :{}", serde_json::json!({"schema_version":1,"command":"expedition add Find Magnetic North --project-root /tmp/fmn","origin":"01J00000000000000000000000"}))
                    .to_owned(),
                "PRIVMSG SledServ :expedition add Find Magnetic North --project-root /tmp/fmn"
                    .to_owned(),
                format!("PRIVMSG SledServ :{}", serde_json::json!({"schema_version":1,"command":"trail add buy supplies","origin":"01J00000000000000000000000"})).to_owned(),
                "PRIVMSG SledServ :trail add buy supplies".to_owned(),
                format!("PRIVMSG SledServ :{}", serde_json::json!({"schema_version":1,"command":"span add write schema","origin":"01J00000000000000000000000"})).to_owned(),
                "PRIVMSG SledServ :span add write schema".to_owned(),
                format!("PRIVMSG SledServ :{}", serde_json::json!({"schema_version":1,"command":"shutdown","origin":"01J00000000000000000000000"})).to_owned(),
                "PRIVMSG SledServ :shutdown".to_owned(),
                format!("PRIVMSG SledServ :{}", serde_json::json!({"schema_version":1,"command":"channel resolve wheel","origin":null})),
            ]
        );

        run_cmd("close", MsgSource::Serv { serv: "127.0.0.1".into() }, &defaults, &ui, &mut clients);
        assert!(ui.pending_sledserv_origins().is_empty());
        drop(snd_input);
    });
}
