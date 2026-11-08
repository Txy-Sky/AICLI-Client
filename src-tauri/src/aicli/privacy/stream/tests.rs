//! E-05 验收测试：流式还原。
//!
//! 覆盖清单：任意字节切分不变性（proptest）、占位符跨事件/裸 token/
//! 转义形式/工具参数流式增量、流中断放出扣留、多字节字符跨块、
//! Gemini 无 type/index 的路径上下文、只读键（signature）不还原、
//! CRLF 与注释行。

use proptest::prelude::*;

use super::*;

/// 测试辅助：字节输出转字符串断言（流是 UTF-8 时无损）。
fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes)
        .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}
use crate::aicli::privacy::placeholder::PlaceholderEngine;

fn engine() -> PlaceholderEngine {
    let mut key = [0u8; 32];
    for (i, b) in key.iter_mut().enumerate() {
        *b = (i * 11 + 7) as u8;
    }
    PlaceholderEngine::with_key(key)
}

fn email_value() -> String {
    // 不用保留示例域（real_email 有意排除 example.com）
    [
        "li", "ming", "@", "mail", ".", "acme", "-", "corp", ".", "cn",
    ]
    .concat()
}

/// 一次喂入 vs 任意切分：输出一致（清单验收第一条）。
#[test]
fn chunk_split_invariance_simple() {
    let mut e = engine();
    let p = e.mask("EMAIL", &email_value());
    let phrase = format!("contact {p} anytime");
    let stream = format!(
        "event: message\ndata: {{\"type\":\"delta\",\"delta\":{{\"text\":\"see {phrase}\"}}}}\n\n"
    );
    let whole = {
        let mut r = StreamRestorer::new(&e);
        let mut out = r.push(stream.as_bytes());
        out.extend_from_slice(&r.finish());
        text(out)
    };
    let split = {
        let mut r = StreamRestorer::new(&e);
        let mut out = r.push(&stream.as_bytes()[..7]);
        out.extend_from_slice(&r.push(&stream.as_bytes()[7..30]));
        out.extend_from_slice(&r.push(&stream.as_bytes()[30..]));
        out.extend_from_slice(&r.finish());
        text(out)
    };
    assert_eq!(whole, split);
    assert!(whole.contains(&email_value()), "{whole}");
    assert!(!whole.contains("{{EMAIL_"), "{whole}");
}

/// 占位符跨事件：`{{EMA` 在一个 delta，`IL_xxx}}` 在下一个——拼起来还原。
#[test]
fn placeholder_split_across_events() {
    let mut e = engine();
    let p = e.mask("EMAIL", &email_value());
    // 从占位符里切开：p = "{{EMAIL_suffix}}" → 前半 "{{EMAIL_xxx" 后半 "xxx}}"
    let (head, tailc) = p.split_at(p.len() / 2);
    let frame = |text: &str| {
        [
            "event: message\ndata: ",
            "{\"type\":\"delta\",\"delta\":{\"text\":\"",
            text,
            "\"}}\n\n",
        ]
        .concat()
    };
    let ev1 = frame(&["mail ", head].concat());
    let ev2 = frame(&[tailc, " ok"].concat());
    let mut r = StreamRestorer::new(&e);
    let mut out = r.push(ev1.as_bytes());
    // ev1 被扣留（尾部像占位符开头）——此刻不应有半截占位符流出
    assert!(!text(out.clone()).contains("{{"), "held: {:?}", text(out));
    out.extend_from_slice(&r.push(ev2.as_bytes()));
    out.extend_from_slice(&r.finish());
    let out = text(out);
    // 尾部移到 ev2 后拼成完整占位符：ev1 留下削去尾部的文本，
    // ev2 的文本 = 尾部 + 原文，整体还原。字节只移不改。
    assert_eq!(
        out,
        [frame("mail "), frame(&[&email_value(), " ok"].concat())].concat(),
        "bytes moved, not changed"
    );
}

