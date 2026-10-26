use super::*;
use libtiny_common::ChanName;

fn channel(serv: &str, chan: &str) -> MsgSource {
    MsgSource::Chan {
        serv: serv.to_owned(),
        chan: ChanName::new(chan.to_owned()),
    }
}

fn ok_response(command: &str, data: &str) -> Response {
    serde_yaml::from_str(&format!(
            r#"{{"schema_version":1,"request_id":"01J00000000000000000000000","command":"{command}","status":"ok","data":{data}}}"#
        ))
        .unwrap()
}

fn tree_response(command: &str, label: &str, data: &str) -> Vec<String> {
    format_response(ok_response(command, data), label)
}

#[test]
fn matches_out_of_order_responses_by_command() {
    let pending = PendingRequests::default();
    pending.record(
        channel("irc", "#first"),
        "trail",
        "01J00000000000000000000000",
    );
    pending.record(
        channel("irc", "#second"),
        "ets",
        "01J00000000000000000000001",
    );

    let response = pending
            .consume(
                "irc",
                "sledserv",
                r#"{"schema_version":1,"request_id":"01J00000000000000000000001","command":"travel.ets","status":"ok","data":{"expedition":{"ulid":"E1","name":"Moonshot"},"trail":{"ulid":"T1","name":"North"},"spans":[]}}"#,
            )
            .unwrap();

    assert_eq!(response.origin, channel("irc", "#second"));
    assert_eq!(response.lines, ["ets", "Moonshot", "└── North"]);
    assert!(!response.is_feedback);
    assert_eq!(pending.origins(), [channel("irc", "#first")]);
}

#[test]
fn shutdown_acknowledgement_uses_runtime_command_and_local_message() {
    let pending = PendingRequests::default();
    let origin = channel("irc", "#01J00000000000000000000000");
    pending.record(origin.clone(), "shutdown", "01J00000000000000000000000");

    let response = pending
            .consume(
                "irc",
                NICK,
                r#"{"schema_version":1,"request_id":"01J00000000000000000000000","command":"runtime.shutdown","status":"ok","data":{"message":"Sledteam shutdown request sent."}}"#,
            )
            .unwrap();

    assert_eq!(response.origin, origin);
    assert_eq!(response.lines, ["Shutting down Sledteam…"]);
    assert!(pending.origins().is_empty());
}

#[test]
fn shutdown_error_shows_only_the_user_facing_message() {
    let pending = PendingRequests::default();
    pending.record(
        channel("irc", "#01J00000000000000000000000"),
        "shutdown",
        "01J00000000000000000000000",
    );

    let response = pending
            .consume(
                "irc",
                NICK,
                r#"{"schema_version":1,"request_id":"01J00000000000000000000000","command":"runtime.shutdown","status":"error","error":{"code":"shutdown_request_failed","message":"Could not contact the runtime."}}"#,
            )
            .unwrap();

    assert_eq!(response.lines, ["Could not contact the runtime."]);
    assert!(response.is_feedback);
    assert!(pending.origins().is_empty());
}

#[test]
fn syntax_errors_render_usage_without_protocol_details() {
    let cases = [
        (
            "trail",
            "travel.trail",
            r#"["trail add <name>","trail list [<expedition>]"]"#,
            vec![
                "Usage: /trail add <name>",
                "       /trail list [<expedition>]",
            ],
        ),
        (
            "expedition",
            "travel.expedition",
            r#"["expedition add <name> --project-root <absolute-project-root>","expedition list"]"#,
            vec![
                "Usage: /expedition add <name> --project-root <absolute-project-root>",
                "       /expedition list",
            ],
        ),
        (
            "span",
            "travel.span",
            r#"["span add <name>","span list [<trail>|<expedition>/<trail>/]"]"#,
            vec![
                "Usage: /span add <name>",
                "       /span list [<trail>|<expedition>/<trail>/]",
            ],
        ),
        (
            "shutdown foo",
            "runtime.shutdown",
            r#"["shutdown"]"#,
            vec!["Usage: /shutdown"],
        ),
    ];

    for (request, command, usage, expected) in cases {
        let pending = PendingRequests::default();
        pending.record(
            channel("irc", "#commands"),
            request,
            "01J00000000000000000000000",
        );
        let packet = format!(
            r#"{{"schema_version":1,"request_id":"01J00000000000000000000000","command":"{command}","status":"error","error":{{"code":"invalid_command_syntax","message":"Invalid command syntax.","usage":{usage}}}}}"#
        );
        let response = pending.consume("irc", NICK, &packet).unwrap();
        assert_eq!(response.lines, expected, "{request}");
        assert!(response.is_feedback, "{request}");
        assert!(response.lines.iter().all(|line| {
            !line.contains("SledServ")
                && !line.contains("invalid_command_syntax")
                && !line.contains("travel.")
                && !line.contains("runtime.")
        }));
    }
}

