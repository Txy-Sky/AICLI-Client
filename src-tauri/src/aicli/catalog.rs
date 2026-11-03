//! Deterministic directory projection of cached account permissions and server pricing.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{cache::AicliCache, newapi::NewApiError};

const PROTOCOLS: [&str; 4] = ["openai-response", "openai", "anthropic", "gemini"];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CatalogGroup {
    pub description: String,
    pub ratio: Option<f64>,
    pub ratio_known: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Compatibility {
    Full,
    Partial,
    TextOnly,
    Unavailable,
    #[default]
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CatalogModel {
    pub groups: Vec<String>,
    pub wire: BTreeMap<String, String>,
    pub compat: BTreeMap<String, Compatibility>,
    /// Original row, including endpoint, expression and future billing metadata.
    /// These are server facts, never a locally computed final charge.
    pub pricing: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Catalog {
    pub groups: BTreeMap<String, CatalogGroup>,
    pub models: BTreeMap<String, CatalogModel>,
    pub auto_available: bool,
    pub auto_groups: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Override {
    wire: Option<String>,
    compat: Option<Compatibility>,
}

impl Catalog {
    /// No network and no refresh side effects. Missing snapshots remain missing.
    pub fn from_cache(cache: &AicliCache) -> Result<Option<Self>, NewApiError> {
        let Some(groups) = cache.read("groups")? else {
            return Ok(None);
        };
        let Some(pricing) = cache.read("pricing")? else {
            return Ok(None);
        };
        Self::from_snapshots(&groups.value, &pricing.value).map(Some)
    }

    pub fn from_snapshots(groups: &Value, pricing: &Value) -> Result<Self, NewApiError> {
        Self::build(groups, pricing, include_str!("compat.json"))
    }

    fn build(groups: &Value, pricing: &Value, overrides: &str) -> Result<Self, NewApiError> {
        let invalid = || NewApiError::InvalidResponse;
        let available = groups["data"].as_object().ok_or_else(invalid)?;
        let mut result = Self {
            groups: BTreeMap::new(),
            models: BTreeMap::new(),
            auto_available: available.contains_key("auto"),
            auto_groups: Vec::new(),
        };
        for (name, item) in available {
            if name == "auto" {
                continue;
            }
            let ratio = item["ratio"].as_f64().filter(|ratio| ratio.is_finite());
            result.groups.insert(
                name.clone(),
                CatalogGroup {
                    description: item["desc"].as_str().unwrap_or_default().to_owned(),
                    ratio,
                    ratio_known: ratio.is_some(),
                },
            );
        }
        if let Some(auto) = pricing.get("auto_groups") {
            let mut seen = BTreeSet::new();
            for group in auto.as_array().ok_or_else(invalid)? {
                let group = group.as_str().ok_or_else(invalid)?;
                if result.groups.contains_key(group) && seen.insert(group) {
                    result.auto_groups.push(group.to_owned());
                }
            }
        }
        let overrides: BTreeMap<String, Override> =
            serde_json::from_str(overrides).map_err(|_| invalid())?;
        for (key, entry) in &overrides {
            let (protocol, model) = key.split_once('|').ok_or_else(invalid)?;
            if !PROTOCOLS.contains(&protocol)
                || model.is_empty()
                || entry
                    .wire
                    .as_deref()
                    .is_some_and(|wire| !PROTOCOLS.contains(&wire))
            {
                return Err(invalid());
            }
        }
        for row in pricing["data"].as_array().ok_or_else(invalid)? {
            let name = row["model_name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .ok_or_else(invalid)?;
            let mut groups = BTreeSet::new();
            for group in row["enable_groups"].as_array().ok_or_else(invalid)? {
                let group = group.as_str().ok_or_else(invalid)?;
                if result.groups.contains_key(group) {
                    groups.insert(group.to_owned());
                }
            }
            if groups.is_empty() {
                continue;
            }
            let endpoints = row["supported_endpoint_types"]
                .as_array()
                .ok_or_else(invalid)?
                .iter()
                .map(|endpoint| endpoint.as_str().ok_or_else(invalid))
                .collect::<Result<BTreeSet<_>, _>>()?;
            let fallback = PROTOCOLS
                .iter()
                .find(|protocol| endpoints.contains(**protocol));
            let mut model = CatalogModel {
                groups: groups.into_iter().collect(),
                wire: BTreeMap::new(),
                compat: BTreeMap::new(),
                pricing: row.clone(),
            };
            for protocol in PROTOCOLS {
                let entry = overrides.get(&format!("{protocol}|{name}"));
                let wire = entry.and_then(|entry| entry.wire.as_deref()).or_else(|| {
                    if endpoints.contains(protocol) {
                        Some(protocol)
                    } else {
                        fallback.copied()
                    }
                });
                if let Some(wire) = wire {
                    model.wire.insert(protocol.to_owned(), wire.to_owned());
                }
                model.compat.insert(
                    protocol.to_owned(),
                    entry.and_then(|entry| entry.compat).unwrap_or_default(),
                );
            }
            if result.models.insert(name.to_owned(), model).is_some() {
                return Err(invalid());
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
