use libtiny_common::{ChanName, ChanNameRef};

use crate::config::*;
use crate::notifier::Notifier;

fn config_with(extra: &str) -> Result<Config, serde_yaml::Error> {
    serde_yaml::from_str(&format!("servers: []\ndefaults: {{}}\n{extra}"))
}

#[test]
fn dark_theme_resolves_to_complete_dark_palette() {
    let config = config_with("theme: dark\n").expect("dark theme should parse");

    assert_eq!(config.colors, Colors::dark());
    assert_eq!(config.colors.user_msg, Style { fg: 252, bg: 234 });
    assert_eq!(config.colors.timestamp, Style { fg: 244, bg: 234 });
    assert_eq!(config.colors.tab_active.bg, 24);
    assert_eq!(config.colors.err_msg.bg, 124);
}

#[test]
fn light_theme_resolves_to_complete_light_palette() {
    let config = config_with("theme: light\n").expect("light theme should parse");

    assert_eq!(config.colors, Colors::light());
    assert_eq!(config.colors.clear, Style { fg: 233, bg: 230 });
    assert_eq!(config.colors.user_msg, Style { fg: 233, bg: 230 });
    assert_eq!(config.colors.cursor, Style { fg: 230, bg: 24 });
    assert_eq!(config.colors.timestamp, Style { fg: 101, bg: 230 });
    assert_eq!(config.colors.tab_active.bg, 24);
    assert_eq!(config.colors.err_msg.bg, 124);
    assert_ne!(config.colors, Colors::dark());
}

#[test]
fn omitted_theme_defaults_to_dark() {
    let config = config_with("").expect("config without theme should parse");

    assert_eq!(config.colors, Colors::dark());
}

#[test]
fn invalid_theme_has_a_clear_config_error() {
    let error = config_with("theme: dusk\n").expect_err("unknown theme should fail");
    let message = error.to_string();

    assert!(message.contains("unknown variant `dusk`"), "{message}");
    assert!(message.contains("`dark` or `light`"), "{message}");
}

#[test]
fn colors_override_the_selected_theme() {
    let config = config_with("theme: light\ncolors:\n  timestamp:\n    fg: red\n    bg: white\n")
        .expect("color override should parse");

    assert_eq!(config.colors.timestamp, Style { fg: 9, bg: 15 });
    assert_eq!(config.colors.user_msg, Colors::light().user_msg);
}

#[test]
fn parsing_tab_configs() {
    let config_str = r##"
        servers:
          - addr: "server"
            join: 
              - name: "#tiny"
                ignore: true
                notify: "messages"
            notify: "mentions"
          - addr: "server2"
            join:
              - "#tiny2" 
            ignore: true
        defaults:
            ignore: false
            notify: off
        "##;
    let config: Config = serde_yaml::from_str(config_str).expect("parsed config");
    let tab_configs: TabConfigs = (&config).into();
    let expected = Config {
        servers: vec![
            Server {
                addr: "server".to_string(),
                join: vec![Chan::WithConfig {
                    name: ChanName::new("#tiny".to_string()),
                    config: TabConfig {
                        ignore: Some(true),
                        notify: Some(Notifier::Messages),
                    },
                }],
                config: TabConfig {
                    notify: Some(Notifier::Mentions),
                    ..Default::default()
                },
            },
            Server {
                addr: "server2".to_string(),
                join: vec![Chan::Name(ChanName::new("#tiny2".to_string()))],
                config: TabConfig {
                    ignore: Some(true),
                    ..Default::default()
                },
            },
        ],
        defaults: Defaults {
            tab_config: TabConfig {
                ignore: Some(false),
                notify: Some(Notifier::Off),
            },
        },
        ..Default::default()
    };
    assert_eq!(config.servers, expected.servers);
    assert_eq!(config.defaults, expected.defaults);

    assert_eq!(
        tab_configs.get("server", None),
        Some(TabConfig {
            ignore: Some(false),              // overwritten by defaults
            notify: Some(Notifier::Mentions)  // configured
        })
    );

    assert_eq!(
        tab_configs.get("server2", None),
        Some(TabConfig {
            ignore: Some(true),          // configured
            notify: Some(Notifier::Off)  // overwritten by defaults
        })
    );

    assert_eq!(tab_configs.get("randomserver", None), None);

    assert_eq!(
        tab_configs.get("server", Some(ChanNameRef::new("#tiny"))),
        Some(TabConfig {
            ignore: Some(true),               // configured
            notify: Some(Notifier::Messages)  // configured
        })
    );

    assert_eq!(
        tab_configs.get("server", Some(ChanNameRef::new("##rust"))),
        None
    );

    assert_eq!(
        tab_configs.get("server2", Some(ChanNameRef::new("#tiny2"))),
        Some(TabConfig {
            ignore: Some(true),          // overwritten by server
            notify: Some(Notifier::Off)  // overwritten by defaults
        })
    );
}

#[test]
fn tab_config_command() {
    assert_eq!(
        TabConfig::from_cmd_args("").unwrap(),
        TabConfig {
            ignore: None,
            notify: None
        }
    );
    assert_eq!(
        TabConfig::from_cmd_args("-ignore").unwrap(),
        TabConfig {
            ignore: Some(true),
            notify: None
        }
    );
    assert_eq!(
        TabConfig::from_cmd_args("-notify off").unwrap(),
        TabConfig {
            ignore: None,
            notify: Some(Notifier::Off)
        }
    );
    assert_eq!(
        TabConfig::from_cmd_args("-notify off -ignore").unwrap(),
        TabConfig {
            ignore: Some(true),
            notify: Some(Notifier::Off)
        }
    );
    assert_eq!(
        TabConfig::from_cmd_args("-ignore -notify off").unwrap(),
        TabConfig {
            ignore: Some(true),
            notify: Some(Notifier::Off)
        }
    );
}