/// 上下文不同（index 变了）：扣留的原样放出，尾部不移过去。
#[test]
fn held_released_when_context_differs() {
    let mut e = engine();
    let p = e.mask("EMAIL", &email_value());
    let (head, tailc) = p.split_at(p.len() / 2);
    let ev1 =
        format!("data: {{\"type\":\"delta\",\"index\":0,\"delta\":{{\"text\":\"a {head}\"}}}}\n\n");
    let ev2 = format!(
        "data: {{\"type\":\"delta\",\"index\":1,\"delta\":{{\"text\":\"b {tailc}\"}}}}\n\n"
    );
    let mut r = StreamRestorer::new(&e);
    let mut out = r.push(ev1.as_bytes());
    out.extend_from_slice(&r.push(ev2.as_bytes()));
    out.extend_from_slice(&r.finish());
    let out = text(out);
    // 两段都没拼成完整占位符：都按原样（未还原）流出
    assert!(out.contains(head), "{out}");
    assert!(out.contains(tailc), "{out}");
}

/// 裸 token（不带花括号，wordTail）：最后一个词是 kind 的前缀时扣留。
#[test]
fn bare_token_word_tail() {
    let mut e = engine();
    let p = e.mask("SECRET", "Zx9qw7Pl2Km4abcd");
    // 取占位符正文的前几个字符做裸词
    let body = p.trim_start_matches("{{").trim_end_matches("}}");
    let w: String = body.chars().take(7).collect(); // "SECRET_" 前 7 字符
    let ev1 = format!("data: {{\"delta\":{{\"text\":\"code is {w}\"}}}}\n\n");
    let ev2 = format!(
        "data: {{\"delta\":{{\"text\":\"{} done\"}}}}\n\n",
        &body[7..]
    );
    let mut r = StreamRestorer::new(&e);
    let mut out = r.push(ev1.as_bytes());
    assert!(
        out.is_empty() || !text(out.clone()).contains(&["code is ", &w].concat()),
        "held: {:?}",
        text(out)
    );
    out.extend_from_slice(&r.push(ev2.as_bytes()));
    out.extend_from_slice(&r.finish());
    let out = text(out);
    assert!(out.contains("Zx9qw7Pl2Km4abcd"), "restored: {out}");
}

/// 工具参数流式增量（Responses function_call_arguments.delta）：
/// partial_json/arguments 键内的还原值做 JSON 转义。
#[test]
fn tool_arguments_streaming_delta() {
    let mut e = engine();
    let p = e.mask("SECRET", "Zx9qw7Pl2Km4");
    let args_json = format!("{{\\\"password\\\":\\\"{p}\\\"}}");
    let stream = format!(
        "event: response.function_call_arguments.delta\ndata: {{\"type\":\"response.function_call_arguments.delta\",\"delta\":\"{args_json}\"}}\n\n"
    );
    let mut r = StreamRestorer::new(&e);
    let mut out = r.push(stream.as_bytes());
    out.extend_from_slice(&r.finish());
    let out = text(out);
    assert!(out.contains("Zx9qw7Pl2Km4"), "value restored: {out}");
    // 增量字符串本身仍是合法 JSON 文本（转义保住了结构）
    let i = out.find("data: ").unwrap();
    let line_end = out[i..].find('\n').unwrap() + i;
    let payload = out[i + 6..line_end].trim();
    let v: serde_json::Value = serde_json::from_str(payload).expect("event still JSON");
    assert!(v["delta"].as_str().unwrap().contains("Zx9qw7Pl2Km4"));
}

/// 只读键不还原：signature_delta 流出原样。
#[test]
fn signature_delta_not_restored() {
    let mut e = engine();
    let p = e.mask("SECRET", "Zx9qw7Pl2Km4");
    // signature 键下的占位符保持（sealed 内容原样送回）
    let stream = format!(
        "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"delta\":{{\"type\":\"signature_delta\",\"signature\":\"{p}\"}}}}\n\n"
    );
    let mut r = StreamRestorer::new(&e);
    let mut out = r.push(stream.as_bytes());
    out.extend_from_slice(&r.finish());
    let out = text(out);
    assert!(out.contains(&p), "placeholder kept: {out}");
    assert!(!out.contains("Zx9qw7Pl2Km4"), "value must not leak: {out}");
}

