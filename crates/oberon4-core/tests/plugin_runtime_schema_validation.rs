use std::{fs, path::PathBuf};

use jsonschema::validator_for;
use serde_json::Value;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate parent")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn schema_dir() -> PathBuf {
    repo_root().join("docs/schemas/wasm-plugin-runtime")
}

fn load_json(path: PathBuf) -> Value {
    let raw = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("failed to read {}: {}", path.display(), err);
    });
    serde_json::from_str(&raw).unwrap_or_else(|err| {
        panic!("failed to parse {}: {}", path.display(), err);
    })
}

fn assert_valid(schema_name: &str, fixture_name: &str) {
    let schema = load_json(schema_dir().join(schema_name));
    let fixture = load_json(schema_dir().join("fixtures").join(fixture_name));
    let validator = validator_for(&schema).expect("schema should compile");
    let errors = validator.iter_errors(&fixture).collect::<Vec<_>>();
    assert!(
        errors.is_empty(),
        "fixture {} failed {} with errors: {:?}",
        fixture_name,
        schema_name,
        errors
    );
}

fn assert_invalid(schema_name: &str, fixture_name: &str) {
    let schema = load_json(schema_dir().join(schema_name));
    let fixture = load_json(schema_dir().join("fixtures").join(fixture_name));
    let validator = validator_for(&schema).expect("schema should compile");
    let errors = validator.iter_errors(&fixture).collect::<Vec<_>>();
    assert!(
        !errors.is_empty(),
        "fixture {} unexpectedly passed {}",
        fixture_name,
        schema_name
    );
}

#[test]
fn plugin_manifest_fixture_validation() {
    assert_valid("plugin-manifest.schema.json", "plugin-manifest.valid.json");
    assert_invalid(
        "plugin-manifest.schema.json",
        "plugin-manifest.invalid-missing-runtime-api-version.json",
    );
}

#[test]
fn approval_and_policy_fixtures_validate() {
    assert_valid(
        "approval-request.schema.json",
        "approval-request.valid.json",
    );
    assert_valid(
        "approval-decision.schema.json",
        "approval-decision.valid.json",
    );
    assert_valid(
        "policy-inspect-result.schema.json",
        "policy-inspect.valid.json",
    );
}

#[test]
fn tool_runtime_fixtures_validate() {
    assert_valid(
        "tool-search-request.schema.json",
        "tool-search-request.valid.json",
    );
    assert_valid(
        "tool-search-response.schema.json",
        "tool-search-response.valid.json",
    );
    assert_valid(
        "tool-describe-request.schema.json",
        "tool-describe-request.valid.json",
    );
    assert_valid(
        "tool-describe-response.schema.json",
        "tool-describe-response.valid.json",
    );
    assert_valid(
        "tool-plan-request.schema.json",
        "tool-plan-request.valid.json",
    );
    assert_valid(
        "tool-plan-response.schema.json",
        "tool-plan-response.valid.json",
    );
    assert_valid(
        "tool-invoke-request.schema.json",
        "tool-invoke-request.valid.json",
    );
    assert_valid(
        "tool-invoke-response.schema.json",
        "tool-invoke-response.valid.json",
    );
    assert_valid("tool-result.schema.json", "tool-result.valid.json");
    assert_valid("tool-error.schema.json", "tool-error.valid.json");
}

#[test]
fn first_package_command_family_fixtures_validate() {
    assert_valid(
        "workspace-commands.schema.json",
        "workspace-write-file-atomic.input.valid.json",
    );
    assert_valid(
        "workspace-commands.schema.json",
        "workspace-write-file-atomic.output.valid.json",
    );
    assert_valid("git-commands.schema.json", "git-push.input.valid.json");
    assert_valid("git-commands.schema.json", "git-push.output.valid.json");
    assert_valid(
        "github-commands.schema.json",
        "github-auth-status.output.valid.json",
    );
    assert_valid(
        "github-commands.schema.json",
        "github-clone-to-workspace.output.valid.json",
    );
}
