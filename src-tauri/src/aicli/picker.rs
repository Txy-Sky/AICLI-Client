//! What a taken-over client's model picker lists for the route profile (Claude Code's
//! `/model`, Codex's model catalog): the order and the line under each name.
//!
//! Order (owner, 2026-11-13): grouped by vendor family (Claude, GPT, Gemini, Grok, then
//! DeepSeek, Qwen, GLM, Kimi, MiniMax, Doubao, then the rest), by name within a family.
//! Line: the first group the route takes and its ratio, how many groups it can fall back
//! to, and the model's capability tags from the catalog, e.g. "首选 vip ×1 · 还可走 2 个分组 · tools".
use super::catalog::Catalog;
use super::profiles::ModelRoute;

/// One row of a client's model picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerModel {
    /// The route profile's public model name.
    pub public: String,
    pub description: String,
}

/// Vendor family rank: lower comes first. Matched on the model name, case-insensitively.
pub(crate) fn vendor_rank(model: &str) -> u8 {
    let m = model.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| m.contains(needle));
    let o_series = m.starts_with('o') && m[1..].starts_with(|c: char| c.is_ascii_digit());
    if has(&["claude", "anthropic"]) {
        0
    } else if has(&["gpt", "openai", "codex"]) || o_series {
        1
    } else if has(&["gemini", "gemma"]) {
        2
    } else if has(&["grok"]) {
        3
    } else if has(&["deepseek"]) {
        4
    } else if has(&["qwen", "qwq"]) {
        5
    } else if has(&["glm", "zhipu"]) {
        6
    } else if has(&["kimi", "moonshot"]) {
        7
    } else if has(&["minimax", "abab"]) {
        8
    } else if has(&["doubao", "seed-"]) {
        9
    } else {
        10
    }
}

/// Sorts public model names by vendor family, then by name.
pub(crate) fn sort_models(models: &mut [String]) {
    models.sort_by(|a, b| {
        vendor_rank(a)
            .cmp(&vendor_rank(b))
            .then_with(|| a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase()))
            .then_with(|| a.cmp(b))
    });
}

struct Words {
    first: &'static str,
    more: fn(usize) -> String,
}

fn words(language: &str) -> Words {
    match language {
        "zh" | "zh-CN" | "zh-SG" | "zh-Hans" | "zh-Hans-CN" => Words {
            first: "首选",
            more: |n| format!("还可走 {n} 个分组"),
        },
        "zh-TW" | "zh-HK" | "zh-Hant" => Words {
            first: "首選",
            more: |n| format!("還可走 {n} 個分組"),
        },
        "ja" => Words {
            first: "優先",
            more: |n| format!("ほか {n} グループ"),
        },
        "tr" => Words {
            first: "Önce",
            more: |n| format!("+{n} grup daha"),
        },
        _ => Words {
            first: "First",
            more: |n| {
                if n == 1 {
                    "+1 more group".to_string()
                } else {
                    format!("+{n} more groups")
                }
            },
        },
    }
}