/// Gemini：没有 type/index 字段，按候选与 part 的路径区分；同一 part 的
/// 连续 delta 拼接还原，不同 part 不拼。
#[test]
fn gemini_part_path_context() {
    let mut e = engine();
    let p = e.mask("EMAIL", &email_value());
    let (head, tailc) = p.split_at(p.len() / 2);
    // 同一 part 的两个 delta
    let ev1 = format!(
        "data: {{\"candidates\":[{{\"content\":{{\"parts\":[{{\"text\":\"hi {head}\"}}]}}}}]}}\n\n"
    );
    let ev2 = format!(
        "data: {{\"candidates\":[{{\"content\":{{\"parts\":[{{\"text\":\"{tailc}!\"}}]}}}}]}}\n\n"
    );
    let mut r = StreamRestorer::new(&e);
    let mut out = r.push(ev1.as_bytes());
    out.extend_from_slice(&r.push(ev2.as_bytes()));
    out.extend_from_slice(&r.finish());
    let out = text(out);
    assert!(out.contains(&email_value()), "same part joined: {out}");
    // 不同 part（parts.1）不拼：尾部原样放出
    let ev3 = format!(
        "data: {{\"candidates\":[{{\"content\":{{\"parts\":[{{\"text\":\"{head}\"}}]}}}}]}}\n\n"
    );
    let ev4 = format!(
        "data: {{\"candidates\":[{{\"content\":{{\"parts\":[{{\"text\":\"x\"}},{{\"text\":\"{tailc}\"}}]}}}}]}}\n\n"
    );
    let mut r = StreamRestorer::new(&e);
    let mut out = r.push(ev3.as_bytes());
    out.extend_from_slice(&r.push(ev4.as_bytes()));
    out.extend_from_slice(&r.finish());
    let out = text(out);
    assert!(
        out.contains(head) && out.contains(tailc),
        "not joined: {out}"
    );
}

/// 流中断（finish）放出全部扣留内容，不丢字节。
#[test]
fn finish_releases_held() {
    let mut e = engine();
    let p = e.mask("EMAIL", &email_value());
    // 后缀截断在半截（restore 的 8 位后缀正则匹配不上，只够 braceTail
    // 扣留）：这样的尾部等下一个事件补齐，流中断则原样放出
    let body = p.trim_start_matches("{{").trim_end_matches("}}");
    let part = &body[..body.len() - 4];
    let ev = ["data: {\"delta\":{\"text\":\"mail {{", part, "\"}}\n\n"].concat();
    let mut r = StreamRestorer::new(&e);
    let out = r.push(ev.as_bytes());
    assert!(out.is_empty(), "held: {:?}", text(out));
    let rest = text(r.finish());
    let out = text(out);
    assert_eq!(format!("{out}{rest}"), ev, "byte-complete");
}

/// 多字节字符切在块边界。
#[test]
fn multibyte_across_chunks() {
    let mut e = engine();
    let p = e.mask("EMAIL", &email_value());
    let phrase = format!("联系 {p} 处理");
    let stream = format!("data: {{\"delta\":{{\"text\":\"{phrase}\"}}}}\n\n");
    // 找一个多字节字符中间切开
    let bytes = stream.as_bytes();
    let cut = bytes
        .windows(3)
        .position(|w| w[0] & 0xe0 == 0xc0 || w[0] & 0xf0 == 0xe0)
        .unwrap()
        + 1;
    let mut r = StreamRestorer::new(&e);
    let mut out = r.push(&bytes[..cut]);
    out.extend_from_slice(&r.push(&bytes[cut..]));
    out.extend_from_slice(&r.finish());
    let out = text(out);
    assert!(out.contains(&email_value()), "{out}");
    assert_eq!(out, stream.replace(&p, &email_value()));
}

/// CRLF 事件与注释行（: keepalive）。
#[test]
fn crlf_and_comments() {
    let mut e = engine();
    let p = e.mask("EMAIL", &email_value());
    let stream =
        format!(": keepalive\r\n\r\ndata: {{\"delta\":{{\"text\":\"a {p}\"}}}}\r\n\r\n: ping\n\n");
    let mut r = StreamRestorer::new(&e);
    let mut out = r.push(stream.as_bytes());
    out.extend_from_slice(&r.finish());
    let out = text(out);
    assert!(out.contains(": keepalive\r\n\r\n"), "{out}");
    assert!(out.contains(&email_value()), "{out}");
}

