//! E-05：流式还原（Magpie `internal/redact/writer.go` 与 `redact.go` 的
//! `partialTail`/`braceTail`/`wordTail`，@f11bf27）。
//!
//! 流式响应按 SSE 事件切帧、逐事件还原：事件文本结尾可能是占位符的开头
//!（`{{API_` 在一个 delta、`KEY_k3v9x2mq}}` 在下一个）——这样的事件被扣留，
//! 下一事件上下文（事件名 + eventCtx）与路径相同则把尾部移过去继续拼；
//! 不同则原样放出。不丢不增，代理拼出的文本与一次性还原一致。
//!
//! 本模块是与 HTTP 解耦的核心状态机（E-06 中间件接线）；`push` 接收任意
//! 切分的字节块（含跨块的多字节字符），`finish` 在流结束时放出全部扣留。

use std::collections::HashSet;

use super::placeholder::PlaceholderEngine;
use super::walker::{self, keep, walk};

/// Magpie `maxKind`（rules.go）：kind 截断长度，占位符最长的形态由此而来。
pub(crate) const MAX_KIND: usize = 24;
/// Magpie `maxPlaceholder`：`{{` + kind(≤24) + `_` + 8 位 + `}}`。
const MAX_PLACEHOLDER: usize = 2 + MAX_KIND + 1 + 8 + 2;

/// 扣留的事件（Magpie `held`）：结尾可能是占位符开头。
struct Held {
    ctx: String,
    path: String,
    tail: String,
    /// 事件原样 / 去掉尾部后。
    full: String,
    cut: String,
    /// 事件从上游收到的原始字节（引擎失败转透传时放出，保证字节完整）。
    raw: String,
}

/// 引擎来源：借用（单测/同作用域）或共享（E-06 中间件，'static 流里持有
/// `Arc<RwLock<…>>`，每次 push 短暂加锁，不跨 .await）。
enum EngineSource<'a> {
    Borrowed(&'a PlaceholderEngine),
    Shared(std::sync::Arc<std::sync::RwLock<PlaceholderEngine>>),
}

/// 流式还原器：喂入任意切分的响应字节，取出已还原的输出。
pub struct StreamRestorer<'a> {
    source: EngineSource<'a>,
    /// 待切帧的原始字节（含跨块的 UTF-8 序列与半截事件）。
    buf: Vec<u8>,
    /// buf 前这么多字节已确认不含事件结束空行：长事件分很多块到达时
    /// 不必每块都从头重扫（否则耗时随块数平方增长）。
    scanned: usize,
    pending: Option<Held>,
    /// 引擎在事件还原中 panic 后：后续 push 原样返回字节（透传）。
    passthrough: bool,
    /// 本次生命周期内是否发生过引擎失败（调用方据此计数/断流）。
    failed: bool,
    /// 失败后不放出任何上游字节（隐私优先）：失败事件、扣留与剩余缓冲
    /// 全部丢弃，后续 push 返回空。默认 false（可用性优先：原样透传）。
    fail_closed: bool,
    /// test-hooks：在第 N 个事件还原时注入 panic（0 = 不注入）。
    #[cfg(feature = "test-hooks")]
    fault_event: u32,
    /// 已处理的事件数（注入计数）。
    #[cfg(feature = "test-hooks")]
    events_seen: u32,
}

impl<'a> StreamRestorer<'a> {
    pub fn new(engine: &'a PlaceholderEngine) -> Self {
        Self {
            source: EngineSource::Borrowed(engine),
            buf: Vec::new(),
            scanned: 0,
            pending: None,
            passthrough: false,
            failed: false,
            fail_closed: false,
            #[cfg(feature = "test-hooks")]
            fault_event: 0,
            #[cfg(feature = "test-hooks")]
            events_seen: 0,
        }
    }

    /// 共享引擎版（E-06 中间件用）：可放进 'static 的响应体流。
    /// push/finish 内部短暂加读锁（restore 只读），不跨 .await。
    pub fn shared(engine: std::sync::Arc<std::sync::RwLock<PlaceholderEngine>>) -> Self {
        Self {
            source: EngineSource::Shared(engine),
            buf: Vec::new(),
            scanned: 0,
            pending: None,
            passthrough: false,
            failed: false,
            fail_closed: false,
            #[cfg(feature = "test-hooks")]
            fault_event: 0,
            #[cfg(feature = "test-hooks")]
            events_seen: 0,
        }
    }

