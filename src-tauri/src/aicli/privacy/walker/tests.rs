//! E-04 验收测试：JSON 遍历与协议保护。
//!
//! 覆盖清单逐条：只读整块（thinking/encrypted reasoning/thoughtSignature）、
//! 不扫描的键（id/call_id/model/signature/data…）、必须扫描的工具参数
//! （tool_use.input、functionCall.args，含其中的同名字段）、defs 只有
//! description 遮盖、未命中时逐字节相同、四协议夹具 + Codex 真实样本。

use super::*;
use crate::aicli::privacy::placeholder::PlaceholderEngine;
use crate::aicli::privacy::rules::redact::MaskOptions;

fn engine() -> PlaceholderEngine {
    let mut key = [0u8; 32];
    for (i, b) in key.iter_mut().enumerate() {
        *b = (i * 11 + 7) as u8;
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

/// 拼接构造敏感值，避免字面量形态。
fn sk() -> String {
    ["sk-", "Zx9QvLtYb3Nc8UmKe4Hd"].concat()
}

fn email() -> String {
    // 注意不能用 example.com：Magpie 有意不遮保留示例域（real_email 排除）
    [
        "zhang", "san", "@", "mail", ".", "acme", "-", "corp", ".", "cn",
    ]
    .concat()
}

// ---- walk：字节保持 ----

#[test]
fn walk_unchanged_is_byte_identical() {
    // 怪格式：空格、换行、unicode 转义、数字、裸 true、重复键
    let body = "{  \"a\" : \"x\\u00e9y\" ,\n\t\"n\": [-12.5e3, 0.25, true, null],\"a\":[\"s\"]}";
    let out = walk(body, &mut |_p, _k, s| s.to_string()).unwrap();
    assert_eq!(out, body);
}

#[test]
fn walk_replaces_only_the_matched_string() {
    let body = r#"{"k":"alpha","keep":"beta"}"#;
    let out = walk(body, &mut |_p, k, s| {
        if k == "k" {
            format!("{s}!")
        } else {
            s.to_string()
        }
    })
    .unwrap();
    assert_eq!(out, r#"{"k":"alpha!","keep":"beta"}"#);
}

#[test]
fn walk_escapes_the_replacement() {
    let body = r#"{"k":"plain"}"#;
    let want = ["a", "\"", "b", "c\nd", "\u{2028}", "e"].concat();
    let out = walk(body, &mut |_p, _k, _s| want.clone()).unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).expect("still JSON");
    assert_eq!(v["k"].as_str().unwrap(), want);
    assert!(out.contains("\\u2028"), "U+2028 escaped: {out}");
}

#[test]
fn walk_not_json_is_none() {
    for bad in ["", "{", "{\"a\":}", "no json", "{\"a\":1}x", "[1,]"] {
        assert_eq!(walk(bad, &mut |_p, _k, s| s.to_string()), None, "{bad}");
    }
}

#[test]
fn walk_reports_paths_and_keys() {
    let body = r#"{"input":[{"content":[{"type":"text","text":"hi"}]}]}"#;
    let mut paths = Vec::new();
    walk(body, &mut |p, k, s| {
        if s == "hi" {
            paths.push((p.to_string(), k.to_string()));
        }
        s.to_string()
    })
    .unwrap();
    assert_eq!(
        paths,
        vec![("input.0.content.0.text".to_string(), "text".to_string())]
    );
}

// ---- scan ----

#[test]
fn scan_anthropic_thinking_tool_use_and_defs() {
    let body = format!(
        r#"{{"model":"m","tools":[{{"name":"t"}}],
        "messages":[{{"role":"user","content":[
            {{"type":"thinking","thinking":"…{}…","signature":"sig12345"}},
            {{"type":"tool_use","id":"tu1","name":"t","input":{{"signature":"{}"}}}},
            {{"type":"text","text":"hi"}}]}}]}}"#,
        "cached",
        sk()
    );
    let (signed, args, defs) = scan(&body);
    assert_eq!(signed, vec!["messages.0.content.0"]);
    assert_eq!(args, vec!["messages.0.content.1.input"]);
    assert!(defs.contains(&"tools".to_string()), "{defs:?}");
}

