use std::panic::Location;

use libtiny_common::{ChanName, ChanNameRef, MsgSource, MsgTarget};
use term_input::{Arrow, Event, FKey, Key};

use crate::test_utils::{buffer_str, expect_screen};
use crate::tui::TUI;

mod layout;
mod resize;

mod config;

fn enter_string(tui: &mut TUI, s: &str) {
    for c in s.chars() {
        tui.handle_input_event(Event::Key(Key::Char(c)));
    }
}

fn single_line_tui(w: u16, h: u16) -> TUI {
    TUI::new_test(w, h)
}

#[test]
fn init_screen() {
    let mut tui = single_line_tui(20, 4);
    tui.draw();

    #[rustfmt::skip]
    let screen =
        "|Any mentions to you |
         |will be listed here.|
         |                    |
         |                    |";
    expect_screen(screen, &tui.get_front_buffer(), 20, 4, Location::caller());
}

#[test]
fn mentions_tab_is_hidden_from_tab_bar() {
    let mut tui = single_line_tui(20, 4);
    let tabs = tui.get_tabs();
    assert_eq!(tabs.len(), 1);
    assert!(tabs[0].hidden);

    tui.new_server_tab("mars.example.org", None);
    let tabs = tui.get_tabs();
    assert_eq!(tabs[1].switch, Some('m'));

    tui.draw();

    #[rustfmt::skip]
    let screen =
        "|Any mentions to you |
         |will be listed here.|
         |                    |
         |mars.example.org    |";
    expect_screen(screen, &tui.get_front_buffer(), 20, 4, Location::caller());
}

#[test]
fn tab_navigation_skips_hidden_mentions_tab() {
    let mut tui = single_line_tui(30, 4);
    let serv = "irc.server_1.org";
    let chan = ChanNameRef::new("#chan");
    tui.new_server_tab(serv, None);
    tui.new_chan_tab(serv, chan);

    tui.next_tab();
    assert_eq!(
        tui.current_tab(),
        &MsgSource::Serv {
            serv: serv.to_owned(),
        }
    );

    tui.next_tab();
    assert_eq!(
        tui.current_tab(),
        &MsgSource::Chan {
            serv: serv.to_owned(),
            chan: chan.to_owned(),
        }
    );

    tui.next_tab();
    assert_eq!(
        tui.current_tab(),
        &MsgSource::Serv {
            serv: serv.to_owned(),
        }
    );

    tui.prev_tab();
    assert_eq!(
        tui.current_tab(),
        &MsgSource::Chan {
            serv: serv.to_owned(),
            chan: chan.to_owned(),
        }
    );
}

#[test]
fn new_tabs_start_with_only_normal_input_and_tab_opens_the_composer() {
    let mut tui = TUI::new_test(30, 10);
    tui.new_server_tab("irc.example.org", None);
    tui.next_tab();

    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "single_line");
    assert_eq!(tui.get_tabs()[1].widget.msg_area_height(), 8);
    tui.draw();
    assert_ne!(tui.get_front_buffer().cells[5 * 30].ch, '┏');

    assert!(tui.handle_input_event(Event::Key(Key::Tab)).is_none());
    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "composer");
    assert_eq!(tui.get_tabs()[1].widget.msg_area_height(), 4);
    tui.draw();
    assert_eq!(tui.get_front_buffer().cells[5 * 30].ch, '┏');
}

#[test]
fn normal_input_and_slash_command_submission_stay_on_the_input_path() {
    let mut tui = TUI::new_test(30, 10);
    tui.new_server_tab("irc.example.org", None);
    tui.next_tab();

    enter_string(&mut tui, "quick riff");
    assert!(matches!(
        tui.handle_input_event(Event::Key(Key::Char('\r'))),
        Some(crate::tui::TUIRet::Input { msg, .. })
            if msg.iter().collect::<String>() == "quick riff"
    ));

    enter_string(&mut tui, "/join #tiny");
    assert!(matches!(
        tui.handle_input_event(Event::Key(Key::Char('\r'))),
        Some(crate::tui::TUIRet::Input { msg, .. })
            if msg.iter().collect::<String>() == "/join #tiny"
    ));
}

