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
    /// Short form after a media model's kind when its server description follows.
    cannot_main: &'static str,
    /// Joins a media model's kind and `cannot_main`.
    comma: &'static str,
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
            cannot_main: "不能当主聊天模型",
            comma: "，",
            codex_auto: "Codex 自动审批用",
        },
        "zh-TW" | "zh-HK" | "zh-Hant" => Words {
            recommended: "⭐ 推薦",
            first: "首選",
            more: |n| format!("還可走 {n} 個分組"),
            image: "生圖模型",
            video: "影片模型",
            not_text: "不是文字模型，不能當主聊天模型",
            cannot_main: "不能當主聊天模型",
            comma: "，",
            codex_auto: "Codex 自動審批用",
        },
        "ja" => Words {
            recommended: "⭐ おすすめ",
            first: "優先",
            more: |n| format!("ほか {n} グループ"),
            image: "画像生成モデル",
            video: "動画生成モデル",
            not_text: "テキストモデルではないため、メインのチャットには使えません",
            cannot_main: "メインのチャットには使えません",
            comma: "、",
            codex_auto: "Codex の自動承認用",
        },
        "tr" => Words {
            recommended: "⭐ Önerilen",
            first: "Önce",
            more: |n| format!("+{n} grup daha"),
            image: "Görsel modeli",
            video: "Video modeli",
            not_text: "metin modeli değil, ana sohbet modeli olarak kullanılamaz",
            cannot_main: "ana sohbet modeli olarak kullanılamaz",
            comma: ", ",
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
            cannot_main: "can't be the main chat model",
            comma: ", ",
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

/// The server's own description of `model` (new-api's model description), on one line,
/// cut to [`MAX_DESCRIPTION_CHARS`].
fn server_description(model: &str, catalog: &Catalog) -> Option<String> {
    let text = catalog
        .models
        .get(model)
        .and_then(|entry| entry.pricing.get("description"))
        .and_then(|value| value.as_str())?;
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    if text.chars().count() <= MAX_DESCRIPTION_CHARS {
        return Some(text);
    }
    let cut: String = text.chars().take(MAX_DESCRIPTION_CHARS).collect();
    Some(format!("{}…", cut.trim_end()))
}

/// A server description longer than this many characters is cut with "…", so a picker
/// row stays on one or two lines.
const MAX_DESCRIPTION_CHARS: usize = 60;

/// The first group and its ratio: `vip ×1`, or `vip` when the ratio is unknown.
fn first_group(route: &ModelRoute, catalog: &Catalog) -> Option<String> {
    let first = route.ordered_groups.first()?;
    let ratio = catalog
        .groups
        .get(first)
        .filter(|group| group.ratio_known)
        .and_then(|group| group.ratio)
        .filter(|ratio| ratio.is_finite());
    Some(match ratio {
        Some(ratio) => format!("{first} {}", ratio_text(ratio)),
        None => first.clone(),
    })
}

/// The line under a model (owner, 2026-11-14): the server's description when it has
/// one — "⭐ 推荐 · <description> · vip ×1", a media model "生图模型，不能当主聊天模型 ·
/// <description>" — else the first group, fallback groups and tags as before.
pub(crate) fn describe(route: &ModelRoute, catalog: &Catalog, language: &str) -> String {
    let w = words(language);
    let kind = kind(&route.upstream_model, catalog);
    let server = server_description(&route.upstream_model, catalog);
    let media = match kind {
        Kind::CodexAuto => return w.codex_auto.to_string(),
        Kind::Image => Some(w.image),
        Kind::Video => Some(w.video),
        Kind::Recommended | Kind::Text => None,
    };
    if let Some(media) = media {
        return match server {
            Some(text) => format!("{media}{}{} · {text}", w.comma, w.cannot_main),
            None => format!("{media} · {}", w.not_text),
        };
    }
    let mut parts = Vec::new();
    if kind == Kind::Recommended {
        parts.push(w.recommended.to_string());
    }
    if let Some(text) = server {
        parts.push(text);
        parts.extend(first_group(route, catalog));
        return parts.join(" · ");
    }
    if let Some(group) = first_group(route, catalog) {
        parts.push(format!("{} {group}", w.first));
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
        let described = |tags: &str, description: &str| CatalogModel {
            groups: vec!["vip".into()],
            wire: BTreeMap::new(),
            compat: BTreeMap::new(),
            pricing: json!({ "tags": tags, "description": description }),
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
                (
                    "claude-haiku-4-5".to_string(),
                    described(
                        "",
                        "快速型 Claude，\n适合轻量智能体、办公任务与即时响应对话",
                    ),
                ),
                (
                    "gpt-5.6-luna".to_string(),
                    described("推荐", "高性价比的 GPT-5.6，适合快速、高吞吐的负载"),
                ),
                (
                    "muse-wonder-1.0".to_string(),
                    described("", &"很长的描述".repeat(20)),
                ),
                (
                    "grok-imagine-video".to_string(),
                    described("", "面向提示词驱动的生成与编辑"),
                ),
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

    #[test]
    fn server_descriptions_lead_and_the_group_follows() {
        let c = catalog();
        // 后台描述放最前，换行压成一行；分组倍率缩短放末尾
        assert_eq!(
            describe(&route("claude-haiku-4-5", &["vip", "g-cheap"]), &c, "zh"),
            "快速型 Claude， 适合轻量智能体、办公任务与即时响应对话 · vip ×1"
        );
        assert_eq!(
            describe(&route("gpt-5.6-luna", &["g-cheap"]), &c, "zh"),
            "⭐ 推荐 · 高性价比的 GPT-5.6，适合快速、高吞吐的负载 · g-cheap ×0.5"
        );
        assert_eq!(
            describe(&route("gpt-5.6-luna", &["odd"]), &c, "en"),
            "⭐ Recommended · 高性价比的 GPT-5.6，适合快速、高吞吐的负载 · odd"
        );
        // 太长的描述截到 60 个字加省略号
        let long = describe(&route("muse-wonder-1.0", &[]), &c, "zh");
        assert_eq!(long.chars().count(), 61, "{long}");
        assert!(long.ends_with('…'));
        // 生图、视频模型：先说不能当主聊天模型，再接后台描述
        assert_eq!(
            describe(&route("grok-imagine-video", &["vip"]), &c, "zh"),
            "视频模型，不能当主聊天模型 · 面向提示词驱动的生成与编辑"
        );
        assert_eq!(
            describe(&route("grok-imagine-video", &["vip"]), &c, "en"),
            "Video model, can't be the main chat model · 面向提示词驱动的生成与编辑"
        );
    }
}