/// A ratio as the client page shows it: `×0.5`, `×1`, `×1.25`.
fn ratio_text(ratio: f64) -> String {
    let text = format!("{ratio:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("×{text}")
}

/// The line under a model: first group and ratio, fallback groups, capability tags.
pub(crate) fn describe(route: &ModelRoute, catalog: &Catalog, language: &str) -> String {
    let w = words(language);
    let mut parts = Vec::new();
    if let Some(first) = route.ordered_groups.first() {
        let ratio = catalog
            .groups
            .get(first)
            .filter(|group| group.ratio_known)
            .and_then(|group| group.ratio)
            .filter(|ratio| ratio.is_finite());
        parts.push(match ratio {
            Some(ratio) => format!("{} {first} {}", w.first, ratio_text(ratio)),
            None => format!("{} {first}", w.first),
        });
    }
    let more = route.ordered_groups.len().saturating_sub(1);
    if more > 0 {
        parts.push((w.more)(more));
    }
    if let Some(tags) = catalog
        .models
        .get(&route.upstream_model)
        .and_then(|model| model.pricing.get("tags"))
        .and_then(|tags| tags.as_str())
    {
        parts.extend(
            tags.split(',')
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(str::to_string),
        );
    }
    if parts.is_empty() {
        "AICLI".to_string()
    } else {
        parts.join(" · ")
    }
}

/// Picker rows for `models` (the client's routable public names): vendor order, and a
/// description from the profile's route for each.
pub(crate) fn picker_models(
    models: &[String],
    routes: &[ModelRoute],
    catalog: &Catalog,
    language: &str,
) -> Vec<PickerModel> {
    let mut ordered = models.to_vec();
    sort_models(&mut ordered);
    ordered
        .into_iter()
        .map(|public| {
            let description = routes
                .iter()
                .find(|route| route.public_model == public)
                .map(|route| describe(route, catalog, language))
                .unwrap_or_else(|| "AICLI".to_string());
            PickerModel {
                public,
                description,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aicli::catalog::{CatalogGroup, CatalogModel};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn catalog() -> Catalog {
        let group = |ratio: Option<f64>| CatalogGroup {
            description: String::new(),
            ratio,
            ratio_known: ratio.is_some(),
        };
        let model = |tags: &str| CatalogModel {
            groups: vec!["vip".into()],
            wire: BTreeMap::new(),
            compat: BTreeMap::new(),
            pricing: json!({ "tags": tags }),
        };
        Catalog {
            groups: BTreeMap::from([
                ("vip".to_string(), group(Some(1.0))),
                ("g-cheap".to_string(), group(Some(0.5))),
                ("odd".to_string(), group(None)),
            ]),
            models: BTreeMap::from([
                ("claude-sonnet-5-5".to_string(), model("tools,vision")),
                ("deepseek-v4".to_string(), model("tools")),
                ("plain".to_string(), model("")),
            ]),
            auto_available: false,
            auto_groups: Vec::new(),
        }
    }

    fn route(public: &str, groups: &[&str]) -> ModelRoute {
        ModelRoute {
            public_model: public.into(),
            upstream_model: public.into(),
            ordered_groups: groups.iter().map(|g| g.to_string()).collect(),
        }
    }

    #[test]
    fn models_are_grouped_by_vendor_then_name() {
        let mut models: Vec<String> = [
            "kimi-k3",
            "deepseek-v4",
            "gpt-6.1-sol",
            "my-own-model",
            "claude-sonnet-5-5",
            "glm-5.3",
            "gemini-4-argon",
            "claude-opus-5-5",
            "o4-mini",
            "qwen3.8-max",
            "grok-4.5",
        ]
        .iter()
        .map(|m| m.to_string())
        .collect();
        sort_models(&mut models);
        assert_eq!(
            models,
            [
                "claude-opus-5-5",
                "claude-sonnet-5-5",
                "gpt-6.1-sol",
                "o4-mini",
                "gemini-4-argon",
                "grok-4.5",
                "deepseek-v4",
                "qwen3.8-max",
                "glm-5.3",
                "kimi-k3",
                "my-own-model",
            ]
        );
    }

    #[test]
    fn description_names_the_first_group_ratio_fallbacks_and_tags() {
        let c = catalog();
        assert_eq!(
            describe(
                &route("claude-sonnet-5-5", &["vip", "g-cheap", "odd"]),
                &c,
                "zh"
            ),
            "首选 vip ×1 · 还可走 2 个分组 · tools · vision"
        );
        assert_eq!(
            describe(&route("deepseek-v4", &["g-cheap"]), &c, "zh"),
            "首选 g-cheap ×0.5 · tools"
        );
        // 倍率未知不写倍率；没有标签就不写标签
        assert_eq!(describe(&route("plain", &["odd"]), &c, "zh"), "首选 odd");
        assert_eq!(
            describe(&route("deepseek-v4", &["g-cheap", "vip"]), &c, "en"),
            "First g-cheap ×0.5 · +1 more group · tools"
        );
        assert_eq!(
            describe(&route("deepseek-v4", &["g-cheap", "vip"]), &c, "zh-TW"),
            "首選 g-cheap ×0.5 · 還可走 1 個分組 · tools"
        );
        // 没有分组也没有标签时仍有一行说明
        assert_eq!(describe(&route("plain", &[]), &c, "zh"), "AICLI");
    }

    #[test]
    fn picker_rows_follow_vendor_order_with_their_descriptions() {
        let c = catalog();
        let routes = vec![
            route("deepseek-v4", &["g-cheap"]),
            route("claude-sonnet-5-5", &["vip"]),
        ];
        let rows = picker_models(
            &["deepseek-v4".into(), "claude-sonnet-5-5".into()],
            &routes,
            &c,
            "zh",
        );
        assert_eq!(
            rows,
            vec![
                PickerModel {
                    public: "claude-sonnet-5-5".into(),
                    description: "首选 vip ×1 · tools · vision".into(),
                },
                PickerModel {
                    public: "deepseek-v4".into(),
                    description: "首选 g-cheap ×0.5 · tools".into(),
                },
            ]
        );
    }
}
