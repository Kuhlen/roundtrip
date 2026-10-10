use std::fs;
use std::path::Path;

use data::environment_dir::EnvironmentDir;
use domain::AppError;
use domain::environment::{Environment, EnvironmentStore, Scope, Variable};

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

#[test]
fn lists_shared_and_personal_sorted_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(
        r,
        ".apiark/environments/staging.yaml",
        "name: staging\nvariables:\n  baseUrl: https://s\n",
    );
    write(
        r,
        ".apiark/environments/dev.yml",
        "name: dev\nvariables:\n  baseUrl: https://d\n",
    );
    write(r, ".apiark/environments.local/mine.yaml", "name: mine\n");
    write(r, ".apiark/environments/readme.txt", "not an env");
    let envs = EnvironmentDir.list(r).unwrap();
    let got: Vec<_> = envs.iter().map(|e| (e.name.as_str(), e.scope)).collect();
    assert_eq!(
        got,
        [
            ("dev", Scope::Shared),
            ("mine", Scope::Personal),
            ("staging", Scope::Shared)
        ]
    );
}

#[test]
fn no_environment_dirs_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    assert!(EnvironmentDir.list(dir.path()).unwrap().is_empty());
}

// parity: upstream parse_dotenv + get_resolved_variables
#[test]
fn resolve_merges_dotenv_files() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(
        r,
        ".env",
        "# comment\n\nhost = root\nquoted=\"a b\"\nsingle='c'\nlone=\"\nurl=http://x?a=b\n",
    );
    write(r, ".apiark/.env", "token=secret\nunlisted=no\n");
    write(
        r,
        ".apiark/environments/dev.yaml",
        "name: dev\nvariables:\n  host: env\nsecrets:\n  - token\n",
    );
    let vars = EnvironmentDir.resolve(r, "dev").unwrap();
    assert_eq!(vars["host"], "env");
    assert_eq!(vars["quoted"], "a b");
    assert_eq!(vars["single"], "c");
    assert_eq!(vars["lone"], "\"");
    assert_eq!(vars["url"], "http://x?a=b");
    assert_eq!(vars["token"], "secret");
    assert!(!vars.contains_key("unlisted"));
}

#[test]
fn numeric_variables_load_as_text() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        ".apiark/environments/dev.yaml",
        "name: dev\nvariables:\n  port: 3000\n  debug: true\n",
    );
    let vars = EnvironmentDir.resolve(dir.path(), "dev").unwrap();
    assert_eq!(vars["port"], "3000");
    assert_eq!(vars["debug"], "true");
}

#[test]
fn broken_environment_file_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), ".apiark/environments/dev.yaml", "name: [x\n");
    assert!(matches!(
        EnvironmentDir.list(dir.path()),
        Err(AppError::Storage(_))
    ));
}

#[test]
fn unknown_environment_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        EnvironmentDir.resolve(dir.path(), "nope"),
        Err(AppError::Storage(_))
    ));
}

fn var(key: &str, value: &str, secret: bool) -> Variable {
    Variable {
        key: key.into(),
        value: value.into(),
        secret,
    }
}

#[test]
fn list_fills_secret_values_from_dotenv() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, ".apiark/.env", "token=abc\n");
    write(
        r,
        ".apiark/environments/dev.yaml",
        "name: dev\nvariables:\n  host: h\n  token: yaml\nsecrets:\n  - token\n  - apiKey\n",
    );
    let envs = EnvironmentDir.list(r).unwrap();
    assert_eq!(
        envs[0].variables,
        [
            var("host", "h", false),
            var("token", "abc", true),
            var("apiKey", "", true)
        ]
    );
}

// parity: upstream keeps the YAML value when .apiark/.env lacks the key
#[test]
fn secret_missing_from_dotenv_keeps_yaml_value() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        ".apiark/environments/dev.yaml",
        "name: dev\nvariables:\n  token: yaml\nsecrets:\n  - token\n",
    );
    let vars = EnvironmentDir.resolve(dir.path(), "dev").unwrap();
    assert_eq!(vars["token"], "yaml");
}

fn env(name: &str, scope: Scope, vars: &[(&str, &str, bool)]) -> Environment {
    Environment {
        name: name.into(),
        scope,
        variables: vars.iter().map(|(k, v, s)| var(k, v, *s)).collect(),
    }
}

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).expect("read")
}

fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read_dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn edit_in_place_keeps_file_name_and_unknown_keys() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(
        r,
        ".apiark/environments/Imported-Env.yml",
        "name: Prod\ncolor: red\nvariables:\n  a: '1'\n",
    );
    let edited = env(
        "Prod",
        Scope::Shared,
        &[("a", "2", false), ("b", "3", false)],
    );
    EnvironmentDir
        .save(r, Some(("Prod", Scope::Shared)), &edited)
        .unwrap();
    assert_eq!(files(&r.join(".apiark/environments")), ["Imported-Env.yml"]);
    assert!(read(r, ".apiark/environments/Imported-Env.yml").contains("color: red"));
    assert_eq!(EnvironmentDir.list(r).unwrap(), [edited]);
}

#[test]
fn rename_leaves_one_file() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, ".apiark/environments/dev.yaml", "name: dev\n");
    EnvironmentDir
        .save(
            r,
            Some(("dev", Scope::Shared)),
            &env("staging", Scope::Shared, &[]),
        )
        .unwrap();
    assert_eq!(files(&r.join(".apiark/environments")), ["staging.yaml"]);
}

#[test]
fn rename_to_same_file_stem_rewrites_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, ".apiark/environments/my-env.yaml", "name: my env\n");
    EnvironmentDir
        .save(
            r,
            Some(("my env", Scope::Shared)),
            &env("My Env", Scope::Shared, &[]),
        )
        .unwrap();
    assert_eq!(files(&r.join(".apiark/environments")), ["my-env.yaml"]);
    assert_eq!(EnvironmentDir.list(r).unwrap()[0].name, "My Env");
}

#[test]
fn move_to_personal_moves_the_file_and_ignores_the_folder() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, ".apiark/environments/dev.yaml", "name: dev\n");
    EnvironmentDir
        .save(
            r,
            Some(("dev", Scope::Shared)),
            &env("dev", Scope::Personal, &[]),
        )
        .unwrap();
    assert!(files(&r.join(".apiark/environments")).is_empty());
    assert_eq!(
        files(&r.join(".apiark/environments.local")),
        [".gitignore", "dev.yaml"]
    );
    assert_eq!(
        read(r, ".apiark/environments.local/.gitignore"),
        "*\n!.gitignore\n"
    );
}

#[test]
fn create_on_a_taken_file_name_is_already_exists() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, ".apiark/environments/dev.yaml", "name: dev\n");
    assert_eq!(
        EnvironmentDir.save(r, None, &env("DEV", Scope::Shared, &[])),
        Err(AppError::AlreadyExists("DEV".into()))
    );
    assert_eq!(read(r, ".apiark/environments/dev.yaml"), "name: dev\n");
}

#[test]
fn variable_order_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    let created = env(
        "dev",
        Scope::Shared,
        &[("z", "1", false), ("a", "2", false), ("m", "3", false)],
    );
    EnvironmentDir.save(r, None, &created).unwrap();
    assert_eq!(EnvironmentDir.list(r).unwrap(), [created]);
}

#[test]
fn secret_value_goes_to_dotenv_only() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    let created = env(
        "dev",
        Scope::Shared,
        &[("host", "h", false), ("token", "abc", true)],
    );
    EnvironmentDir.save(r, None, &created).unwrap();
    assert!(!read(r, ".apiark/environments/dev.yaml").contains("abc"));
    assert_eq!(read(r, ".apiark/.env"), "token=abc\n");
    assert_eq!(read(r, ".apiark/.gitignore"), ".env\n");
    assert_eq!(EnvironmentDir.list(r).unwrap(), [created]);
}

#[test]
fn plain_save_writes_no_dotenv() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    EnvironmentDir
        .save(r, None, &env("dev", Scope::Shared, &[("a", "1", false)]))
        .unwrap();
    assert!(!r.join(".apiark/.env").exists());
    assert!(!r.join(".apiark/.gitignore").exists());
}

