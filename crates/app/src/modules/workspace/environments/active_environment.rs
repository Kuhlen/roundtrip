//! Sidebar environment picker, its variables, the URL overlay and the variable popover.

use std::collections::HashMap;

use domain::AppError;
use domain::environment::{Environment, Scope, Variable};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::environment_rules::error_text;
use crate::modules::workspace::workspace_controller::WorkspaceController;
use crate::modules::workspace::workspace_rules::{self, PartKind};
use crate::ui::{SegmentKind, UrlSegment, VarMode, WorkspaceState};

impl WorkspaceController {
    pub(crate) fn load_environments(&self, preferred: Option<&str>) {
        let Some(root) = self.env_root() else { return };
        let envs = self.deps.environments.list(&root).unwrap_or_else(|e| {
            self.banner(&e);
            Vec::new()
        });
        let names: Vec<SharedString> = std::iter::once("No environment".into())
            .chain(envs.iter().map(|e| match e.scope {
                Scope::Shared => e.name.as_str().into(),
                Scope::Personal => format!("{} (personal)", e.name).into(),
            }))
            .collect();
        let index = preferred
            .and_then(|p| envs.iter().position(|e| e.name == p))
            .map_or(0, |i| i + 1);
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_environments(ModelRc::new(VecModel::from(names)));
        s.set_environment_index(index as i32);
        *self.envs.borrow_mut() = envs;
        self.refresh_vars();
    }

    fn with_active_env<T>(&self, f: impl FnOnce(&Environment) -> T) -> Option<T> {
        let index = self.ui().global::<WorkspaceState>().get_environment_index();
        let i = usize::try_from(index).ok()?.checked_sub(1)?;
        self.envs.borrow().get(i).map(f)
    }

    pub(crate) fn active_environment(&self) -> Option<String> {
        self.with_active_env(|e| e.name.clone())
    }

    pub(crate) fn environment_selected(&self) {
        self.refresh_vars();
        self.save_session();
        self.changed();
    }

    pub(crate) fn refresh_vars(&self) {
        let vars = match (self.env_root(), self.active_environment()) {
            (Some(root), Some(name)) => self
                .deps
                .environments
                .resolve(&root, &name)
                .unwrap_or_else(|e| {
                    self.banner(&e);
                    HashMap::new()
                }),
            _ => HashMap::new(),
        };
        *self.vars.borrow_mut() = vars;
    }

    pub(crate) fn lookup(&self, name: &str) -> Option<String> {
        self.vars
            .borrow()
            .get(name)
            .cloned()
            .or_else(|| (self.deps.dynamic_var)(name))
    }

    /// Secret keys of the picked environment; `envs` already carries the flags.
    pub(crate) fn active_secrets(&self) -> Vec<String> {
        self.with_active_env(|e| {
            e.variables
                .iter()
                .filter(|v| v.secret)
                .map(|v| v.key.clone())
                .collect()
        })
        .unwrap_or_default()
    }

    pub(crate) fn url_segments(&self, url: &str) -> ModelRc<UrlSegment> {
        let secrets = self.active_secrets();
        let parts = {
            let vars = self.vars.borrow();
            workspace_rules::url_segments(
                url,
                |n| vars.get(n).cloned(),
                |n| secrets.iter().any(|k| k == n),
                |n| (self.deps.dynamic_var)(n).is_some(),
            )
        };
        let rows: Vec<UrlSegment> = parts
            .into_iter()
            .map(|p| UrlSegment {
                text: p.text.into(),
                name: p.name.into(),
                kind: match p.kind {
                    PartKind::Text => SegmentKind::Text,
                    PartKind::Resolved => SegmentKind::Resolved,
                    PartKind::Unresolved => SegmentKind::Unresolved,
                    PartKind::Dynamic => SegmentKind::Dynamic,
                },
                tip: p.tip.into(),
            })
            .collect();
        ModelRc::new(VecModel::from(rows))
    }

    pub(crate) fn var_clicked(&self, name: SharedString) {
        let name = name.to_string();
        let value = self.vars.borrow().get(&name).cloned();
        let active = self.active_environment();
        let mode = if value.is_some() {
            if self.active_secrets().contains(&name) {
                VarMode::Secret
            } else {
                VarMode::Edit
            }
        } else if (self.deps.dynamic_var)(&name).is_some() {
            VarMode::Dynamic
        } else if self.env_root().is_none() {
            VarMode::NoCollection
        } else if active.is_none() {
            VarMode::NoEnvironment
        } else {
            VarMode::Edit
        };
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_var_name(name.into());
        s.set_var_value(value.unwrap_or_default().into());
        s.set_var_mode(mode);
        s.set_var_env(active.unwrap_or_default().into());
        s.set_var_error("".into());
        s.set_var_open(true);
    }

    /// Into the active environment: a root `.env` value gets an override.
    pub(crate) fn var_save(&self) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        if !matches!(s.get_var_mode(), VarMode::Edit | VarMode::Secret) {
            return;
        }
        let (Some(root), Some(active)) = (self.env_root(), self.active_environment()) else {
            return;
        };
        let name = s.get_var_name().to_string();
        let value = s.get_var_value().to_string();
        let result = self.deps.environments.list(&root).and_then(|envs| {
            let mut env = envs
                .into_iter()
                .find(|e| e.name == active)
                .ok_or_else(|| AppError::Storage(format!("environment {active} not found")))?;
            match env.variables.iter_mut().find(|v| v.key == name) {
                Some(v) => v.value = value,
                None => env.variables.push(Variable {
                    key: name,
                    value,
                    secret: false,
                }),
            }
            self.deps
                .environments
                .save(&root, Some((env.name.as_str(), env.scope)), &env)
        });
        match result {
            Ok(()) => {
                s.set_var_open(false);
                self.load_environments(Some(&active));
                self.changed();
            }
            Err(e) => s.set_var_error(error_text(&e).into()),
        }
    }

    pub(crate) fn var_close(&self) {
        self.ui().global::<WorkspaceState>().set_var_open(false);
    }
}
