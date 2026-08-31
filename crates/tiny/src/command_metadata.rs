//! Curated Sledteam-facing slash-command metadata.
//!
//! This is deliberately separate from Tiny's complete command inventory: `/help` and the input
//! completion pane use this catalog, while `/??` continues to describe every executable command.

use libtiny_common::CommandInfo;

pub(crate) static SLEDTEAM_COMMANDS: [CommandInfo; 11] = [
    CommandInfo::new("help", "/help, /?", "Show Sledteam commands"),
    CommandInfo::new(
        "ets",
        "/ets",
        "Show journey position (expedition/trail/spans)",
    ),
    CommandInfo::new("expedition", "/expedition <arg>", "Manage expeditions"),
    CommandInfo::new("trail", "/trail <arg>", "Manage trails"),
    CommandInfo::new("span", "/span <arg>", "Manage spans"),
    CommandInfo::new("clear", "/clear", "Clear screen above cursor"),
    CommandInfo::new("switch", "/switch <tab>", "Switch to another named tab"),
    CommandInfo::new("close", "/close [reason]", "Close the current tab"),
    CommandInfo::new(
        "quit",
        "/quit [reason]",
        "Exit this Sled session without shutting down Sledteam",
    ),
    CommandInfo::new("reload", "/reload", "Reload configuration"),
    CommandInfo::new("shutdown", "/shutdown", "Shut down the Sledteam system"),
];

pub(crate) fn sledteam_commands() -> &'static [CommandInfo] {
    &SLEDTEAM_COMMANDS
}