#[test]
fn dotenv_keeps_comments_and_foreign_lines() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, ".apiark/.gitignore", "node_modules\n.env\n");
    write(r, ".apiark/.env", "# keys\nother=1\ntoken=old\n");
    write(
        r,
        ".apiark/environments/dev.yaml",
        "name: dev\nsecrets:\n  - token\n",
    );
    EnvironmentDir
        .save(
            r,
            Some(("dev", Scope::Shared)),
            &env("dev", Scope::Shared, &[("token", "new", true)]),
        )
        .unwrap();
    assert_eq!(read(r, ".apiark/.env"), "# keys\nother=1\ntoken=new\n");
    assert_eq!(read(r, ".apiark/.gitignore"), "node_modules\n.env\n");
}

#[test]
fn shared_secret_survives_unsecreting_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, ".apiark/.env", "token=t\n");
    write(
        r,
        ".apiark/environments/dev.yaml",
        "name: dev\nsecrets:\n  - token\n",
    );
    write(
        r,
        ".apiark/environments/prod.yaml",
        "name: prod\nsecrets:\n  - token\n",
    );
    EnvironmentDir
        .save(
            r,
            Some(("dev", Scope::Shared)),
            &env("dev", Scope::Shared, &[("token", "plain", false)]),
        )
        .unwrap();
    assert_eq!(read(r, ".apiark/.env"), "token=t\n", "prod still uses it");
    EnvironmentDir.delete(r, "prod", Scope::Shared).unwrap();
    assert_eq!(read(r, ".apiark/.env"), "");
    assert_eq!(files(&r.join(".apiark/environments")), ["dev.yaml"]);
}

#[test]
fn dotenv_values_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    let values = ["a b", " lead", "\"q\"", "'s'", "#hash", "x=y", ""];
    let keys: Vec<String> = (0..values.len()).map(|i| format!("k{i}")).collect();
    let rows: Vec<(&str, &str, bool)> = keys
        .iter()
        .zip(values)
        .map(|(k, v)| (k.as_str(), v, true))
        .collect();
    let created = env("dev", Scope::Shared, &rows);
    EnvironmentDir.save(r, None, &created).unwrap();
    assert_eq!(EnvironmentDir.list(r).unwrap(), [created]);
}

#[test]
fn delete_unknown_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        EnvironmentDir.delete(dir.path(), "nope", Scope::Shared),
        Err(AppError::Storage(_))
    ));
}

#[test]
fn unsafe_secret_is_rejected_before_writing() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    let bad = env("dev", Scope::Shared, &[("token", "a\nb=c", true)]);
    assert!(matches!(
        EnvironmentDir.save(r, None, &bad),
        Err(AppError::Storage(_))
    ));
    assert!(!r.join(".apiark/environments/dev.yaml").exists());
    assert!(!r.join(".apiark/.env").exists());
}

#[test]
fn personal_save_in_place_adds_gitignore() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, ".apiark/environments.local/mine.yaml", "name: mine\n");
    EnvironmentDir
        .save(
            r,
            Some(("mine", Scope::Personal)),
            &env("mine", Scope::Personal, &[("a", "1", false)]),
        )
        .unwrap();
    assert_eq!(
        read(r, ".apiark/environments.local/.gitignore"),
        "*\n!.gitignore\n"
    );
}

#[test]
fn crlf_dotenv_keeps_its_line_endings() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, ".apiark/.env", "# c\r\ntoken=old\r\n");
    write(
        r,
        ".apiark/environments/dev.yaml",
        "name: dev\nsecrets:\n  - token\n",
    );
    EnvironmentDir
        .save(
            r,
            Some(("dev", Scope::Shared)),
            &env("dev", Scope::Shared, &[("token", "new", true)]),
        )
        .unwrap();
    assert_eq!(read(r, ".apiark/.env"), "# c\r\ntoken=new\r\n");
}

#[test]
fn in_place_edit_keeps_a_name_that_is_no_file_stem() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, ".apiark/environments/odd.yaml", "name: .hidden\n");
    EnvironmentDir
        .save(
            r,
            Some((".hidden", Scope::Shared)),
            &env(".hidden", Scope::Shared, &[("a", "1", false)]),
        )
        .unwrap();
    let envs = EnvironmentDir.list(r).unwrap();
    assert_eq!(envs[0].variables, [var("a", "1", false)]);
}
