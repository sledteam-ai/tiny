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
    request_id: String,
    origin: MsgSource,
    response_command_prefix: String,
    command_label: String,
    continuation: Continuation,
}

#[derive(Clone, Copy)]
enum Continuation {
    Display,
    JoinChannel,
    SpanCreated,
}

pub(crate) struct LocalResponse {
    pub(crate) origin: MsgSource,
    pub(crate) lines: Vec<String>,
    pub(crate) is_feedback: bool,
    pub(crate) join_ulid: Option<String>,
    pub(crate) join_label: Option<String>,
    pub(crate) background_join: bool,
}

#[derive(Deserialize)]
struct Response {
    schema_version: u32,
    request_id: Option<String>,
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
    pub(crate) fn record(&self, origin: MsgSource, command: &str, request_id: &str) -> bool {
        let mut words = command.trim_start_matches('/').split_whitespace();
        let continuation = if words.next() == Some("span") && words.next() == Some("add") {
            Continuation::SpanCreated
        } else {
            Continuation::Display
        };
        self.record_with_continuation(origin, command, request_id, continuation)
    }

    pub(crate) fn record_join(&self, origin: MsgSource, command: &str, request_id: &str) -> bool {
        self.record_with_continuation(origin, command, request_id, Continuation::JoinChannel)
    }

    fn record_with_continuation(
        &self,
        origin: MsgSource,
        command: &str,
        request_id: &str,
        continuation: Continuation,
    ) -> bool {
        let mut pending = self.inner.borrow_mut();
        if !valid_request_id(request_id)
            || pending.iter().any(|entry| entry.request_id == request_id)
        {
            return false;
        }
        let command_label = command.trim_start_matches('/');
        let command_name = command_label
            .split_whitespace()
            .next()
            .unwrap_or(command_label);
        let response_command_prefix = match command_name {
            "span" if matches!(continuation, Continuation::SpanCreated) => {
                "travel.span.add".to_owned()
            }
            "shutdown" => "runtime.shutdown".to_owned(),
            "channel" => "channel.resolve".to_owned(),
            _ => format!("travel.{command_name}"),
        };
        pending.push_back(PendingRequest {
            request_id: request_id.to_owned(),
            origin,
            response_command_prefix,
            command_label: command_label.to_owned(),
            continuation,
        });
        true
    }

    pub(crate) fn consume(&self, serv: &str, sender: &str, msg: &str) -> Option<LocalResponse> {
        if !sender.eq_ignore_ascii_case(NICK) {
            return None;
        }

        // Require JSON framing and a structurally valid envelope before matching an invocation.
        if !msg.starts_with('{') || !msg.ends_with('}') {
            return None;
        }
        let response: Response = serde_json::from_str(msg).ok()?;
        if response.schema_version != 1 || response.command.is_empty() {
            return None;
        }

        let request_id = response
            .request_id
            .as_deref()
            .filter(|id| valid_request_id(id))?;
        let mut pending = self.inner.borrow_mut();
        let idx = pending.iter().position(|request| {
            request.origin.serv_name() == serv
                && request.request_id == request_id
                && (response.command == request.response_command_prefix
                    || (!matches!(request.continuation, Continuation::SpanCreated)
                        && response
                            .command
                            .strip_prefix(&request.response_command_prefix)
                            .is_some_and(|suffix| suffix.starts_with('.')))
                    || (response.command == "sledserv.command"
                        && matches!(response.outcome, Outcome::Error { .. })))
        })?;
        let request = pending.remove(idx).unwrap();
        drop(pending);

        let is_feedback = matches!(&response.outcome, Outcome::Error { .. });
        #[derive(Deserialize)]
        struct ResolvedChannel {
            ulid: String,
            label: String,
        }
        let background_join = matches!(request.continuation, Continuation::SpanCreated);
        let join_resolution = match (&request.continuation, &response.outcome) {
            (Continuation::JoinChannel, Outcome::Ok { data: Some(data) }) => {
                serde_yaml::from_value::<ResolvedChannel>(data.clone())
                    .ok()
                    .filter(|resolved| crate::sledteam_session::is_canonical_ulid(&resolved.ulid))
            }
            (Continuation::SpanCreated, Outcome::Ok { data: Some(data) }) => {
                #[derive(Deserialize)]
                #[serde(rename_all = "snake_case")]
                enum Committed {
                    Completed,
                    Degraded,
                }
                #[derive(Deserialize)]
                struct CreatedSpan {
                    ulid: String,
                    label: String,
                    outcome: Committed,
                }
                serde_yaml::from_value::<CreatedSpan>(data.clone())
                    .ok()
                    .filter(|span| {
                        crate::sledteam_session::is_canonical_ulid(&span.ulid)
                            && !span.label.trim().is_empty()
                    })
                    .map(|span| {
                        let _ = span.outcome;
                        ResolvedChannel {
                            ulid: span.ulid,
                            label: span.label,
                        }
                    })
            }
            _ => None,
        };
        let join_ulid = join_resolution
            .as_ref()
            .map(|resolved| resolved.ulid.clone());
        let join_label = join_resolution.map(|resolved| resolved.label);
        let lines = match (request.continuation, &response.outcome, &join_ulid) {
            (Continuation::JoinChannel, Outcome::Ok { .. }, Some(_)) => Vec::new(),
            (Continuation::JoinChannel, Outcome::Ok { .. }, None) => {
                vec!["SledServ returned an invalid channel resolution.".to_owned()]
            }
            _ => format_response(response, &request.command_label),
        };
        Some(LocalResponse {
            origin: request.origin,
            lines,
            is_feedback: is_feedback
                || (matches!(request.continuation, Continuation::JoinChannel)
                    && join_ulid.is_none()),
            join_ulid,
            join_label,
            background_join,
        })
    }

    pub(crate) fn clear_server(&self, serv: &str) {
        self.inner
            .borrow_mut()
            .retain(|request| request.origin.serv_name() != serv);
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

/// Late identified Span-creation replies must not create private-message tabs.
pub(crate) fn is_span_creation_response(msg: &str) -> bool {
    serde_json::from_str::<Response>(msg).is_ok_and(|response| {
        response.schema_version == 1
            && response.command == "travel.span.add"
            && response.request_id.as_deref().is_some_and(valid_request_id)
    })
}

fn valid_request_id(id: &str) -> bool {
    id.parse::<ulid::Ulid>()
        .is_ok_and(|parsed| parsed.to_string().eq_ignore_ascii_case(id))
}

/// Bound pending replies to the same connection task token as JOIN intents.
/// A retiring task must not clear requests registered for its replacement.
pub(crate) struct ConnectionLifetime<'a> {
    ui: &'a crate::ui::UI,
    connection: crate::joins::ConnectionLifetime<'a>,
    serv: &'a str,
}
impl<'a> ConnectionLifetime<'a> {
    pub(crate) fn begin(ui: &'a crate::ui::UI, serv: &'a str) -> Self {
        Self {
            ui,
            connection: crate::joins::ConnectionLifetime::begin(&ui.joins, serv),
            serv,
        }
    }
    pub(crate) fn is_current(&self) -> bool {
        self.connection.is_current()
    }
}
impl Drop for ConnectionLifetime<'_> {
    fn drop(&mut self) {
        if self.is_current() {
            self.ui.clear_sledserv_requests(self.serv);
        }
    }
}

#[cfg(test)]
#[path = "sledserv_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sledserv_correlation_tests.rs"]
mod correlation_tests;

#[path = "sledserv_reply.rs"]
mod reply;
pub(crate) use reply::apply_reply;
