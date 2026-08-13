use std::panic::Location;

use libtiny_common::{ChanNameRef, MsgSource, MsgTarget};
use term_input::{Event, Key};
use termbox_simple::TB_BOLD;

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
    let mut tui = TUI::new_test(w, h);
    tui.use_single_line_input();
    tui
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
fn dual_inputs_are_visible_and_tab_toggles_focus() {
    let mut tui = TUI::new_test(30, 10);
    tui.new_server_tab("irc.example.org", None);
    tui.next_tab();

    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "composer");
    tui.draw();
    let buffer = tui.get_front_buffer();
    let single_line = 3 * 30;
    let single_line_focus = 4 * 30;
    let composer_top = 5 * 30;
    assert_eq!(buffer.cells[composer_top].ch, '┏');
    assert_eq!(buffer.cells[single_line_focus + 29].ch, ' ');
    assert_ne!(buffer.cells[composer_top].fg & TB_BOLD, 0);

    tui.handle_input_event(Event::Key(Key::Tab));
    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "single_line");
    tui.draw();
    let buffer = tui.get_front_buffer();
    assert_eq!(buffer.cells[single_line + 29].ch, ' ');
    assert_eq!(buffer.cells[single_line_focus + 29].ch, '━');
    assert_ne!(buffer.cells[single_line_focus + 29].fg & TB_BOLD, 0);
    assert_eq!(buffer.cells[composer_top].fg & TB_BOLD, 0);

    enter_string(&mut tui, "command text");
    tui.draw();
    let buffer = tui.get_front_buffer();
    assert_eq!(buffer.cells[single_line_focus + 29].ch, '━');
    assert_ne!(buffer.cells[single_line_focus + 29].fg & TB_BOLD, 0);

    tui.handle_input_event(Event::Key(Key::Tab));
    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "composer");
    tui.draw();
    let buffer = tui.get_front_buffer();
    assert_eq!(buffer.cells[single_line_focus + 29].ch, ' ');
    assert_eq!(buffer.cells[single_line_focus + 29].fg & TB_BOLD, 0);
}

#[test]
fn composer_enter_and_ctrl_s_submit_one_logical_message() {
    let mut tui = TUI::new_test(30, 10);
    tui.new_server_tab("irc.example.org", None);
    tui.next_tab();

    enter_string(&mut tui, "first");
    assert!(
        tui.handle_input_event(Event::Key(Key::Char('\r')))
            .is_none()
    );
    enter_string(&mut tui, "second");

    assert!(matches!(
        tui.handle_input_event(Event::Key(Key::Ctrl('s'))),
        Some(crate::tui::TUIRet::Lines { lines, .. })
            if lines == ["first", "second"]
    ));
    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "composer");

    enter_string(&mut tui, "next");
    assert!(matches!(
        tui.handle_input_event(Event::Key(Key::Ctrl('s'))),
        Some(crate::tui::TUIRet::Lines { lines, .. }) if lines == ["next"]
    ));
}

#[test]
fn multiline_paste_targets_composer_without_changing_focus() {
    let mut tui = TUI::new_test(20, 10);
    tui.new_server_tab("irc.example.org", None);
    tui.next_tab();
    tui.handle_input_event(Event::Key(Key::Tab));
    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "single_line");

    assert!(
        tui.handle_input_event(Event::String("one\r\ntwo".to_owned()))
            .is_none()
    );
    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "single_line");

    tui.handle_input_event(Event::Key(Key::Tab));
    assert!(matches!(
        tui.handle_input_event(Event::Key(Key::Ctrl('s'))),
        Some(crate::tui::TUIRet::Lines { lines, .. }) if lines == ["one", "two"]
    ));
}

#[test]
fn scrollback_and_tab_movement_do_not_change_input_focus() {
    let mut tui = TUI::new_test(24, 10);
    let serv = "irc.example.org";
    let first = ChanNameRef::new("#first");
    let second = ChanNameRef::new("#second");
    tui.new_server_tab(serv, None);
    tui.new_chan_tab(serv, first);
    tui.new_chan_tab(serv, second);
    tui.next_tab();
    tui.next_tab();

    for line in 0..20 {
        tui.add_msg(
            &format!("line{line}"),
            time::empty_tm(),
            &MsgTarget::Chan { serv, chan: first },
        );
    }
    tui.handle_input_event(Event::String("draft\ntext".to_owned()));
    tui.handle_input_event(Event::Key(Key::AltArrow(term_input::Arrow::Up)));
    assert_eq!(tui.get_tabs()[2].widget.scroll_offset(), 5);
    assert_eq!(tui.get_tabs()[2].widget.input_focus(), "composer");

    let source = tui.current_tab().clone();
    tui.handle_input_event(Event::Key(Key::AltArrow(term_input::Arrow::Right)));
    assert_eq!(tui.current_tab(), &source);
    assert_eq!(tui.get_tabs()[3].widget.input_focus(), "composer");
    tui.handle_input_event(Event::Key(Key::AltArrow(term_input::Arrow::Left)));
    assert_eq!(tui.current_tab(), &source);
    assert_eq!(tui.get_tabs()[2].widget.input_focus(), "composer");

    tui.handle_input_event(Event::Key(Key::Tab));
    tui.handle_input_event(Event::Key(Key::AltArrow(term_input::Arrow::Down)));
    assert_eq!(tui.get_tabs()[2].widget.scroll_offset(), 0);
    assert_eq!(tui.get_tabs()[2].widget.input_focus(), "single_line");
}