#[test]
fn tab_submits_multiline_composer_content_and_closes_it() {
    let mut tui = TUI::new_test(30, 10);
    tui.new_server_tab("irc.example.org", None);
    tui.next_tab();
    tui.handle_input_event(Event::Key(Key::Tab));

    enter_string(&mut tui, "first");
    tui.handle_input_event(Event::Key(Key::Char('\r')));
    enter_string(&mut tui, "second");
    assert!(matches!(
        tui.handle_input_event(Event::Key(Key::Tab)),
        Some(crate::tui::TUIRet::Lines { lines, .. })
            if lines == ["first", "second"]
    ));
    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "single_line");
    assert_eq!(tui.get_tabs()[1].widget.msg_area_height(), 8);

    tui.handle_input_event(Event::Key(Key::Tab));
    assert!(tui.handle_input_event(Event::Key(Key::Tab)).is_none());
}

#[test]
fn slash_and_whitespace_composer_content_use_lines_but_empty_content_does_not() {
    let mut tui = TUI::new_test(30, 10);
    tui.new_server_tab("irc.example.org", None);
    tui.next_tab();

    tui.handle_input_event(Event::Key(Key::Tab));
    enter_string(&mut tui, "/trail still message content");
    assert!(matches!(
        tui.handle_input_event(Event::Key(Key::Tab)),
        Some(crate::tui::TUIRet::Lines { lines, .. })
            if lines == ["/trail still message content"]
    ));

    tui.handle_input_event(Event::Key(Key::Tab));
    enter_string(&mut tui, "  ");
    assert!(matches!(
        tui.handle_input_event(Event::Key(Key::Tab)),
        Some(crate::tui::TUIRet::Lines { lines, .. }) if lines == ["  "]
    ));

    tui.handle_input_event(Event::Key(Key::Tab));
    assert!(tui.handle_input_event(Event::Key(Key::Tab)).is_none());
    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "single_line");
}

#[test]
fn escape_cancels_and_discards_the_composer() {
    let mut tui = TUI::new_test(30, 10);
    tui.new_server_tab("irc.example.org", None);
    tui.next_tab();
    tui.handle_input_event(Event::Key(Key::Tab));
    enter_string(&mut tui, "discard me");

    assert!(tui.handle_input_event(Event::Key(Key::Esc)).is_none());
    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "single_line");
    tui.handle_input_event(Event::Key(Key::Tab));
    assert!(tui.handle_input_event(Event::Key(Key::Tab)).is_none());
}

#[test]
fn f1_toggles_help_without_destroying_normal_or_composer_drafts() {
    let mut tui = TUI::new_test(70, 30);
    tui.new_server_tab("irc.example.org", None);
    tui.next_tab();
    enter_string(&mut tui, "command draft");

    tui.handle_input_event(Event::Key(Key::Esc));
    assert!(!tui.navigation_dialog_visible());
    tui.handle_input_event(Event::Key(Key::FKey(FKey::F1)));
    assert!(tui.navigation_dialog_visible());
    tui.draw();
    assert!(buffer_str(&tui.get_front_buffer(), 70, 30).contains("Navigation"));
    tui.handle_input_event(Event::Key(Key::Esc));
    assert!(tui.navigation_dialog_visible());
    enter_string(&mut tui, " ignored");
    tui.handle_input_event(Event::Key(Key::FKey(FKey::F1)));
    assert!(!tui.navigation_dialog_visible());
    assert!(matches!(
        tui.handle_input_event(Event::Key(Key::Char('\r'))),
        Some(crate::tui::TUIRet::Input { msg, .. })
            if msg.iter().collect::<String>() == "command draft"
    ));

    tui.handle_input_event(Event::Key(Key::Tab));
    enter_string(&mut tui, "composer draft");
    tui.handle_input_event(Event::Key(Key::FKey(FKey::F1)));
    tui.handle_input_event(Event::Key(Key::FKey(FKey::F1)));
    assert!(matches!(
        tui.handle_input_event(Event::Key(Key::Tab)),
        Some(crate::tui::TUIRet::Lines { lines, .. }) if lines == ["composer draft"]
    ));
}

#[test]
fn multiline_paste_opens_focuses_and_populates_the_composer() {
    let mut tui = TUI::new_test(20, 10);
    tui.new_server_tab("irc.example.org", None);
    tui.next_tab();

    assert!(
        tui.handle_input_event(Event::String("one\r\ntwo".to_owned()))
            .is_none()
    );
    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "composer");
    tui.draw();
    assert!(buffer_str(&tui.get_front_buffer(), 20, 10).contains("one"));
    assert!(matches!(
        tui.handle_input_event(Event::Key(Key::Tab)),
        Some(crate::tui::TUIRet::Lines { lines, .. }) if lines == ["one", "two"]
    ));
}

