use ai_stock_forum::app::ApplicationCommand;
use ai_stock_forum::policy::{
    evaluate, ApprovalAction, ApprovalRecord, ApprovalStatus, Capability, Effect,
    PolicyDecision, PolicyRule,
};
use ai_stock_forum::ui::command::{parse_line, ParsedLine};

#[test]
fn explicit_denial_wins_over_a_grant() {
    let rules = [
        PolicyRule::new(Effect::Grant, Capability::AuditRead),
        PolicyRule::new(Effect::Deny, Capability::AuditRead),
    ];

    assert_eq!(
        evaluate(Capability::AuditRead, &rules),
        PolicyDecision::Denied
    );
}

#[test]
fn missing_rule_denies_by_default() {
    assert_eq!(
        evaluate(Capability::GitPush, &[]),
        PolicyDecision::DeniedByDefault
    );
}

#[test]
fn commands_map_to_exact_safe_capabilities() {
    assert_eq!(
        ApplicationCommand::ShowHelp.required_capability(),
        Capability::HelpRead
    );
    assert_eq!(
        ApplicationCommand::RequestShutdown.required_capability(),
        Capability::Shutdown
    );

    let ParsedLine::Command(rejected) = parse_line(b"/not-supported") else {
        panic!("expected rejected command");
    };
    assert_eq!(rejected.required_capability(), Capability::HelpRead);
}

#[test]
fn approval_requires_an_exact_object_and_pending_status() {
    assert!(ApprovalRecord::builder(ApprovalAction::GitPush)
        .build()
        .is_err());
    assert!(!ApprovalStatus::Pending.is_terminal());
}