#[test]
fn scan_gemini_function_call_and_thought_signature() {
    let body = r#"{"contents":[{"role":"user","parts":[
        {"text":"hi"},
        {"functionCall":{"name":"f","args":{"q":"x"}}},
        {"text":"t","thoughtSignature":"sig"}]}],
        "generationConfig":{"temperature":1}}"#;
    let (signed, args, defs) = scan(body);
    assert_eq!(
        signed,
        vec!["contents.0.parts.2"],
        "thoughtSignature part is sealed"
    );
    assert_eq!(args, vec!["contents.0.parts.1.functionCall.args"]);
    assert!(defs.contains(&"generationConfig".to_string()));
}

#[test]
fn scan_responses_encrypted_reasoning_and_additional_tools() {
    let body = r#"{"input":[
        {"type":"reasoning","summary":[],"encrypted_content":"enc"},
        {"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]},
        {"type":"additional_tools","tools":[{"type":"function","name":"f"}]}],
        "reasoning":{"effort":"low"},"include":[]}"#;
    let (signed, _args, defs) = scan(body);
    assert_eq!(signed, vec!["input.0"], "encrypted reasoning is sealed");
    assert!(defs.contains(&"input.2.tools".to_string()), "{defs:?}");
    assert!(defs.contains(&"reasoning".to_string()) && defs.contains(&"include".to_string()));
}

// ---- mask_json：四协议 ----

#[test]
fn mask_json_anthropic() {
    let mut e = engine();
    let key = sk();
    let mail = email();
    let body = format!(
        r#"{{"model":"claude-x","messages":[{{"role":"user","content":[
            {{"type":"text","text":"my mail {mail}"}},
            {{"type":"thinking","thinking":"user said {mail}","signature":"sigabcdef1"}},
            {{"type":"tool_use","id":"tu_1","name":"deploy","input":{{"signature":"{key}","region":"cn"}}}}]}}]}}"#
    );
    let (out, n) = mask_json(&body, &opts(), &mut e);
    assert!(n >= 2, "{out}");
    // 正文与工具参数里的都遮了
    assert!(!out.contains(&mail), "{out}");
    assert!(!out.contains(&key), "{out}");
    // thinking 块只读整块：请求里遮过的值在这里变回占位符（供应商当初
    // 写的就是它），不出现原值
    let sealed_at = out.find(r#""thinking":"user said "#).unwrap();
    let tail = &out[sealed_at..];
    assert!(tail.contains("{{EMAIL_"), "sealed holds placeholder: {out}");
    assert!(
        !tail.contains(&mail),
        "sealed must not hold the value: {out}"
    );
    assert!(
        out.contains(r#""signature":"sigabcdef1""#),
        "signature kept"
    );
    assert!(out.contains(r#""id":"tu_1""#), "tool_use id kept");
    assert!(out.contains(r#""region":"cn""#), "args sibling kept");
    // 结构仍合法
    serde_json::from_str::<serde_json::Value>(&out).unwrap();
}

#[test]
fn mask_json_chat_completions_tools_and_ids() {
    let mut e = engine();
    let key = sk();
    let card = ["4111", "1111", "1111", "1111"].concat(); // Luhn 通过
    let body = format!(
        r#"{{"model":"gpt-x","tools":[{{"type":"function","function":{{
            "name":"deploy","description":"run with password={key}","parameters":{{"type":"object"}}}}}}],
        "messages":[{{"role":"tool","tool_call_id":"call_{card}","content":"done {mail}"}}]}}"#,
        mail = email()
    );
    let (out, n) = mask_json(&body, &opts(), &mut e);
    assert!(n >= 2, "{out}");
    assert!(!out.contains(&key), "description masked: {out}");
    assert!(out.contains(r#""name":"deploy""#), "tool name kept");
    assert!(
        out.contains(&format!("\"tool_call_id\":\"call_{card}\"")),
        "{out}"
    );
    assert!(!out.contains(&email()), "tool result text masked: {out}");
}

#[test]
fn mask_json_responses_function_call_arguments() {
    let mut e = engine();
    let pw = "Zx9qw7Pl2Km4"; // 12 字符，SECRET 赋值规则命中
    let args = format!("{{\\\"DB_PASSWORD\\\":\\\"{pw}\\\"}}");
    let body = format!(
        r#"{{"model":"r","input":[{{"type":"function_call","call_id":"c1","name":"run",
        "arguments":"{args}"}},{{"type":"message","role":"user","content":[{{"type":"input_text","text":"go"}}]}}],
        "reasoning":{{"effort":"low"}}}}"#
    );
    let (out, n) = mask_json(&body, &opts(), &mut e);
    assert!(n >= 1, "{out}");
    assert!(!out.contains(pw), "password inside arguments masked: {out}");
    assert!(out.contains(r#""call_id":"c1""#), "call_id kept");
    // arguments 仍是合法的 JSON 字符串
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let inner = v["input"][0]["arguments"].as_str().unwrap();
    let inner: serde_json::Value = serde_json::from_str(inner).expect("arguments still JSON");
    assert!(inner["DB_PASSWORD"]
        .as_str()
        .unwrap()
        .starts_with("{{SECRET_"));
    // 还原闭环
    let restored = restore_json(&out, &e);
    assert_eq!(restored, body, "roundtrip");
}

#[test]
fn mask_json_gemini_thought_signature_and_defs() {
    let mut e = engine();
    let mail = email();
    let body = format!(
        r#"{{"contents":[{{"role":"model","parts":[
            {{"text":"mail {mail} here","thoughtSignature":"c2ln"}},
            {{"inlineData":{{"mimeType":"image/png","data":"data:image/png;base64,iVBOR"}}}}]}}],
        "generationConfig":{{"temperature":1}}}}"#
    );
    let (out, n) = mask_json(&body, &opts(), &mut e);
    assert!(n == 0, "sealed part and inline data untouched: {out}");
    assert_eq!(out, body, "byte identical when nothing to mask");
}

