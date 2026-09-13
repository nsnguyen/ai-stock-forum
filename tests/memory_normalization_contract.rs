use ai_stock_forum::{
    domain::{DomainError, canonical_json_bytes},
    memory::{
        CredentialPatternSetV1, MemoryEntryDraft, NormalizedMemoryKey,
        PLAINTEXT_VALIDATION_VERSION_V1, PlaintextField, normalize_memory_key, validate_plaintext,
    },
};
use proptest::prelude::*;

const DISPLAY_KEY_MAX_BYTES: usize = 96;
const VALUE_MAX_BYTES: usize = 4_096;
const TAG_MAX_BYTES: usize = 32;

fn valid_drafts() -> impl Strategy<Value = (String, String, Vec<String>)> {
    ("[A-Za-z][A-Za-z0-9 ]{0,20}", "[A-Za-z0-9 .,!?]{1,64}")
        .prop_map(|(key, value)| (key, value, vec!["analysis".to_owned()]))
}

proptest! {
    #[test]
    fn canonical_memory_drafts_round_trip_without_digest_drift(
        (key, value, tags) in valid_drafts()
    ) {
        let canonical = MemoryEntryDraft::new(key, value, tags).unwrap();
        let bytes = canonical_json_bytes(&canonical).unwrap();
        let decoded: MemoryEntryDraft = serde_json::from_slice(&bytes).unwrap();
        prop_assert_eq!(decoded, canonical);
    }
}

#[test]
fn memory_draft_enforces_exact_display_key_and_value_byte_boundaries() {
    for len in [1, DISPLAY_KEY_MAX_BYTES] {
        assert!(
            MemoryEntryDraft::new("k".repeat(len), "value".to_owned(), vec!["tag".to_owned()])
                .is_ok()
        );
    }
    assert!(
        MemoryEntryDraft::new("é".repeat(48), "value".to_owned(), vec!["tag".to_owned()],).is_ok()
    );
    assert_eq!(
        MemoryEntryDraft::new("é".repeat(49), "value".to_owned(), vec!["tag".to_owned()])
            .unwrap_err()
            .code(),
        "invalid_memory_field"
    );
    assert!(
        MemoryEntryDraft::new(
            format!("  {}  ", "k".repeat(DISPLAY_KEY_MAX_BYTES)),
            "value".to_owned(),
            vec!["tag".to_owned()],
        )
        .is_ok()
    );
    assert_eq!(
        MemoryEntryDraft::new(
            format!("  {}  ", "k".repeat(DISPLAY_KEY_MAX_BYTES + 1)),
            "value".to_owned(),
            vec!["tag".to_owned()],
        )
        .unwrap_err()
        .code(),
        "invalid_memory_field"
    );
    for len in [0, DISPLAY_KEY_MAX_BYTES + 1] {
        assert_eq!(
            MemoryEntryDraft::new("k".repeat(len), "value".to_owned(), vec!["tag".to_owned()])
                .unwrap_err()
                .code(),
            "invalid_memory_field"
        );
    }
    for len in [1, VALUE_MAX_BYTES] {
        assert!(
            MemoryEntryDraft::new("key".to_owned(), "v".repeat(len), vec!["tag".to_owned()])
                .is_ok()
        );
    }
    for len in [0, VALUE_MAX_BYTES + 1] {
        assert_eq!(
            MemoryEntryDraft::new("key".to_owned(), "v".repeat(len), vec!["tag".to_owned()])
                .unwrap_err()
                .code(),
            "invalid_memory_field"
        );
    }
}

#[test]
fn memory_draft_enforces_exact_tag_byte_boundaries() {
    for len in [1, TAG_MAX_BYTES] {
        assert!(
            MemoryEntryDraft::new("key".to_owned(), "value".to_owned(), vec!["t".repeat(len)])
                .is_ok()
        );
    }
    for len in [0, TAG_MAX_BYTES + 1] {
        assert_eq!(
            MemoryEntryDraft::new("key".to_owned(), "value".to_owned(), vec!["t".repeat(len)])
                .unwrap_err()
                .code(),
            "invalid_memory_field"
        );
    }
}

#[test]
fn memory_draft_normalizes_line_endings_and_only_folds_single_line_fields() {
    let draft = MemoryEntryDraft::new(
        "  Market\r\n Thesis  ".to_owned(),
        "first\r\nsecond\rthird".to_owned(),
        vec!["  long\tterm  ".to_owned()],
    )
    .unwrap_err();
    assert_eq!(draft.code(), "unsafe_memory_text");

    let draft = MemoryEntryDraft::new(
        "  Market   Thesis  ".to_owned(),
        "first\r\nsecond\rthird".to_owned(),
        vec!["  long   term  ".to_owned()],
    )
    .unwrap();
    assert_eq!(draft.display_key(), "Market Thesis");
    assert_eq!(draft.value(), "first\nsecond\nthird");
    assert_eq!(draft.purpose_tags(), ["long term"]);
}

