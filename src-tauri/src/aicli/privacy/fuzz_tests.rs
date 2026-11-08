//! 隐私引擎的随机输入测试（审查修复）。
//!
//! 引擎从 Go（Magpie）移植：Go 按字节切字符串不会出错，Rust 切到多字节字符
//! 中间会 panic（`phone.rs` 已出过一次）。golden 语料只覆盖固定样本，这里用
//! 随机拼接的「敏感值片段 + 多字节字符 + 分隔符」覆盖规则边界，断言：
//! - `mask` / `mask_json` / `restore` / `restore_json` / 流式还原都不 panic；
//! - 纯文本与 JSON 正文遮盖后再还原与原文一致；
//! - 流式还原任意切分的结果与一次喂入一致，且与非流式还原一致。

use proptest::prelude::*;

use super::placeholder::PlaceholderEngine;
use super::rules::redact::{self, MaskOptions};
use super::stream::StreamRestorer;
use super::walker;

fn engine() -> PlaceholderEngine {
    let mut key = [0u8; 32];
    for (i, b) in key.iter_mut().enumerate() {
        *b = (i * 13 + 5) as u8;
    }
    PlaceholderEngine::with_key(key)
}

fn opts() -> MaskOptions {
    MaskOptions {
        secrets: true,
        personal: true,
        ..Default::default()
    }
}

/// 规则关心的片段：敏感值形态、字段名、全角/半角标点、多字节字符、占位符残片。
/// 敏感值用拼接构造，避免仓库里出现字面量形态。
fn fragment() -> impl Strategy<Value = String> {
    let fixed: Vec<String> = vec![
        ["sk-", "Zx9QvLtYb3Nc8UmKe4Hd"].concat(),
        [
            "zhang", "san", "@", "mail", ".", "acme", "-", "corp", ".", "cn",
        ]
        .concat(),
        ["138", "0013", "8000"].concat(),
        ["+86 ", "139 ", "1234 ", "5678"].concat(),
        ["4111", "1111", "1111", "1111"].concat(),
        ["11010519", "491231002X"].concat(),
        [
            "https://",
            "admin:",
            "hunter2",
            "pw",
            "@",
            "db.acme-corp.cn/x",
        ]
        .concat(),
        [
            "eyJ",
            "hbGciOiJIUzI1NiJ9",
            ".eyJ",
            "zdWIiOiIxMjM0NTYifQ",
            ".",
            "abcdefghijk",
        ]
        .concat(),
        "password=".into(),
        "API_KEY: ".into(),
        "手机号：".into(),
        "电话:".into(),
        "记录ID ".into(),
        "我的邮箱是".into(),
        "：".into(),
        "，".into(),
        "。".into(),
        "“".into(),
        "”".into(),
        "{{".into(),
        "}}".into(),
        "{{EMAIL_".into(),
        "PHONE_ab".into(),
        "_".into(),
        "\\\"".into(),
        "\\n".into(),
        "\n".into(),
        "\r\n".into(),
        " ".into(),
        "//".into(),
        ":".into(),
        "@".into(),
        "=".into(),
        "é".into(),
        "\u{0301}".into(),
        "😀".into(),
        "𠀀".into(),
        "１２３".into(),
        "ＡＢＣ".into(),
    ];
    prop_oneof![
        3 => prop::sample::select(fixed),
        1 => "[a-zA-Z0-9]{1,12}",
        1 => any::<char>().prop_map(String::from),
        1 => "[\\u{4e00}-\\u{4e20}]{1,4}",
    ]
}

fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(fragment(), 0..24).prop_map(|parts| parts.concat())
}

/// 把文本放进几种协议形态的 JSON 正文（serde_json 负责转义）。
fn json_body(t: &str, shape: u8) -> String {
    let v = match shape % 4 {
        0 => serde_json::json!({"messages": [{"role": "user", "content": t}]}),
        1 => {
            serde_json::json!({"input": [{"type": "message", "content": [{"type": "input_text", "text": t}]}]})
        }
        2 => serde_json::json!({"contents": [{"parts": [{"text": t}]}]}),
        _ => serde_json::json!({
            "messages": [{"role": "assistant", "content": [
                {"type": "tool_use", "id": "toolu_1", "name": "run", "input": {"cmd": t}}
            ]}],
            "tools": [{"name": "run", "description": t}]
        }),
    };
    v.to_string()
}

/// 任意切分点（相对位置，避免依赖长度），可落在多字节字符中间。
fn cuts() -> impl Strategy<Value = Vec<f64>> {
    prop::collection::vec(0.0f64..1.0, 0..8)
}

fn split_at(bytes: &[u8], cuts: &[f64]) -> Vec<Vec<u8>> {
    let mut points: Vec<usize> = cuts
        .iter()
        .map(|c| ((bytes.len() as f64) * c) as usize)
        .collect();
    points.push(0);
    points.push(bytes.len());
    points.sort_unstable();
    points.dedup();
    points
        .windows(2)
        .map(|w| bytes[w[0]..w[1]].to_vec())
        .collect()
}

