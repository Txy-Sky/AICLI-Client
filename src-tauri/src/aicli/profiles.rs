//! Local route profiles. Server catalog facts are never rewritten by compilation.
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::catalog::Catalog;
use crate::Database;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMode {
    FixedPriority,
    CostFirst,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelBinding {
    pub model: String,
    pub source_group: Option<String>,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileBody {
    pub selected_groups: Vec<String>,
    pub selection_mode: SelectionMode,
    pub model_bindings: Vec<ModelBinding>,
    pub resolved_conflicts: Vec<String>,
    pub line_policy: String,
    pub allow_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub source: String,
    pub preset_id: Option<String>,
    pub revision: i64,
    pub body: ProfileBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelRoute {
    pub public_model: String,
    pub upstream_model: String,
    pub ordered_groups: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct RepairStatus {
    pub conservative: bool,
    pub needs_repair: bool,
    pub missing_groups: Vec<String>,
    pub affected_models: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    #[error("Profile storage is unavailable")]
    Database,
    #[error("Profile data is invalid")]
    Invalid,
    #[error("System presets are read only")]
    ReadOnly,
    #[error("Profile revision has changed")]
    RevisionConflict,
    #[error("Profile references unavailable groups")]
    MissingGroups(Vec<String>),
    #[error("Model conflicts need confirmation")]
    UnresolvedConflicts(Vec<String>),
    #[error("Public model aliases must be unique")]
    DuplicateAlias,
    #[error("Model group count exceeds the server limit")]
    TooManyGroups {
        model: String,
        count: usize,
        max_count: usize,
    },
    #[error("Profile needs repair")]
    NeedsRepair(RepairStatus),
}

fn ratio_order(catalog: &Catalog, a: &str, b: &str) -> Ordering {
    let ratio = |name: &str| {
        catalog
            .groups
            .get(name)
            .and_then(|g| g.ratio.filter(|r| g.ratio_known && r.is_finite()))
    };
    match (ratio(a), ratio(b)) {
        (Some(a), Some(b)) => a.total_cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

pub fn preset(id: &str, catalog: &Catalog) -> Result<Profile, ProfileError> {
    let mut selected_groups = match id {
        "recommended" => catalog
            .auto_groups
            .iter()
            .filter(|g| catalog.groups.contains_key(*g))
            .cloned()
            .collect(),
        "low_cost" => catalog.groups.keys().cloned().collect::<Vec<_>>(),
        _ => return Err(ProfileError::Invalid),
    };
    if id == "low_cost" {
        selected_groups.sort_by(|a, b| ratio_order(catalog, a, b));
    }
    Ok(Profile {
        id: id.into(),
        name: id.into(),
        source: "system".into(),
        preset_id: Some(id.into()),
        revision: 0,
        body: ProfileBody {
            selected_groups,
            selection_mode: SelectionMode::FixedPriority,
            model_bindings: Vec::new(),
            resolved_conflicts: Vec::new(),
            line_policy: "inherit".into(),
            allow_fallback: true,
        },
    })
}

/// No explicit binding means the recommended preset, computed from the current snapshot.
pub fn effective_profile(
    profile: Option<&Profile>,
    catalog: &Catalog,
) -> Result<Profile, ProfileError> {
    profile
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| preset("recommended", catalog))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileBaseline {
    pub fingerprint: String,
    pub compiled_at: i64,
    pub routes: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompiledProfile {
    pub routes: Vec<ModelRoute>,
    pub repair: RepairStatus,
}

pub fn needs_repair(profile: &Profile, catalog: &Catalog) -> RepairStatus {
    repair_with_baseline(profile, catalog, None)
}

fn repair_with_baseline(
    profile: &Profile,
    catalog: &Catalog,
    baseline: Option<&ProfileBaseline>,
) -> RepairStatus {
    let missing: BTreeSet<_> = profile
        .body
        .selected_groups
        .iter()
        .chain(
            profile
                .body
                .model_bindings
                .iter()
                .filter_map(|b| b.source_group.as_ref()),
        )
        .filter(|g| !catalog.groups.contains_key(*g))
        .cloned()
        .collect();
    if missing.is_empty() {
        return RepairStatus::default();
    }
    let mut affected = BTreeSet::new();
    if let Some(baseline) = baseline {
        for (model, groups) in &baseline.routes {
            if groups.iter().any(|g| missing.contains(g)) {
                affected.insert(model.clone());
            }
        }
    } else {
        for (name, model) in &catalog.models {
            if model
                .groups
                .iter()
                .any(|g| profile.body.selected_groups.contains(g))
            {
                let bindings: Vec<_> = profile
                    .body
                    .model_bindings
                    .iter()
                    .filter(|b| b.model == *name)
                    .collect();
                if bindings.is_empty() {
                    affected.insert(name.clone());
                } else {
                    affected.extend(
                        bindings
                            .iter()
                            .map(|b| b.alias.as_ref().unwrap_or(&b.model).clone()),
                    );
                }
            }
        }
        affected.extend(
            profile
                .body
                .model_bindings
                .iter()
                .map(|b| b.alias.as_ref().unwrap_or(&b.model).clone()),
        );
    }
    for binding in &profile.body.model_bindings {
        if binding
            .source_group
            .as_ref()
            .is_some_and(|g| missing.contains(g))
        {
            affected.insert(binding.alias.as_ref().unwrap_or(&binding.model).clone());
        }
    }
    RepairStatus {
        conservative: baseline.is_none(),
        needs_repair: true,
        missing_groups: missing.into_iter().collect(),
        affected_models: affected.into_iter().collect(),
    }
}

fn ordered_groups(profile: &Profile, catalog: &Catalog) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut groups: Vec<_> = profile
        .body
        .selected_groups
        .iter()
        .filter(|g| seen.insert((*g).clone()))
        .cloned()
        .collect();
    if profile.body.selection_mode == SelectionMode::CostFirst {
        groups.sort_by(|a, b| ratio_order(catalog, a, b));
    }
    groups
}

pub fn validate(profile: &Profile, catalog: &Catalog) -> Result<(), ProfileError> {
    if profile.id.is_empty()
        || profile.name.trim().is_empty()
        || profile.revision < 0
        || (profile.body.line_policy != "inherit"
            && super::lines::line(&profile.body.line_policy).is_none())
    {
        return Err(ProfileError::Invalid);
    }
    let repair = needs_repair(profile, catalog);
    if repair.needs_repair {
        return Err(ProfileError::MissingGroups(repair.missing_groups));
    }
    let selected: BTreeSet<_> = profile.body.selected_groups.iter().collect();
    if selected.len() != profile.body.selected_groups.len() {
        return Err(ProfileError::Invalid);
    }
    let mut public = BTreeSet::new();
    let bound: BTreeSet<_> = profile
        .body
        .model_bindings
        .iter()
        .map(|b| b.model.as_str())
        .collect();
    for binding in &profile.body.model_bindings {
        let model = catalog
            .models
            .get(&binding.model)
            .ok_or(ProfileError::Invalid)?;
        if !model.groups.iter().any(|g| selected.contains(g))
            || binding
                .source_group
                .as_ref()
                .is_some_and(|g| !selected.contains(g) || !model.groups.contains(g))
        {
            return Err(ProfileError::Invalid);
        }
        let name = binding.alias.as_ref().unwrap_or(&binding.model);
        if name.trim().is_empty() {
            return Err(ProfileError::Invalid);
        }
        if !public.insert(name.clone()) {
            return Err(ProfileError::DuplicateAlias);
        }
    }
    let mut unresolved = Vec::new();
    for (name, model) in &catalog.models {
        let count = model.groups.iter().filter(|g| selected.contains(g)).count();
        if count == 0 {
            continue;
        }
        if !bound.contains(name.as_str()) && !public.insert(name.clone()) {
            return Err(ProfileError::DuplicateAlias);
        }
        if count > 1
            && !bound.contains(name.as_str())
            && !profile.body.resolved_conflicts.contains(name)
        {
            unresolved.push(name.clone());
        }
    }
    if !unresolved.is_empty() {
        return Err(ProfileError::UnresolvedConflicts(unresolved));
    }
    Ok(())
}

pub fn compile(
    profile: &Profile,
    catalog: &Catalog,
    max_count: usize,
) -> Result<Vec<ModelRoute>, ProfileError> {
    let repair = needs_repair(profile, catalog);
    if repair.needs_repair {
        return Err(ProfileError::NeedsRepair(repair));
    }
    // New catalog conflicts require confirmation only at save time. Runtime
    // compilation continues with the profile's existing ordered choices.
    compile_available(profile, catalog, max_count)
}

fn compile_available(
    profile: &Profile,
    catalog: &Catalog,
    max_count: usize,
) -> Result<Vec<ModelRoute>, ProfileError> {
    let groups = ordered_groups(profile, catalog);
    let mut routes = Vec::new();
    for (name, model) in &catalog.models {
        let candidates: Vec<_> = groups
            .iter()
            .filter(|g| model.groups.contains(g))
            .cloned()
            .collect();
        if candidates.is_empty() {
            continue;
        }
        let bindings: Vec<_> = profile
            .body
            .model_bindings
            .iter()
            .filter(|b| b.model == *name)
            .collect();
        let emit = |binding: Option<&ModelBinding>| -> Result<ModelRoute, ProfileError> {
            let mut ordered = candidates.clone();
            if let Some(source) = binding.and_then(|b| b.source_group.as_ref()) {
                if !candidates.contains(source) {
                    return Err(ProfileError::Invalid);
                }
                ordered = vec![source.clone()];
                if profile.body.allow_fallback {
                    ordered.extend(candidates.iter().filter(|g| *g != source).cloned());
                }
            }
            let public = binding
                .and_then(|b| b.alias.as_ref())
                .unwrap_or(name)
                .clone();
            if ordered.len() > max_count {
                return Err(ProfileError::TooManyGroups {
                    model: public,
                    count: ordered.len(),
                    max_count,
                });
            }
            Ok(ModelRoute {
                public_model: public,
                upstream_model: name.clone(),
                ordered_groups: ordered,
            })
        };
        if bindings.is_empty() {
            routes.push(emit(None)?);
        } else {
            for binding in bindings {
                routes.push(emit(Some(binding))?);
            }
        }
    }
    let mut public = BTreeSet::new();
    for route in &routes {
        if route.public_model.trim().is_empty() {
            return Err(ProfileError::Invalid);
        }
        if !public.insert(&route.public_model) {
            return Err(ProfileError::DuplicateAlias);
        }
    }
    Ok(routes)
}

fn conflicts(_profile: &Profile, catalog: &Catalog) -> Vec<String> {
    catalog
        .models
        .iter()
        .filter(|(_, model)| model.groups.len() > 1)
        .map(|(name, _)| name.clone())
        .collect()
}

pub struct ProfileStore {
    db: Arc<Database>,
}

impl ProfileStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub fn list(&self) -> Result<Vec<Profile>, ProfileError> {
        let conn = self.db.conn.lock().map_err(|_| ProfileError::Database)?;
        let mut statement = conn
            .prepare(
                "SELECT id,name,source,preset_id,revision,body_json FROM aicli_profile ORDER BY id",
            )
            .map_err(|_| ProfileError::Database)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(|_| ProfileError::Database)?;
        rows.map(|row| {
            let (id, name, source, preset_id, revision, body) =
                row.map_err(|_| ProfileError::Database)?;
            Ok(Profile {
                id,
                name,
                source,
                preset_id,
                revision,
                body: serde_json::from_str(&body).map_err(|_| ProfileError::Invalid)?,
            })
        })
        .collect()
    }

    pub fn copy_preset(
        &self,
        preset_id: &str,
        id: &str,
        name: &str,
        catalog: &Catalog,
    ) -> Result<Profile, ProfileError> {
        let mut profile = preset(preset_id, catalog)?;
        profile.id = id.into();
        profile.name = name.into();
        profile.source = "system_copy".into();
        // Copying is an explicit confirmation of the preset's ordered choices.
        profile.body.resolved_conflicts = conflicts(&profile, catalog);
        self.save(&profile, catalog)
    }

    pub fn reset_to_preset(
        &self,
        profile: &Profile,
        preset_id: &str,
        catalog: &Catalog,
    ) -> Result<Profile, ProfileError> {
        let mut reset = preset(preset_id, catalog)?;
        reset.id = profile.id.clone();
        reset.name = profile.name.clone();
        reset.revision = profile.revision;
        reset.source = "system_copy".into();
        reset.body.resolved_conflicts = conflicts(&reset, catalog);
        self.save(&reset, catalog)
    }

    pub fn baseline(&self, id: &str) -> Result<Option<ProfileBaseline>, ProfileError> {
        let conn = self.db.conn.lock().map_err(|_| ProfileError::Database)?;
        let body: Option<String> = conn
            .query_row(
                "SELECT body_json FROM aicli_cache WHERE kind=?1",
                [format!("profile_baseline:{id}")],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| ProfileError::Database)?;
        body.map(|body| serde_json::from_str(&body).map_err(|_| ProfileError::Invalid))
            .transpose()
    }

    pub fn compile(
        &self,
        profile: &Profile,
        catalog: &Catalog,
        max_count: usize,
    ) -> Result<CompiledProfile, ProfileError> {
        let baseline = self.baseline(&profile.id)?;
        let repair = repair_with_baseline(profile, catalog, baseline.as_ref());
        if repair.conservative {
            return Ok(CompiledProfile {
                routes: Vec::new(),
                repair,
            });
        }
        let mut available = profile.clone();
        if repair.needs_repair {
            available
                .body
                .selected_groups
                .retain(|g| catalog.groups.contains_key(g));
            available.body.model_bindings.retain(|b| {
                catalog.models.contains_key(&b.model)
                    && !b
                        .source_group
                        .as_ref()
                        .is_some_and(|g| repair.missing_groups.contains(g))
            });
        }
        let mut routes = compile_available(&available, catalog, max_count)?;
        routes.retain(|r| !repair.affected_models.contains(&r.public_model));
        if !repair.needs_repair {
            let compiled_at = chrono::Utc::now().timestamp();
            let baseline_routes: BTreeMap<_, _> = routes
                .iter()
                .map(|r| (r.public_model.clone(), r.ordered_groups.clone()))
                .collect();
            let route_json =
                serde_json::to_string(&baseline_routes).map_err(|_| ProfileError::Invalid)?;
            let fingerprint = format!("{:x}", Sha256::digest(route_json.as_bytes()));
            let snapshot = ProfileBaseline {
                fingerprint: fingerprint.clone(),
                compiled_at,
                routes: baseline_routes,
            };
            let body = serde_json::to_string(&snapshot).map_err(|_| ProfileError::Invalid)?;
            self.db.conn.lock().map_err(|_| ProfileError::Database)?.execute(
                "INSERT INTO aicli_cache(kind,fingerprint,fetched_at,body_json) VALUES (?1,?2,?3,?4) ON CONFLICT(kind) DO UPDATE SET fingerprint=excluded.fingerprint,fetched_at=excluded.fetched_at,body_json=excluded.body_json",
                params![format!("profile_baseline:{}",profile.id), fingerprint, compiled_at, body]).map_err(|_| ProfileError::Database)?;
        }
        Ok(CompiledProfile { routes, repair })
    }

    pub fn delete(&self, id: &str, revision: i64) -> Result<(), ProfileError> {
        if matches!(id, "recommended" | "low_cost") {
            return Err(ProfileError::ReadOnly);
        }
        let mut conn = self.db.conn.lock().map_err(|_| ProfileError::Database)?;
        let tx = conn.transaction().map_err(|_| ProfileError::Database)?;
        if tx
            .execute(
                "DELETE FROM aicli_profile WHERE id=?1 AND revision=?2",
                params![id, revision],
            )
            .map_err(|_| ProfileError::Database)?
            != 1
        {
            return Err(ProfileError::RevisionConflict);
        }
        tx.execute(
            "DELETE FROM aicli_cache WHERE kind=?1",
            [format!("profile_baseline:{id}")],
        )
        .map_err(|_| ProfileError::Database)?;
        tx.execute("DELETE FROM aicli_scope_binding WHERE profile_id=?1", [id])
            .map_err(|_| ProfileError::Database)?;
        tx.commit().map_err(|_| ProfileError::Database)
    }

    pub fn save(&self, profile: &Profile, catalog: &Catalog) -> Result<Profile, ProfileError> {
        if matches!(profile.id.as_str(), "recommended" | "low_cost") || profile.source == "system" {
            return Err(ProfileError::ReadOnly);
        }
        if !matches!(profile.source.as_str(), "user" | "system_copy") {
            return Err(ProfileError::Invalid);
        }
        validate(profile, catalog)?;
        let body = serde_json::to_string(&profile.body).map_err(|_| ProfileError::Invalid)?;
        let mut conn = self.db.conn.lock().map_err(|_| ProfileError::Database)?;
        let tx = conn.transaction().map_err(|_| ProfileError::Database)?;
        let revision: Option<i64> = tx
            .query_row(
                "SELECT revision FROM aicli_profile WHERE id=?1",
                [&profile.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| ProfileError::Database)?;
        if revision.unwrap_or(0) != profile.revision {
            return Err(ProfileError::RevisionConflict);
        }
        let next = profile
            .revision
            .checked_add(1)
            .ok_or(ProfileError::Invalid)?;
        let now = chrono::Utc::now().timestamp();
        tx.execute("INSERT INTO aicli_profile(id,name,source,preset_id,revision,body_json,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?7) ON CONFLICT(id) DO UPDATE SET name=excluded.name,source=excluded.source,preset_id=excluded.preset_id,revision=excluded.revision,body_json=excluded.body_json,updated_at=excluded.updated_at",
            params![profile.id,profile.name,profile.source,profile.preset_id,next,body,now]).map_err(|_| ProfileError::Database)?;
        tx.commit().map_err(|_| ProfileError::Database)?;
        let mut saved = profile.clone();
        saved.revision = next;
        Ok(saved)
    }
}

#[cfg(test)]
mod tests;