#[test]
fn unknown_and_domain_errors_keep_their_readable_messages() {
    for (request, command, code, message) in [
        (
            "bearings",
            "sledserv.command",
            "unknown_command",
            "Unknown command: /bearings",
        ),
        (
            "trail list missing",
            "travel.trail.list",
            "expedition_not_found",
            "An expedition named \"missing\" was not found.",
        ),
    ] {
        let pending = PendingRequests::default();
        pending.record(
            channel("irc", "#commands"),
            request,
            "01J00000000000000000000000",
        );
        let packet = format!(
            r#"{{"schema_version":1,"request_id":"01J00000000000000000000000","command":"{command}","status":"error","error":{{"code":"{code}","message":"{}"}}}}"#,
            message.replace('"', "\\\"")
        );
        let response = pending.consume("irc", NICK, &packet).unwrap();
        assert_eq!(response.lines, [message]);
        assert!(response.is_feedback, "{request}");
    }
}

#[test]
fn successful_state_change_remains_transcript_output() {
    let pending = PendingRequests::default();
    pending.record(
        channel("irc", "#commands"),
        "trail add supplies",
        "01J00000000000000000000000",
    );

    let response = pending
            .consume(
                "irc",
                NICK,
                r#"{"schema_version":1,"request_id":"01J00000000000000000000000","command":"travel.trail.add","status":"ok","data":null}"#,
            )
            .unwrap();

    assert_eq!(response.lines, ["Sledteam travel.trail.add: ok"]);
    assert!(!response.is_feedback);
}

#[test]
fn leaves_unrecognized_messages_and_senders_alone() {
    let pending = PendingRequests::default();
    let origin = channel("irc", "#01J00000000000000000000000");
    pending.record(origin.clone(), "ets", "01J00000000000000000000000");

    assert!(pending.consume("irc", "alice", "hello").is_none());
    assert!(pending.consume("irc", NICK, "not json").is_none());
    assert!(
            pending
                .consume(
                    "irc",
                    NICK,
                    r#"{"schema_version":1,"request_id":"01J00000000000000000000000","command":"travel.trail.list","status":"ok"}"#,
                )
                .is_none()
        );
    assert_eq!(pending.origins(), [origin]);
}

#[test]
fn addressed_request_supplies_the_local_label_without_a_slash() {
    let pending = PendingRequests::default();
    pending.record(
        channel("irc", "#01J00000000000000000000000"),
        "/span list Beta",
        "01J00000000000000000000000",
    );

    let response = pending
            .consume(
                "irc",
                NICK,
                r#"{"schema_version":1,"request_id":"01J00000000000000000000000","command":"travel.span.list","status":"ok","data":{"trail":{"ulid":"T1","name":"Beta"},"spans":[]}}"#,
            )
            .unwrap();

    assert_eq!(response.lines, ["span list Beta", "└── Beta"]);
}

#[test]
fn ets_tree_handles_zero_one_and_multiple_spans() {
    let zero = tree_response(
        "travel.ets",
        "ets",
        r#"{"expedition":{"ulid":"E1","name":"Moonshot"},"trail":{"ulid":"T1","name":"irc"},"spans":[]}"#,
    );
    assert_eq!(zero, ["ets", "Moonshot", "└── irc"]);

    let one = tree_response(
        "travel.ets",
        "ets",
        r#"{"expedition":{"ulid":"E1","name":"Moonshot"},"trail":{"ulid":"T1","name":"irc"},"spans":[{"ulid":"S1","name":"ux"}]}"#,
    );
    assert_eq!(one, ["ets", "Moonshot", "└── irc", "    └── ux"]);

    let many = tree_response(
        "travel.ets",
        "ets",
        r#"{"expedition":{"ulid":"E1","name":"Moonshot"},"trail":{"ulid":"T1","name":"irc"},"spans":[{"ulid":"S1","name":"ux"},{"ulid":"S2","name":"multiline"}]}"#,
    );
    assert_eq!(
        many,
        [
            "ets",
            "Moonshot",
            "└── irc",
            "    ├── ux",
            "    └── multiline"
        ]
    );
}