#[test]
fn mask_json_data_urls_and_media_kept() {
    let mut e = engine();
    // data: URL 的 base64 里凑出能过 Luhn 的片段也不遮（keep data:）
    let body = r#"{"messages":[{"content":[{"type":"image","source":{
        "type":"base64","media_type":"image/png","data":"data:image/png;base64,MTIzNDU2Nzg5MDEyMzQ1"}}]}],
        "url":"https://x.example/a@b.c?k=1"}"#;
    let (out, n) = mask_json(body, &opts(), &mut e);
    assert_eq!(n, 0, "{out}");
    assert_eq!(out, body);
}

#[test]
fn mask_json_no_hits_byte_identical() {
    let mut e = engine();
    let body = "{ \"model\" : \"m\" ,\n\"input\":[ {\"type\":\"message\",\"content\":[{\"type\":\"input_text\",\"text\":\"hello\\u00e9\"}]}]}";
    let (out, n) = mask_json(body, &opts(), &mut e);
    assert_eq!(n, 0);
    assert_eq!(out, body);
}

#[test]
fn mask_json_not_json_falls_back_to_text() {
    let mut e = engine();
    let key = sk();
    let body = format!("the key is {key} and nothing else");
    let (out, n) = mask_json(&body, &opts(), &mut e);
    assert!(!out.contains(&key), "{out}");
    assert!(n >= 1);
}

// ---- 封签块的重遮（值变回占位符） ----

#[test]
fn signed_blocks_remask_prior_values() {
    let mut e = engine();
    let mail = email();
    let body = format!(
        r#"{{"messages":[{{"content":[
            {{"type":"text","text":"mail {mail}"}},
            {{"type":"thinking","thinking":"mail {mail}","signature":"sig99887766"}}]}}]}}"#
    );
    let (out, n) = mask_json(&body, &opts(), &mut e);
    assert!(n >= 2, "{out}");
    // thinking 块里的值变回占位符（供应商当初写的就是它），不是原值
    let i = out.find(r#""thinking":"mail "#).unwrap();
    let tail = &out[i..];
    assert!(tail.contains("{{"), "sealed holds placeholder: {out}");
    assert!(
        !tail.contains(&mail),
        "sealed must not hold the value: {out}"
    );
    // 还原闭环：客户端拿回原文
    assert_eq!(restore_json(&out, &e), body);
}

// ---- restore ----

#[test]
fn restore_text_handles_braces_and_case() {
    let mut e = engine();
    let p = e.mask("EMAIL", &email());
    // 花括号变体与大小写变体都还原（braces/canonical）；未知占位符留着
    let phone_unknown = ["{{", "PHONE_", &"x".repeat(8), "}}"].concat();
    let body = [
        "a ",
        &p,
        " b { ",
        &p,
        " } c {",
        &p,
        "} d ",
        &p,
        "} e ",
        &phone_unknown,
        " f ",
        &p.to_lowercase(),
    ]
    .concat();
    let out = restore(&body, false, &e);
    assert!(out.contains(&email()), "{out}");
    assert!(out.contains(&phone_unknown), "unknown stays: {out}");
    assert!(
        !out.contains(&p.to_lowercase()),
        "case variant restored: {out}"
    );
}

