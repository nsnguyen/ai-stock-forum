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
