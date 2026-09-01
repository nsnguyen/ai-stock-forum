use ai_stock_forum::app::{ApplicationCommand, InputRejectionCategory};
use ai_stock_forum::ui::command::{parse_line, ParsedLine};

fn command(bytes: &[u8]) -> ApplicationCommand {
    match parse_line(bytes) {
        ParsedLine::Command(command) => command,
        ParsedLine::Ignored => panic!("expected command"),
    }
}

#[test]
fn parses_the_complete_phase_zero_grammar() {
    assert_eq!(command(b" /help \n"), ApplicationCommand::ShowHelp);
    assert_eq!(command(b"/status"), ApplicationCommand::ShowStatus);
    assert_eq!(command(b"/setup status"), ApplicationCommand::ShowSetupStatus);
    assert_eq!(
        command(b"/audit tail"),
        ApplicationCommand::audit_tail(20).unwrap()
    );
    assert_eq!(
        command(b"/audit tail 100"),
        ApplicationCommand::audit_tail(100).unwrap()
    );
    assert_eq!(command(b"/quit"), ApplicationCommand::RequestShutdown);
}

#[test]
fn rejects_bad_audit_limits_without_defaulting() {
    for input in [
        b"/audit tail 0".as_slice(),
        b"/audit tail 101",
        b"/audit tail nope",
    ] {
        let ApplicationCommand::RejectInput(rejection) = command(input) else {
            panic!("expected rejection");
        };
        assert_eq!(rejection.category, InputRejectionCategory::Malformed);
        assert!(!serde_json::to_string(&rejection)
            .unwrap()
            .contains("raw_input"));
    }
}

#[test]
fn never_carries_unknown_raw_input() {
    let ApplicationCommand::RejectInput(rejection) = command(b"/secret hunter2") else {
        panic!("expected rejection");
    };
    assert_eq!(rejection.safe_token.as_deref(), Some("/secret"));
    assert_eq!(rejection.byte_length, 15);
    let encoded = serde_json::to_string(&rejection).unwrap();
    assert!(!encoded.contains("hunter2"));
    assert!(!encoded.contains("raw_input"));
}

#[test]
fn rejects_invalid_utf8_and_oversized_input() {
    let invalid = command(&[0xff, 0xfe]);
    assert!(matches!(invalid, ApplicationCommand::RejectInput(ref value)
        if value.category == InputRejectionCategory::InvalidEncoding));

    let oversized = command(&vec![b'x'; 4097]);
    assert!(matches!(oversized, ApplicationCommand::RejectInput(ref value)
        if value.category == InputRejectionCategory::Oversized));
}

#[test]
fn escapes_control_characters_in_unknown_command_tokens() {
    let ApplicationCommand::RejectInput(rejection) = command(b"/secret\x1bhunter2") else {
        panic!("expected rejection");
    };

    assert_eq!(
        rejection.safe_token.as_deref(),
        Some("/secret\\u{1b}hunter2")
    );
}

#[test]
fn truncates_unknown_tokens_without_emitting_partial_escape_fragments() {
    let mut input = vec![b'a'; 63];
    input.push(0x1b);

    let ApplicationCommand::RejectInput(rejection) = command(&input) else {
        panic!("expected rejection");
    };
    let safe_token = rejection.safe_token.expect("expected safe token");

    assert_eq!(safe_token.chars().count(), 63);
    assert!(safe_token.chars().all(|character| character == 'a'));
}

#[test]
fn accepts_a_valid_command_at_exactly_4096_bytes() {
    let mut input = b"/help".to_vec();
    input.resize(4096, b' ');

    assert_eq!(command(&input), ApplicationCommand::ShowHelp);
}

#[test]
fn rejects_commands_with_extra_arguments() {
    let ApplicationCommand::RejectInput(rejection) = command(b"/status extra") else {
        panic!("expected rejection");
    };

    assert_eq!(rejection.category, InputRejectionCategory::Malformed);
}
