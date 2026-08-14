//! Reassembles receive-side IRCv3 `draft/multiline` batches between wire parsing and client
//! events. Only explicit batch tags join fragments; malformed or unrelated traffic is passed
//! through or discarded without carrying partial state into later messages.

use std::collections::HashMap;

use libtiny_wire::{Cmd, Msg};

pub(crate) enum Output {
    Message(Msg),
    Multiline(Msg),
    Pending,
}

#[derive(Default)]
pub(crate) struct Reassembler {
    batches: HashMap<String, Batch>,
}

#[derive(Default)]
struct Batch {
    target: String,
    message: Option<Msg>,
}

impl Batch {
    fn new(target: String) -> Self {
        Self {
            target,
            message: None,
        }
    }
}

impl Reassembler {
    pub(crate) fn process(&mut self, msg: Msg) -> Output {
        match &msg.cmd {
            Cmd::BATCH {
                reference,
                batch_type: Some(batch_type),
                params,
            } if !reference.is_empty() && batch_type == "draft/multiline" && params.len() == 1 => {
                // Reusing a reference abandons the incomplete batch rather than joining two
                // logical messages under malformed framing.
                self.batches
                    .insert(reference.clone(), Batch::new(params[0].clone()));
                Output::Pending
            }
            Cmd::BATCH {
                reference,
                batch_type: Some(_),
                ..
            } => {
                self.batches.remove(reference);
                Output::Message(msg)
            }
            Cmd::BATCH {
                reference,
                batch_type: None,
                ..
            } => match self.batches.remove(reference) {
                Some(Batch {
                    message: Some(message),
                    ..
                }) => Output::Multiline(message),
                Some(Batch { message: None, .. }) => Output::Pending,
                None => Output::Message(msg),
            },
            Cmd::PRIVMSG { .. } => self.process_fragment(msg),
            _ => Output::Message(msg),
        }
    }

    fn process_fragment(&mut self, msg: Msg) -> Output {
        let Some(reference) = msg
            .tag("batch")
            .and_then(|tag| tag.value.as_deref())
            .map(str::to_owned)
        else {
            return Output::Message(msg);
        };
        let concat = msg.tag("draft/multiline-concat").is_some();
        let Some(batch) = self.batches.get_mut(&reference) else {
            return Output::Message(msg);
        };

        if batch.accepts(&msg, concat) {
            batch.push(msg, concat);
            Output::Pending
        } else {
            self.batches.remove(&reference);
            Output::Message(msg)
        }
    }
}

impl Batch {
    fn accepts(&self, fragment: &Msg, concat: bool) -> bool {
        let Cmd::PRIVMSG {
            target,
            is_notice,
            ctcp,
            ..
        } = &fragment.cmd
        else {
            return false;
        };
        let target_matches = match target {
            libtiny_wire::MsgTarget::Chan(chan) => chan.display() == self.target,
            libtiny_wire::MsgTarget::User(user) => user == &self.target,
        };
        if !target_matches || (self.message.is_none() && concat) {
            return false;
        }
        match &self.message {
            None => true,
            Some(message) => match &message.cmd {
                Cmd::PRIVMSG {
                    target: first_target,
                    is_notice: first_is_notice,
                    ctcp: first_ctcp,
                    ..
                } => {
                    message.pfx == fragment.pfx
                        && first_target == target
                        && first_is_notice == is_notice
                        && first_ctcp == ctcp
                }
                _ => false,
            },
        }
    }

