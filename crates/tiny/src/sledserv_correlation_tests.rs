use super::*;
use libtiny_common::ChanName;

const A: &str = "01J00000000000000000000000";
const B: &str = "01J00000000000000000000001";
fn origin(serv: &str, chan: &str) -> MsgSource {
    MsgSource::Chan {
        serv: serv.into(),
        chan: ChanName::new(chan.into()),
    }
}
fn packet(id: &str, command: &str) -> String {
    serde_json::json!({"schema_version":1,"request_id":id,"command":command,"status":"ok"})
        .to_string()
}

#[test]
fn same_family_out_of_order_preserves_invocation_origin_and_label() {
    let pending = PendingRequests::default();
    assert!(pending.record(origin("irc", "#first"), "span add first", A));
    assert!(pending.record(origin("irc", "#second"), "span add second", B));
    for (id, chan) in [(B, "#second"), (A, "#first")] {
        let reply = pending
            .consume("irc", NICK, &packet(id, "travel.span.add"))
            .unwrap();
        assert_eq!(reply.origin, origin("irc", chan));
        assert!(reply.join_ulid.is_none());
    }
    assert!(pending.origins().is_empty());
}

#[test]
fn invalid_unmatched_and_duplicate_responses_cannot_consume_another_invocation() {
    let pending = PendingRequests::default();
    assert!(pending.record(origin("irc", "#first"), "span add first", A));
    assert!(!pending.record_join(origin("irc", "#other"), "channel resolve other", A));
    for message in [
        packet(B, "travel.span.add"),
        packet("invalid", "travel.span.add"),
        packet(&A.to_lowercase(), "travel.span.add"),
        packet("81J00000000000000000000000", "travel.span.add"),
        packet(A, "travel.trail.add"),
        packet(A, "sledserv.command"), // generic success is not a command error
        r#"{"schema_version":1,"command":"travel.span.add","status":"ok"}"#.into(),
        r#"{"schema_version":1,"request_id":7,"command":"travel.span.add","status":"ok"}"#.into(),
        format!(
            r#"{{"schema_version":1,"request_id":"{A}","request_id":"{A}","command":"travel.span.add","status":"ok"}}"#
        ),
        format!(
            r#"{{"schema_version":1,"request_id":"{A}","command":"travel.span.add","status":"error"}}"#
        ),
    ] {
        assert!(
            pending.consume("irc", NICK, &message).is_none(),
            "{message}"
        );
        assert_eq!(pending.origins(), [origin("irc", "#first")]);
    }
    assert!(
        pending
            .consume("other", NICK, &packet(A, "travel.span.add"))
            .is_none()
    );
    assert!(
        pending
            .consume("irc", "other", &packet(A, "travel.span.add"))
            .is_none()
    );
    assert!(
        pending
            .consume("irc", NICK, &packet(A, "travel.span.add"))
            .is_some()
    );
    assert!(
        pending
            .consume("irc", NICK, &packet(A, "travel.span.add"))
            .is_none()
    );
}

#[test]
fn continuation_is_selected_by_id_and_invalid_data_never_joins() {
    let pending = PendingRequests::default();
    assert!(pending.record(origin("irc", "#display"), "channel resolve display", A));
    assert!(pending.record_join(origin("irc", "#join"), "channel resolve join", B));
    let reply = |id| {
        serde_json::json!({"schema_version":1,"request_id":id,"command":"channel.resolve","status":"ok","data":{"ulid":A,"label":"resolved"}}).to_string()
    };
    let result = pending.consume("irc", NICK, &reply(B)).unwrap();
    assert_eq!(result.origin, origin("irc", "#join"));
    assert_eq!(result.join_ulid.as_deref(), Some(A));
    assert!(
        pending
            .consume("irc", NICK, &reply(A))
            .unwrap()
            .join_ulid
            .is_none()
    );
    assert!(pending.record_join(origin("irc", "#join"), "channel resolve bad", B));
    let result = pending
        .consume("irc", NICK, &packet(B, "channel.resolve"))
        .unwrap();
    assert!(result.join_ulid.is_none());
    assert!(result.is_feedback);
}

#[test]
fn cleanup_is_scoped_to_server() {
    let pending = PendingRequests::default();
    assert!(pending.record(origin("irc", "#first"), "ets", A));
    assert!(pending.record(origin("other", "#second"), "ets", B));
    pending.clear_server("irc");
    assert_eq!(pending.origins(), [origin("other", "#second")]);
    assert!(
        pending
            .consume("irc", NICK, &packet(A, "travel.ets"))
            .is_none()
    );
}