#[test]
fn memory_key_folds_internal_whitespace_nfkc_and_case() {
    let key = normalize_memory_key("  Ｍarket   THESIS ").unwrap();
    assert_eq!(key.as_str(), "market thesis");
    assert_eq!(NormalizedMemoryKey::new("Market Thesis").unwrap(), key);
}

#[test]
fn memory_draft_sorts_tags_and_rejects_duplicates_and_reserved_general_tag() {
    let draft = MemoryEntryDraft::new(
        "thesis".to_owned(),
        "value".to_owned(),
        vec!["Zeta".to_owned(), "alpha".to_owned()],
    )
    .unwrap();
    assert_eq!(draft.purpose_tags(), ["alpha", "Zeta"]);
    assert_eq!(
        MemoryEntryDraft::new(
            "thesis".to_owned(),
            "value".to_owned(),
            vec!["Alpha".to_owned(), " alpha ".to_owned()],
        )
        .unwrap_err()
        .code(),
        "invalid_memory_field"
    );
    assert_eq!(normalize_memory_key("general").unwrap().as_str(), "general");
    assert_eq!(
        MemoryEntryDraft::new(
            "thesis".to_owned(),
            "value".to_owned(),
            vec!["ＧＥＮＥＲＡＬ".to_owned()],
        )
        .unwrap_err()
        .code(),
        "invalid_memory_field"
    );
}

#[test]
fn memory_draft_rejects_controls_bidi_and_unsafe_line_separators() {
    for unsafe_value in [
        "tab\ttext",
        "nul\0text",
        "escape\u{001b}text",
        "c1\u{0085}text",
        "bidi\u{202e}text",
        "line\u{2028}text",
    ] {
        assert_eq!(
            MemoryEntryDraft::new(
                "key".to_owned(),
                unsafe_value.to_owned(),
                vec!["tag".to_owned()]
            )
            .unwrap_err()
            .code(),
            "unsafe_memory_text",
            "{unsafe_value:?}"
        );
    }
}

#[test]
fn credential_oriented_keys_use_exact_separator_aware_reserved_words() {
    for value in [
        "password",
        "passwords",
        "passphrase",
        "passphrases",
        "api_key",
        "api-keys",
        "access.token",
        "access tokens",
        "refresh/token",
        "refresh\\tokens",
        "session:token",
        "session tokens",
        "private-key",
        "private keys",
        "secret",
        "secrets",
        "credential",
        "credentials",
    ] {
        assert_eq!(
            normalize_memory_key(value).unwrap_err().code(),
            "invalid_memory_field",
            "{value}"
        );
    }
    for value in [
        "secretary",
        "apiary key",
        "password manager",
        "private keyring",
    ] {
        assert!(normalize_memory_key(value).is_ok(), "{value}");
    }
}

