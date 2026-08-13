//! Tracks SledServ command origins and turns response envelopes into local transcript lines.
//! Command dispatch records pending origins here; IRC receive routing consumes only recognized
//! SledServ JSON replies, leaving ordinary private messages on Tiny's normal query-tab path.

use libtiny_common::MsgSource;
use serde::Deserialize;
use serde_yaml::Value;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

pub(crate) const NICK: &str = "SledServ";

#[derive(Clone, Default)]
pub(crate) struct PendingRequests {
    inner: Rc<RefCell<VecDeque<PendingRequest>>>,
}

struct PendingRequest {
    origin: MsgSource,
    response_command_prefix: String,
}

pub(crate) struct LocalResponse {
    pub(crate) origin: MsgSource,
    pub(crate) lines: Vec<String>,
}

#[derive(Deserialize)]
struct Response {
    schema_version: u32,
    command: String,
    #[serde(flatten)]
    outcome: Outcome,
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Outcome {
    Ok { data: Option<Value> },
    Error { error: ResponseError },
}

#[derive(Deserialize)]
struct ResponseError {
    code: String,
    message: String,
}

impl PendingRequests {
    pub(crate) fn record(&self, origin: MsgSource, command: &str) {
        self.inner.borrow_mut().push_back(PendingRequest {
            origin,
            response_command_prefix: format!("travel.{command}"),
        });
    }

    pub(crate) fn consume(&self, serv: &str, sender: &str, msg: &str) -> Option<LocalResponse> {
        if !sender.eq_ignore_ascii_case(NICK) {
            return None;
        }

        // JSON is a YAML subset, so Tiny's existing configuration dependency can decode the
        // envelope without adding a second serialization stack. Framing keeps YAML prose from
        // being mistaken for a SledServ packet.
        if !msg.starts_with('{') || !msg.ends_with('}') {
            return None;
        }
        let response: Response = serde_yaml::from_str(msg).ok()?;
        if response.schema_version != 1 || response.command.is_empty() {
            return None;
        }

        let mut pending = self.inner.borrow_mut();
        let idx = pending.iter().position(|request| {
            request.origin.serv_name() == serv
                && (response.command == request.response_command_prefix
                    || response
                        .command
                        .strip_prefix(&request.response_command_prefix)
                        .is_some_and(|suffix| suffix.starts_with('.'))
                    || response.command == "sledserv.command")
        })?;
        let request = pending.remove(idx).unwrap();
        drop(pending);

        Some(LocalResponse {
            origin: request.origin,
            lines: format_response(response),
        })
    }

    #[cfg(test)]
    pub(crate) fn origins(&self) -> Vec<MsgSource> {
        self.inner
            .borrow()
            .iter()
            .map(|request| request.origin.clone())
            .collect()
    }
}

fn format_response(response: Response) -> Vec<String> {
    match response.outcome {
        Outcome::Error { error } => vec![format!(
            "Sledteam {}: {} ({})",
            response.command, error.message, error.code
        )],
        Outcome::Ok { data: None } => vec![format!("Sledteam {}: ok", response.command)],
        Outcome::Ok { data: Some(data) } => {
            let mut lines = vec![format!("Sledteam {}:", response.command)];
            format_value(&data, 0, &mut lines);
            lines
        }
    }
}

fn format_value(value: &Value, indent: usize, lines: &mut Vec<String>) {
    let padding = "  ".repeat(indent);
    match value {
        Value::Mapping(object) => {
            for (key, value) in object {
                let key = scalar(key);
                match value {
                    Value::Sequence(_) | Value::Mapping(_) => {
                        lines.push(format!("{padding}{key}:"));
                        format_value(value, indent + 1, lines);
                    }
                    _ => lines.push(format!("{padding}{key}: {}", scalar(value))),
                }
            }
        }
        Value::Sequence(values) => {
            if values.is_empty() {
                lines.push(format!("{padding}(none)"));
            }
            for value in values {
                match value {
                    Value::Sequence(_) | Value::Mapping(_) => {
                        lines.push(format!("{padding}-"));
                        format_value(value, indent + 1, lines);
                    }
                    _ => lines.push(format!("{padding}- {}", scalar(value))),
                }
            }
        }
        _ => lines.push(format!("{padding}{}", scalar(value))),
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "none".to_owned(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Sequence(_) | Value::Mapping(_) => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libtiny_common::ChanName;

    fn channel(serv: &str, chan: &str) -> MsgSource {
        MsgSource::Chan {
            serv: serv.to_owned(),
            chan: ChanName::new(chan.to_owned()),
        }
    }

    #[test]
    fn matches_out_of_order_responses_by_command() {
        let pending = PendingRequests::default();
        pending.record(channel("irc", "#first"), "trail");
        pending.record(channel("irc", "#second"), "ets");

        let response = pending
            .consume(
                "irc",
                "sledserv",
                r#"{"schema_version":1,"command":"travel.ets","status":"ok","data":{"trail":{"name":"North"}}}"#,
            )
            .unwrap();

        assert_eq!(response.origin, channel("irc", "#second"));
        assert_eq!(
            response.lines,
            ["Sledteam travel.ets:", "trail:", "  name: North"]
        );
        assert_eq!(pending.origins(), [channel("irc", "#first")]);
    }

    #[test]
    fn leaves_unrecognized_messages_and_senders_alone() {
        let pending = PendingRequests::default();
        let origin = channel("irc", "#camp");
        pending.record(origin.clone(), "ets");

        assert!(pending.consume("irc", "alice", "hello").is_none());
        assert!(pending.consume("irc", NICK, "not json").is_none());
        assert!(
            pending
                .consume(
                    "irc",
                    NICK,
                    r#"{"schema_version":1,"command":"travel.trail.list","status":"ok"}"#,
                )
                .is_none()
        );
        assert_eq!(pending.origins(), [origin]);
    }
}