#[test]
fn target_navigation_is_suppressed_while_composing() {
    let mut tui = TUI::new_test(30, 10);
    let serv = "irc.example.org";
    let first = ChanNameRef::new("#first");
    let second = ChanNameRef::new("#second");
    tui.new_server_tab(serv, None);
    tui.new_chan_tab(serv, first);
    tui.new_chan_tab(serv, second);
    tui.next_tab();
    tui.next_tab();
    let source = tui.current_tab().clone();
    let tab_order: Vec<_> = tui.get_tabs().iter().map(|tab| tab.src.clone()).collect();
    tui.handle_input_event(Event::Key(Key::Tab));

    for key in [
        Key::Ctrl('n'),
        Key::Ctrl('p'),
        Key::AltArrow(Arrow::Left),
        Key::AltArrow(Arrow::Right),
        Key::AltChar('1'),
    ] {
        tui.handle_input_event(Event::Key(key));
        assert_eq!(tui.current_tab(), &source);
        assert_eq!(
            tui.get_tabs()
                .iter()
                .map(|tab| tab.src.clone())
                .collect::<Vec<_>>(),
            tab_order
        );
    }

    enter_string(&mut tui, "still here");
    assert!(matches!(
        tui.handle_input_event(Event::Key(Key::Tab)),
        Some(crate::tui::TUIRet::Lines { lines, from })
            if lines == ["still here"] && from == source
    ));
}

#[test]
fn composer_layout_resizes_and_restores_scrollback_safely() {
    let mut tui = TUI::new_test(24, 12);
    tui.new_server_tab("irc.example.org", None);
    tui.next_tab();
    for line in 0..30 {
        tui.add_msg(
            &format!("line{line}"),
            time::empty_tm(),
            &MsgTarget::CurrentTab,
        );
    }
    tui.handle_input_event(Event::Key(Key::AltArrow(Arrow::Up)));
    let initial_scroll = tui.get_tabs()[1].widget.scroll_offset();
    let initial_height = tui.get_tabs()[1].widget.msg_area_height();

    tui.handle_input_event(Event::Key(Key::Tab));
    assert!(tui.get_tabs()[1].widget.msg_area_height() < initial_height);
    assert!(tui.get_tabs()[1].widget.scroll_offset() > 0);
    tui.set_size(32, 16);
    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "composer");
    assert!(tui.get_tabs()[1].widget.msg_area_height() > 0);
    tui.handle_input_event(Event::Key(Key::Esc));
    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "single_line");
    assert!(tui.get_tabs()[1].widget.msg_area_height() > initial_height);
    assert!(tui.get_tabs()[1].widget.scroll_offset() > 0);
    assert!(initial_scroll > 0);

    for size in 0..=4 {
        tui.set_size(size, size);
        tui.handle_input_event(Event::Key(Key::Tab));
        tui.draw();
        tui.handle_input_event(Event::Key(Key::Esc));
        tui.draw();
    }
}

#[test]
fn hidden_mentions_tab_still_receives_messages() {
    let mut tui = single_line_tui(20, 4);
    let target = MsgTarget::Server { serv: "mentions" };

    tui.add_msg("osa1 in x.y.z:#tiny: hello", time::empty_tm(), &target);

    let lines = tui.tab_lines_text(&target).unwrap();
    assert!(lines.iter().any(|line| line.contains("Any mentions")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("osa1 in x.y.z:#tiny: hello"))
    );
}

#[test]
fn channel_alias_changes_presentation_without_changing_source_identity() {
    let mut tui = single_line_tui(70, 4);
    let server = "sled.local";
    let canonical = ChanName::new("#01J00000000000000000000000".to_owned());
    tui.new_server_tab(server, None);
    tui.new_chan_tab_with_alias(server, &canonical, Some("camp".to_owned()));
    tui.focus_chan_tab(server, &canonical);
    tui.draw();

    assert!(crate::test_utils::buffer_str(&tui.get_front_buffer(), 70, 4).contains("camp"));
    assert_eq!(
        tui.current_tab(),
        &MsgSource::Chan {
            serv: server.to_owned(),
            chan: canonical,
        }
    );
}