/// 非 JSON 的 data 行：整事件按纯文本还原。
#[test]
fn non_json_data_restored_as_text() {
    let mut e = engine();
    let p = e.mask("EMAIL", &email_value());
    let stream = format!("data: [DONE]\n\ndata: plain {p} text\n\n");
    let mut r = StreamRestorer::new(&e);
    let mut out = r.push(stream.as_bytes());
    out.extend_from_slice(&r.finish());
    let out = text(out);
    assert!(out.contains("[DONE]"), "{out}");
    assert!(out.contains(&email_value()), "{out}");
}

// ---- partial_tail / brace_tail / word_tail 单元 ----

#[test]
fn brace_tail_shapes() {
    let e = engine();
    let kinds = e.kinds().clone();
    assert_eq!(partial_tail("call me at {{", &kinds), 2);
    assert_eq!(partial_tail("at {{EMAIL_a", &kinds), 9); // "{{EMAIL_a"
    assert_eq!(partial_tail("at {{EMAIL_abcd234", &kinds), 15);
    assert_eq!(partial_tail("at {{EMAIL_abcd2345}", &kinds), 17);
    // 完整占位符之后没有 }} 结束：不算部分
    assert_eq!(partial_tail("x {{EMAIL_abcd2345}}", &kinds), 0);
    // 太长不像占位符
    let long = ["{{", &"A".repeat(MAX_PLACEHOLDER + 4)].concat();
    assert_eq!(partial_tail(&long, &kinds), 0);
}

#[test]
fn word_tail_needs_known_kind() {
    let mut e = engine();
    let _ = e.mask("PHONE", "13800138000");
    let kinds = e.kinds().clone();
    assert!(kinds.contains("PHONE"));
    assert_eq!(word_tail("dial PHO", &kinds), 3);
    assert_eq!(word_tail("dial PHONE_a", &kinds), 7);
    // 非 kind 开头的大写词不扣留
    assert_eq!(word_tail("dial XY", &kinds), 0);
    // 小写开头不扣留
    assert_eq!(word_tail("dial phone", &kinds), 0);
}

// ---- 属性测试：任意切分不变性（清单验收） ----

fn stream_case() -> String {
    let mut e = engine();
    let p1 = e.mask("EMAIL", &email_value());
    let p2 = e.mask("SECRET", "Zx9qw7Pl2Km4");
    [
        "event: message\n",
        "data: {\"type\":\"delta\",\"delta\":{\"text\":\"mail ",
        &p1,
        " now\"}}\n\n",
        "data: {\"type\":\"delta\",\"delta\":{\"text\":\"key ",
        &p2,
        " 中文字符\"}}\n\n",
        ": keepalive\n\n",
        "data: [DONE]\n\n",
    ]
    .concat()
}

proptest! {
    /// 完整响应在任意字节位置切成两块：输出与一次喂入完全相同。
    #[test]
    fn prop_two_chunk_split(cut in 1usize..1200) {
        let e = engine();
        let stream = stream_case();
        let cut = cut.min(stream.len() - 1);
        let whole = {
            let mut r = StreamRestorer::new(&e);
            let mut out = r.push(stream.as_bytes());
            out.extend_from_slice(&r.finish());
            text(out)
        };
        let split = {
            let mut r = StreamRestorer::new(&e);
            let mut out = r.push(&stream.as_bytes()[..cut]);
            out.extend_from_slice(&r.push(&stream.as_bytes()[cut..]));
            out.extend_from_slice(&r.finish());
            text(out)
        };
        prop_assert_eq!(whole, split);
    }

    /// 切成任意多块（每块 1..=7 字节）：仍与一次喂入相同。
    #[test]
    fn prop_many_small_chunks(sizes in proptest::collection::vec(1usize..7, 10..400)) {
        let e = engine();
        let stream = stream_case();
        let whole = {
            let mut r = StreamRestorer::new(&e);
            let mut out = r.push(stream.as_bytes());
            out.extend_from_slice(&r.finish());
            text(out)
        };
        let mut r = StreamRestorer::new(&e);
        let mut out = Vec::new();
        let mut at = 0usize;
        for n in sizes {
            if at >= stream.len() { break; }
            let end = (at + n).min(stream.len());
            out.extend_from_slice(&r.push(&stream.as_bytes()[at..end]));
            at = end;
        }
        if at < stream.len() {
            out.extend_from_slice(&r.push(&stream.as_bytes()[at..]));
        }
        out.extend_from_slice(&r.finish());
        let out = text(out);
        prop_assert_eq!(whole, out);
    }
}

