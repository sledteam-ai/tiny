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
            if let Some(lines) = format_travel_tree(&response.command, &data) {
                return lines;
            }
            let mut lines = vec![format!("Sledteam {}:", response.command)];
            format_value(&data, 0, &mut lines);
            lines
        }
    }
}

#[derive(Deserialize)]
struct NamedResource {
    ulid: String,
    name: String,
}

#[derive(Deserialize)]
struct CurrentPosition {
    expedition_id: String,
    trail_id: String,
}

#[derive(Deserialize)]
struct EtsData {
    expedition: Option<NamedResource>,
    trail: Option<NamedResource>,
    spans: Option<Vec<NamedResource>>,
}

#[derive(Deserialize)]
struct ExpeditionListData {
    current: Option<CurrentPosition>,
    expeditions: Vec<NamedResource>,
}

#[derive(Deserialize)]
struct TrailListData {
    current: Option<CurrentPosition>,
    expedition: NamedResource,
    trails: Vec<NamedResource>,
}

#[derive(Deserialize)]
struct SpanListData {
    trail: NamedResource,
    spans: Vec<NamedResource>,
}

fn format_travel_tree(command: &str, data: &Value) -> Option<Vec<String>> {
    match command {
        "travel.ets" => {
            let data: EtsData = serde_yaml::from_value(data.clone()).ok()?;
            let Some(expedition) = data.expedition else {
                return Some(vec!["(none)".to_owned()]);
            };
            let mut lines = vec![expedition.name];
            if let Some(trail) = data.trail {
                lines.push(format!("└── {}", trail.name));
                append_children(&mut lines, "    ", data.spans.unwrap_or_default(), None);
            }
            Some(lines)
        }
        "travel.expedition.list" => {
            let data: ExpeditionListData = serde_yaml::from_value(data.clone()).ok()?;
            let current = data
                .current
                .as_ref()
                .map(|current| current.expedition_id.as_str());
            Some(top_level_list(data.expeditions, current))
        }
        "travel.trail.list" => {
            let data: TrailListData = serde_yaml::from_value(data.clone()).ok()?;
            let current = data
                .current
                .as_ref()
                .map(|current| current.trail_id.as_str());
            let mut lines = vec![format!("└── {}", data.expedition.name)];
            append_children(&mut lines, "    ", data.trails, current);
            Some(lines)
        }
        "travel.span.list" => {
            let data: SpanListData = serde_yaml::from_value(data.clone()).ok()?;
            let mut lines = vec![format!("└── {}", data.trail.name)];
            append_children(&mut lines, "    ", data.spans, None);
            Some(lines)
        }
        _ => None,
    }
}

fn top_level_list(resources: Vec<NamedResource>, current: Option<&str>) -> Vec<String> {
    if resources.is_empty() {
        return vec!["(none)".to_owned()];
    }
    let last = resources.len() - 1;
    resources
        .into_iter()
        .enumerate()
        .map(|(idx, resource)| {
            let branch = if idx == last {
                "└──"
            } else {
                "├──"
            };
            format!("{branch} {}", active_name(&resource, current))
        })
        .collect()
}

fn append_children(
    lines: &mut Vec<String>,
    indent: &str,
    resources: Vec<NamedResource>,
    current: Option<&str>,
) {
    let Some(last) = resources.len().checked_sub(1) else {
        return;
    };
    for (idx, resource) in resources.into_iter().enumerate() {
        let branch = if idx == last {
            "└──"
        } else {
            "├──"
        };
        lines.push(format!(
            "{indent}{branch} {}",
            active_name(&resource, current)
        ));
    }
}