#[test]
fn switch_accepts_unique_labels_and_canonical_channels_but_not_ambiguous_labels() {
    let mut tui = single_line_tui(100, 4);
    let server = "sled.local";
    let trail = ChanName::new("#01J00000000000000000000000".to_owned());
    let span_a = ChanName::new("#01J00000000000000000000001".to_owned());
    let span_b = ChanName::new("#01J00000000000000000000002".to_owned());
    tui.new_server_tab(server, None);
    tui.new_chan_tab_with_alias(server, &trail, Some("Pack sled".to_owned()));
    tui.new_chan_tab_with_alias(server, &span_a, Some("build".to_owned()));
    tui.new_chan_tab_with_alias(server, &span_b, Some("build".to_owned()));

    tui.switch("Pack sled");
    assert!(matches!(tui.current_tab(), MsgSource::Chan { chan, .. } if chan == &trail));
    tui.switch("build");
    assert!(matches!(tui.current_tab(), MsgSource::Chan { chan, .. } if chan == &trail));
    tui.switch(span_b.display());
    assert!(matches!(tui.current_tab(), MsgSource::Chan { chan, .. } if chan == &span_b));
}

#[test]
fn alt_arrows_scroll_each_tab_independently_and_show_indicator() {
    let mut tui = single_line_tui(24, 6);
    let first = "first.example.org";
    let second = "second.example.org";
    tui.new_server_tab(first, None);
    tui.new_server_tab(second, None);
    tui.next_tab();

    let ts = time::empty_tm();
    for line in 0..10 {
        tui.add_msg(
            &format!("line{line}"),
            ts,
            &MsgTarget::Server { serv: first },
        );
    }

    tui.handle_input_event(Event::Key(Key::AltArrow(term_input::Arrow::Up)));
    assert_eq!(tui.get_tabs()[1].widget.scroll_offset(), 5);
    assert_eq!(tui.get_tabs()[2].widget.scroll_offset(), 0);

    tui.add_msg("new", ts, &MsgTarget::Server { serv: first });
    assert_eq!(tui.get_tabs()[1].widget.scroll_offset(), 6);

    tui.draw();
    let screen = crate::test_utils::buffer_str(&tui.get_front_buffer(), 24, 6);
    assert!(screen.lines().next().unwrap().ends_with('↑'));

    tui.next_tab();
    assert_eq!(tui.get_tabs()[2].widget.scroll_offset(), 0);
    tui.prev_tab();
    assert_eq!(tui.get_tabs()[1].widget.scroll_offset(), 6);

    tui.handle_input_event(Event::Key(Key::AltArrow(term_input::Arrow::Down)));
    tui.handle_input_event(Event::Key(Key::AltArrow(term_input::Arrow::Down)));
    assert_eq!(tui.get_tabs()[1].widget.scroll_offset(), 0);

    tui.add_msg("live", ts, &MsgTarget::Server { serv: first });
    assert_eq!(tui.get_tabs()[1].widget.scroll_offset(), 0);
    tui.draw();
    let screen = crate::test_utils::buffer_str(&tui.get_front_buffer(), 24, 6);
    assert!(!screen.contains('↑'));
}

#[test]
fn close_rightmost_tab() {
    // After closing right-most tab the tab bar should scroll left.
    let mut tui = single_line_tui(20, 4);
    tui.new_server_tab("irc.server_1.org", None);
    tui.new_server_tab("irc.server_2.org", None);
    tui.next_tab();
    tui.next_tab();
    tui.draw();

    #[rustfmt::skip]
    let screen =
        "|                    |
         |                    |
         |                    |
         |< irc.server_2.org  |";
    expect_screen(screen, &tui.get_front_buffer(), 20, 4, Location::caller());

    // Should scroll left when the server tab is closed. Left arrow should still be visible as
    // there are still tabs to the left.
    tui.close_server_tab("irc.server_2.org");
    tui.draw();

    #[rustfmt::skip]
    let screen =
        "|                    |
         |                    |
         |                    |
         |irc.server_1.org    |";
    expect_screen(screen, &tui.get_front_buffer(), 20, 4, Location::caller());

    // Scroll left again, left arrow should disappear this time.
    tui.close_server_tab("irc.server_1.org");
    tui.draw();

    #[rustfmt::skip]
    let screen =
        "|Any mentions to you |
         |will be listed here.|
         |                    |
         |                    |";
    expect_screen(screen, &tui.get_front_buffer(), 20, 4, Location::caller());
}

