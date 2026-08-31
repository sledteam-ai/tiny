//! Sledteam-specific commands forwarded to SledServ.

use super::{Cmd, CmdArgs, find_client};
use libtiny_common::MsgTarget;

static COMMANDS: [&Cmd; 5] = [
    &ETS_CMD,
    &EXPEDITION_CMD,
    &SPAN_CMD,
    &TRAIL_CMD,
    &SHUTDOWN_CMD,
];

pub(super) fn commands() -> impl Iterator<Item = &'static Cmd> {
    COMMANDS.into_iter()
}

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
            .show_command_feedback(&["Usage: /ets".to_owned()], &MsgTarget::CurrentTab);
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

static SHUTDOWN_CMD: Cmd = Cmd {
    name: "shutdown",
    cmd_fn: shutdown,
    summary: "Shuts down the running Sledteam runtime",
    usage: "`/shutdown`",
};

fn shutdown(args: CmdArgs) {
    if !args.args.is_empty() {
        return args
            .ui
            .show_command_feedback(&["Usage: /shutdown".to_owned()], &MsgTarget::CurrentTab);
    }

    send_sledserv(args, "shutdown");
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
