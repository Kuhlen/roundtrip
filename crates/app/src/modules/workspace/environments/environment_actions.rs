//! Environment picker in the sidebar and the environment editor dialog.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use domain::environment::{self, Environment, Scope, Variable};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::environment_rules::{error_text, shared_secret_notes, unique_name};
use crate::modules::workspace::request::{grow_rows, placeholder, remove_row};
use crate::modules::workspace::workspace_controller::{WorkspaceController, strings};
use crate::ui::{DialogKind, EnvListRow, KvRow, WorkspaceState};

/// Environment dialog while open.
pub(crate) struct EnvEditor {
    root: PathBuf,
    /// stored environments, `list` order
    saved: Vec<Environment>,
    selected: Selected,
    /// what the unsaved-changes answer continues with
    leave: Option<Leave>,
}

#[derive(Clone)]
enum Selected {
    None,
    Saved(usize),
    /// not on disk yet; shown last in the list
    Draft {
        env: Environment,
        /// `saved` index Cancel returns to; `saved` is fixed while a draft exists
        back: Option<usize>,
    },
}

enum Leave {
    // by name + scope: a Save before it can rename and re-sort
    Select(String, Scope),
    New,
    Duplicate,
    Close,
}

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

    pub(crate) fn active_environment(&self) -> Option<String> {
        let index = self.ui().global::<WorkspaceState>().get_environment_index();
        let i = usize::try_from(index).ok()?.checked_sub(1)?;
        self.envs.borrow().get(i).map(|e| e.name.clone())
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

    pub(crate) fn open_environments(&self) {
        let Some(root) = self.env_root() else { return };
        let saved = match self.deps.environments.list(&root) {
            Ok(saved) => saved,
            Err(e) => {
                self.banner(&e);
                return;
            }
        };
        let selected = self
            .active_environment()
            .and_then(|name| saved.iter().position(|e| e.name == name))
            .or((!saved.is_empty()).then_some(0))
            .map_or(Selected::None, Selected::Saved);
        let title = self
            .collections
            .borrow()
            .first()
            .map(|c| format!("Environments · {}", c.name))
            .unwrap_or_default();
        *self.env_editor.borrow_mut() = Some(EnvEditor {
            root,
            saved,
            selected,
            leave: None,
        });
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_envs_title(title.into());
        s.set_env_show_secrets(false);
        self.show_env_form();
        s.set_envs_open(true);
    }

    /// List + form from the editor state; unsaved edits are dropped.
    fn show_env_form(&self) {
        let (list, index, env) = {
            let editor = self.env_editor.borrow();
            let Some(ed) = editor.as_ref() else { return };
            let mut list: Vec<EnvListRow> = ed.saved.iter().map(|e| list_row(e, false)).collect();
            let (index, env) = match &ed.selected {
                Selected::None => (-1, None),
                Selected::Saved(i) => (*i as i32, ed.saved.get(*i).cloned()),
                Selected::Draft { env, .. } => {
                    list.push(list_row(env, true));
                    (list.len() as i32 - 1, Some(env.clone()))
                }
            };
            (list, index, env)
        };
        let env = env.unwrap_or_else(|| Environment {
            name: String::new(),
            scope: Scope::Shared,
            variables: Vec::new(),
        });
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_env_list(ModelRc::new(VecModel::from(list)));
        s.set_env_index(index);
        s.set_env_name(env.name.as_str().into());
        s.set_env_scope_index(i32::from(env.scope == Scope::Personal));
        self.env_rows.set_vec(
            env.variables
                .iter()
                .map(kv_row)
                .chain(std::iter::once(placeholder()))
                .collect::<Vec<_>>(),
        );
        s.set_env_error("".into());
        self.env_changed();
    }

    /// Form as typed; the placeholder and cleared rows are not variables.
    fn env_form(&self) -> Environment {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        Environment {
            name: s.get_env_name().to_string(),
            scope: if s.get_env_scope_index() == 1 {
                Scope::Personal
            } else {
                Scope::Shared
            },
            variables: self
                .env_rows
                .iter()
                .filter(|r| !r.key.is_empty() || !r.value.is_empty())
                .map(|r| Variable {
                    key: r.key.to_string(),
                    value: r.value.to_string(),
                    secret: r.secret,
                })
                .collect(),
        }
    }

    /// (stored copy of the selection, None for a draft; every other stored one); None = nothing selected.
    fn env_baseline(&self) -> Option<(Option<Environment>, Vec<Environment>)> {
        let editor = self.env_editor.borrow();
        let ed = editor.as_ref()?;
        match &ed.selected {
            Selected::None => None,
            Selected::Draft { .. } => Some((None, ed.saved.clone())),
            Selected::Saved(i) => {
                let mut others = ed.saved.clone();
                let saved = (*i < others.len()).then(|| others.remove(*i));
                Some((saved, others))
            }
        }
    }

    pub(crate) fn env_changed(&self) {
        let (dirty, notes) = match self.env_baseline() {
            None => (false, Vec::new()),
            Some((saved, others)) => {
                let form = self.env_form();
                (
                    saved.as_ref() != Some(&form),
                    shared_secret_notes(&form, &others),
                )
            }
        };
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_env_dirty(dirty);
        s.set_env_notes(strings(notes.iter().map(String::as_str)));
    }

    pub(crate) fn env_edited(&self, index: i32) {
        grow_rows(&self.env_rows, index);
        self.env_changed();
    }

    pub(crate) fn env_removed(&self, index: i32) {
        remove_row(&self.env_rows, index);
        self.env_changed();
    }

    pub(crate) fn env_selected(&self, index: i32) {
        if index == self.ui().global::<WorkspaceState>().get_env_index() {
            return;
        }
        // only stored rows: a draft row is always the selected one
        let name = self.env_editor.borrow().as_ref().and_then(|ed| {
            usize::try_from(index)
                .ok()
                .and_then(|i| ed.saved.get(i))
                .map(|e| (e.name.clone(), e.scope))
        });
        if let Some((name, scope)) = name {
            self.env_leave(Leave::Select(name, scope));
        }
    }

    pub(crate) fn env_new(&self) {
        self.env_leave(Leave::New);
    }

    pub(crate) fn env_duplicate(&self) {
        self.env_leave(Leave::Duplicate);
    }

    pub(crate) fn env_close(&self) {
        self.env_leave(Leave::Close);
    }

    /// Unsaved edits ask Save / Discard / Cancel first.
    fn env_leave(&self, leave: Leave) {
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        if !s.get_env_dirty() {
            self.env_go(leave);
            return;
        }
        if let Some(ed) = self.env_editor.borrow_mut().as_mut() {
            ed.leave = Some(leave);
        }
        s.set_dialog_kind(DialogKind::EnvironmentUnsaved);
        s.set_confirm_name(s.get_env_name());
        s.set_confirm_open(true);
    }

    fn env_go(&self, leave: Leave) {
        if let Leave::Close = leave {
            *self.env_editor.borrow_mut() = None;
            self.ui().global::<WorkspaceState>().set_envs_open(false);
            return;
        }
        if let Some(ed) = self.env_editor.borrow_mut().as_mut() {
            let taken: Vec<&str> = ed.saved.iter().map(|e| e.name.as_str()).collect();
            let back = match &ed.selected {
                Selected::None => None,
                Selected::Saved(i) => Some(*i),
                Selected::Draft { back, .. } => *back,
            };
            let next = match leave {
                Leave::Select(name, scope) => ed
                    .saved
                    .iter()
                    .position(|e| e.name == name && e.scope == scope)
                    .map_or(Selected::None, Selected::Saved),
                Leave::New => Selected::Draft {
                    env: Environment {
                        name: unique_name("New environment", &taken),
                        scope: Scope::Shared,
                        variables: Vec::new(),
                    },
                    back,
                },
                Leave::Duplicate => match &ed.selected {
                    Selected::Saved(i) => {
                        ed.saved
                            .get(*i)
                            .map_or(Selected::None, |e| Selected::Draft {
                                env: Environment {
                                    name: unique_name(&format!("{} copy", e.name), &taken),
                                    ..e.clone()
                                },
                                back,
                            })
                    }
                    other => other.clone(),
                },
                Leave::Close => Selected::None,
            };
            ed.selected = next;
        }
        self.show_env_form();
    }

    pub(crate) fn env_cancel(&self) {
        if let Some(ed) = self.env_editor.borrow_mut().as_mut()
            && let Selected::Draft { back, .. } = ed.selected
        {
            ed.selected = back.map_or(Selected::None, Selected::Saved);
        }
        self.show_env_form();
    }

    /// false when not written; the dialog shows why.
    pub(crate) fn env_save(&self) -> bool {
        let Some((saved, others)) = self.env_baseline() else {
            return false;
        };
        let Some(root) = self.env_editor.borrow().as_ref().map(|ed| ed.root.clone()) else {
            return false;
        };
        let result = environment::validate(self.env_form(), &others).and_then(|env| {
            let old = saved.as_ref().map(|e| (e.name.as_str(), e.scope));
            self.deps.environments.save(&root, old, &env).map(|()| env)
        });
        let env = match result {
            Ok(env) => env,
            Err(e) => {
                self.ui()
                    .global::<WorkspaceState>()
                    .set_env_error(error_text(&e).into());
                return false;
            }
        };
        // the active one renamed: the picker follows it
        let active = match (self.active_environment(), &saved) {
            (Some(a), Some(old)) if a == old.name => Some(env.name.clone()),
            (active, _) => active,
        };
        self.env_reload(&root, Some(&env.name), 0, active.as_deref());
        true
    }

    pub(crate) fn env_delete(&self) {
        let Some((Some(env), _)) = self.env_baseline() else {
            return;
        };
        let ui = self.ui();
        let s = ui.global::<WorkspaceState>();
        s.set_dialog_kind(DialogKind::EnvironmentDelete);
        s.set_confirm_name(env.name.as_str().into());
        s.set_confirm_open(true);
    }

    pub(crate) fn confirm_env_delete(&self) {
        self.close_dialog();
        let Some((Some(env), _)) = self.env_baseline() else {
            return;
        };
        let Some((root, index)) = self.env_editor.borrow().as_ref().map(|ed| {
            let index = match ed.selected {
                Selected::Saved(i) => i,
                _ => 0,
            };
            (ed.root.clone(), index)
        }) else {
            return;
        };
        let result = self.deps.environments.delete(&root, &env.name, env.scope);
        // reload even on error: the file may be gone already (.env sync failed)
        let active = self.active_environment().filter(|a| *a != env.name);
        self.env_reload(&root, None, index, active.as_deref());
        if let Err(e) = result {
            self.ui()
                .global::<WorkspaceState>()
                .set_env_error(error_text(&e).into());
        }
    }

    pub(crate) fn confirm_env_save(&self) {
        self.close_dialog();
        let leave = self
            .env_editor
            .borrow_mut()
            .as_mut()
            .and_then(|ed| ed.leave.take());
        if self.env_save()
            && let Some(leave) = leave
        {
            self.env_go(leave);
        }
    }

    pub(crate) fn confirm_env_discard(&self) {
        self.close_dialog();
        let leave = self
            .env_editor
            .borrow_mut()
            .as_mut()
            .and_then(|ed| ed.leave.take());
        if let Some(leave) = leave {
            self.env_go(leave);
        }
    }

    /// After a write: dialog list and sidebar picker from disk; variables and resolved URL follow.
    fn env_reload(&self, root: &Path, select: Option<&str>, fallback: usize, active: Option<&str>) {
        let fresh = self.deps.environments.list(root);
        if let (Ok(saved), Some(ed)) = (&fresh, self.env_editor.borrow_mut().as_mut()) {
            let at = select
                .and_then(|name| saved.iter().position(|e| e.name == name))
                .or_else(|| saved.len().checked_sub(1).map(|last| fallback.min(last)));
            ed.selected = at.map_or(Selected::None, Selected::Saved);
            ed.saved = saved.clone();
        }
        self.show_env_form();
        if let Err(e) = &fresh {
            self.ui()
                .global::<WorkspaceState>()
                .set_env_error(error_text(e).into());
        }
        self.load_environments(active);
        self.save_session();
        self.changed();
    }
}

fn list_row(env: &Environment, draft: bool) -> EnvListRow {
    EnvListRow {
        name: env.name.as_str().into(),
        personal: env.scope == Scope::Personal,
        draft,
    }
}

fn kv_row(v: &Variable) -> KvRow {
    KvRow {
        enabled: true,
        key: v.key.as_str().into(),
        value: v.value.as_str().into(),
        file: false,
        secret: v.secret,
    }
}