#[test]
fn single_line_send_is_unchanged() {
    let mut tui = TUI::new_test(20, 10);
    tui.new_server_tab("irc.example.org", None);
    tui.next_tab();
    tui.handle_input_event(Event::Key(Key::Tab));
    enter_string(&mut tui, "quick riff");

    assert!(matches!(
        tui.handle_input_event(Event::Key(Key::Char('\r'))),
        Some(crate::tui::TUIRet::Input { msg, .. })
            if msg.iter().collect::<String>() == "quick riff"
    ));
}

#[test]
fn composer_content_is_not_parsed_as_a_command() {
    let mut tui = TUI::new_test(30, 10);
    tui.new_server_tab("irc.example.org", None);
    tui.next_tab();
    enter_string(&mut tui, "/trail still message content");

    assert!(matches!(
        tui.handle_input_event(Event::Key(Key::Ctrl('s'))),
        Some(crate::tui::TUIRet::Lines { lines, .. })
            if lines == ["/trail still message content"]
    ));
}

#[test]
fn navigation_dialog_preserves_focus_and_drafts_and_suppresses_input() {
    let mut tui = TUI::new_test(70, 30);
    tui.new_server_tab("irc.example.org", None);
    tui.new_server_tab("irc.other.example", None);
    tui.next_tab();
    enter_string(&mut tui, "composer draft");
    tui.handle_input_event(Event::Key(Key::Tab));
    enter_string(&mut tui, "command draft");
    for line in 0..30 {
        tui.add_msg(
            &format!("line{line}"),
            time::empty_tm(),
            &MsgTarget::Server {
                serv: "irc.example.org",
            },
        );
    }
    tui.draw();
    tui.handle_input_event(Event::Key(Key::AltArrow(term_input::Arrow::Up)));
    let scroll_offset = tui.get_tabs()[1].widget.scroll_offset();
    assert!(scroll_offset > 0);

    tui.handle_input_event(Event::Key(Key::Esc));
    assert!(tui.navigation_dialog_visible());
    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "single_line");
    tui.draw();
    assert!(buffer_str(&tui.get_front_buffer(), 70, 30).contains("Navigation"));

    enter_string(&mut tui, " ignored");
    tui.handle_input_event(Event::String("\npasted".to_owned()));
    tui.handle_input_event(Event::Key(Key::Tab));
    tui.handle_input_event(Event::Key(Key::AltArrow(term_input::Arrow::Right)));
    tui.handle_input_event(Event::Key(Key::AltArrow(term_input::Arrow::Down)));
    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "single_line");
    assert_eq!(tui.get_tabs()[1].widget.scroll_offset(), scroll_offset);

    tui.handle_input_event(Event::Key(Key::Esc));
    assert!(!tui.navigation_dialog_visible());
    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "single_line");
    tui.handle_input_event(Event::Key(Key::AltArrow(term_input::Arrow::Down)));
    assert_eq!(tui.get_tabs()[1].widget.scroll_offset(), 0);
    assert!(matches!(
        tui.handle_input_event(Event::Key(Key::Char('\r'))),
        Some(crate::tui::TUIRet::Input { msg, .. })
            if msg.iter().collect::<String>() == "command draft"
    ));

    tui.handle_input_event(Event::Key(Key::Tab));
    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "composer");
    assert!(matches!(
        tui.handle_input_event(Event::Key(Key::Ctrl('s'))),
        Some(crate::tui::TUIRet::Lines { lines, .. }) if lines == ["composer draft"]
    ));

    let source = tui.current_tab().clone();
    tui.handle_input_event(Event::Key(Key::AltArrow(term_input::Arrow::Right)));
    assert_eq!(tui.current_tab(), &source);
    tui.handle_input_event(Event::Key(Key::AltArrow(term_input::Arrow::Left)));
    assert_eq!(tui.current_tab(), &source);
    assert_eq!(tui.get_tabs()[1].widget.input_focus(), "composer");
}

#[test]
fn hidden_mentions_tab_still_receives_messages() {
    let mut tui = single_line_tui(20, 4);
    let target = MsgTarget::Server { serv: "mentions" };

    tui.add_msg("osa1 in x.y.z:#camp: hello", time::empty_tm(), &target);

    let lines = tui.tab_lines_text(&target).unwrap();
    assert!(lines.iter().any(|line| line.contains("Any mentions")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("osa1 in x.y.z:#camp: hello"))
    );
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
