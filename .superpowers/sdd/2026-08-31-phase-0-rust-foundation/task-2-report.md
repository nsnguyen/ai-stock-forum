# Task 2: Deterministic domain primitives report

## Scope delivered

- Added the `ai_stock_forum::domain` public module.
- Added UUID-backed typed IDs: `InstallationId`, `SessionId`, `CommandId`, `EventId`, `CorrelationId`, `CausationId`, `ApprovalId`, `SetupDraftId`, and `ConfigurationVersionId`.
- Added `Clock`, `IdGenerator`, `SystemClock`, `UuidGenerator`, and the two-variant `Actor` enum.
- Added validated `ObjectVersion`, `ObjectRef`, `Sha256Digest`, canonical JSON serialization, SHA-256 hashing, and `DomainError`.
- Added the requested `domain_contract` integration test.

## Dependency changes

Added exactly the requested runtime dependencies: `serde` with `derive`, `serde_json`, `sha2`, `hex`, `uuid` with `v4` and `serde`, and `thiserror`.

## TDD evidence

### RED

The first focused test command initially could not download crates because sandbox DNS resolution failed. The same command was rerun with approved Cargo network access. It then compiled the crate and failed for the intended missing public domain export:

```text
$ /Users/nguyen-mini/.cargo/bin/cargo test --test domain_contract --locked
error[E0432]: unresolved import `ai_stock_forum::domain`
 --> tests/domain_contract.rs:1:21
  |
1 | use ai_stock_forum::domain::{
  |                     ^^^^^^ could not find `domain` in `ai_stock_forum`

error: could not compile `ai-stock-forum` (test "domain_contract") due to 1 previous error
```

This demonstrates that the test failed because the required domain surface did not exist, rather than due to a test typo or an unrelated failure.

### GREEN

After the minimal implementation was added, the required focused suite passed:

```text
$ /Users/nguyen-mini/.cargo/bin/cargo test --test domain_contract --locked
running 4 tests
test object_versions_reject_zero ... ok
test typed_ids_do_not_interchange ... ok
test digest_requires_lowercase_sha256_hex ... ok
test canonical_json_sorts_nested_object_keys ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## Full-suite verification

```text
$ /Users/nguyen-mini/.cargo/bin/cargo test --locked
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

The full current suite passed: 5 integration tests in total, plus empty unit-test and doc-test suites.

## Self-review

- Typed ID wrappers derive `Copy`, `Clone`, `Eq`, `Ord`, and `Hash`, so `SessionId` is usable as a `BTreeMap` key.
- Every wrapper serializes transparently as its UUID and parses through `Uuid`'s standard parser.
- Canonical JSON recursively places object members in a `BTreeMap`, preserves array order, emits compact JSON, and propagates `serde_json` serialization failures (including non-finite floating-point values).
- SHA-256 digests are always generated as 64-character lowercase hexadecimal, and parsing plus deserialization rejects invalid or uppercase input.
- `ObjectVersion::new` rejects zero, and `ObjectRef::new` rejects empty or whitespace-only kind and ID values.
- No concern remains within the requested scope. `SystemClock` intentionally reflects wall-clock time; deterministic consumers can provide their own `Clock` implementation.

## Fix round 1: validate deserialization boundaries

### Changed behavior

`ObjectVersion` no longer accepts `0` through deserialization, and `ObjectRef` no longer accepts empty or whitespace-only `kind` or `id` values through deserialization. The serialized wire shapes are unchanged: `ObjectVersion` remains a JSON number and `ObjectRef` remains an object with `kind`, `id`, `version`, and `digest` fields.

### Root cause

The derived `Deserialize` implementations constructed the private fields directly, bypassing `ObjectVersion::new` and `ObjectRef::new`. The replacement implementations deserialize the existing wire shapes and delegate to those constructors.

### Files

- `src/domain/object.rs`
- `tests/domain_contract.rs`
- `.superpowers/sdd/2026-08-31-phase-0-rust-foundation/task-2-report.md`

### Regression tests

- `object_versions_reject_zero_when_deserialized`
- `object_refs_reject_empty_kind_when_deserialized`
- `object_refs_reject_whitespace_kind_when_deserialized`
- `object_refs_reject_empty_id_when_deserialized`
- `object_refs_reject_whitespace_id_when_deserialized`

### RED

```text
$ /Users/nguyen-mini/.cargo/bin/cargo test --test domain_contract --locked
running 9 tests
test object_refs_reject_whitespace_kind_when_deserialized ... FAILED
test object_refs_reject_whitespace_id_when_deserialized ... FAILED
test object_refs_reject_empty_id_when_deserialized ... FAILED
test object_versions_reject_zero_when_deserialized ... FAILED
test object_refs_reject_empty_kind_when_deserialized ... FAILED

test result: FAILED. 4 passed; 5 failed; 0 ignored; 0 measured; 0 filtered out
```

Each failure was an `assertion failed: serde_json::from_value::<...>(...).is_err()`, proving the invalid payload was accepted before the fix.

### GREEN

```text
$ /Users/nguyen-mini/.cargo/bin/cargo test --test domain_contract --locked
running 9 tests
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ /Users/nguyen-mini/.cargo/bin/cargo test --locked
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

### Commit

`fix: validate deserialized domain invariants`
