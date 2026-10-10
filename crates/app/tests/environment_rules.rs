use app::modules::workspace::environments::environment_rules::{
    error_text, shared_secret_notes, unique_name,
};
use domain::AppError;
use domain::environment::{Environment, Scope, Variable};

fn env(name: &str, vars: &[(&str, bool)]) -> Environment {
    Environment {
        name: name.into(),
        scope: Scope::Shared,
        variables: vars
            .iter()
            .map(|(k, secret)| Variable {
                key: (*k).into(),
                value: String::new(),
                secret: *secret,
            })
            .collect(),
    }
}

#[test]
fn unique_name_numbers_taken_names() {
    assert_eq!(unique_name("dev copy", &["dev"]), "dev copy");
    assert_eq!(unique_name("dev copy", &["dev", "dev copy"]), "dev copy 2");
    assert_eq!(
        unique_name("dev copy", &["dev copy", "dev copy 2"]),
        "dev copy 3"
    );
}

#[test]
fn notes_name_other_environments_sharing_a_secret() {
    let others = [
        env("prod", &[("token", true)]),
        env("qa", &[("token", false)]),
        env("stage", &[("token", true), ("key", true)]),
    ];
    let notes = shared_secret_notes(&env("dev", &[("token", true), ("key", false)]), &others);
    assert_eq!(
        notes,
        ["\"token\" is also a secret in prod, stage: they share one value"]
    );
}

#[test]
fn error_text_per_kind() {
    assert_eq!(
        error_text(&AppError::InvalidName(String::new())),
        "Name can't be empty"
    );
    assert_eq!(
        error_text(&AppError::InvalidName("a/b".into())),
        "\"a/b\" can't be used as a file name"
    );
    assert_eq!(
        error_text(&AppError::DuplicateKey("a".into())),
        "Variable \"a\" appears twice"
    );
    assert_eq!(
        error_text(&AppError::AlreadyExists("dev".into())),
        "An environment named \"dev\" already exists"
    );
    assert_eq!(
        error_text(&AppError::Storage("disk full".into())),
        "storage error: disk full"
    );
}
