//! What a taken-over client's model picker lists for the route profile (Claude Code's
//! `/model`, Codex's model catalog): which models, in what order, and the line under each.
//!
//! Owner decisions (2026-11-13/14):
//! - Order: recommended models first (the catalog tag 推荐, set in new-api's admin), then the
//!   rest, then image and video models; within each, by vendor family (Claude, GPT, Gemini,
//!   Grok, DeepSeek, Qwen, GLM, Kimi, MiniMax, Doubao, the rest), then by name.
//! - Line: "⭐ 推荐" for a recommended model; the first group the route takes and its ratio,
//!   how many groups it can fall back to, and the catalog's tags. An image or video model
//!   says plainly that it is not a text model and can't be the main chat model.
//! - Codex's own `codex-auto-*` models (`codex-auto-review` runs its auto-review) are kept
//!   in Codex's catalog but hidden: any listed `codex-auto-*` model turns Codex's `/model`
//!   into "quick auto modes + All models", which hid the whole profile behind All models.
//!   Claude Code doesn't list them.
use super::catalog::Catalog;
use super::profiles::ModelRoute;

/// One row of a client's model picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerModel {
    /// The route profile's public model name.
    pub public: String,
    pub description: String,
    /// Kept in the list for the client to use, never shown in its picker (Codex's
    /// `codex-auto-*`). Claude Code leaves such a row out.
    pub hidden: bool,
}

/// What kind of row a model is, in list order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    Recommended,
    Text,
    Image,
    Video,
    /// Codex's own auto models: hidden.
    CodexAuto,
}

/// The catalog tag that marks a recommended model (set in new-api's admin).
const RECOMMENDED_TAGS: &[&str] = &["推荐", "recommended"];
const IMAGE_TAGS: &[&str] = &["生图", "图像生成", "image", "image-generation"];
const VIDEO_TAGS: &[&str] = &["视频", "生视频", "视频生成", "video", "video-generation"];
/// Names that are image or video models even without a tag.
const IMAGE_NAMES: &[&str] = &["gpt-image", "dall-e", "imagen", "flux", "-image"];
const VIDEO_NAMES: &[&str] = &["video", "seedance", "sora", "veo-", "kling"];

pub(crate) fn is_codex_auto(model: &str) -> bool {
    model.starts_with("codex-auto-")
}