/// 扣留事件 + 下一事件还原中 panic（注入点在引擎调用内、扣留已被取走
/// 之后）：可用性优先下输出 = 扣留事件原文 + 出错事件 + 其后原始字节。
#[cfg(feature = "test-hooks")]
fn held_then_fault_case(e: &mut PlaceholderEngine) -> (String, String, String) {
    let p = e.mask("EMAIL", &email_value());
    let (head, tail) = p.split_at(p.len() / 2);
    let ev1 = format!("data: {{\"delta\":{{\"text\":\"见 {head}\"}}}}\n\n");
    let ev2 = format!("data: {{\"delta\":{{\"text\":\"{tail} 吧\"}}}}\n\n");
    let ev3 = "data: {\"delta\":{\"text\":\"收尾😀\"}}\n\ndata: [DONE]\n\n".to_string();
    (ev1, ev2, ev3)
}

#[cfg(feature = "test-hooks")]
#[test]
fn fault_after_held_event_releases_its_raw_bytes() {
    let mut e = engine();
    let (ev1, ev2, ev3) = held_then_fault_case(&mut e);
    let mut r = StreamRestorer::new(&e);
    r.set_fault_event(2);
    let first = r.push(ev1.as_bytes());
    assert!(first.is_empty(), "ev1 held: {:?}", text(first));
    // 块 2 = ev2 + ev3 的前半（切在 emoji 中间）
    let rest = format!("{ev2}{ev3}");
    let cut = ev2.len() + ev3.find('😀').unwrap() + 1;
    let mut out = r.push(&rest.as_bytes()[..cut]);
    assert!(r.engine_failed());
    out.extend_from_slice(&r.push(&rest.as_bytes()[cut..]));
    out.extend_from_slice(&r.finish());
    assert_eq!(out, format!("{ev1}{ev2}{ev3}").into_bytes(), "no dup/loss");
}

#[cfg(feature = "test-hooks")]
#[test]
fn fail_closed_releases_nothing_after_fault() {
    let mut e = engine();
    let (ev1, ev2, ev3) = held_then_fault_case(&mut e);
    let mut r = StreamRestorer::new(&e);
    r.set_fail_closed(true);
    r.set_fault_event(2);
    assert!(r.push(ev1.as_bytes()).is_empty());
    assert!(r.push(format!("{ev2}{ev3}").as_bytes()).is_empty());
    assert!(r.engine_failed());
    assert!(r.push(b"data: more\n\n").is_empty());
    assert!(r.finish().is_empty());
}

/// finish 的剩余缓冲还原 panic：可用性优先按原始字节放出，并报告失败。
#[cfg(feature = "test-hooks")]
#[test]
fn fault_in_finish_releases_raw_buffer() {
    let e = engine();
    let mut r = StreamRestorer::new(&e);
    r.set_fault_event(1);
    let partial = "data: {\"delta\":{\"text\":\"半截 😀".as_bytes();
    assert!(r.push(partial).is_empty());
    assert!(!r.engine_failed());
    assert_eq!(r.finish(), partial);
    assert!(r.engine_failed());

    let mut r = StreamRestorer::new(&e);
    r.set_fail_closed(true);
    r.set_fault_event(1);
    assert!(r.push(partial).is_empty());
    assert!(r.finish().is_empty());
    assert!(r.engine_failed());
}
