use crate::config::Defaults;
use crate::ui::UI;
use crate::utils;
use libtiny_client::{Client, ServerInfo};
use libtiny_common::{ChanNameRef, CommandInfo, MsgSource, MsgTarget};
use libtiny_tui::config::Chan;

use std::borrow::Borrow;

pub(crate) fn run_cmd(
    cmd: &str,
    src: MsgSource,
    defaults: &Defaults,
    ui: &UI,
    clients: &mut Vec<Client>,
) {
    match parse_cmd(cmd) {
        Some(ParsedCmd { cmd, args }) => {
            let cmd_args = CmdArgs {
                args,
                defaults,
                ui,
                clients,
                src,
            };
            (cmd.cmd_fn)(cmd_args);
        }

        None => {
            ui.add_client_err_msg(
                &format!("Unsupported command: \"/{cmd}\""),
                &MsgTarget::CurrentTab,
            );
        }
    }
}

struct ParsedCmd<'a> {
    cmd: &'static Cmd,

    /// Rest of the command after extracting command name.
    args: &'a str,
}

fn parse_cmd(cmd: &str) -> Option<ParsedCmd<'_>> {
    let cmd_name = cmd.split_whitespace().next()?;
    let mut ws_idxs = utils::split_whitespace_indices(cmd);
    ws_idxs.next(); // cmd_name
    let rest = match ws_idxs.next() {
        None => "",
        Some(rest_idx) => &cmd[rest_idx..],
    };
    for cmd in &CMDS {
        if cmd_name == cmd.name {
            return Some(ParsedCmd { cmd, args: rest });
        }
    }
    None
}

struct CmdArgs<'a> {
    args: &'a str,
    defaults: &'a Defaults,
    ui: &'a UI,
    clients: &'a mut Vec<Client>,
    src: MsgSource,
}

struct Cmd {
    /// Command name. If this is `"cmd"`, `/cmd ...` will call this command.
    name: &'static str,

    /// Command function.
    cmd_fn: fn(CmdArgs),

    /// Short user-facing command summary.
    summary: &'static str,

    /// Command usage. Shown in `/help` and error messages.
    usage: &'static str,
}

impl Cmd {
    const fn info(&self) -> CommandInfo {
        CommandInfo::new(self.name, self.usage, self.summary)
    }
}

fn find_client_idx(clients: &[Client], serv_name: &str) -> Option<usize> {
    for (client_idx, client) in clients.iter().enumerate() {
        if client.get_serv_name() == serv_name {
            return Some(client_idx);
        }
    }
    None
}