    fn push(&mut self, mut fragment: Msg, concat: bool) {
        fragment
            .tags
            .retain(|tag| tag.key != "batch" && tag.key != "draft/multiline-concat");
        match &mut self.message {
            None => self.message = Some(fragment),
            Some(message) => {
                let Cmd::PRIVMSG { msg: body, .. } = &mut message.cmd else {
                    unreachable!()
                };
                let Cmd::PRIVMSG { msg, .. } = fragment.cmd else {
                    unreachable!()
                };
                if !concat {
                    body.push('\n');
                }
                body.push_str(&msg);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Output, Reassembler};
    use libtiny_wire::{Cmd, Msg, parse_irc_msg};

    fn parse(line: &str) -> Msg {
        let mut bytes = format!("{line}\r\n").into_bytes();
        parse_irc_msg(&mut bytes).unwrap().unwrap()
    }

    fn feed(lines: &[&str]) -> Vec<(bool, String)> {
        let mut reassembler = Reassembler::default();
        lines
            .iter()
            .filter_map(|line| match reassembler.process(parse(line)) {
                Output::Message(msg) => body(msg).map(|body| (false, body)),
                Output::Multiline(msg) => body(msg).map(|body| (true, body)),
                Output::Pending => None,
            })
            .collect()
    }

    fn body(msg: Msg) -> Option<String> {
        match msg.cmd {
            Cmd::PRIVMSG { msg, .. } => Some(msg),
            _ => None,
        }
    }

    #[test]
    fn reconstructs_newlines_blank_lines_and_exact_text() {
        let messages = feed(&[
            "BATCH +m draft/multiline #tiny",
            "@batch=m :mushbot!u@h PRIVMSG #tiny :first",
            "@batch=m :mushbot!u@h PRIVMSG #tiny :",
            "@batch=m :mushbot!u@h PRIVMSG #tiny :    ```rust",
            "@batch=m :mushbot!u@h PRIVMSG #tiny :    let s = \"雪犬\";",
            "BATCH -m",
        ]);
        assert_eq!(
            messages,
            [(
                true,
                "first\n\n    ```rust\n    let s = \"雪犬\";".to_owned()
            )]
        );
    }

    #[test]
    fn concat_continues_the_previous_logical_line() {
        let messages = feed(&[
            "BATCH +m draft/multiline #tiny",
            "@batch=m :mushbot!u@h PRIVMSG #tiny :long ",
            "@batch=m;draft/multiline-concat :mushbot!u@h PRIVMSG #tiny :logical",
            "@batch=m;draft/multiline-concat :mushbot!u@h PRIVMSG #tiny : line",
            "@batch=m :mushbot!u@h PRIVMSG #tiny :next",
            "BATCH -m",
        ]);
        assert_eq!(messages, [(true, "long logical line\nnext".to_owned())]);
    }

    #[test]
    fn concat_only_transport_split_is_still_one_multiline_event() {
        let messages = feed(&[
            "BATCH +m draft/multiline #tiny",
            "@batch=m :mushbot!u@h PRIVMSG #tiny :one long ",
            "@batch=m;draft/multiline-concat :mushbot!u@h PRIVMSG #tiny :logical line",
            "BATCH -m",
        ]);
        assert_eq!(messages, [(true, "one long logical line".to_owned())]);
    }

    #[test]
    fn keeps_separate_batches_and_ordinary_messages_separate() {
        let messages = feed(&[
            ":mushbot!u@h PRIVMSG #tiny :ordinary one",
            ":mushbot!u@h PRIVMSG #tiny :ordinary two",
            "BATCH +a draft/multiline #tiny",
            "@batch=a :mushbot!u@h PRIVMSG #tiny :a1",
            "@batch=a :mushbot!u@h PRIVMSG #tiny :a2",
            "BATCH -a",
            "BATCH +b draft/multiline #tiny",
            "@batch=b :mushbot!u@h PRIVMSG #tiny :b1",
            "@batch=b :mushbot!u@h PRIVMSG #tiny :b2",
            "BATCH -b",
        ]);
        assert_eq!(
            messages,
            [
                (false, "ordinary one".to_owned()),
                (false, "ordinary two".to_owned()),
                (true, "a1\na2".to_owned()),
                (true, "b1\nb2".to_owned()),
            ]
        );
    }

    #[test]
    fn malformed_references_do_not_poison_later_messages() {
        let messages = feed(&[
            "BATCH -unknown",
            "@batch=unknown :mushbot!u@h PRIVMSG #tiny :visible",
            "BATCH +history chathistory #tiny",
            "@batch=history :mushbot!u@h PRIVMSG #tiny :history visible",
            "BATCH -history",
            "BATCH +m draft/multiline #tiny",
            "@batch=m;draft/multiline-concat :mushbot!u@h PRIVMSG #tiny :invalid first",
            "BATCH -m",
            ":mushbot!u@h PRIVMSG #tiny :still visible",
        ]);
        assert_eq!(
            messages,
            [
                (false, "visible".to_owned()),
                (false, "history visible".to_owned()),
                (false, "invalid first".to_owned()),
                (false, "still visible".to_owned())
            ]
        );
    }
}