fn active_name(resource: &NamedResource, current: Option<&str>) -> String {
    if current == Some(resource.ulid.as_str()) {
        format!("\u{2}{}\u{2}", resource.name)
    } else {
        resource.name.clone()
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

    fn ok_response(command: &str, data: &str) -> Response {
        serde_yaml::from_str(&format!(
            r#"{{"schema_version":1,"command":"{command}","status":"ok","data":{data}}}"#
        ))
        .unwrap()
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
                r#"{"schema_version":1,"command":"travel.ets","status":"ok","data":{"expedition":{"ulid":"E1","name":"Moonshot"},"trail":{"ulid":"T1","name":"North"},"spans":[]}}"#,
            )
            .unwrap();

        assert_eq!(response.origin, channel("irc", "#second"));
        assert_eq!(response.lines, ["Moonshot", "└── North"]);
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

    #[test]
    fn ets_tree_handles_zero_one_and_multiple_spans() {
        let zero = format_response(ok_response(
            "travel.ets",
            r#"{"expedition":{"ulid":"E1","name":"Moonshot"},"trail":{"ulid":"T1","name":"irc"},"spans":[]}"#,
        ));
        assert_eq!(zero, ["Moonshot", "└── irc"]);

        let one = format_response(ok_response(
            "travel.ets",
            r#"{"expedition":{"ulid":"E1","name":"Moonshot"},"trail":{"ulid":"T1","name":"irc"},"spans":[{"ulid":"S1","name":"ux"}]}"#,
        ));
        assert_eq!(one, ["Moonshot", "└── irc", "    └── ux"]);

        let many = format_response(ok_response(
            "travel.ets",
            r#"{"expedition":{"ulid":"E1","name":"Moonshot"},"trail":{"ulid":"T1","name":"irc"},"spans":[{"ulid":"S1","name":"ux"},{"ulid":"S2","name":"multiline"}]}"#,
        ));
        assert_eq!(
            many,
            ["Moonshot", "└── irc", "    ├── ux", "    └── multiline"]
        );
    }

    #[test]
    fn expedition_tree_marks_current_ulid_and_handles_no_current() {
        let data = r#"{"current":{"expedition_id":"E2","trail_id":"T9"},"expeditions":[{"ulid":"E1","name":"Moonshot","project_root":"/secret","kind":"standard"},{"ulid":"E2","name":"Another Expedition"}]}"#;
        assert_eq!(
            format_response(ok_response("travel.expedition.list", data)),
            ["├── Moonshot", "└── \u{2}Another Expedition\u{2}"]
        );

        let data = r#"{"current":null,"expeditions":[{"ulid":"E1","name":"Moonshot"}]}"#;
        assert_eq!(
            format_response(ok_response("travel.expedition.list", data)),
            ["└── Moonshot"]
        );
    }

    #[test]
    fn trail_tree_only_marks_an_exact_current_ulid() {
        let data = r#"{"current":{"expedition_id":"OTHER","trail_id":"T9"},"expedition":{"ulid":"E1","name":"Moonshot"},"trails":[{"ulid":"T1","name":"planning"},{"ulid":"T2","name":"irc"}]}"#;
        assert_eq!(
            format_response(ok_response("travel.trail.list", data)),
            ["└── Moonshot", "    ├── planning", "    └── irc"]
        );

        let data = r#"{"current":{"expedition_id":"E1","trail_id":"T2"},"expedition":{"ulid":"E1","name":"Moonshot"},"trails":[{"ulid":"T1","name":"planning"},{"ulid":"T2","name":"irc"}]}"#;
        assert_eq!(
            format_response(ok_response("travel.trail.list", data)),
            ["└── Moonshot", "    ├── planning", "    └── \u{2}irc\u{2}"]
        );
    }

    #[test]
    fn span_tree_omits_expedition_and_active_styling() {
        let data = r#"{"expedition":{"ulid":"E1","name":"Moonshot"},"trail":{"ulid":"T1","name":"irc"},"spans":[{"ulid":"S1","name":"ux"},{"ulid":"S2","name":"multiline"}]}"#;
        let lines = format_response(ok_response("travel.span.list", data));
        assert_eq!(lines, ["└── irc", "    ├── ux", "    └── multiline"]);
        let rendered = lines.join("");
        assert!(!rendered.contains('\u{2}'));
        assert!(!rendered.contains("Moonshot"));
        assert!(!rendered.contains("S1"));
    }

    #[test]
    fn unsupported_success_response_keeps_structured_fallback() {
        assert_eq!(
            format_response(ok_response(
                "travel.expedition.add",
                r#"{"name":"Moonshot"}"#
            )),
            ["Sledteam travel.expedition.add:", "name: Moonshot"]
        );
    }
}
