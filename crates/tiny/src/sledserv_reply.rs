/// Apply only continuations already validated and consumed by exact request ID.
pub(crate) fn apply_reply(
    client: &dyn crate::conn::Client,
    ui: &crate::ui::UI,
    serv: &str,
    reply: crate::ui::SledservReply,
) {
    use crate::ui::SledservReply;
    let (ulid, label, background) = match reply {
        SledservReply::Join { ulid, label } => (ulid, label, false),
        SledservReply::BackgroundJoin { ulid, label } => (ulid, label, true),
        SledservReply::Displayed => return,
    };
    if crate::sledteam_session::is_canonical_ulid(&ulid) {
        ui.set_sledteam_channel_label(&ulid, label);
        if background {
            crate::joins::join_background(client, ui, &ulid);
        } else {
            crate::cmd::join_canonical(client, ui, serv, &ulid);
        }
    } else {
        ui.show_command_feedback(
            &["SledServ returned an invalid channel ULID.".to_owned()],
            &libtiny_common::MsgTarget::Server { serv },
        );
    }
}