fn find_client<'a>(clients: &'a mut [Client], serv_name: &str) -> Option<&'a mut Client> {
    match find_client_idx(clients, serv_name) {
        None => None,
        Some(idx) => Some(&mut clients[idx]),
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////

static CMDS: [&Cmd; 14] = [
    &AWAY_CMD,
    &CLOSE_CMD,
    &CMDS_CMD,
    &CONNECT_CMD,
    &ETS_CMD,
    &EXPEDITION_CMD,
    &JOIN_CMD,
    &ME_CMD,
    &MSG_CMD,
    &NAMES_CMD,
    &NICK_CMD,
    &SPAN_CMD,
    &TRAIL_CMD,
    &HELP_CMD,
];

pub(crate) fn command_infos() -> impl Iterator<Item = CommandInfo> {
    libtiny_tui::command_infos()
        .iter()
        .copied()
        .chain(CMDS.iter().map(|cmd| cmd.info()))
}

////////////////////////////////////////////////////////////////////////////////////////////////////

static CMDS_CMD: Cmd = Cmd {
    name: "?",
    cmd_fn: cmds,
    summary: "Lists available commands",
    usage: "`/?`",
};

fn cmds(args: CmdArgs) {
    if !args.args.is_empty() {
        return args.ui.add_client_err_msg(
            &format!("Usage: {}", CMDS_CMD.usage),
            &MsgTarget::CurrentTab,
        );
    }

    args.ui
        .add_client_msg("Available Commands:", &MsgTarget::CurrentTab);
    for info in command_infos() {
        args.ui.add_client_msg(
            &format!("{:<45} - {}", info.usage, info.summary),
            &MsgTarget::CurrentTab,
        );
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////

static ETS_CMD: Cmd = Cmd {
    name: "ets",
    cmd_fn: ets,
    summary: "Requests ETS data from SledServ",
    usage: "`/ets`",
};

fn ets(args: CmdArgs) {
    if !args.args.is_empty() {
        return args
            .ui
            .add_client_err_msg(&format!("Usage: {}", ETS_CMD.usage), &MsgTarget::CurrentTab);
    }

    send_sledserv(args, "ets");
}

static EXPEDITION_CMD: Cmd = Cmd {
    name: "expedition",
    cmd_fn: expedition,
    summary: "Sends an expedition command to SledServ",
    usage: "`/expedition <arguments>`",
};

fn expedition(args: CmdArgs) {
    send_sledserv(args, "expedition");
}

static SPAN_CMD: Cmd = Cmd {
    name: "span",
    cmd_fn: span,
    summary: "Sends a span command to SledServ",
    usage: "`/span <arguments>`",
};

fn span(args: CmdArgs) {
    send_sledserv(args, "span");
}

static TRAIL_CMD: Cmd = Cmd {
    name: "trail",
    cmd_fn: trail,
    summary: "Sends a trail command to SledServ",
    usage: "`/trail <arguments>`",
};

fn trail(args: CmdArgs) {
    send_sledserv(args, "trail");
}

fn send_sledserv(args: CmdArgs, command: &str) {
    let CmdArgs {
        args,
        ui,
        clients,
        src,
        ..
    } = args;

    let msg = if args.is_empty() {
        command.to_owned()
    } else {
        format!("{command} {args}")
    };
    let client = find_client(clients, src.serv_name()).unwrap();
    for chunk in client.split_privmsg(crate::sledserv::NICK.len(), &msg) {
        client.privmsg(crate::sledserv::NICK, chunk, false);
        ui.record_sledserv_request(src.clone(), &msg);
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////

static AWAY_CMD: Cmd = Cmd {
    name: "away",
    cmd_fn: away,
    summary: "Sets/removes away message",
    usage: "`/away` or `/away <message>`",
};

fn away(args: CmdArgs) {
    let msg = if args.args.is_empty() {
        None
    } else {
        Some(args.args)
    };
    if let Some(client) = find_client(args.clients, args.src.serv_name()) {
        client.away(msg);
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////

static CLOSE_CMD: Cmd = Cmd {
    name: "close",
    cmd_fn: close,
    summary: "Closes current tab",
    usage: "`/close` or `/close <reason>`",
};

fn close(args: CmdArgs) {
    let CmdArgs {
        args,
        ui,
        clients,
        src,
        ..
    } = args;
    match src {
        MsgSource::Serv { ref serv } if serv == "mentions" => {
            // ignore
        }
        MsgSource::Serv { serv } => {
            ui.close_server_tab(&serv);
            let client_idx = find_client_idx(clients, &serv).unwrap();
            // TODO: this probably won't close the connection?
            let mut client = clients.remove(client_idx);
            if args.is_empty() {
                client.quit(None);
            } else {
                client.quit(Some(args.to_string()));
            }
        }
        MsgSource::Chan { serv, chan } => {
            ui.close_chan_tab(&serv, chan.borrow());
            let client_idx = find_client_idx(clients, &serv).unwrap();
            if args.is_empty() {
                clients[client_idx].part(&chan, None);
            } else {
                clients[client_idx].part(&chan, Some(args.to_string()));
            }
        }
        MsgSource::User { serv, nick } => {
            ui.close_user_tab(&serv, &nick);
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////

static CONNECT_CMD: Cmd = Cmd {
    name: "connect",
    cmd_fn: connect,
    summary: "Connects to a server",
    usage: "`/connect <host>:<port>` or `/connect` to reconnect",
};

fn connect(args: CmdArgs) {
    let CmdArgs {
        args,
        defaults,
        ui,
        clients,
        src,
        ..
    } = args;
    let words: Vec<&str> = args.split_whitespace().collect();

    match words.len() {
        0 => reconnect(ui, clients, src),
        1 => connect_(words[0], None, defaults, ui, clients),
        2 => connect_(words[0], Some(words[1]), defaults, ui, clients),
        _ => ui.add_client_err_msg(
            &format!("Usage: {}", CONNECT_CMD.usage),
            &MsgTarget::CurrentTab,
        ),
    }
}

fn reconnect(ui: &UI, clients: &mut [Client], src: MsgSource) {
    if let Some(client) = find_client(clients, src.serv_name()) {
        ui.add_client_msg(
            "Reconnecting...",
            &MsgTarget::AllServTabs {
                serv: src.serv_name(),
            },
        );
        client.reconnect(None);
    }
}

fn connect_(
    serv_addr: &str,
    pass: Option<&str>,
    defaults: &Defaults,
    ui: &UI,
    clients: &mut Vec<Client>,
) {
    fn split_port(s: &str) -> Option<(&str, &str)> {
        s.find(':').map(|split| (&s[0..split], &s[split + 1..]))
    }

    // parse host name and port
    let (serv_name, serv_port) = {
        match split_port(serv_addr) {
            None => {
                return ui
                    .add_client_err_msg("connect: Need a <host>:<port>", &MsgTarget::CurrentTab);
            }
            Some((serv_name, serv_port)) => match serv_port.parse::<u16>() {
                Err(err) => {
                    return ui.add_client_err_msg(
                        &format!("connect: Can't parse port {serv_port}: {err}"),
                        &MsgTarget::CurrentTab,
                    );
                }
                Ok(serv_port) => (serv_name, serv_port),
            },
        }
    };

    // if we already connected to this server reconnect using new port
    if let Some(client) = find_client(clients, serv_name) {
        ui.add_client_msg("Connecting...", &MsgTarget::AllServTabs { serv: serv_name });
        client.reconnect(Some(serv_port));
        return;
    }

    // otherwise create a new connection
    // can't move the rest to an else branch because of borrowchk

    // otherwise create a new Conn, tab etc.
    ui.new_server_tab(serv_name, None);
    let msg_target = MsgTarget::Server { serv: serv_name };
    ui.add_client_msg("Connecting...", &msg_target);

    let (client, rcv_ev) = Client::new(ServerInfo {
        addr: serv_name.to_owned(),
        port: serv_port,
        tls: defaults.tls,
        user: None,
        realname: defaults.realname.clone(),
        pass: pass.map(str::to_owned),
        nicks: defaults.nicks.clone(),
        auto_join: defaults
            .join
            .iter()
            .map(|c| ChanNameRef::new(c).to_owned())
            .collect(),
        nickserv_ident: None,
        sasl_auth: None,
    });

    // Spawn UI task
    let ui_clone = ui.clone();
    let client_clone = client.clone();
    tokio::task::spawn_local(crate::conn::task(rcv_ev, ui_clone, Box::new(client_clone)));

    clients.push(client);
}

////////////////////////////////////////////////////////////////////////////////////////////////////

static JOIN_CMD: Cmd = Cmd {
    name: "join",
    cmd_fn: join,
    summary: "Joins a channel",
    usage: "`/join <chan1> [-ignore] [-notify [off|mentions|messages]],<chan2>...` or `/join` in a channel tab to rejoin",
};

fn join(args: CmdArgs) {
    let CmdArgs {
        args,
        ui,
        clients,
        src,
        ..
    } = args;

    if let MsgSource::Serv { serv } = &src
        && serv == "mentions"
    {
        return ui.add_client_err_msg(
            "Switch to a server tab to join a channel",
            &MsgTarget::CurrentTab,
        );
    }

    let chans = args
        .split(',')
        .map(str::trim)
        .filter_map(|c| match Chan::from_cmd_args(c) {
            Ok(c) => Some(c),
            Err(err) => {
                ui.add_client_err_msg(&err, &MsgTarget::CurrentTab);
                None
            }
        })
        .collect::<Vec<Chan>>();

    let chans = if chans.is_empty() {
        match ui.current_tab() {
            None => return,
            Some(MsgSource::Chan { serv, chan }) => {
                // Rejoin current tab's channel.
                let config = ui.get_tab_config(&serv, Some(chan.as_ref()));
                vec![Chan::WithConfig { name: chan, config }]
            }
            Some(MsgSource::Serv { .. } | MsgSource::User { .. }) => {
                return ui.add_client_err_msg(
                    &format!("Usage: {}", JOIN_CMD.usage),
                    &MsgTarget::CurrentTab,
                );
            }
        }
    } else {
        chans
    };

    let serv = src.serv_name();
    match find_client(clients, serv) {
        Some(client) => {
            let iter_ref = chans.iter().map(|c| c.name());
            // set tab configs of new channel tabs (creates new tab)
            for chan in &chans {
                match chan {
                    Chan::Name(name) => {
                        let config = ui.get_tab_config(serv, Some(name.as_ref()));
                        ui.set_tab_config(serv, Some(name), config)
                    }
                    Chan::WithConfig { name, config } => {
                        ui.set_tab_config(serv, Some(name), config.to_owned())
                    }
                }
            }
            client.join(iter_ref);
        }
        None => ui.add_client_err_msg(
            &format!("Can't join: Not connected to server {}", src.serv_name()),
            &MsgTarget::CurrentTab,
        ),
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////

static ME_CMD: Cmd = Cmd {
    name: "me",
    cmd_fn: me,
    summary: "Sends emote message",
    usage: "`/me <message>`",
};

fn me(args: CmdArgs) {
    let CmdArgs {
        args,
        ui,
        clients,
        src,
        ..
    } = args;
    if args.is_empty() {
        return ui.add_client_err_msg(&format!("Usage: {}", ME_CMD.usage), &MsgTarget::CurrentTab);
    }
    crate::ui::send_msg(ui, clients, &src, args.to_string(), true);
}

////////////////////////////////////////////////////////////////////////////////////////////////////

static MSG_CMD: Cmd = Cmd {
    name: "msg",
    cmd_fn: msg,
    summary: "Sends a message to a user",
    usage: "`/msg <nick> <message>`",
};

fn split_msg_args(args: &str) -> Option<(&str, &str)> {
    let mut char_indices = args.char_indices();

    // We could check for validity of the nick according to RFC 2812 but we do the simple thing for
    // now and and only check the first character, to avoid confusing the UI by returning a
    // `MsgSource::User` with a channel name as `nick`.
    match char_indices.next() {
        None => {
            return None;
        }
        Some((_, c)) => {
            if !utils::is_nick_first_char(c) {
                return None;
            }
        }
    }

    for (i, c) in char_indices {
        if c.is_whitespace() {
            return Some((&args[0..i], &args[i + 1..]));
        }
    }

    None
}

fn msg(args: CmdArgs) {
    let CmdArgs {
        args,
        ui,
        clients,
        src,
        ..
    } = args;
    let fail = || {
        ui.add_client_err_msg(&format!("Usage: {}", MSG_CMD.usage), &MsgTarget::CurrentTab);
    };

    let (target, msg) = match split_msg_args(args) {
        None => return fail(),
        Some((target, msg)) => {
            if msg.is_empty() {
                return fail();
            } else {
                (target, msg)
            }
        }
    };

    let src = if clients
        .iter()
        .any(|client| client.get_serv_name() == target)
    {
        MsgSource::Serv {
            serv: target.to_owned(),
        }
    } else {
        let serv = src.serv_name();
        MsgSource::User {
            serv: serv.to_owned(),
            nick: target.to_owned(),
        }
    };

    crate::ui::send_msg(ui, clients, &src, msg.to_owned(), false);
}

////////////////////////////////////////////////////////////////////////////////////////////////////

static NAMES_CMD: Cmd = Cmd {
    name: "names",
    cmd_fn: names,
    summary: "Shows users in channel",
    usage: "`/names`",
};

fn names(args: CmdArgs) {
    let CmdArgs {
        args,
        ui,
        src,
        clients,
        ..
    } = args;
    let words: Vec<&str> = args.split_whitespace().collect();

    let client = match find_client(clients, src.serv_name()) {
        None => {
            return;
        }
        Some(client) => client,
    };

    if let MsgSource::Chan { ref serv, ref chan } = src {
        let nicks_vec = client.get_chan_nicks(chan);
        let target = MsgTarget::Chan { serv, chan };
        if words.is_empty() {
            ui.add_client_msg(
                &format!("{} users: {}", nicks_vec.len(), nicks_vec.join(", ")),
                &target,
            );
        } else {
            let nick = words[0];
            if nicks_vec.iter().any(|v| v == nick) {
                ui.add_client_msg(&format!("{nick} is online"), &target);
            } else {
                ui.add_client_msg(&format!("{nick} is not in the channel"), &target);
            }
        }
    } else {
        ui.add_client_err_msg("/names only supported in chan tabs", &MsgTarget::CurrentTab);
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////

static NICK_CMD: Cmd = Cmd {
    name: "nick",
    cmd_fn: nick,
    summary: "Sets your nick",
    usage: "`/nick <nick>`",
};

fn nick(args: CmdArgs) {
    let CmdArgs {
        args,
        ui,
        clients,
        src,
        ..
    } = args;
    let words: Vec<&str> = args.split_whitespace().collect();
    if words.len() == 1 {
        if let Some(client) = find_client(clients, src.serv_name()) {
            let new_nick = words[0];
            client.nick(new_nick);
        }
    } else {
        ui.add_client_err_msg(
            &format!("Usage: {}", NICK_CMD.usage),
            &MsgTarget::CurrentTab,
        );
    }
}

static HELP_CMD: Cmd = Cmd {
    name: "help",
    cmd_fn: help,
    summary: "Displays this message",
    usage: "`/help`",
};

fn help(args: CmdArgs) {
    let CmdArgs { ui, .. } = args;
    ui.add_client_msg("Client Commands:", &MsgTarget::CurrentTab);
    for cmd in CMDS.iter() {
        ui.add_client_msg(
            &format!(
                "/{:<10} - {:<25} - Usage: {}",
                cmd.name, cmd.summary, cmd.usage
            ),
            &MsgTarget::CurrentTab,
        )
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////

#[test]
fn test_parse_cmd() {
    let ParsedCmd { cmd, args } = parse_cmd("msg NickServ identify notMyPassword").unwrap();
    assert_eq!(cmd.name, "msg");
    assert_eq!(args, "NickServ identify notMyPassword");

    let ParsedCmd { cmd, args } = parse_cmd("join #foo").unwrap();
    assert_eq!(cmd.name, "join");
    assert_eq!(args, "#foo");

    let ParsedCmd { cmd, args } = parse_cmd("ets").unwrap();
    assert_eq!(cmd.name, "ets");
    assert_eq!(args, "");

    let ParsedCmd { cmd, args } = parse_cmd("?").unwrap();
    assert_eq!(cmd.name, "?");
    assert_eq!(args, "");

    let ParsedCmd { cmd, args } =
        parse_cmd("expedition add Find Magnetic North --project-root /tmp/fmn").unwrap();
    assert_eq!(cmd.name, "expedition");
    assert_eq!(args, "add Find Magnetic North --project-root /tmp/fmn");

    let ParsedCmd { cmd, args } = parse_cmd("trail add buy supplies").unwrap();
    assert_eq!(cmd.name, "trail");
    assert_eq!(args, "add buy supplies");

    let ParsedCmd { cmd, args } = parse_cmd("span add write schema").unwrap();
    assert_eq!(cmd.name, "span");
    assert_eq!(args, "add write schema");

    assert!(parse_cmd("bearings").is_none());
}

#[test]
fn test_command_metadata() {
    use std::collections::HashSet;

    let infos = command_infos().collect::<Vec<_>>();
    let mut names = HashSet::new();
    for info in &infos {
        assert!(names.insert(info.name), "duplicate command: {}", info.name);
        assert!(
            !info.summary.trim().is_empty(),
            "{} has no summary",
            info.name
        );
        assert!(
            info.usage.starts_with("`/"),
            "{} has invalid usage: {}",
            info.name,
            info.usage
        );
    }

    for name in ["?", "ets", "expedition", "trail", "span"] {
        assert!(names.contains(name), "missing command metadata: {name}");
    }

    for cmd in CMDS {
        let parsed = parse_cmd(cmd.name).unwrap();
        assert_eq!(parsed.cmd.info(), cmd.info());
    }
}

#[test]
fn test_cmds_is_local_and_uses_command_metadata() {
    use libtiny_tui::TUI;
    use libtiny_tui::test_utils::buffer_str;
    use tokio::runtime::Builder;
    use tokio::sync::mpsc;
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::ReceiverStream;

    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&runtime, async {
        let (snd_input, rcv_input) = mpsc::channel(1);
        let input = ReceiverStream::new(rcv_input).map(Ok);
        let (tui, _tui_events) = TUI::run_test(160, 60, input);
        let ui = UI::new(tui.clone(), None);
        ui.new_server_tab("test-server", None);
        let defaults = Defaults {
            nicks: vec!["tiny-test".to_owned()],
            realname: "Tiny test".to_owned(),
            join: Vec::new(),
            tls: false,
        };

        // An empty client list also verifies that this local command does not use the IRC send
        // path, which requires a client for the current server.
        run_cmd(
            "?",
            MsgSource::Serv {
                serv: "test-server".to_owned(),
            },
            &defaults,
            &ui,
            &mut Vec::new(),
        );
        ui.draw();

        let output = buffer_str(&tui.get_front_buffer(), 160, 60);
        assert!(output.contains("Available Commands:"));
        for info in command_infos() {
            assert!(
                output.contains(&format!("{:<45} - {}", info.usage, info.summary)),
                "missing command output for {}",
                info.name
            );
        }

        drop(snd_input);
    });
}

#[test]
fn test_msg_args() {
    assert_eq!(split_msg_args("foo,bar"), None);
    assert_eq!(split_msg_args("foo bar"), Some(("foo", "bar")));
    assert_eq!(split_msg_args("foo, bar"), Some(("foo,", "bar"))); // nick not valid according to RFC but whatever
    assert_eq!(split_msg_args("foo ,bar"), Some(("foo", ",bar")));
    assert_eq!(split_msg_args("#blah blah"), None);
}

#[test]
fn test_sledserv_aliases_match_msg_wire_actions() {
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
                    if privmsgs.len() == 8 {
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
        let (tui, _tui_events) = TUI::run_test(40, 5, input);
        let ui = UI::new(tui.clone(), None);
        ui.new_server_tab("127.0.0.1", None);
        let chan = ChanName::new("#current-channel".to_owned());
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
        assert!(buffer_str(&tui.get_front_buffer(), 40, 5).contains("Usage: `/ets`"));

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

        assert_eq!(
            ui.pending_sledserv_origins(),
            [source.clone(), source.clone(), source.clone(), source]
        );

        assert_eq!(
            server.await.unwrap(),
            vec![
                "PRIVMSG SledServ :ets".to_owned(),
                "PRIVMSG SledServ :ets".to_owned(),
                "PRIVMSG SledServ :expedition add Find Magnetic North --project-root /tmp/fmn"
                    .to_owned(),
                "PRIVMSG SledServ :expedition add Find Magnetic North --project-root /tmp/fmn"
                    .to_owned(),
                "PRIVMSG SledServ :trail add buy supplies".to_owned(),
                "PRIVMSG SledServ :trail add buy supplies".to_owned(),
                "PRIVMSG SledServ :span add write schema".to_owned(),
                "PRIVMSG SledServ :span add write schema".to_owned(),
            ]
        );

        drop(snd_input);
    });
}
