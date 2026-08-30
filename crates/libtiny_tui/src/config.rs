// To see how color numbers map to actual colors in your terminal run
// `cargo run --example colors`. Use tab to swap fg/bg colors.

use libtiny_common::{ChanName, ChanNameRef};
use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, Visitor};
use std::collections::HashMap;
use std::path::Path;

use termbox_simple::*;

use crate::key_map::KeyMap;
use crate::notifier::Notifier;

#[derive(Debug)]
pub(crate) struct Config {
    pub(crate) servers: Vec<Server>,

    pub(crate) defaults: Defaults,

    pub(crate) colors: Colors,

    pub(crate) scrollback: usize,

    pub(crate) layout: Option<Layout>,

    pub(crate) max_nick_length: usize,

    pub(crate) key_map: Option<KeyMap>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Theme {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Default, Deserialize)]
struct RawConfig {
    servers: Vec<Server>,
    defaults: Defaults,
    #[serde(default)]
    theme: Theme,
    #[serde(default)]
    colors: ColorsOverride,
    #[serde(default = "usize::max_value")]
    scrollback: usize,
    layout: Option<Layout>,
    #[serde(default = "default_max_nick_length")]
    max_nick_length: usize,
    #[serde(default)]
    key_map: Option<KeyMap>,
}