#[test]
fn expedition_tree_marks_current_ulid_and_handles_no_current() {
    let data = r#"{"current":{"expedition_id":"E2","trail_id":"T9"},"expeditions":[{"ulid":"E1","name":"Moonshot","project_root":"/secret","kind":"standard"},{"ulid":"E2","name":"Another Expedition"}]}"#;
    assert_eq!(
        tree_response("travel.expedition.list", "expedition list", data),
        [
            "expedition list",
            "├── Moonshot",
            "└── \u{2}Another Expedition\u{2}"
        ]
    );

    let data = r#"{"current":null,"expeditions":[{"ulid":"E1","name":"Moonshot"}]}"#;
    assert_eq!(
        tree_response("travel.expedition.list", "expedition list", data),
        ["expedition list", "└── Moonshot"]
    );
}

#[test]
fn trail_tree_only_marks_an_exact_current_ulid() {
    let data = r#"{"current":{"expedition_id":"OTHER","trail_id":"T9"},"expedition":{"ulid":"E1","name":"Moonshot"},"trails":[{"ulid":"T1","name":"planning"},{"ulid":"T2","name":"irc"}]}"#;
    assert_eq!(
        tree_response("travel.trail.list", "trail list Moonshot", data),
        [
            "trail list Moonshot",
            "└── Moonshot",
            "    ├── planning",
            "    └── irc"
        ]
    );

    let data = r#"{"current":{"expedition_id":"E1","trail_id":"T2"},"expedition":{"ulid":"E1","name":"Moonshot"},"trails":[{"ulid":"T1","name":"planning"},{"ulid":"T2","name":"irc"}]}"#;
    assert_eq!(
        tree_response("travel.trail.list", "trail list", data),
        [
            "trail list",
            "└── Moonshot",
            "    ├── planning",
            "    └── \u{2}irc\u{2}"
        ]
    );
}

#[test]
fn span_tree_omits_expedition_and_active_styling() {
    let data = r#"{"expedition":{"ulid":"E1","name":"Moonshot"},"trail":{"ulid":"T1","name":"irc"},"spans":[{"ulid":"S1","name":"ux"},{"ulid":"S2","name":"multiline"}]}"#;
    let lines = tree_response("travel.span.list", "span list irc", data);
    assert_eq!(
        lines,
        [
            "span list irc",
            "└── irc",
            "    ├── ux",
            "    └── multiline"
        ]
    );
    let rendered = lines.join("");
    assert!(!rendered.contains('\u{2}'));
    assert!(!rendered.contains("Moonshot"));
    assert!(!rendered.contains("S1"));
}

#[test]
fn unsupported_success_response_keeps_structured_fallback() {
    assert_eq!(
        format_response(
            ok_response("travel.expedition.add", r#"{"name":"Moonshot"}"#),
            "expedition add Moonshot"
        ),
        ["Sledteam travel.expedition.add:", "name: Moonshot"]
    );
}

#[test]
fn channel_resolution_continuation_returns_only_a_canonical_ulid() {
    let pending = PendingRequests::default();
    pending.record_join(
        channel("irc", "#current"),
        "channel resolve wheel",
        "01J00000000000000000000000",
    );
    let response = pending.consume("irc", NICK, r#"{"schema_version":1,"request_id":"01J00000000000000000000000","command":"channel.resolve","status":"ok","data":{"ulid":"01J00000000000000000000000","label":"wheel"}}"#).unwrap();
    assert_eq!(
        response.join_ulid.as_deref(),
        Some("01J00000000000000000000000")
    );
    assert_eq!(response.join_label.as_deref(), Some("wheel"));

    pending.record_join(
        channel("irc", "#current"),
        "channel resolve wheel",
        "01J00000000000000000000000",
    );
    let response = pending.consume("irc", NICK, r#"{"schema_version":1,"request_id":"01J00000000000000000000000","command":"channel.resolve","status":"ok","data":{"ulid":"not-a-ulid","label":"wheel"}}"#).unwrap();
    assert!(response.join_ulid.is_none());
    assert!(response.join_label.is_none());
    assert!(response.is_feedback);
}

#[test]
fn channel_resolution_failures_never_resume_a_join() {
    for code in [
        "channel_not_found",
        "channel_ambiguous",
        "channel_resolution_failed",
    ] {
        let pending = PendingRequests::default();
        pending.record_join(
            channel("irc", "#current"),
            "channel resolve Some label",
            "01J00000000000000000000000",
        );
        let packet = format!(
            r#"{{"schema_version":1,"request_id":"01J00000000000000000000000","command":"channel.resolve","status":"error","error":{{"code":"{code}","message":"resolution failed"}}}}"#
        );
        let response = pending.consume("irc", NICK, &packet).unwrap();
        assert!(response.join_ulid.is_none());
        assert!(response.is_feedback);
        assert_eq!(response.lines, ["resolution failed"]);
    }
}