#[test]
fn small_screen_1() {
    let mut tui = single_line_tui(21, 3);
    let serv = "irc.server_1.org";
    let chan = ChanNameRef::new("#chan");
    tui.new_server_tab(serv, None);
    tui.set_nick(serv, "osa1");
    tui.new_chan_tab(serv, chan);
    tui.next_tab();
    tui.next_tab();

    let target = MsgTarget::Chan { serv, chan };
    let ts = time::at_utc(time::Timespec::new(0, 0));
    tui.add_nick("123456", Some(ts), &target);
    tui.add_nick("abcdef", Some(ts), &target);

    tui.draw();

    #[rustfmt::skip]
    let screen =
        "|00:00 +123456 +abcdef|
         |osa1:                |
         |< #chan              |";

    expect_screen(screen, &tui.get_front_buffer(), 21, 3, Location::caller());

    tui.set_size(24, 3);
    tui.draw();

    #[rustfmt::skip]
    let screen =
        "|00:00 +123456 +abcdef   |
         |osa1:                   |
         |irc.server_1.org #chan  |";

    expect_screen(screen, &tui.get_front_buffer(), 24, 3, Location::caller());

    tui.set_size(31, 3);
    tui.draw();

    #[rustfmt::skip]
    let screen =
        "|00:00 +123456 +abcdef          |
         |osa1:                          |
         |irc.server_1.org #chan         |";

    expect_screen(screen, &tui.get_front_buffer(), 31, 3, Location::caller());
}

#[test]
fn small_screen_2() {
    let mut tui = single_line_tui(21, 4);
    let serv = "irc.server_1.org";
    let chan = ChanNameRef::new("#chan");
    tui.new_server_tab(serv, None);
    tui.set_nick(serv, "osa1");
    tui.new_chan_tab(serv, chan);
    tui.next_tab();
    tui.next_tab();

    let target = MsgTarget::Chan { serv, chan };
    let ts = time::at_utc(time::Timespec::new(0, 0));
    tui.set_topic("Blah blah blah-", ts, serv, chan);

    tui.draw();

    #[rustfmt::skip]
    let screen =
        "|                     |
         |00:00 Blah blah blah-|
         |osa1:                |
         |< #chan              |";
    expect_screen(screen, &tui.get_front_buffer(), 21, 4, Location::caller());

    tui.add_nick("123456", Some(ts), &target);
    tui.draw();

    #[rustfmt::skip]
    let screen =
        "|00:00 Blah blah blah-|
         |+123456              |
         |osa1:                |
         |< #chan              |";
    expect_screen(screen, &tui.get_front_buffer(), 21, 4, Location::caller());
}

#[test]
fn ctrl_w() {
    let mut tui = single_line_tui(30, 3);
    let serv = "irc.server_1.org";
    let chan = ChanNameRef::new("#chan");
    tui.new_server_tab(serv, None);
    tui.set_nick(serv, "osa1");
    tui.new_chan_tab(serv, chan);
    tui.next_tab();
    tui.next_tab();

    enter_string(&mut tui, "alskdfj asldkf asldkf aslkdfj aslkdfj asf");

    tui.draw();

    #[rustfmt::skip]
    let screen =
        "|                              |
         |osa1: dkf aslkdfj aslkdfj asf |
         |irc.server_1.org #chan        |";
    expect_screen(screen, &tui.get_front_buffer(), 30, 3, Location::caller());

    tui.handle_input_event(Event::Key(Key::Ctrl('w')));
    tui.draw();

    #[rustfmt::skip]
    let screen =
        "|                              |
         |osa1: asldkf aslkdfj aslkdfj  |
         |irc.server_1.org #chan        |";

    expect_screen(screen, &tui.get_front_buffer(), 30, 3, Location::caller());

    println!("~~~~~~~~~~~~~~~~~~~~~~");
    tui.handle_input_event(Event::Key(Key::Ctrl('w')));
    println!("~~~~~~~~~~~~~~~~~~~~~~");
    tui.draw();

    /*
    The buggy behavior was as below:

    let screen =
        "|                              |
         |osa1:  asldkf aslkdfj         |
         |irc.server_1.org #chan        |";
    */

    #[rustfmt::skip]
    let screen =
        "|                              |
         |osa1:  asldkf asldkf aslkdfj  |
         |irc.server_1.org #chan        |";

    expect_screen(screen, &tui.get_front_buffer(), 30, 3, Location::caller());

    tui.handle_input_event(Event::Key(Key::Ctrl('w')));
    tui.draw();

    #[rustfmt::skip]
    let screen =
        "|                              |
         |osa1: alskdfj asldkf asldkf   |
         |irc.server_1.org #chan        |";

    expect_screen(screen, &tui.get_front_buffer(), 30, 3, Location::caller());
}

