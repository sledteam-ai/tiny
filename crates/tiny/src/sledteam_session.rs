//! Sledteam launch-session presentation adapter.
//!
//! The snapshot is a one-shot input from Sled. Canonical IRC channel names
//! remain the keys; terrain kinds and labels exist only for presentation.

use std::collections::HashMap;

use libtiny_common::{ChanName, ChanNameRef};
use serde::Deserialize;

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Default)]
pub(crate) struct SledteamSession {
    terrain: Vec<TerrainChannel>,
    labels: HashMap<ChanName, String>,
}

#[derive(Debug, Clone)]
struct TerrainChannel {
    channel: ChanName,
    #[allow(dead_code)]
    kind: TerrainKind,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TerrainKind {
    Expedition,
    Trail,
    Span,
}

impl SledteamSession {
    pub(crate) fn parse(json: &str) -> Result<Self, String> {
        let snapshot: SnapshotWire = serde_yaml::from_str(json)
            .map_err(|error| format!("invalid Sledteam session snapshot: {error}"))?;
        if snapshot.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "unsupported Sledteam session schema_version {}; expected {SCHEMA_VERSION}",
                snapshot.schema_version
            ));
        }

        let mut session = Self::default();
        for entry in snapshot.terrain {
            let id = entry
                .channel
                .strip_prefix('#')
                .ok_or_else(|| format!("terrain channel is not canonical: {:?}", entry.channel))?;
            if !is_canonical_ulid(id) {
                return Err(format!(
                    "terrain channel is not canonical: {:?}",
                    entry.channel
                ));
            }
            if entry.label.trim().is_empty() {
                return Err(format!(
                    "terrain channel {} has a blank label",
                    entry.channel
                ));
            }
            let channel = ChanName::new(entry.channel);
            if session.labels.contains_key(&channel) {
                return Err(format!("duplicate terrain channel {}", channel.display()));
            }
            session.labels.insert(channel.clone(), entry.label);
            session.terrain.push(TerrainChannel {
                channel,
                kind: entry.kind,
            });
        }
        Ok(session)
    }

    pub(crate) fn channels(&self) -> Vec<ChanName> {
        self.terrain
            .iter()
            .map(|terrain| terrain.channel.clone())
            .collect()
    }

    pub(crate) fn display_channel<'a>(&'a self, channel: &'a ChanNameRef) -> &'a str {
        self.labels
            .get(channel)
            .map(String::as_str)
            .unwrap_or_else(|| channel.display())
    }
}

fn is_canonical_ulid(value: &str) -> bool {
    value.len() == 26
        && value.as_bytes()[0].is_ascii_digit()
        && value.as_bytes()[0] <= b'7'
        && value.bytes().all(|byte| {
            byte.is_ascii_digit()
                || matches!(byte, b'A'..=b'H' | b'J'..=b'K' | b'M'..=b'N' | b'P'..=b'T' | b'V'..=b'Z')
        })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotWire {
    schema_version: u32,
    terrain: Vec<TerrainWire>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TerrainWire {
    channel: String,
    kind: TerrainKind,
    label: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_channels_labels_and_kinds() {
        let session = SledteamSession::parse(
            r##"{"schema_version":1,"terrain":[{"channel":"#01J00000000000000000000000","kind":"expedition","label":"camp"},{"channel":"#01J00000000000000000000001","kind":"trail","label":"Pack sled"},{"channel":"#01J00000000000000000000002","kind":"span","label":"Find rope"}]}"##,
        )
        .unwrap();

        assert_eq!(session.channels().len(), 3);
        assert_eq!(session.terrain[0].kind, TerrainKind::Expedition);
        assert_eq!(session.display_channel(&session.channels()[0]), "camp");
        assert_eq!(session.display_channel(&session.channels()[1]), "Pack sled");
        assert_eq!(session.display_channel(&session.channels()[2]), "Find rope");
    }

    #[test]
    fn distinct_channels_remain_distinct_with_the_same_label() {
        let session = SledteamSession::parse(
            r##"{"schema_version":1,"terrain":[{"channel":"#01J00000000000000000000000","kind":"span","label":"build"},{"channel":"#01J00000000000000000000001","kind":"span","label":"build"}]}"##,
        )
        .unwrap();
        let channels = session.channels();

        assert_ne!(channels[0], channels[1]);
        assert_eq!(session.display_channel(&channels[0]), "build");
        assert_eq!(session.display_channel(&channels[1]), "build");
    }

    #[test]
    fn rejects_noncanonical_or_duplicate_channels() {
        assert!(SledteamSession::parse(
            r##"{"schema_version":1,"terrain":[{"channel":"#camp","kind":"expedition","label":"camp"}]}"##
        )
        .is_err());
        assert!(SledteamSession::parse(
            r##"{"schema_version":1,"terrain":[{"channel":"#01J00000000000000000000000","kind":"trail","label":"one"},{"channel":"#01J00000000000000000000000","kind":"span","label":"two"}]}"##
        )
        .is_err());
    }
}
