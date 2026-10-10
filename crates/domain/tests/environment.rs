use std::collections::HashMap;

use domain::AppError;
use domain::environment::{Environment, Scope, Variable, resolve, validate};

fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn var(key: &str, value: &str) -> Variable {
    Variable {
        key: key.into(),
        value: value.into(),
        secret: false,
    }
}

fn env(name: &str, scope: Scope, variables: Vec<Variable>) -> Environment {
    Environment {
        name: name.into(),
        scope,
        variables,
    }
}

// parity: apiark storage/environment.rs get_resolved_variables
#[test]
fn environment_overrides_root_dotenv() {
    let vars = resolve(
        map(&[("host", "root"), ("only_root", "r")]),
        &env(
            "dev",
            Scope::Shared,
            vec![var("host", "env"), var("token", "t")],
        ),
    );
    assert_eq!(
        vars,
        map(&[("host", "env"), ("token", "t"), ("only_root", "r")])
    );
}

#[test]
fn validate_trims_name_and_drops_blank_keys() {
    let got = validate(
        env(
            " dev ",
            Scope::Shared,
            vec![var("a", "1"), var(" ", "lost"), var(" b ", "2")],
        ),
        &[],
    )
    .unwrap();
    assert_eq!(
        got,
        env("dev", Scope::Shared, vec![var("a", "1"), var("b", "2")])
    );
}

#[test]
fn validate_rejects_blank_name() {
    assert_eq!(
        validate(env("  ", Scope::Shared, vec![]), &[]),
        Err(AppError::InvalidName(String::new()))
    );
}

#[test]
fn validate_rejects_a_name_used_in_either_scope() {
    let others = [env("dev", Scope::Personal, vec![])];
    assert_eq!(
        validate(env("dev", Scope::Shared, vec![]), &others),
        Err(AppError::AlreadyExists("dev".into()))
    );
}

#[test]
fn validate_rejects_duplicate_key() {
    assert_eq!(
        validate(
            env("dev", Scope::Shared, vec![var("a", "1"), var("a", "2")]),
            &[]
        ),
        Err(AppError::DuplicateKey("a".into()))
    );
}
