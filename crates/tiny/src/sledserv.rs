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
    command_label: String,
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
    #[serde(default)]
    usage: Vec<String>,
}

impl PendingRequests {
    pub(crate) fn record(&self, origin: MsgSource, command: &str) {
        let command_label = command.trim_start_matches('/');
        let command_name = command_label
            .split_whitespace()
            .next()
            .unwrap_or(command_label);
        let response_command_prefix = match command_name {
            "shutdown" => "runtime.shutdown".to_owned(),
            _ => format!("travel.{command_name}"),
        };
        self.inner.borrow_mut().push_back(PendingRequest {
            origin,
            response_command_prefix,
            command_label: command_label.to_owned(),
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
            lines: format_response(response, &request.command_label),
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

fn format_response(response: Response, command_label: &str) -> Vec<String> {
    match response.outcome {
        Outcome::Error { error } if !error.usage.is_empty() => error
            .usage
            .into_iter()
            .enumerate()
            .map(|(index, form)| {
                if index == 0 {
                    format!("Usage: /{form}")
                } else {
                    format!("       /{form}")
                }
            })
            .collect(),
        Outcome::Error { error } => {
            log::debug!(
                "SledServ command error: command={}, code={}",
                response.command,
                error.code
            );
            vec![error.message]
        }
        Outcome::Ok { .. } if response.command == "runtime.shutdown" => {
            vec!["Shutting down Sledteam…".to_owned()]
        }
        Outcome::Ok { data: None } => vec![format!("Sledteam {}: ok", response.command)],
        Outcome::Ok { data: Some(data) } => {
            if let Some(lines) = format_travel_tree(&response.command, &data) {
                let mut labeled = Vec::with_capacity(lines.len() + 1);
                labeled.push(command_label.to_owned());
                labeled.extend(lines);
                return labeled;
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

    fn tree_response(command: &str, label: &str, data: &str) -> Vec<String> {
        format_response(ok_response(command, data), label)
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
        assert_eq!(response.lines, ["ets", "Moonshot", "└── North"]);
        assert_eq!(pending.origins(), [channel("irc", "#first")]);
    }

    #[test]
    fn shutdown_acknowledgement_uses_runtime_command_and_local_message() {
        let pending = PendingRequests::default();
        let origin = channel("irc", "#01J00000000000000000000000");
        pending.record(origin.clone(), "shutdown");

        let response = pending
            .consume(
                "irc",
                NICK,
                r#"{"schema_version":1,"command":"runtime.shutdown","status":"ok","data":{"message":"Sledteam shutdown request sent."}}"#,
            )
            .unwrap();

        assert_eq!(response.origin, origin);
        assert_eq!(response.lines, ["Shutting down Sledteam…"]);
        assert!(pending.origins().is_empty());
    }

    #[test]
    fn shutdown_error_shows_only_the_user_facing_message() {
        let pending = PendingRequests::default();
        pending.record(channel("irc", "#01J00000000000000000000000"), "shutdown");

        let response = pending
            .consume(
                "irc",
                NICK,
                r#"{"schema_version":1,"command":"runtime.shutdown","status":"error","error":{"code":"shutdown_request_failed","message":"Could not contact the runtime."}}"#,
            )
            .unwrap();

        assert_eq!(response.lines, ["Could not contact the runtime."]);
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
            pending.record(channel("irc", "#commands"), request);
            let packet = format!(
                r#"{{"schema_version":1,"command":"{command}","status":"error","error":{{"code":"invalid_command_syntax","message":"Invalid command syntax.","usage":{usage}}}}}"#
            );
            let response = pending.consume("irc", NICK, &packet).unwrap();
            assert_eq!(response.lines, expected, "{request}");
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
            pending.record(channel("irc", "#commands"), request);
            let packet = format!(
                r#"{{"schema_version":1,"command":"{command}","status":"error","error":{{"code":"{code}","message":"{}"}}}}"#,
                message.replace('"', "\\\"")
            );
            let response = pending.consume("irc", NICK, &packet).unwrap();
            assert_eq!(response.lines, [message]);
        }
    }

    #[test]
    fn leaves_unrecognized_messages_and_senders_alone() {
        let pending = PendingRequests::default();
        let origin = channel("irc", "#01J00000000000000000000000");
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
    fn addressed_request_supplies_the_local_label_without_a_slash() {
        let pending = PendingRequests::default();
        pending.record(
            channel("irc", "#01J00000000000000000000000"),
            "/span list Beta",
        );

        let response = pending
            .consume(
                "irc",
                NICK,
                r#"{"schema_version":1,"command":"travel.span.list","status":"ok","data":{"trail":{"ulid":"T1","name":"Beta"},"spans":[]}}"#,
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
}