fn tags_of<'a>(model: &str, catalog: &'a Catalog) -> Vec<&'a str> {
    catalog
        .models
        .get(model)
        .and_then(|entry| entry.pricing.get("tags"))
        .and_then(|tags| tags.as_str())
        .map(|tags| {
            tags.split(',')
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn has_tag(tags: &[&str], wanted: &[&str]) -> bool {
    tags.iter()
        .any(|tag| wanted.iter().any(|w| tag.eq_ignore_ascii_case(w)))
}

/// The row kind of `model` (its upstream name, which the catalog keys).
pub(crate) fn kind(model: &str, catalog: &Catalog) -> Kind {
    if is_codex_auto(model) {
        return Kind::CodexAuto;
    }
    let tags = tags_of(model, catalog);
    let name = model.to_ascii_lowercase();
    let named = |needles: &[&str]| needles.iter().any(|needle| name.contains(needle));
    if has_tag(&tags, VIDEO_TAGS) {
        Kind::Video
    } else if has_tag(&tags, IMAGE_TAGS) {
        Kind::Image
    } else if named(VIDEO_NAMES) {
        Kind::Video
    } else if named(IMAGE_NAMES) {
        Kind::Image
    } else if has_tag(&tags, RECOMMENDED_TAGS) {
        Kind::Recommended
    } else {
        Kind::Text
    }
}

/// Vendor family rank: lower comes first. Matched on the model name, case-insensitively.
pub(crate) fn vendor_rank(model: &str) -> u8 {
    let m = model.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| m.contains(needle));
    let o_series = m.starts_with('o') && m[1..].starts_with(|c: char| c.is_ascii_digit());
    if has(&["claude", "anthropic"]) {
        0
    } else if has(&["gpt", "openai", "codex", "dall-e"]) || o_series {
        1
    } else if has(&["gemini", "gemma", "imagen", "veo-"]) {
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
    } else if has(&["doubao", "seed"]) {
        9
    } else {
        10
    }
}

/// The list order of `model`: kind, vendor family, then name.
pub(crate) fn sort_key(model: &str, catalog: &Catalog) -> (Kind, u8, String, String) {
    (
        kind(model, catalog),
        vendor_rank(model),
        model.to_ascii_lowercase(),
        model.to_string(),
    )
}

struct Words {
    recommended: &'static str,
    first: &'static str,
    more: fn(usize) -> String,
    image: &'static str,
    video: &'static str,
    not_text: &'static str,
    codex_auto: &'static str,
}

fn words(language: &str) -> Words {
    match language {
        "zh" | "zh-CN" | "zh-SG" | "zh-Hans" | "zh-Hans-CN" => Words {
            recommended: "⭐ 推荐",
            first: "首选",
            more: |n| format!("还可走 {n} 个分组"),
            image: "生图模型",
            video: "视频模型",
            not_text: "不是文本模型，不能当主聊天模型",
            codex_auto: "Codex 自动审批用",
        },
        "zh-TW" | "zh-HK" | "zh-Hant" => Words {
            recommended: "⭐ 推薦",
            first: "首選",
            more: |n| format!("還可走 {n} 個分組"),
            image: "生圖模型",
            video: "影片模型",
            not_text: "不是文字模型，不能當主聊天模型",
            codex_auto: "Codex 自動審批用",
        },
        "ja" => Words {
            recommended: "⭐ おすすめ",
            first: "優先",
            more: |n| format!("ほか {n} グループ"),
            image: "画像生成モデル",
            video: "動画生成モデル",
            not_text: "テキストモデルではないため、メインのチャットには使えません",
            codex_auto: "Codex の自動承認用",
        },
        "tr" => Words {
            recommended: "⭐ Önerilen",
            first: "Önce",
            more: |n| format!("+{n} grup daha"),
            image: "Görsel modeli",
            video: "Video modeli",
            not_text: "metin modeli değil, ana sohbet modeli olarak kullanılamaz",
            codex_auto: "Codex otomatik onayı için",
        },
        _ => Words {
            recommended: "⭐ Recommended",
            first: "First",
            more: |n| {
                if n == 1 {
                    "+1 more group".to_string()
                } else {
                    format!("+{n} more groups")
                }
            },
            image: "Image model",
            video: "Video model",
            not_text: "not a text model, can't be the main chat model",
            codex_auto: "For Codex's auto-review",
        },
    }
}

/// A ratio as the client page shows it: `×0.5`, `×1`, `×1.25`.
fn ratio_text(ratio: f64) -> String {
    let text = format!("{ratio:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("×{text}")
}

/// The line under a model.
pub(crate) fn describe(route: &ModelRoute, catalog: &Catalog, language: &str) -> String {
    let w = words(language);
    let kind = kind(&route.upstream_model, catalog);
    match kind {
        Kind::CodexAuto => return w.codex_auto.to_string(),
        Kind::Image => return format!("{} · {}", w.image, w.not_text),
        Kind::Video => return format!("{} · {}", w.video, w.not_text),
        Kind::Recommended | Kind::Text => {}
    }
    let mut parts = Vec::new();
    if kind == Kind::Recommended {
        parts.push(w.recommended.to_string());
    }
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
    parts.extend(
        tags_of(&route.upstream_model, catalog)
            .into_iter()
            .filter(|tag| !has_tag(&[tag], RECOMMENDED_TAGS))
            .map(str::to_string),
    );
    if parts.is_empty() {
        "AICLI".to_string()
    } else {
        parts.join(" · ")
    }
}

/// Picker rows for `models` (the client's routable public names), in list order, each
/// described from the profile's route.
pub(crate) fn picker_models(
    models: &[String],
    routes: &[ModelRoute],
    catalog: &Catalog,
    language: &str,
) -> Vec<PickerModel> {
    let upstream = |public: &str| {
        routes
            .iter()
            .find(|route| route.public_model == public)
            .map(|route| route.upstream_model.clone())
            .unwrap_or_else(|| public.to_string())
    };
    let mut ordered = models.to_vec();
    ordered.sort_by_cached_key(|public| sort_key(&upstream(public), catalog));
    ordered
        .into_iter()
        .map(|public| {
            let route = routes
                .iter()
                .find(|route| route.public_model == public)
                .cloned()
                .unwrap_or_else(|| ModelRoute {
                    public_model: public.clone(),
                    upstream_model: public.clone(),
                    ordered_groups: Vec::new(),
                });
            PickerModel {
                description: describe(&route, catalog, language),
                hidden: is_codex_auto(&route.upstream_model) || is_codex_auto(&public),
                public,
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
                ("claude-sonnet-5-5".to_string(), model("推荐,tools")),
                ("claude-opus-5-5".to_string(), model("")),
                ("deepseek-v4-pro".to_string(), model("推荐")),
                ("gpt-6.1-sol".to_string(), model("")),
                ("kimi-k3".to_string(), model("")),
                ("plain".to_string(), model("")),
                ("gpt-image-2".to_string(), model("生图")),
                ("gemini-3-pro-image-preview".to_string(), model("")),
                ("seedance-2.0-0826-720p".to_string(), model("")),
                ("codex-auto-review".to_string(), model("")),
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
    fn kinds_come_from_tags_first_then_names() {
        let c = catalog();
        assert_eq!(kind("claude-sonnet-5-5", &c), Kind::Recommended);
        assert_eq!(kind("claude-opus-5-5", &c), Kind::Text);
        assert_eq!(kind("gpt-image-2", &c), Kind::Image);
        // 没打标签也按名字认出生图、视频模型
        assert_eq!(kind("gemini-3-pro-image-preview", &c), Kind::Image);
        assert_eq!(kind("seedance-2.0-0826-720p", &c), Kind::Video);
        assert_eq!(kind("grok-imagine-video", &c), Kind::Video);
        assert_eq!(kind("codex-auto-review", &c), Kind::CodexAuto);
    }

    #[test]
    fn recommended_first_media_last_vendor_then_name_within() {
        let c = catalog();
        let models: Vec<String> = [
            "seedance-2.0-0826-720p",
            "kimi-k3",
            "gpt-image-2",
            "codex-auto-review",
            "deepseek-v4-pro",
            "gpt-6.1-sol",
            "claude-opus-5-5",
            "gemini-3-pro-image-preview",
            "claude-sonnet-5-5",
            "plain",
        ]
        .iter()
        .map(|m| m.to_string())
        .collect();
        let routes: Vec<ModelRoute> = models.iter().map(|m| route(m, &["vip"])).collect();
        let rows = picker_models(&models, &routes, &c, "zh");
        assert_eq!(
            rows.iter().map(|r| r.public.as_str()).collect::<Vec<_>>(),
            [
                "claude-sonnet-5-5",
                "deepseek-v4-pro",
                "claude-opus-5-5",
                "gpt-6.1-sol",
                "kimi-k3",
                "plain",
                "gpt-image-2",
                "gemini-3-pro-image-preview",
                "seedance-2.0-0826-720p",
                "codex-auto-review",
            ]
        );
        assert_eq!(
            rows.iter()
                .filter(|r| r.hidden)
                .map(|r| r.public.as_str())
                .collect::<Vec<_>>(),
            ["codex-auto-review"]
        );
    }

    #[test]
    fn descriptions_say_recommended_groups_and_media_warnings() {
        let c = catalog();
        assert_eq!(
            describe(
                &route("claude-sonnet-5-5", &["vip", "g-cheap", "odd"]),
                &c,
                "zh"
            ),
            "⭐ 推荐 · 首选 vip ×1 · 还可走 2 个分组 · tools"
        );
        assert_eq!(
            describe(&route("kimi-k3", &["g-cheap"]), &c, "zh"),
            "首选 g-cheap ×0.5"
        );
        assert_eq!(describe(&route("plain", &["odd"]), &c, "zh"), "首选 odd");
        assert_eq!(
            describe(&route("gpt-image-2", &["vip"]), &c, "zh"),
            "生图模型 · 不是文本模型，不能当主聊天模型"
        );
        assert_eq!(
            describe(&route("seedance-2.0-0826-720p", &["vip"]), &c, "zh"),
            "视频模型 · 不是文本模型，不能当主聊天模型"
        );
        assert_eq!(
            describe(&route("deepseek-v4-pro", &["g-cheap", "vip"]), &c, "en"),
            "⭐ Recommended · First g-cheap ×0.5 · +1 more group"
        );
        assert_eq!(
            describe(&route("codex-auto-review", &["vip"]), &c, "zh"),
            "Codex 自动审批用"
        );
        assert_eq!(describe(&route("plain", &[]), &c, "zh"), "AICLI");
    }
}
