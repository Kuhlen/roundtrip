//! Imported collection → new ApiArk folder. All or nothing: a failure removes the folder.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use domain::AppError;
use domain::collection::Protocol;
use domain::import::{ImportItem, ImportedCollection, ImportedRequest};
use serde_yaml::{Mapping, Value};

use crate::collection_dir::{
    clean_name, create_error, fill_doc, request_stem, set_auth, write_atomic,
};
use crate::storage;

pub(crate) fn write(parent: &Path, data: &ImportedCollection) -> Result<PathBuf, AppError> {
    let root = parent.join(clean_name(&data.name)?);
    // create_dir, not _all: an existing folder is never written into
    fs::create_dir(&root).map_err(|e| create_error(&root, e))?;
    if let Err(e) = fill(&root, data) {
        let _ = fs::remove_dir_all(&root);
        return Err(e);
    }
    Ok(root)
}

fn fill(root: &Path, data: &ImportedCollection) -> Result<(), AppError> {
    let apiark = root.join(".apiark");
    fs::create_dir(&apiark).map_err(|e| storage(&apiark, e))?;
    let mut config = Mapping::new();
    config.insert("name".into(), data.name.trim().into());
    config.insert("version".into(), 1.into());
    if data.auth.is_some() {
        let mut defaults = Mapping::new();
        set_auth(&mut defaults, data.auth.as_ref());
        config.insert("defaults".into(), Value::Mapping(defaults));
    }
    write_atomic(&apiark.join("apiark.yaml"), &config)?;
    let ignore = apiark.join(".gitignore");
    fs::write(&ignore, ".env\n").map_err(|e| storage(&ignore, e))?;
    if !data.environments.is_empty() {
        let dir = apiark.join("environments");
        fs::create_dir(&dir).map_err(|e| storage(&dir, e))?;
        let mut used = HashSet::new();
        for (name, vars) in &data.environments {
            let mut doc = Mapping::new();
            doc.insert("name".into(), name.as_str().into());
            let vars: Mapping = vars
                .iter()
                .map(|(k, v)| (Value::from(k.as_str()), Value::from(v.as_str())))
                .collect();
            doc.insert("variables".into(), Value::Mapping(vars));
            let stem = unique(
                &request_stem(name).unwrap_or_else(|_| "environment".into()),
                &mut used,
                '-',
            );
            write_atomic(&dir.join(format!("{stem}.yaml")), &doc)?;
        }
    }
    write_items(root, &data.items)
}

fn write_items(dir: &Path, items: &[ImportItem]) -> Result<(), AppError> {
    let mut used = HashSet::new();
    let mut order = Vec::new();
    for item in items {
        match item {
            ImportItem::Folder { name, items } => {
                let base = clean_name(name).unwrap_or_else(|_| "folder".into());
                let name = unique(&base, &mut used, ' ');
                let path = dir.join(&name);
                fs::create_dir(&path).map_err(|e| storage(&path, e))?;
                write_items(&path, items)?;
                order.push(name);
            }
            ImportItem::Request(r) => {
                let base = request_stem(&r.name).unwrap_or_else(|_| "request".into());
                let stem = unique(&base, &mut used, '-');
                write_atomic(&dir.join(format!("{stem}.yaml")), &request_doc(r))?;
                order.push(stem);
            }
        }
    }
    if !order.is_empty() {
        let mut doc = Mapping::new();
        let order: Vec<Value> = order.into_iter().map(Value::from).collect();
        doc.insert("order".into(), Value::Sequence(order));
        write_atomic(&dir.join("_folder.yaml"), &doc)?;
    }
    Ok(())
}

/// ApiArk key order: name, protocol, description, then what save_request owns, then scripts.
fn request_doc(r: &ImportedRequest) -> Mapping {
    let mut doc = Mapping::new();
    let name = if r.name.trim().is_empty() {
        "Untitled request"
    } else {
        r.name.as_str()
    };
    doc.insert("name".into(), name.into());
    if r.protocol == Protocol::Graphql {
        doc.insert("protocol".into(), "graphql".into());
    }
    if let Some(d) = &r.description {
        doc.insert("description".into(), d.as_str().into());
    }
    fill_doc(&mut doc, &r.request);
    if let Some(s) = &r.pre_request_script {
        doc.insert("preRequestScript".into(), s.as_str().into());
    }
    if let Some(t) = &r.tests {
        doc.insert("tests".into(), t.as_str().into());
    }
    doc
}

/// `base`, `base<sep>2`, …; case-insensitive so macOS and Windows disks do not clash.
fn unique(base: &str, used: &mut HashSet<String>, sep: char) -> String {
    let mut name = base.to_owned();
    let mut n = 2;
    while !used.insert(name.to_lowercase()) {
        name = format!("{base}{sep}{n}");
        n += 1;
    }
    name
}