// Tests text field wrapping (text_field_wrap setting)
#[test]
fn test_text_field_wrap() {
    // Screen should be wide enough to enable wrapping. See SCROLL_FALLBACK_WIDTH in text_field.rs
    let mut tui = single_line_tui(40, 8);

    let server = "chat.freenode.net";
    tui.new_server_tab(server, None);
    tui.set_nick(server, "x");

    // Switch to server tab
    tui.next_tab();

    // Write some stuff
    let target = MsgTarget::CurrentTab;
    let ts = time::empty_tm();
    tui.add_msg("test test test", ts, &target);

    for _ in 0..37 {
        let event = term_input::Event::Key(Key::Char('a'));
        tui.handle_input_event(event);
    }
    for _ in 0..5 {
        let event = term_input::Event::Key(Key::Char('b'));
        tui.handle_input_event(event);
    }

    tui.draw();

    #[rustfmt::skip]
    let screen =
    "|                                        |
     |                                        |
     |                                        |
     |                                        |
     |00:00 test test test                    |
     |x: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa|
     |bbbbb                                   |
     |chat.freenode.net                       |";

    expect_screen(screen, &tui.get_front_buffer(), 40, 8, Location::caller());

    // Test resizing
    tui.set_size(46, 8);
    tui.draw();

    #[rustfmt::skip]
    let screen =
    "|                                              |
     |                                              |
     |                                              |
     |                                              |
     |                                              |
     |00:00 test test test                          |
     |x: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaabbbbb |
     |chat.freenode.net                             |";

    expect_screen(screen, &tui.get_front_buffer(), 46, 8, Location::caller());

    // Reset size
    tui.set_size(40, 8);

    // If we remove a few characters now the line above the text field should still be right above
    // the text field
    for _ in 0..6 {
        let event = term_input::Event::Key(Key::Backspace);
        tui.handle_input_event(event);
    }

    tui.draw();

    #[rustfmt::skip]
    let screen =
    "|                                        |
     |                                        |
     |                                        |
     |                                        |
     |                                        |
     |00:00 test test test                    |
     |x: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa |
     |chat.freenode.net                       |";

    expect_screen(screen, &tui.get_front_buffer(), 40, 8, Location::caller());

    // On making screen smaller we should fall back to scrolling
    tui.set_size(30, 8);
    for _ in 0..5 {
        let event = term_input::Event::Key(Key::Char('b'));
        tui.handle_input_event(event);
    }
    tui.draw();

    #[rustfmt::skip]
    let screen =
    "|                              |
     |                              |
     |                              |
     |                              |
     |                              |
     |00:00 test test test          |
     |x: aaaaaaaaaaaaaaaaaaaaabbbbb |
     |chat.freenode.net             |";

    expect_screen(screen, &tui.get_front_buffer(), 30, 8, Location::caller());

    tui.set_size(40, 8);
    tui.draw();

    #[rustfmt::skip]
    let screen =
    "|                                        |
     |                                        |
     |                                        |
     |                                        |
     |00:00 test test test                    |
     |x: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab|
     |bbbb                                    |
     |chat.freenode.net                       |";

    expect_screen(screen, &tui.get_front_buffer(), 40, 8, Location::caller());

    // Wrapping on words - splits lines on whitespace
    for _ in 0..6 {
        let event = term_input::Event::Key(Key::Backspace);
        tui.handle_input_event(event);
    }
    // InputLine cache gets invalidated after backspace, need to redraw to calculate.
    tui.draw();
    let event = term_input::Event::Key(Key::Char(' '));
    tui.handle_input_event(event);

    for _ in 0..5 {
        let event = term_input::Event::Key(Key::Char('b'));
        tui.handle_input_event(event);
    }

    tui.draw();

    #[rustfmt::skip]
    let screen =
    "|                                        |
     |                                        |
     |                                        |
     |                                        |
     |00:00 test test test                    |
     |x: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  |
     |bbbbb                                   |
     |chat.freenode.net                       |";

    expect_screen(screen, &tui.get_front_buffer(), 40, 8, Location::caller());

    // TODO: Test changing nick (osa: I don't understand how nick length is taken into account when
    // falling back to scrolling)
}
