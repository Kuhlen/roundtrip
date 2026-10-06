use std::collections::HashMap;

use domain::environment::{Environment, Scope, resolve};

fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn env(variables: &[(&str, &str)], secrets: &[&str]) -> Environment {
    Environment {
        name: "dev".into(),
        scope: Scope::Shared,
        variables: map(variables),
        secrets: secrets.iter().map(|s| s.to_string()).collect(),
    }
}

// parity: apiark storage/environment.rs get_resolved_variables
#[test]
fn merge_priority_root_then_env_then_secrets() {
    let vars = resolve(
        map(&[("host", "root"), ("only_root", "r")]),
        &env(&[("host", "env"), ("token", "env")], &["token"]),
        &map(&[("token", "secret")]),
    );
    assert_eq!(
        vars,
        map(&[("host", "env"), ("token", "secret"), ("only_root", "r")])
    );
}

#[test]
fn secrets_not_listed_are_ignored() {
    let vars = resolve(
        HashMap::new(),
        &env(&[("a", "1")], &[]),
        &map(&[("a", "leak"), ("b", "leak")]),
    );
    assert_eq!(vars, map(&[("a", "1")]));
}
