use std::fs;
use std::path::Path;

use data::environment_dir::EnvironmentDir;
use domain::AppError;
use domain::environment::{EnvironmentStore, Scope};

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