fn run_stream(e: &PlaceholderEngine, chunks: &[Vec<u8>]) -> String {
    let mut r = StreamRestorer::new(e);
    let mut out = Vec::new();
    for chunk in chunks {
        out.extend_from_slice(&r.push(chunk));
    }
    out.extend_from_slice(&r.finish());
    String::from_utf8_lossy(&out).into_owned()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 512, ..ProptestConfig::default() })]

    #[test]
    fn plain_text_mask_restore_round_trips(t in text()) {
        let mut e = engine();
        let (masked, _) = redact::mask(&t, &opts(), &mut e);
        prop_assert_eq!(walker::restore(&masked, false, &e), t);
    }

    #[test]
    fn json_mask_restore_round_trips(t in text(), shape in any::<u8>()) {
        let body = json_body(&t, shape);
        let mut e = engine();
        let (masked, _) = walker::mask_json(&body, &opts(), &mut e);
        let restored = walker::restore_json(&masked, &e);
        let want: serde_json::Value = serde_json::from_str(&body).unwrap();
        let got: serde_json::Value = serde_json::from_str(&restored)
            .map_err(|err| TestCaseError::fail(format!("restored body is not JSON: {err}")))?;
        prop_assert_eq!(got, want);
    }

    /// 任意字节（不一定是 UTF-8、不一定是 JSON）都不 panic。
    #[test]
    fn arbitrary_bytes_never_panic(raw in prop::collection::vec(any::<u8>(), 0..256), cuts in cuts()) {
        let lossy = String::from_utf8_lossy(&raw).into_owned();
        let mut e = engine();
        let _ = redact::mask(&lossy, &opts(), &mut e);
        let _ = walker::mask_json(&lossy, &opts(), &mut e);
        let _ = walker::restore(&lossy, true, &e);
        let _ = walker::restore_json(&lossy, &e);
        let _ = run_stream(&e, &split_at(&raw, &cuts));
    }

    /// 流式：模型回显带占位符的文本（含花括号与裸正文），任意切分与一次喂入一致，
    /// 且与非流式还原逐事件拼接一致。
    #[test]
    fn stream_restore_matches_one_shot(t in text(), deltas in 1usize..5, cuts in cuts()) {
        let mut e = engine();
        let (masked, _) = redact::mask(&t, &opts(), &mut e);
        // 把遮盖后的文本切成若干 delta，模拟模型逐段吐出（切点可能落在占位符中间）。
        let chars: Vec<char> = masked.chars().collect();
        let step = chars.len().div_ceil(deltas).max(1);
        let mut stream = String::from(": keepalive\n\n");
        for piece in chars.chunks(step) {
            let piece: String = piece.iter().collect();
            let data = serde_json::json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "text_delta", "text": piece}});
            stream.push_str(&format!("event: content_block_delta\ndata: {data}\n\n"));
        }
        let whole = run_stream(&e, &[stream.clone().into_bytes()]);
        let split = run_stream(&e, &split_at(stream.as_bytes(), &cuts));
        prop_assert_eq!(&split, &whole);

        // 拼回所有 delta 文本，应等于原文（跨事件的占位符被接起来还原）。
        let mut joined = String::new();
        for line in whole.lines() {
            if let Some(json) = line.strip_prefix("data: ") {
                let v: serde_json::Value = serde_json::from_str(json).unwrap();
                joined.push_str(v["delta"]["text"].as_str().unwrap_or_default());
            }
        }
        prop_assert_eq!(joined, t);
    }
}

/// 随机测试找到的两处 panic（修复前必崩）：
/// - `redact.rs` fold 窗口按字节回退 `back` 后切 `&s[start..at]`，落在 emoji 中间；
/// - `phone.rs` 号码前 32 字节切 `&s[a-32..a]`，落在汉字中间。
#[test]
fn multibyte_context_before_markers_does_not_panic() {
    for t in [
        "😀一一一一手机号：",
        "记录ID aAA0A0：：一一一一13800138000",
        "我的邮箱是{{一一{{一一记录ID 13800138000",
    ] {
        let mut e = engine();
        let (masked, _) = redact::mask(t, &opts(), &mut e);
        assert_eq!(walker::restore(&masked, false, &e), t);
    }
}

/// 非流式还原也接受模型不带花括号写出的占位符正文（与流式 wordTail 一致）。
#[test]
fn restore_json_handles_brace_less_placeholders() {
    let mut e = engine();
    let value = [
        "zhang", "san", "@", "mail", ".", "acme", "-", "corp", ".", "cn",
    ]
    .concat();
    let placeholder = e.mask("EMAIL", &value);
    let bare = placeholder.trim_matches(['{', '}']);
    let body =
        serde_json::json!({"content": [{"type": "text", "text": format!("mail {bare} now")}]})
            .to_string();
    let restored = walker::restore_json(&body, &e);
    assert!(
        restored.contains(&value),
        "brace-less placeholder not restored: {restored}"
    );
}