    /// 在引擎的一次短暂访问内运行 f（Borrowed 直接用；Shared 加读锁）。
    fn with_engine<R>(&self, f: impl FnOnce(&PlaceholderEngine) -> R) -> R {
        match &self.source {
            EngineSource::Borrowed(engine) => f(engine),
            EngineSource::Shared(arc) => {
                let guard = arc.read().unwrap_or_else(|p| p.into_inner());
                f(&guard)
            }
        }
    }

    /// 喂入一块字节，返回此刻可写出的字节（注释行、完整事件的还原结果、
    /// 放出的扣留）。多字节字符跨块安全：不完整的 UTF-8 尾部留在 buf，
    /// 全程不经过有损转换。
    ///
    /// **每个事件的还原单独 catch_unwind**（E-06）：引擎在事件还原中 panic
    /// 时，本次 push 中已还原的事件照常输出；之前被扣留的事件、出错事件
    /// 及 buf 剩余字节按**原始字节**放出，restorer 进入透传（后续 push 原样
    /// 返回），字节不重不漏。`set_fail_closed(true)` 时这些原始字节一律
    /// 不放出，后续 push 返回空。
    pub fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        if self.passthrough {
            if self.fail_closed {
                return Vec::new();
            }
            return chunk.to_vec();
        }
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            let c = comment_lines(&self.buf);
            if c > 0 {
                out.extend_from_slice(&self.buf[..c]);
                self.buf.drain(..c);
                self.scanned = 0;
                continue;
            }
            // 从上次扫到的位置往回留 3 字节重叠，接住跨块的 `\r\n\r\n`。
            let from = self.scanned.saturating_sub(3);
            let Some((i, n)) = event_end(&self.buf[from..]).map(|(i, n)| (from + i, n)) else {
                self.scanned = self.buf.len();
                break;
            };
            let ev: Vec<u8> = self.buf[..i + n].to_vec();
            self.buf.drain(..i + n);
            self.scanned = 0;
            match std::str::from_utf8(&ev) {
                // 完整事件以 \n 结束，多字节字符不会跨过事件边界切坏；
                // 非 UTF-8（二进制）事件原样放出不尝试还原
                Ok(ev) => {
                    // event() 在调用引擎之前就会取走扣留事件：panic 时它的
                    // 输出只在 event() 的局部变量里，所以先在这里记下原文。
                    let held_raw = self.pending.as_ref().map(|p| p.raw.clone());
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.event(ev)))
                    {
                        Ok(processed) => out.extend_from_slice(&processed.into_bytes()),
                        Err(_payload) => {
                            self.fail();
                            if !self.fail_closed {
                                // 之前被扣留的事件 + 出错事件 + buf 剩余（含下一个
                                // 事件的开头）：按原始字节放出，不重不漏
                                if let Some(raw) = held_raw {
                                    out.extend_from_slice(raw.as_bytes());
                                }
                                out.extend_from_slice(ev.as_bytes());
                                out.append(&mut self.buf);
                            }
                            self.buf.clear();
                            self.scanned = 0;
                            break;
                        }
                    }
                }
                Err(_) => out.extend_from_slice(&ev),
            }
        }
        out
    }

    /// 是否发生过引擎失败（事件还原 panic；一次生命周期内粘滞）。
    pub fn engine_failed(&self) -> bool {
        self.failed
    }

    /// test-hooks：在第 N 个事件的还原中注入 panic。
    #[cfg(feature = "test-hooks")]
    pub fn set_fault_event(&mut self, event: u32) {
        self.fault_event = event;
    }

    /// 隐私优先：失败后不放出任何上游字节（见 `fail_closed` 字段）。
    pub fn set_fail_closed(&mut self, on: bool) {
        self.fail_closed = on;
    }

    /// 引擎失败：进入透传（或隐私优先下的静默），扣留内容作废。
    fn fail(&mut self) {
        log::error!("隐私引擎流式还原 panic：转入透传或断流（按失败模式）");
        self.failed = true;
        self.passthrough = true;
        self.pending = None;
    }

    /// test-hooks：本事件是否注入（第 N 个事件起）。注入点放在引擎调用
    /// 内部、扣留事件已被取走之后，与真实 panic 的位置一致。
    #[cfg(feature = "test-hooks")]
    fn next_event_faults(&mut self) -> bool {
        self.events_seen += 1;
        self.fault_event != 0 && self.events_seen >= self.fault_event
    }

    /// 流结束（错误、取消同样调用）：放出全部扣留，剩余缓冲按纯文本还原。
    pub fn finish(&mut self) -> Vec<u8> {
        if self.passthrough {
            // 透传态不持有内容；剩余 buf 已在失败瞬间放出或丢弃
            self.buf.clear();
            return Vec::new();
        }
        let mut out = Vec::new();
        if let Some(p) = self.pending.take() {
            out.extend_from_slice(p.full.as_bytes());
        }
        if !self.buf.is_empty() {
            let rest = String::from_utf8_lossy(&self.buf).into_owned();
            #[cfg(feature = "test-hooks")]
            let inject = self.next_event_faults();
            let restored = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.with_engine(|engine| {
                    #[cfg(feature = "test-hooks")]
                    if inject {
                        panic!("privacy fault injection: stream finish");
                    }
                    walker::restore(&rest, false, engine)
                })
            }));
            match restored {
                Ok(restored) => out.extend_from_slice(restored.as_bytes()),
                Err(_payload) => {
                    self.fail();
                    if !self.fail_closed {
                        out.extend_from_slice(&self.buf);
                    }
                }
            }
        }
        self.buf.clear();
        self.scanned = 0;
        out
    }

    /// 还原一个完整事件并返回应写出的内容，或扣留它。
    fn event(&mut self, ev: &str) -> String {
        #[cfg(feature = "test-hooks")]
        let inject = self.next_event_faults();
        let mut out = String::new();
        let Some((name, data, pre, post)) = split_event(ev) else {
            // 不是单 data 行 JSON 的事件：放出扣留，整段按纯文本还原
            out.push_str(&self.release());
            out.push_str(&self.with_engine(|engine| {
                #[cfg(feature = "test-hooks")]
                if inject {
                    panic!("privacy fault injection: stream event");
                }
                walker::restore(ev, false, engine)
            }));
            return out;
        };
        if self.pending.is_some() && waiting(&name, &data) {
            // Magpie 76f00399c（#5287）：两段文字之间的 keepalive 先放出，扣留的那段继续等
            // 下文。原先 keepalive 上下文不同，扣留段带着占位符开头原样放出，下一段带着剩下
            // 一半，两段都还原不了；Claude Code 的 Write/Edit 会把 {{SECRET_…}} 写进文件。
            let restored = self.with_engine(|engine| walker::restore_json(&data, engine));
            out.push_str(&rebuild(pre.as_str(), &restored, post.as_str()));
            return out;
        }
        let ctx = format!("{name}|{}", event_ctx(&data));
        // 上一事件结尾的占位符开头：上下文与路径相同则移过来继续
        let mut carry = String::new();
        let mut at = String::new();
        if let Some(p) = self.pending.take() {
            if p.ctx == ctx && has_path(&data, &p.path) {
                out.push_str(&p.cut);
                carry = p.tail;
                at = p.path;
            } else {
                out.push_str(&p.full);
            }
        }
        let mut typ = walker::type_of(&data);
        if typ.is_empty() {
            typ = name.to_string();
        }
        let mut tail_path = String::new();
        let mut tail = String::new();
        let mut cut = String::new();
        let restored = self.with_engine(|engine| {
            #[cfg(feature = "test-hooks")]
            if inject {
                panic!("privacy fault injection: stream event");
            }
            let kinds = engine.kinds();
            walk(&data, &mut |path, key, s| {
                let mut s = s.to_string();
                if path == at && !carry.is_empty() {
                    s = format!("{carry}{s}");
                }
                if keep(key, &s) {
                    return s;
                }
                // 先找尾部再还原：最后的花括号还在下一个事件里的占位符，
                // 不会先被还原掉一半
                let n = partial_tail(&s, kinds);
                if n > 0 {
                    tail_path = path.to_string();
                    tail = s[s.len() - n..].to_string();
                    cut = walker::restore(&s[..s.len() - n], as_json_keyed(key, &typ), engine);
                }
                walker::restore(&s, as_json_keyed(key, &typ), engine)
            })
            .unwrap_or_else(|| data.clone())
        });
        let full = rebuild(pre.as_str(), &restored, post.as_str());
        if tail.is_empty() {
            out.push_str(&full);
            return out;
        }
        let cut_data = walk(&restored, &mut |path, _k, s| {
            if path == tail_path {
                return cut.clone();
            }
            s.to_string()
        })
        .unwrap_or(restored);
        self.pending = Some(Held {
            ctx,
            path: tail_path,
            tail,
            full,
            cut: rebuild(pre.as_str(), &cut_data, post.as_str()),
            raw: ev.to_string(),
        });
        out
    }

    fn release(&mut self) -> String {
        match self.pending.take() {
            Some(p) => p.full,
            None => String::new(),
        }
    }
}