#[test]
fn restore_json_roundtrip_anthropic() {
    let mut e = engine();
    let mail = email();
    let key = sk();
    let body = format!(
        r#"{{"messages":[{{"content":[
            {{"type":"text","text":"mail {mail}"}},
            {{"type":"tool_use","id":"t1","name":"n","input":{{"api_key":"{key}"}}}}]}}]}}"#
    );
    let (out, n) = mask_json(&body, &opts(), &mut e);
    assert!(n >= 2);
    assert_eq!(restore_json(&out, &e), body);
}

// ---- Codex 真实样本（Magpie testdata，MIT） ----

#[test]
fn codex_sample_untouched_and_valid() {
    let mut e = engine();
    let body = include_str!("../../../../tests/fixtures/privacy/codex_request.json");
    let (out, n) = mask_json(body, &opts(), &mut e);
    assert_eq!(n, 0, "sanitized sample masks nothing: {out}");
    assert_eq!(out, body, "byte identical");
    assert_eq!(restore_json(body, &e), body);
}

// ---- escaped 工具参数（inStrings/outsideEscapes/bare 值） ----

#[test]
fn escaped_arguments_bare_number_masked_quoted() {
    let mut e = engine();
    // password 是裸数字（不是字符串）：inStrings 判为 bare，占位符带引号；
    // 数字拼接构造，避免源码里出现卡号字面量
    let card = ["4111", "1111", "1111", "1111"].concat();
    let args = ["{\"DB_PASSWORD\": ", &card, "}"].concat();
    let escaped_args = serde_json::to_string(&args).unwrap();
    let body = format!(
        r#"{{"input":[{{"type":"function_call","call_id":"c","name":"run","arguments":{escaped_args}}}]}}"#
    );
    let (out, n) = mask_json(&body, &opts(), &mut e);
    assert!(n >= 1, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let inner: serde_json::Value =
        serde_json::from_str(v["input"][0]["arguments"].as_str().unwrap()).unwrap();
    assert!(inner["DB_PASSWORD"]
        .as_str()
        .unwrap()
        .starts_with("{{BANK_CARD_"));
    // 还原后数值回来，但遮盖时加的引号保留（Magpie 同：bare 的占位符带
    // 引号放回，restore 只换占位符本体，Go 侧亦如此）
    let restored = restore_json(&out, &e);
    assert!(restored.contains(&card), "digits restored: {restored}");
    let v: serde_json::Value = serde_json::from_str(&restored).unwrap();
    let inner_r: serde_json::Value =
        serde_json::from_str(v["input"][0]["arguments"].as_str().unwrap()).unwrap();
    assert_eq!(inner_r["DB_PASSWORD"].as_str().unwrap(), card);
}

#[test]
fn escaped_arguments_value_with_quotes() {
    let mut e = engine();
    // 值里带转义的引号（Magpie #3022 场景）：反斜杠与引号都用字节构造，
    // 避免源码转义歧义
    let bs = String::from_utf8(vec![92u8]).unwrap();
    let qq = String::from_utf8(vec![34u8]).unwrap();
    let pw = ["Zx9qw7Pl", &bs, &qq, "2Km4"].concat();
    let args_inner = ["{\"password\":\"", &pw, "\"}"].concat();
    let escaped_args = serde_json::to_string(&args_inner).unwrap();
    let body = format!(
        r#"{{"input":[{{"type":"function_call","call_id":"c","name":"run","arguments":{escaped_args}}}]}}"#
    );
    let (out, n) = mask_json(&body, &opts(), &mut e);
    assert!(n >= 1, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).expect("still JSON");
    let inner: serde_json::Value =
        serde_json::from_str(v["input"][0]["arguments"].as_str().unwrap())
            .expect("arguments still JSON");
    assert!(!inner["password"].as_str().unwrap().contains(&pw));
    assert_eq!(restore_json(&out, &e), body, "roundtrip restores the value");
}

// ---- json_escape 与 Go 逐点一致 ----

#[test]
fn json_escape_go_parity() {
    assert_eq!(json_escape("a\"b\\c"), "a\\\"b\\\\c");
    assert_eq!(json_escape("\n\r\t"), "\\n\\r\\t");
    assert_eq!(json_escape("\u{8}\u{c}\u{0}"), "\\u0008\\u000c\\u0000");
    assert_eq!(json_escape("\u{2028}\u{2029}"), "\\u2028\\u2029");
    assert_eq!(json_escape("héllo"), "héllo"); // 非 ASCII 原样
    assert_eq!(json_escape("<>&"), "<>&"); // 不转义 HTML
}

// ---- 每一种工具调用的参数都按工具参数处理（Magpie d7714b3f3） ----

/// 移植 Magpie `TestEveryToolCallsArgumentsAreMasked`：15 种调用形态，参数里放在
/// 协议豁免键名下的手机号（data）、密钥（url 查询串）、卡号（customer_id）、
/// 邮箱（signature）都必须遮盖；调用自己的 id、name、server_name 保持原样。
/// 修复前 mcp_tool_use、Chat tool_calls / function_call 和 8 种 Responses *_call
/// 都原样发出。
#[test]
fn every_tool_calls_arguments_are_masked() {
    let key = ["sk-", "ant-api03-", "abcdefghijklmnopqrstuvwxyz0123456789"].concat();
    let phone = "13800138000";
    let card = "4111 1111 1111 1111";
    let mail = ["rina.k", "@", "fastmail.com"].concat();
    let args = format!(
        r#"{{"data":"call {phone}","url":"https://hooks.acme.io/?k={key}","customer_id":"{card}","signature":"mail {mail}"}}"#
    );
    let gone = [
        phone.to_string(),
        key.clone(),
        card.to_string(),
        mail.clone(),
    ];

    let anthropic = |typ: &str| {
        format!(
            r#"{{"messages":[{{"role":"assistant","content":[{{"type":"{typ}","id":"toolu_1","name":"send","server_name":"acme","input":{args}}}]}}]}}"#
        )
    };
    let responses = |typ: &str, field: &str| {
        format!(
            r#"{{"input":[{{"type":"{typ}","call_id":"call_1","name":"send","{field}":{args}}}]}}"#
        )
    };
    let mut cases: Vec<(String, String)> = vec![
        (
            "chat tool_calls".into(),
            format!(
                r#"{{"messages":[{{"role":"assistant","tool_calls":[{{"id":"call_1","type":"function","function":{{"name":"send","arguments":{args}}}}}]}}]}}"#
            ),
        ),
        (
            "chat function_call".into(),
            format!(
                r#"{{"messages":[{{"role":"assistant","function_call":{{"name":"send","arguments":{args}}}}}]}}"#
            ),
        ),
        (
            "gemini functionCall".into(),
            format!(
                r#"{{"contents":[{{"role":"model","parts":[{{"functionCall":{{"name":"send","args":{args}}}}}]}}]}}"#
            ),
        ),
        (
            "gemini function_call".into(),
            format!(
                r#"{{"contents":[{{"role":"model","parts":[{{"function_call":{{"name":"send","args":{args}}}}}]}}]}}"#
            ),
        ),
    ];
    for typ in ["tool_use", "server_tool_use", "mcp_tool_use"] {
        cases.push((format!("anthropic {typ}"), anthropic(typ)));
    }
    for (typ, field) in [
        ("function_call", "arguments"),
        ("mcp_call", "arguments"),
        ("tool_search_call", "arguments"),
        ("custom_tool_call", "input"),
        ("local_shell_call", "action"),
        ("shell_call", "action"),
        ("computer_call", "action"),
        ("apply_patch_call", "operation"),
    ] {
        cases.push((format!("responses {typ}"), responses(typ, field)));
    }
    assert_eq!(cases.len(), 15);

    for (name, body) in &cases {
        let mut e = engine();
        let (out, _) = mask_json(body, &opts(), &mut e);
        for value in &gone {
            assert!(
                !out.contains(value.as_str()),
                "{name}: {value:?} went out: {out}"
            );
        }
        for kept in [
            r#""call_1""#,
            r#""toolu_1""#,
            r#""name":"send""#,
            r#""server_name":"acme""#,
        ] {
            if body.contains(kept) {
                assert!(out.contains(kept), "{name}: {kept} changed: {out}");
            }
        }
        // 还原后与原文逐字节一致
        assert_eq!(&restore_json(&out, &e), body, "{name}");
    }
}
