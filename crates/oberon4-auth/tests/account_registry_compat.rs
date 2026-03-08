use std::fs;
use std::path::{Path, PathBuf};

use oberon4_auth::{AccountMetadataStore, FileAccountMetadataStore};
use oberon4_core::{AccountState, OAuthProviderKind, OberonError};

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "oberon4-auth-account-registry-compat-{label}-{}.json",
        std::process::id()
    ))
}

fn load_registry_fixture(name: &str) -> Result<oberon4_auth::AccountRegistry, OberonError> {
    let path = temp_path(name);
    fs::copy(fixture_path(name), &path).expect("fixture copy should succeed");
    let store = FileAccountMetadataStore::new(&path);
    let result = store.load_registry();
    let _ = fs::remove_file(&path);
    result
}

#[test]
fn registry_loads_when_active_accounts_field_is_missing() {
    let registry = load_registry_fixture("registry-missing-active-accounts.json").unwrap();

    let accounts = registry.list();
    assert_eq!(accounts.len(), 2);
    assert_eq!(
        accounts
            .iter()
            .map(|account| account.id.as_str())
            .collect::<Vec<_>>(),
        vec!["acct-openai-primary", "acct-anthropic-primary"]
    );
    assert_eq!(registry.active_account_id(OAuthProviderKind::OpenAi), None);
    assert_eq!(
        registry.active_account_id(OAuthProviderKind::Anthropic),
        None
    );
}

#[test]
fn registry_loads_when_only_one_provider_is_active() {
    let registry = load_registry_fixture("registry-partial-active-accounts.json").unwrap();

    let accounts = registry.list();
    assert_eq!(accounts.len(), 2);
    assert_eq!(
        registry.active_account_id(OAuthProviderKind::OpenAi),
        Some("acct-openai-primary")
    );
    assert_eq!(
        registry.active_account_id(OAuthProviderKind::Anthropic),
        None
    );
}

#[test]
fn registry_ignores_unknown_top_level_fields() {
    let registry = load_registry_fixture("registry-with-unknown-fields.json").unwrap();

    let accounts = registry.list();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].id, "acct-openai-primary");
    assert_eq!(accounts[0].state, AccountState::Active);
    assert_eq!(
        registry.active_account_id(OAuthProviderKind::OpenAi),
        Some("acct-openai-primary")
    );
}

#[test]
fn malformed_registry_still_fails_to_load() {
    let error = load_registry_fixture("registry-malformed.json").unwrap_err();
    match error {
        OberonError::Validation(message) => {
            assert!(message.contains("account store decode failed"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
