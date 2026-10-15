//! Sledteam-specific commands forwarded to SledServ.

use super::{Cmd, CmdArgs, find_client};
use libtiny_common::{MsgSource, MsgTarget};

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
    let request = encode_request(&src, &msg);
    if client
        .split_privmsg(crate::sledserv::NICK.len(), &request)
        .next()
        != Some(&request)
    {
        ui.show_command_feedback(
            &["SledServ request exceeds one IRC message; shorten the command.".to_owned()],
            &MsgTarget::CurrentTab,
        );
        return;
    }
    client.privmsg(crate::sledserv::NICK, &request, false);
    ui.record_sledserv_request(src, &msg);
}

// An address is only a candidate Terrain identity. SledServ determines its kind.
fn encode_request(src: &MsgSource, command: &str) -> String {
    let origin = match src {
        MsgSource::Chan { chan, .. } => chan
            .display()
            .strip_prefix('#')
            .filter(|id| crate::sledteam_session::is_canonical_ulid(id)),
        _ => None,
    };
    serde_json::json!({"schema_version":1,"command":command,"origin":origin}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use libtiny_common::ChanName;

    #[test]
    fn sends_only_canonical_addresses_and_preserves_command_body() {
        for (channel, expected) in [
            (
                "#01J00000000000000000000000",
                Some("01J00000000000000000000000"),
            ),
            (
                "#01J00000000000000000000001",
                Some("01J00000000000000000000001"),
            ),
            ("#a label", None),
            ("#01j00000000000000000000000", None),
        ] {
            let source = MsgSource::Chan {
                serv: "irc".into(),
                chan: ChanName::new(channel.into()),
            };
            let command = "trail add a \"quoted\" name {with braces}";
            let value: serde_json::Value =
                serde_json::from_str(&encode_request(&source, command)).unwrap();
            assert_eq!(value["origin"], serde_json::json!(expected));
            assert_eq!(value["command"], command);
        }
        for source in [
            MsgSource::Serv { serv: "irc".into() },
            MsgSource::User {
                serv: "irc".into(),
                nick: "alice".into(),
            },
        ] {
            let value: serde_json::Value =
                serde_json::from_str(&encode_request(&source, "ets")).unwrap();
            assert!(value["origin"].is_null());
        }
    }
}