#[test]
fn serde_rejects_noncanonical_or_unsafe_memory_drafts() {
    for invalid in [
        r#"{"display_key":" Market  Thesis ","value":"value","purpose_tags":["tag"]}"#,
        r#"{"display_key":"key","value":"value","purpose_tags":[" tag "]}"#,
        r#"{"display_key":"key","value":"value","purpose_tags":["Zeta","alpha"]}"#,
        r#"{"display_key":"key","value":"value","purpose_tags":["ＧＥＮＥＲＡＬ"]}"#,
        r#"{"display_key":"key","value":"value","purpose_tags":["Alpha","alpha"]}"#,
        r#"{"display_key":"api-key","value":"value","purpose_tags":["tag"]}"#,
        r#"{"display_key":"key","value":"tab\ttext","purpose_tags":["tag"]}"#,
        r#"{"display_key":"key","value":"sk-ant-abcdefghijklmnopqrst","purpose_tags":["tag"]}"#,
    ] {
        assert!(
            serde_json::from_str::<MemoryEntryDraft>(invalid).is_err(),
            "{invalid}"
        );
    }
    assert!(serde_json::from_str::<NormalizedMemoryKey>(r#""Market Thesis""#).is_err());
    assert_eq!(
        serde_json::from_str::<NormalizedMemoryKey>(r#""market thesis""#)
            .unwrap()
            .as_str(),
        "market thesis"
    );
}

#[test]
fn credential_pattern_v1_is_delimiter_aware_and_versioned() {
    assert!(
        CredentialPatternSetV1::validate(
            PlaintextField::MemoryValue,
            "sk-ant-abcdefghijklmnopqrst",
        )
        .is_err()
    );
    assert!(
        CredentialPatternSetV1::validate(
            PlaintextField::MemoryValue,
            "task-ant-abcdefghijklmnopqrst-note",
        )
        .is_ok()
    );
    assert!(validate_plaintext(99, PlaintextField::MemoryValue, "ordinary prose",).is_err());
    assert_eq!(PLAINTEXT_VALIDATION_VERSION_V1, 1);
}

#[test]
fn credential_scanner_matches_every_private_key_marker_and_only_private_key_markers() {
    for value in [
        "-----BEGIN PRIVATE KEY-----",
        "-----BEGIN RSA PRIVATE KEY-----",
        "-----BEGIN DSA PRIVATE KEY-----",
        "-----BEGIN EC PRIVATE KEY-----",
        "-----begin openssh private key-----",
    ] {
        assert_credential_match(PlaintextField::MemoryValue, value);
    }
    assert_no_credential_match(PlaintextField::MemoryValue, "-----BEGIN PUBLIC KEY-----");
}

#[test]
fn bearer_scanner_honors_allowed_alphabet_and_exact_optional_spaces() {
    assert_credential_match(
        PlaintextField::MemoryValue,
        "Authorization: Bearer A0._~+/=-abcdefg",
    );
    assert_credential_match(
        PlaintextField::MemoryValue,
        "authorization : bearer abcdefghijklmnop",
    );
    assert_credential_match(
        PlaintextField::MemoryValue,
        "authorization:bearer abcdefghijklmnop",
    );
    for value in [
        "Authorization: Bearer abcdefghijklmno%",
        "Authorization  : Bearer abcdefghijklmnop",
        "Authorization:  Bearer abcdefghijklmnop",
        "Authorization: Basic abcdefghijklmnop",
    ] {
        assert_no_credential_match(PlaintextField::MemoryValue, value);
    }
}

#[test]
fn credential_token_scanners_enforce_minimum_and_field_specific_maximums() {
    for (field, maximum) in plaintext_field_limits() {
        let bearer_prefix = "Authorization: Bearer ";
        assert_no_credential_match(field, &format!("{bearer_prefix}{}", "a".repeat(15)));
        assert_credential_match(field, &format!("{bearer_prefix}{}", "a".repeat(16)));
        assert_credential_match(field, &format!("{bearer_prefix}{}", "a".repeat(maximum)));
        assert_no_credential_match(
            field,
            &format!("{bearer_prefix}{}", "a".repeat(maximum + 1)),
        );

        for prefix in ["sk-", "sk-ant-", "xai-"] {
            assert_no_credential_match(field, &format!("{prefix}{}", "a".repeat(19)));
            assert_credential_match(field, &format!("{prefix}{}", "a".repeat(20)));
            assert_credential_match(field, &format!("{prefix}{}", "a".repeat(maximum)));
            assert_no_credential_match(field, &format!("{prefix}{}", "a".repeat(maximum + 1)));
        }
    }
}

#[test]
fn prefixed_token_scanner_uses_longest_prefix_and_token_delimiters() {
    for suffix_length in 16..20 {
        assert_no_credential_match(
            PlaintextField::MemoryValue,
            &format!("sk-ant-{}", "a".repeat(suffix_length)),
        );
    }
    assert_credential_match(
        PlaintextField::MemoryValue,
        &format!("?sk-ant-{}!", "a".repeat(20)),
    );
    assert_no_credential_match(
        PlaintextField::MemoryValue,
        &format!("xsk-ant-{}", "a".repeat(20)),
    );
    assert_no_credential_match(
        PlaintextField::MemoryValue,
        &format!("sk-{}%", "a".repeat(19)),
    );
    assert_credential_match(
        PlaintextField::MemoryValue,
        &format!("sk-A0_-{}.", "a".repeat(16)),
    );
}

fn plaintext_field_limits() -> [(PlaintextField, usize); 4] {
    [
        (PlaintextField::MemoryValue, 4_096),
        (PlaintextField::ProposalRationale, 512),
        (PlaintextField::EpisodicLabel, 128),
        (PlaintextField::EpisodicBody, 8_192),
    ]
}

fn assert_credential_match(field: PlaintextField, value: &str) {
    assert_eq!(
        CredentialPatternSetV1::validate(field, value)
            .unwrap_err()
            .code(),
        "unsafe_memory_text",
        "{field:?}: {value:?}"
    );
}

fn assert_no_credential_match(field: PlaintextField, value: &str) {
    assert!(
        CredentialPatternSetV1::validate(field, value).is_ok(),
        "{field:?}: {value:?}"
    );
}

#[test]
fn plaintext_error_values_are_content_free_and_stable() {
    let error = MemoryEntryDraft::new("key".to_owned(), "\t".to_owned(), vec!["tag".to_owned()])
        .unwrap_err();
    assert_eq!(error, DomainError::UnsafeMemoryText { field: "value" });
    assert_eq!(error.code(), "unsafe_memory_text");
}