impl<'de> Deserialize<'de> for Config {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawConfig::deserialize(deserializer)?;
        Ok(Config {
            servers: raw.servers,
            defaults: raw.defaults,
            colors: raw.colors.apply(Colors::for_theme(raw.theme)),
            scrollback: raw.scrollback,
            layout: raw.layout,
            max_nick_length: raw.max_nick_length,
            key_map: raw.key_map,
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            servers: Vec::new(),
            defaults: Defaults::default(),
            colors: Colors::default(),
            scrollback: usize::MAX,
            layout: None,
            max_nick_length: default_max_nick_length(),
            key_map: None,
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct Server {
    pub(crate) addr: String,
    pub(crate) join: Vec<Chan>,
    #[serde(flatten)]
    pub(crate) config: TabConfig,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct Defaults {
    #[serde(default, flatten)]
    pub(crate) tab_config: TabConfig,
}

impl Default for Defaults {
    fn default() -> Self {
        Defaults {
            tab_config: TabConfig {
                ignore: Some(false),
                notify: Some(Notifier::default()),
            },
        }
    }
}

#[derive(Clone, Deserialize, Debug, PartialEq, Eq)]
#[serde(untagged)]
pub enum Chan {
    #[serde(deserialize_with = "deser_chan_name")]
    Name(ChanName),
    WithConfig {
        #[serde(deserialize_with = "deser_chan_name")]
        name: ChanName,
        #[serde(flatten)]
        config: TabConfig,
    },
}

impl Chan {
    pub fn from_cmd_args(s: &str) -> Result<Chan, String> {
        // Make sure channel starts with '#'
        let s = if !s.starts_with('#') {
            format!("#{s}")
        } else {
            s.to_string()
        };
        // Try to split chan name and args
        match s.split_once(' ') {
            // with args
            Some((name, args)) => {
                let config = TabConfig::from_cmd_args(args)?;
                Ok(Chan::WithConfig {
                    name: ChanName::new(name.to_string()),
                    config,
                })
            }
            // chan name only
            None => Ok(Chan::Name(ChanName::new(s))),
        }
    }

    pub fn name(&self) -> &ChanNameRef {
        match self {
            Chan::Name(name) => name,
            Chan::WithConfig { name, .. } => name,
        }
        .as_ref()
    }
}

fn deser_chan_name<'de, D>(d: D) -> Result<ChanName, D::Error>
where
    D: Deserializer<'de>,
{
    let name: String = serde::de::Deserialize::deserialize(d)?;
    Ok(ChanName::new(name))
}

/// Map of TabConfigs by tab names
#[derive(Debug, Default)]
pub(crate) struct TabConfigs(HashMap<String, TabConfig>);

impl TabConfigs {
    pub(crate) fn get(
        &self,
        serv_name: &str,
        chan_name: Option<&ChanNameRef>,
    ) -> Option<TabConfig> {
        let key = if let Some(chan) = chan_name {
            format!("{}_{}", serv_name, chan.display())
        } else {
            serv_name.to_string()
        };
        self.0.get(&key).cloned()
    }

    pub(crate) fn get_mut(
        &mut self,
        serv_name: &str,
        chan_name: Option<&ChanNameRef>,
    ) -> Option<&mut TabConfig> {
        let key = if let Some(chan) = chan_name {
            format!("{}_{}", serv_name, chan.display())
        } else {
            serv_name.to_string()
        };
        self.0.get_mut(&key)
    }

    pub(crate) fn set(
        &mut self,
        serv_name: &str,
        chan_name: Option<&ChanNameRef>,
        config: TabConfig,
    ) {
        let key = if let Some(chan) = chan_name {
            format!("{}_{}", serv_name, chan.display())
        } else {
            serv_name.to_string()
        };
        self.0.insert(key, config);
    }

    pub(crate) fn set_by_server(&mut self, serv_name: &str, config: TabConfig) {
        for c in self
            .0
            .iter_mut()
            .filter(|entry| entry.0.starts_with(serv_name))
        {
            *c.1 = config;
        }
    }
}

impl From<&Config> for TabConfigs {
    fn from(config: &Config) -> Self {
        let mut tab_configs = HashMap::new();
        for server in &config.servers {
            let serv_tc = server.config.or_use(&config.defaults.tab_config);
            tab_configs.insert(server.addr.clone(), serv_tc);
            for chan in &server.join {
                let (name, tc) = match chan {
                    Chan::Name(name) => (name, serv_tc),
                    Chan::WithConfig { name, config } => (name, config.or_use(&serv_tc)),
                };
                tab_configs.insert(format!("{}_{}", server.addr, name.display()), tc);
            }
        }
        tab_configs.insert("_defaults".to_string(), config.defaults.tab_config);
        debug!("new {tab_configs:?}");
        Self(tab_configs)
    }
}

#[derive(Debug, Default, Copy, Clone, Deserialize, PartialEq, Eq)]
pub struct TabConfig {
    /// Whether the join/part messages are ignored.
    #[serde(default)]
    pub ignore: Option<bool>,

    /// Notification setting for tab.
    #[serde(default)]
    pub notify: Option<Notifier>,
}

impl TabConfig {
    pub(crate) fn from_cmd_args(s: &str) -> Result<TabConfig, String> {
        let mut config = TabConfig::default();
        let mut words = s.trim().split(' ').map(str::trim);

        while let Some(word) = words.next() {
            // `"".split(' ')` yields one empty string.
            if word.is_empty() {
                continue;
            }
            match word {
                "-ignore" => config.ignore = Some(true),
                "-notify" => match words.next() {
                    Some(notify_setting) => {
                        config.notify = Some(Notifier::from_cmd_args(notify_setting)?)
                    }
                    None => return Err("-notify parameter missing".to_string()),
                },
                other => return Err(format!("Unexpected channel parameter: {other:?}")),
            }
        }

        Ok(config)
    }

    pub(crate) fn or_use(&self, config: &TabConfig) -> TabConfig {
        TabConfig {
            ignore: self.ignore.or(config.ignore),
            notify: self.notify.or(config.notify),
        }
    }

    pub(crate) fn toggle_ignore(&mut self) -> bool {
        let ignore = self.ignore.get_or_insert(false);
        *ignore = !&*ignore;
        *ignore
    }
}

fn default_max_nick_length() -> usize {
    12
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    /// Termbox fg
    pub fg: u16,

    /// Termbox bg
    pub bg: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Layout {
    Compact,
    Aligned,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Colors {
    pub nick: Vec<u8>,
    pub clear: Style,
    pub user_msg: Style,
    pub err_msg: Style,
    pub topic: Style,
    pub cursor: Style,
    pub join: Style,
    pub part: Style,
    pub nick_change: Style,
    pub faded: Style,
    pub exit_dialogue: Style,
    pub highlight: Style,
    pub completion: Style,
    pub timestamp: Style,
    pub tab_active: Style,
    pub tab_normal: Style,
    pub tab_new_msg: Style,
    pub tab_highlight: Style,
    pub tab_joinpart: Style,
}

#[derive(Debug, Default, Deserialize)]
struct ColorsOverride {
    nick: Option<Vec<u8>>,
    clear: Option<Style>,
    user_msg: Option<Style>,
    err_msg: Option<Style>,
    topic: Option<Style>,
    cursor: Option<Style>,
    join: Option<Style>,
    part: Option<Style>,
    nick_change: Option<Style>,
    faded: Option<Style>,
    exit_dialogue: Option<Style>,
    highlight: Option<Style>,
    completion: Option<Style>,
    timestamp: Option<Style>,
    tab_active: Option<Style>,
    tab_normal: Option<Style>,
    tab_new_msg: Option<Style>,
    tab_highlight: Option<Style>,
    tab_joinpart: Option<Style>,
}

impl ColorsOverride {
    fn apply(self, mut colors: Colors) -> Colors {
        macro_rules! apply {
            ($($field:ident),+ $(,)?) => {
                $(if let Some(value) = self.$field { colors.$field = value; })+
            };
        }
        apply!(
            nick,
            clear,
            user_msg,
            err_msg,
            topic,
            cursor,
            join,
            part,
            nick_change,
            faded,
            exit_dialogue,
            highlight,
            completion,
            timestamp,
            tab_active,
            tab_normal,
            tab_new_msg,
            tab_highlight,
            tab_joinpart,
        );
        colors
    }
}

impl Default for Colors {
    fn default() -> Self {
        Self::for_theme(Theme::Dark)
    }
}

impl Colors {
    fn for_theme(theme: Theme) -> Self {
        match theme {
            Theme::Dark => Self::dark(),
            Theme::Light => Self::light(),
        }
    }

    pub(crate) fn dark() -> Self {
        Colors {
            nick: vec![109, 110, 114, 139, 145, 151, 173, 179, 180, 181, 186, 216],
            clear: Style { fg: 252, bg: 234 },
            user_msg: Style { fg: 252, bg: 234 },
            err_msg: Style {
                fg: 231 | TB_BOLD,
                bg: 124,
            },
            topic: Style {
                fg: 117 | TB_BOLD,
                bg: 234,
            },
            cursor: Style { fg: 234, bg: 252 },
            join: Style { fg: 108, bg: 234 },
            part: Style { fg: 174, bg: 234 },
            nick_change: Style { fg: 110, bg: 234 },
            faded: Style { fg: 244, bg: 234 },
            exit_dialogue: Style { fg: 252, bg: 237 },
            highlight: Style {
                fg: 203 | TB_BOLD,
                bg: 234,
            },
            completion: Style { fg: 151, bg: 234 },
            timestamp: Style { fg: 244, bg: 234 },
            tab_active: Style {
                fg: 231 | TB_BOLD,
                bg: 24,
            },
            tab_normal: Style { fg: 245, bg: 234 },
            tab_new_msg: Style {
                fg: 151 | TB_BOLD,
                bg: 234,
            },
            tab_highlight: Style {
                fg: 203 | TB_BOLD,
                bg: 234,
            },
            tab_joinpart: Style { fg: 244, bg: 234 },
        }
    }

    pub(crate) fn light() -> Self {
        Colors {
            nick: vec![24, 25, 28, 52, 58, 60, 88, 94, 95, 100, 130, 131],
            clear: Style { fg: 233, bg: 230 },
            user_msg: Style { fg: 233, bg: 230 },
            err_msg: Style {
                fg: 230 | TB_BOLD,
                bg: 124,
            },
            topic: Style {
                fg: 24 | TB_BOLD,
                bg: 230,
            },
            cursor: Style { fg: 230, bg: 24 },
            join: Style { fg: 28, bg: 230 },
            part: Style { fg: 124, bg: 230 },
            nick_change: Style { fg: 30, bg: 230 },
            faded: Style { fg: 101, bg: 230 },
            exit_dialogue: Style { fg: 233, bg: 223 },
            highlight: Style {
                fg: 124 | TB_BOLD,
                bg: 230,
            },
            completion: Style { fg: 58, bg: 230 },
            timestamp: Style { fg: 101, bg: 230 },
            tab_active: Style {
                fg: 230 | TB_BOLD,
                bg: 24,
            },
            tab_normal: Style { fg: 240, bg: 230 },
            tab_new_msg: Style {
                fg: 58 | TB_BOLD,
                bg: 230,
            },
            tab_highlight: Style {
                fg: 124 | TB_BOLD,
                bg: 230,
            },
            tab_joinpart: Style { fg: 101, bg: 230 },
        }
    }
}

//
// Parsing
//

// Color names are taken from https://en.wikipedia.org/wiki/List_of_software_palettes
const COLORS: [(&str, u16); 17] = [
    ("default", TB_DEFAULT), // Default fg/bg color of the terminal
    ("black", 0),
    ("maroon", 1),
    ("green", 2),
    ("olive", 3),
    ("navy", 4),
    ("purple", 5),
    ("teal", 6),
    ("silver", 7),
    ("gray", 8),
    ("red", 9),
    ("lime", 10),
    ("yellow", 11),
    ("blue", 12),
    ("magenta", 13),
    ("cyan", 14),
    ("white", 15),
];

const ATTRS: [(&str, u16); 4] = [
    ("bold", TB_BOLD),
    ("underline", TB_UNDERLINE),
    ("italic", TB_ITALIC),
    ("strikethrough", TB_STRIKETHROUGH),
];

fn parse_color(val: String) -> Option<u16> {
    for &(name, color) in &COLORS {
        if val == name {
            return Some(color);
        }
    }

    // If color name doesn't match try get a number
    val.parse().ok()
}

fn parse_attr(val: String) -> u16 {
    for &(name, attr) in &ATTRS {
        if name == val {
            return attr;
        }
    }
    0
}

impl<'de> Deserialize<'de> for Style {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "lowercase")]
        enum Field {
            Fg,
            Bg,
            Attrs,
        }

        use std::fmt;

        struct StyleVisitor;
        impl<'de> Visitor<'de> for StyleVisitor {
            type Value = Style;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                let colors = COLORS
                    .iter()
                    .map(|&(name, _)| name)
                    .collect::<Vec<&str>>()
                    .join(", ");
                let attrs = ATTRS
                    .iter()
                    .map(|&(name, _)| name)
                    .collect::<Vec<&str>>()
                    .join(", ");

                writeln!(
                    formatter,
                    "fg: 0-255 or color name\n\
                     bg: 0-255 or color name\n\
                     attrs: [{attrs}]\n\n\
                     color names: {colors}"
                )
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut fg: Option<u16> = None;
                let mut bg: Option<u16> = None;
                let mut attr: u16 = 0;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Fg => {
                            let color = parse_color(map.next_value()?).ok_or_else(|| {
                                de::Error::invalid_value(de::Unexpected::UnitVariant, &self)
                            })?;

                            fg = Some(color);
                        }

                        Field::Bg => {
                            let color = parse_color(map.next_value()?).ok_or_else(|| {
                                de::Error::invalid_value(de::Unexpected::UnitVariant, &self)
                            })?;

                            bg = Some(color);
                        }

                        Field::Attrs => {
                            let attrs: Vec<String> = map.next_value()?;
                            attr = attrs
                                .into_iter()
                                .map(parse_attr)
                                .fold(0, |style, a| style | a);
                        }
                    }
                }

                let fg = fg.ok_or_else(|| de::Error::missing_field("fg"))?;
                let bg = bg.ok_or_else(|| de::Error::missing_field("bg"))?;

                Ok(Style { fg: fg | attr, bg })
            }
        }

        d.deserialize_map(StyleVisitor)
    }
}

pub(crate) fn parse_config(config_path: &Path) -> Result<Config, serde_yaml::Error> {
    // tiny creates a config file with the defaults when it can't find one, but the config file can
    // be deleted before a `/reload`.
    let contents = std::fs::read_to_string(config_path).map_err(|err| {
        de::Error::custom(format!(
            "Can't read config file '{}': {}",
            config_path.to_string_lossy(),
            err
        ))
    })?;
    serde_yaml::from_str(&contents)
}