/// waitEvents（Magpie 76f00399c）：模型慢时流里发的、与回答内容无关的事件——Anthropic 的
/// ping（会夹在工具参数中间发）、Responses 的 keepalive、response.in_progress 和
/// response.queued。
const WAIT_EVENTS: &[&str] = &[
    "ping",
    "keepalive",
    "response.in_progress",
    "response.queued",
];

/// waiting（Magpie）：事件 name、数据 data 是 WAIT_EVENTS 之一（看 data 的 type，没有时看事件名）。
fn waiting(name: &str, data: &str) -> bool {
    let mut typ = walker::type_of(data);
    if typ.is_empty() {
        typ = name.to_string();
    }
    WAIT_EVENTS.contains(&typ.as_str())
}

/// commentLines（Magpie）：b 开头的整行注释（`: keepalive`）与空行的
/// 长度——b 从事件开头开始，它们不属于任何事件；事件间的注释照原样流出，
/// 否则代理后的 agent 在厂商静默思考时什么都收不到（#2402）。
fn comment_lines(b: &[u8]) -> usize {
    let mut n = 0usize;
    while n < b.len() && (b[n] == b':' || b[n] == b'\n' || b[n] == b'\r') {
        let Some(i) = b[n..].iter().position(|&c| c == b'\n') else {
            break;
        };
        if b[n] == b'\r' && i != 1 {
            break;
        }
        n += i + 1;
    }
    n
}

/// eventEnd（Magpie）：b 里第一个完整事件的结尾位置与结束空行的长度
///（`\n\n` 或 `\r\n\r\n`）。
fn event_end(b: &[u8]) -> Option<(usize, usize)> {
    let i = find(b, b"\n\n");
    let j = find(b, b"\r\n\r\n");
    match (i, j) {
        (None, None) => None,
        (Some(j_), None) => Some((j_, 2)),
        (None, Some(i_)) => Some((i_, 4)),
        (Some(i_), Some(j_)) => {
            if j_ < i_ {
                Some((j_, 4))
            } else {
                Some((i_, 2))
            }
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn rebuild(pre: &str, data: &str, post: &str) -> String {
    let mut out = String::with_capacity(pre.len() + data.len() + post.len());
    out.push_str(pre);
    out.push_str(data);
    out.push_str(post);
    out
}

/// splitEvent（Magpie）：找事件里唯一一行装着 JSON 的 data 行——JSON 前、
/// JSON、JSON 后。event 名、data 超过一行、或 data 不以 { [ 开头都算
/// 不是（整段按纯文本还原）。
fn split_event(ev: &str) -> Option<(String, String, String, String)> {
    let mut name = String::new();
    let mut data_line: Option<(usize, usize)> = None; // data 行的（起点，含换行的终点）
    let mut pos = 0usize;
    while pos < ev.len() {
        let end = match ev[pos..].find('\n') {
            Some(i) => pos + i + 1,
            None => ev.len(),
        };
        let t = ev[pos..end].trim_end_matches(['\r', '\n']);
        if let Some(rest) = t.strip_prefix("event:") {
            name = rest.trim().to_string();
        } else if t.starts_with("data:") {
            if data_line.is_some() {
                return None; // data 超过一行
            }
            data_line = Some((pos, end));
        }
        pos = end;
    }
    let (dstart, _dend) = data_line?;
    let line_end = dstart + ev[dstart..].find('\n').unwrap_or(ev.len() - dstart);
    let t = ev[dstart..line_end].trim_end_matches(['\r', '\n']);
    let mut ds = 5usize;
    if t.len() > 5 && t.as_bytes()[5] == b' ' {
        ds = 6;
    }
    let d = &t[ds..];
    if d.is_empty() || (!d.starts_with('{') && !d.starts_with('[')) {
        return None;
    }
    Some((
        name,
        d.to_string(),
        ev[..dstart + ds].to_string(),
        ev[dstart + t.len()..].to_string(),
    ))
}

/// eventCtx（Magpie）：说明事件接着哪段文本的上下文——type/item_id 的
/// 值，加上原始字节里的 index/output_index/content_index（Gemini 事件
/// 没有 type/index 字段，路径本身按候选与 part 下标区分）。
fn event_ctx(data: &str) -> String {
    let mut b = String::new();
    walk(data, &mut |path, _k, s| {
        if path == "type" || path == "item_id" {
            b.push_str(path);
            b.push('=');
            b.push_str(s);
            b.push(';');
        }
        s.to_string()
    });
    for k in ["\"index\":", "\"output_index\":", "\"content_index\":"] {
        if let Some(i) = data.find(k) {
            let j = i + k.len();
            let bb = data.as_bytes();
            let mut e = j;
            while e < bb.len() && (bb[e].is_ascii_digit() || bb[e] == b' ') {
                e += 1;
            }
            b.push_str(k);
            b.push_str(&data[j..e]);
            b.push(';');
        }
    }
    b
}

fn has_path(data: &str, path: &str) -> bool {
    let mut found = false;
    walk(data, &mut |p, _k, s| {
        if p == path {
            found = true;
        }
        s.to_string()
    });
    found
}

/// asJSON 按事件类型（Magpie `asJSON(key, eventType)`）。
fn as_json_keyed(key: &str, event_type: &str) -> bool {
    key == "arguments"
        || key == "partial_json"
        || key == "delta" && event_type.contains("arguments")
}

// ---- partialTail / braceTail / wordTail（Magpie redact.go） ----

/// partialTail（Magpie）：s 结尾有多少可能是一个占位符的开头——
/// {{API_、{{ phone_b5、PHONE_b5puzbfv}，或无花括号的最后一个是某 kind
/// 开头的词。shapeTail（合成键的尾部）在 Shape 模式接线前恒 0（偏差）。
pub(crate) fn partial_tail(s: &str, kinds: &HashSet<String>) -> usize {
    let n = brace_tail(s);
    if n > 0 {
        return n;
    }
    word_tail(s, kinds)
}

/// braceTail（Magpie）：从 `{` 或 `{{` 起的尾部——之后空格、占位符正文
/// 或其开头、再空格、至多一个 `}`。
fn brace_tail(s: &str) -> usize {
    let b = s.as_bytes();
    let Some(mut i) = (0..b.len()).rev().find(|&i| b[i] == b'{') else {
        return 0;
    };
    if i > 0 && b[i - 1] == b'{' {
        i -= 1;
    }
    let t = &s[i..];
    if t.len() >= MAX_PLACEHOLDER + 4 || t.contains("}}") {
        return 0;
    }
    let body = t.trim_start_matches('{');
    let body = body.trim_start_matches([' ', '\t']);
    if body.is_empty() {
        return t.len(); // {、{{、{{␠
    }
    let c = body.as_bytes()[0];
    if !c.is_ascii_alphabetic() {
        return 0;
    }
    let mut j = 0usize;
    let bb = body.as_bytes();
    while j < bb.len() && walker::is_word_byte(bb[j]) {
        j += 1;
    }
    let rest = body[j..].trim_start_matches([' ', '\t']);
    if !rest.is_empty() && rest != "}" {
        return 0;
    }
    if j < bb.len() {
        // 词后还有内容：词须正好是一个完整的占位符正文
        let word = &body[..j];
        match walker::placeholder_body().find(word) {
            Some(m) if m.as_str() == word => {}
            _ => return 0,
        }
    }
    t.len()
}

/// wordTail（Magpie）：s 的最后一个词可能是模型不带花括号写的占位符
/// 正文——是本引擎造过占位符的某个 kind（加 `_`）的前缀，或 kind 前缀
/// 加至多 8 位 base32。只首字母大写的词才扣留。
fn word_tail(s: &str, kinds: &HashSet<String>) -> usize {
    let b = s.as_bytes();
    let mut i = s.len();
    while i > 0 && walker::is_word_byte(b[i - 1]) {
        i -= 1;
    }
    let w = &s[i..];
    if w.is_empty() || w.len() > MAX_KIND + 9 || !b[i].is_ascii_uppercase() {
        return 0;
    }
    let u = w.to_uppercase();
    for k in kinds {
        let k = format!("{k}_");
        // w 是某 kind 的前缀（PHO、PHONE_b5pu）
        if k.starts_with(u.as_str()) {
            return w.len();
        }
        // w = kind + 至多 8 位 base32
        if u.starts_with(&k) {
            let rest = u[k.len()..].to_lowercase();
            if rest.len() <= 8
                && rest
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || (b'2'..=b'7').contains(&c))
            {
                return w.len();
            }
        }
    }
    0
}

#[cfg(test)]
mod tests;
