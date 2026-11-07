//! E-04：JSON 遍历与协议保护（Magpie `internal/redact/json.go`，@f11bf27，
//! 及 `redact.go` 中 escaped JSON 的三个函数 unescapedView/outsideEscapes/
//! inStrings）。
//!
//! 请求/响应体里的字符串按路径遍历与遮盖：
//! - 供应商封签的内容（Anthropic thinking 签名、Gemini thoughtSignature、
//!   Responses encrypted reasoning）只读整块，逐字节送回；
//! - 工具调用参数（tool_use.input、functionCall.args）里的键是工具自己的，
//!   一律扫描，即使里面出现同名字段 `signature`；
//! - 工具定义（tools/schemas/grammars）只有 description 遮盖；
//! - 未命中时输出与输入逐字节相同，不改键顺序。

use regex::Regex;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::OnceLock;

use super::placeholder::PlaceholderEngine;
use super::rules::known;
use super::rules::redact::{self, MaskOptions};
use super::rules::Span;

// ---- 保持原样的键（Magpie `kept`） ----

/// kept（Magpie）：这些键的字符串不动——id/名称/类别供应商要原样收，
/// 封签或编码的内容（签名、加密 reasoning、图片 data）也一样。
const KEPT: &[&str] = &[
    "signature",
    "encrypted_content",
    "data",
    "thoughtSignature",
    "thought_signature",
    "model",
    "id",
    "tool_use_id",
    "call_id",
    "item_id",
    "type",
    "role",
    "name",
    "namespace",
    "server_label",
    "server_name",
    "previous_response_id",
    "prompt_cache_key",
    "media_type",
    "mime_type",
    "mimeType",
    "url",
    "image_url",
    "file_id",
    "reasoning_effort",
    "effort",
    "stop_reason",
    "finish_reason",
    "status",
    "object",
    "event",
];

/// keep（Magpie）：键在 kept 里，或值是 data: URL。流式路径（E-05）使用。
#[allow(dead_code)]
fn keep(key: &str, s: &str) -> bool {
    KEPT.contains(&key) || s.starts_with("data:")
}

/// keepIn（Magpie）：请求里 path 处 key 下的字符串是否保持原样。
/// args（工具调用参数）里键是工具的：只有 data: 值保持；defs（代理为供应
/// 商定义的结构：tools、schema、grammar 等）里只有 description 遮盖。
pub(crate) fn keep_in(path: &str, key: &str, s: &str, args: &[String], defs: &[String]) -> bool {
    if s.starts_with("data:") {
        return true;
    }
    if under(path, args) {
        return false;
    }
    if under(path, defs) {
        return key != "description";
    }
    KEPT.contains(&key) || is_id(key)
}

/// isID（Magpie）：键装的是供应商要比对的标识符（tool_call_id 对 id、
/// approval_request_id 对请求）；GLM 的 call_+19 位能读成银行卡号，
/// 只遮一边两边就对不上了。
fn is_id(key: &str) -> bool {
    key == "id"
        || key == "ids"
        || key.ends_with("_id")
        || key.ends_with("_ids")
        || key.ends_with("Id")
        || key.ends_with("Ids")
}

/// under（Magpie）：path 在 roots 某个对象之内。
pub(crate) fn under(path: &str, roots: &[String]) -> bool {
    roots
        .iter()
        .any(|r| r.is_empty() || path == r || path.starts_with(&format!("{r}.")))
}

// ---- scan：封签块 / 工具参数 / 定义结构 ----

/// signatureKeys（Magpie）：供应商用来封签旁边内容的键。
const SIGNATURE_KEYS: &[&str] = &[
    "signature",
    "thoughtSignature",
    "thought_signature",
    "encrypted_content",
];

/// defKeys（Magpie）：请求自身由代理定义、供应商当结构读的键。
const DEF_KEYS: &[&str] = &[
    "tools",
    "tool_choice",
    "functions",
    "function_call",
    "toolConfig",
    "tool_config",
    "response_format",
    "text",
    "reasoning",
    "include",
    "output_format",
    "output_config",
    "generationConfig",
    "generation_config",
    "stream_options",
    "client_metadata",
];

/// defItems（Magpie）：Responses input 里携带工具的条目类型。
const DEF_ITEMS: &[&str] = &["additional_tools", "tool_search_output"];

/// scan（Magpie）：在请求体里找三类对象——
/// signed：供应商封签的（原样送回）；args：工具调用参数（键是工具的）；
/// defs：代理定义的结构。返回各自的路径（点连接）。
pub(crate) fn scan(body: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let empty = (Vec::new(), Vec::new(), Vec::new());
    let mut found = false;
    for k in SIGNATURE_KEYS
        .iter()
        .chain(["tool_use", "functionCall", "function_call"].iter())
    {
        found = found || body.contains(&format!("\"{k}\""));
    }
    for k in DEF_KEYS {
        found = found || body.contains(&format!("\"{k}\""));
    }
    if !found {
        return empty;
    }
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return empty;
    };
    let defs = definitions(&v);
    let mut signed = Vec::new();
    let mut args = Vec::new();
    fn visit(x: &Value, path: &str, key: &str, signed: &mut Vec<String>, args: &mut Vec<String>) {
        match x {
            Value::Object(m) => {
                if sealed(m) {
                    signed.push(path.to_string());
                    return;
                }
                let t = m.get("type").and_then(Value::as_str).unwrap_or("");
                for (k, c) in m {
                    if k == "input" && (t == "tool_use" || t == "server_tool_use")
                        || k == "args" && (key == "functionCall" || key == "function_call")
                    {
                        args.push(join(path, k));
                        continue;
                    }
                    visit(c, &join(path, k), k, signed, args);
                }
            }
            Value::Array(items) => {
                for (i, c) in items.iter().enumerate() {
                    visit(c, &join(path, &i.to_string()), key, signed, args);
                }
            }
            _ => {}
        }
    }
    visit(&v, "", "", &mut signed, &mut args);
    (signed, args, defs)
}

/// definitions（Magpie）：请求 v 里代理定义而非对模型说的路径——请求
/// 自身的 defKeys，加上 input 里 defItems 条目的 tools。只有它们在协议
/// 里的位置算数：更深处的 "tools"/"text"（消息里、工具参数里）是代理或
/// 模型写的。
fn definitions(v: &Value) -> Vec<String> {
    let Value::Object(m) = v else {
        return Vec::new();
    };
    let mut defs = Vec::new();
    for k in DEF_KEYS {
        if !m.contains_key(*k) {
            continue;
        }
        if *k == "text" {
            // Responses 的 text 是输出设置；提示词的 text（补全、Gemini
            // contents part）是字符串
            if !matches!(m[*k], Value::Object(_)) {
                continue;
            }
        }
        defs.push(k.to_string());
    }
    if let Some(items) = m.get("input").and_then(Value::as_array) {
        for (i, it) in items.iter().enumerate() {
            if let Some(im) = it.as_object() {
                let t = im.get("type").and_then(Value::as_str).unwrap_or("");
                if DEF_ITEMS.contains(&t) {
                    defs.push(join(&join("input", &i.to_string()), "tools"));
                }
            }
        }
    }
    defs
}

/// sealed（Magpie）：x 是供应商封签的块——带 Gemini thoughtSignature 的
/// part，或带签名/加密内容的 thinking、reasoning 块。别的对象里同名键
/// （文本块、请求本身）装的是代理的话。
fn sealed(x: &serde_json::Map<String, Value>) -> bool {
    let t = x.get("type").and_then(Value::as_str).unwrap_or("");
    for k in SIGNATURE_KEYS {
        let Some(s) = x.get(*k).and_then(Value::as_str) else {
            continue;
        };
        if s.is_empty() {
            continue;
        }
        if *k == "thoughtSignature"
            || *k == "thought_signature"
            || t == "thinking"
            || t == "reasoning"
            || t.starts_with("reasoning.")
        {
            return true;
        }
    }
    false
}

// ---- 字节保持的 JSON 遍历（Magpie `walk`） ----

/// walk（Magpie）：对 JSON 文档 b 里每个字符串值调用 f——路径（键与
/// 下标以点连接）与它所在的键——并把返回值写在原位。其余逐字节保留；
/// 没有变化时返回原文；不是 JSON 返回 None。
pub(crate) fn walk(b: &str, f: &mut dyn FnMut(&str, &str, &str) -> String) -> Option<String> {
    let mut w = Walker {
        in_: b.as_bytes(),
        out: Vec::new(),
        last: 0,
        changed: false,
        f,
    };
    let i = w.value(w.ws(0), "", "")?;
    if w.ws(i) != b.len() {
        return None;
    }
    if !w.changed {
        return Some(b.to_string());
    }
    w.out.extend_from_slice(&b.as_bytes()[w.last..]);
    Some(String::from_utf8(w.out).expect("walk output is utf-8"))
}

struct Walker<'a> {
    in_: &'a [u8],
    out: Vec<u8>,
    last: usize,
    changed: bool,
    f: &'a mut dyn FnMut(&str, &str, &str) -> String,
}

impl Walker<'_> {
    fn ws(&self, mut i: usize) -> usize {
        while i < self.in_.len() && matches!(self.in_[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        i
    }

    /// str（Magpie）：i 处字符串字面量的结尾（含引号）、是否含转义。
    fn str_end(&self, i: usize) -> Option<(usize, bool)> {
        let mut escaped = false;
        let mut j = i + 1;
        while j < self.in_.len() {
            match self.in_[j] {
                b'\\' => {
                    escaped = true;
                    j += 2;
                }
                b'"' => return Some((j + 1, escaped)),
                _ => j += 1,
            }
        }
        None
    }

    fn value(&mut self, i: usize, path: &str, key: &str) -> Option<usize> {
        if i >= self.in_.len() {
            return None;
        }
        match self.in_[i] {
            b'{' => self.object(i, path),
            b'[' => self.array(i, path, key),
            b'"' => self.string(i, path, key),
            _ => {
                // 数字 / true / false / null：读到分隔符为止（Magpie 不深校验）
                let mut j = i;
                while j < self.in_.len()
                    && !matches!(
                        self.in_[j],
                        b',' | b']' | b'}' | b' ' | b'\t' | b'\r' | b'\n'
                    )
                {
                    j += 1;
                }
                if j == i {
                    None
                } else {
                    Some(j)
                }
            }
        }
    }

    fn object(&mut self, i: usize, path: &str) -> Option<usize> {
        let mut i = self.ws(i + 1);
        if i < self.in_.len() && self.in_[i] == b'}' {
            return Some(i + 1);
        }
        loop {
            if i >= self.in_.len() || self.in_[i] != b'"' {
                return None;
            }
            let (end, esc) = self.str_end(i)?;
            let k = if esc {
                serde_json::from_slice::<String>(&self.in_[i..end]).ok()?
            } else {
                std::str::from_utf8(&self.in_[i + 1..end - 1])
                    .ok()?
                    .to_string()
            };
            i = self.ws(end);
            if i >= self.in_.len() || self.in_[i] != b':' {
                return None;
            }
            i = self.value(self.ws(i + 1), &join(path, &k), &k)?;
            i = self.ws(i);
            if i < self.in_.len() && self.in_[i] == b',' {
                i = self.ws(i + 1);
                continue;
            }
            if i < self.in_.len() && self.in_[i] == b'}' {
                return Some(i + 1);
            }
            return None;
        }
    }

    fn array(&mut self, i: usize, path: &str, key: &str) -> Option<usize> {
        let mut i = self.ws(i + 1);
        if i < self.in_.len() && self.in_[i] == b']' {
            return Some(i + 1);
        }
        let mut n = 0usize;
        loop {
            // 数组里的字符串挂在数组所在的键下（Magpie）
            i = self.value(i, &join(path, &n.to_string()), key)?;
            i = self.ws(i);
            if i < self.in_.len() && self.in_[i] == b',' {
                i = self.ws(i + 1);
                n += 1;
                continue;
            }
            if i < self.in_.len() && self.in_[i] == b']' {
                return Some(i + 1);
            }
            return None;
        }
    }

    fn string(&mut self, i: usize, path: &str, key: &str) -> Option<usize> {
        let (end, esc) = self.str_end(i)?;
        let s = if esc {
            serde_json::from_slice::<String>(&self.in_[i..end]).ok()?
        } else {
            std::str::from_utf8(&self.in_[i + 1..end - 1])
                .ok()?
                .to_string()
        };
        let t = (self.f)(path, key, &s);
        if t != s {
            self.changed = true;
            if self.out.is_empty() {
                // 遮盖后的 body 约是原大小（Magpie 的容量预留）
                self.out.reserve(self.in_.len() + self.in_.len() / 16 + 64);
            }
            self.out.extend_from_slice(&self.in_[self.last..i]);
            self.out.push(b'"');
            self.out.extend_from_slice(json_escape(&t).as_bytes());
            self.out.push(b'"');
            self.last = end;
        }
        Some(end)
    }
}

fn join(path: &str, k: &str) -> String {
    if path.is_empty() {
        k.to_string()
    } else {
        format!("{path}.{k}")
    }
}

/// jsonEscape（Magpie：Go `json.Encoder` + SetEscapeHTML(false)）：字符串
/// 进 JSON 引号之间的形态——`\"`、`\\`、`\n`/`\r`/`\t`，其余 <0x20 为
/// 小写十六进制 `\u00xx`，U+2028/U+2029 转义，其它原样。
pub(crate) fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    out
}

// ---- escaped JSON 文本（工具参数）的三个函数（Magpie redact.go） ----

/// unescapedView（Magpie）：把 s 读作供应商解开一层 JSON 转义后的样子，
/// 与 s 等长（\n 的 n 读作换行、\u 的 u 和数字读作断行；\" 和 C:\\Users
/// 保持原样）。
pub(crate) fn unescaped_view(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let mut b = s.as_bytes().to_vec();
    let mut i = 0usize;
    while i + 1 < b.len() {
        if b[i] != b'\\' {
            i += 1;
            continue;
        }
        i += 1;
        match b[i] {
            b'n' | b'b' | b'f' => b[i] = b'\n',
            b'r' => b[i] = b'\r',
            b't' => b[i] = b'\t',
            b'u' => {
                let end = (i + 5).min(b.len());
                b[i..end].fill(b'\n');
                i += 4;
            }
            _ => {}
        }
        i += 1;
    }
    String::from_utf8(b).expect("unescapedView replaces ascii only")
}

/// outsideEscapes（Magpie）：found 里没有值起止在 JSON 转义序列内部的
/// （\ 后的字母、\u00e9 的数字）：遮在那里会破坏 JSON。值退到转义的
/// 边界；退空的丢弃。
pub(crate) fn outside_escapes(s: &str, found: Vec<Span>) -> Vec<Span> {
    if found.is_empty() || !s.contains('\\') {
        return found;
    }
    let b = s.as_bytes();
    // esc[i]: i 在转义内部时为，否则 (0,0)
    let mut esc_from = vec![0usize; b.len() + 1];
    let mut esc_to = vec![0usize; b.len() + 1];
    let mut i = 0usize;
    while i < b.len() {
        if b[i] != b'\\' {
            i += 1;
            continue;
        }
        let n = if i + 1 < b.len() && b[i + 1] == b'u' {
            6
        } else {
            2
        };
        let end = (i + n).min(b.len());
        for j in i + 1..end {
            esc_from[j] = i + 1;
            esc_to[j] = end + 1;
        }
        i = end;
    }
    let mut out = Vec::with_capacity(found.len());
    for mut f in found {
        if esc_to[f.start] > 0 {
            f.start = esc_to[f.start] - 1;
        }
        if esc_from[f.end] > 0 {
            f.end = esc_from[f.end] - 1;
        }
        if f.start < f.end {
            out.push(f);
        }
    }
    out
}

/// inStrings（Magpie）：s 是 JSON 文本（工具调用参数）时，把 found 里
/// 每个值约束在它起始的那个字符串内：截到字符串的闭引号，或从里面第一
/// 个字符串开始。整个是裸值（"password": 12345）的保留，占位符带引号
/// 放回；跨结构（引号、冒号、大括号）的丢弃。
pub(crate) fn in_strings(s: &str, found: Vec<Span>) -> Vec<Span> {
    if found.is_empty() || serde_json::from_str::<Value>(s).is_err() {
        return found;
    }
    let b = s.as_bytes();
    let mut in_ = vec![0u32; b.len()];
    let mut n = 0u32;
    let mut i = 0usize;
    while i < b.len() {
        if b[i] != b'"' {
            i += 1;
            continue;
        }
        n += 1;
        let mut j = i + 1;
        while j < b.len() && b[j] != b'"' {
            in_[j] = n;
            if b[j] == b'\\' && j + 1 < b.len() {
                j += 1;
                in_[j] = n;
            }
            j += 1;
        }
        i = j + 1; // 跳过闭引号（Go 的 for i++）
    }
    let mut out = Vec::with_capacity(found.len());
    for mut f in found {
        let mut a = f.start;
        while a < f.end && in_[a] == 0 {
            a += 1;
        }
        if a == f.end {
            if bare_value(s, f.start, f.end) {
                f.bare = true;
                out.push(f);
            }
            continue;
        }
        let mut e = a;
        while e < f.end && in_[e] == in_[a] {
            e += 1;
        }
        f.start = a;
        f.end = e;
        out.push(f);
    }
    out
}

/// bareValue（Magpie）：字符串之外的 s[a:b] 是一个完整的裸值——对象里
///（: 之后）或数组里（[ 或 , 之后），后面跟 , } ]。
fn bare_value(s: &str, a: usize, b: usize) -> bool {
    let seg = &s[a..b];
    if a >= b || seg.bytes().any(|c| b" \t\r\n{}[]\",:".contains(&c)) {
        return false;
    }
    let bytes = s.as_bytes();
    let mut i = a as isize - 1;
    while i >= 0 && matches!(bytes[i as usize], b' ' | b'\t' | b'\r' | b'\n') {
        i -= 1;
    }
    let mut k = b;
    while k < bytes.len() && matches!(bytes[k], b' ' | b'\t' | b'\r' | b'\n') {
        k += 1;
    }
    i >= 0 && b":[".contains(&bytes[i as usize]) && (k == bytes.len() || b",}]".contains(&bytes[k]))
}

// ---- MaskJSON（Magpie） ----

/// MaskJSON（Magpie）：遮盖请求体里的字符串，返回结果与替换数。不是
/// JSON 的体按纯文本遮盖。
///
/// 供应商封签的块（thinking、Gemini thoughts、Responses 加密 reasoning）
/// 原样送回：请求别处遮过的值在这里变回占位符（vendor 当初写的正是它），
/// 加上之前遮过的已知值；其余不动。
///
/// 偏差（E-04，见 docs/aicli-evidence/E-04.txt）：Magpie 在 defs 上另跑
/// 模型检测（withModel），E-03 决定不移植 model.go，这里 defs 与正文用
/// 同一规则引擎；形状模式（Shape）的 restore 待 E-05/E-06 接线。
pub fn mask_json(body: &str, o: &MaskOptions, engine: &mut PlaceholderEngine) -> (String, usize) {
    let (signed, args, defs) = scan(body);
    let mut total = 0usize;
    let mut seen: Vec<(String, String)> = Vec::new(); // 值, 占位符, …

    let masked = |path: &str, key: &str, s: &str| {
        !keep_in(path, key, s, &args, &defs) && (signed.is_empty() || !under(path, &signed))
    };

    let Some(mut out) = walk(body, &mut |path, key, s| {
        if !masked(path, key, s) || under(path, &defs) {
            return s.to_string();
        }
        let (t, n) = if as_json(key, "") {
            redact::mask_escaped(s, o, engine, Some(&mut seen))
        } else {
            redact::mask_with(s, o, engine, Some(&mut seen))
        };
        total += n;
        t
    }) else {
        // 不是 JSON：按纯文本
        return redact::mask(body, o, engine);
    };
    // defs 在正文之后：正文里遮过的值在 defs 里也作为已知值遮盖
    if !defs.is_empty() {
        if let Some(out2) = walk(&out, &mut |path, key, s| {
            if !masked(path, key, s) || !under(path, &defs) {
                return s.to_string();
            }
            let (t, n) = redact::mask_with(s, o, engine, Some(&mut seen));
            total += n;
            t
        }) {
            out = out2;
        }
    }
    if signed.is_empty() {
        return (out, total);
    }
    // 封签块：把请求里遮过的值换回占位符（replacer），加上之前就遮过的
    // 已知值（known），其余不动
    let replacer = replacer_of(&seen);
    let out2 = walk(&out, &mut |path, key, s| {
        if keep_in(path, key, s, &args, &defs) || !under(path, &signed) {
            return s.to_string();
        }
        let t = replace_all(s, &replacer);
        if t != s {
            total += 1;
        }
        let found = {
            let idx = known::shared_index()
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            idx.spans(&t, |_kind| o.secrets)
        };
        let (t2, n) = redact::apply(&t, found, o, engine);
        total += n;
        t2
    })
    .unwrap_or(out);
    (out2, total)
}

/// replacerOf（Magpie）：pairs（值, 占位符, …）去重后按值长度降序稳定
/// 排列（同一位置最长者先）。
fn replacer_of(pairs: &[(String, String)]) -> Vec<(String, String)> {
    let mut have = HashSet::new();
    let mut ps: Vec<(String, String)> = Vec::new();
    for (v, p) in pairs {
        if have.insert(v.clone()) {
            ps.push((v.clone(), p.clone()));
        }
    }
    // 值长度降序（稳定），同一位置最长者先
    ps.sort_by_key(|p| std::cmp::Reverse(p.0.len()));
    ps
}

/// Go `strings.NewReplacer` 语义：每个位置最长的匹配先替换。
fn replace_all(s: &str, ps: &[(String, String)]) -> String {
    if ps.is_empty() {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    'outer: while i < b.len() {
        for (v, p) in ps {
            if b[i..].starts_with(v.as_bytes()) {
                out.push_str(p);
                i += v.len();
                continue 'outer;
            }
        }
        let step = s[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        out.push_str(&s[i..i + step]);
        i += step;
    }
    out
}

/// asJSON（Magpie）：key 下的字符串本身装着 JSON 文本（工具调用的
/// arguments，整块或流式片段），放回的值要转义。
fn as_json(key: &str, event_type: &str) -> bool {
    key == "arguments"
        || key == "partial_json"
        || key == "delta" && event_type.contains("arguments")
}

/// typeOf（Magpie）：JSON 对象自报的 type，"type" 键在键序里任何位置
/// 都行（Responses function_call_arguments.delta 的 delta 可能在 type 前）。
fn type_of(data: &str) -> String {
    let mut typ = String::new();
    let _ = walk(data, &mut |path, _key, s| {
        if path == "type" {
            typ = s.to_string();
        }
        s.to_string()
    });
    typ
}

// ---- RestoreJSON（Magpie） ----

/// RestoreJSON（Magpie）：把响应体里的占位符换回原值。
/// 非 JSON 体按纯文本还原。形状模式（Shape）的还原见 E-05/E-06。
pub fn restore_json(body: &str, engine: &PlaceholderEngine) -> String {
    if !body.contains("{{") {
        return body.to_string();
    }
    let typ = type_of(body);
    match walk(body, &mut |_path, key, s| {
        restore(s, as_json(key, &typ), engine)
    }) {
        Some(out) => out,
        None => restore(body, false, engine),
    }
}

/// placeholderBody（Magpie）：占位符去掉大括号的正文。
fn placeholder_body() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[A-Za-z][A-Za-z0-9_]*_[A-Za-z2-7]{8}").unwrap())
}

/// restore（Magpie）：把 s 里的占位符换回值。escaped 时值按 JSON 转义
/// 放回（装着 JSON 文本的字符串）。
pub fn restore(s: &str, escaped: bool, engine: &PlaceholderEngine) -> String {
    if engine.is_empty() {
        return s.to_string();
    }
    if !s.contains('_') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut last = 0usize;
    for m in placeholder_body().find_iter(s) {
        let (a, z) = (m.start(), m.end());
        if a < last || a > 0 && is_word_byte(b[a - 1]) || z < b.len() && is_word_byte(b[z]) {
            continue;
        }
        let p = canonical(m.as_str());
        let Some(v) = engine.restore(&p) else {
            continue;
        };
        let (a, z) = braces(s, a, z);
        if a < last {
            continue;
        }
        out.push_str(&s[last..a]);
        if escaped {
            out.push_str(&json_escape(v));
        } else {
            out.push_str(v);
        }
        last = z;
    }
    if last == 0 {
        return s.to_string();
    }
    out.push_str(&s[last..]);
    out
}

/// canonical（Magpie）：占位符正文不论大小写，规范成 Mask 写它的形态。
fn canonical(body: &str) -> String {
    let cut = body.len() - 8;
    let mut p = String::from("{{");
    p.push_str(&body[..cut].to_uppercase());
    p.push_str(&body[cut..].to_lowercase());
    p.push_str("}}");
    p
}

fn is_word_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// braces（Magpie）：把占位符正文两侧扩大到模型写它的 {{ }}（或丢了
/// 一边、或只有一边）上；单边单个花括号是文本自己的，不动。
fn braces(s: &str, a: usize, z: usize) -> (usize, usize) {
    let b = s.as_bytes();
    let mut la = a;
    let mut l = 0;
    while la > 0 && matches!(b[la - 1], b' ' | b'\t') {
        la -= 1;
    }
    while l < 2 && la > 0 && b[la - 1] == b'{' {
        la -= 1;
        l += 1;
    }
    let mut rz = z;
    let mut r = 0;
    while rz < b.len() && matches!(b[rz], b' ' | b'\t') {
        rz += 1;
    }
    while r < 2 && rz < b.len() && b[rz] == b'}' {
        rz += 1;
        r += 1;
    }
    let a = if l == 2 || l == 1 && r >= 1 { la } else { a };
    let z = if r == 2 || r == 1 && l >= 1 { rz } else { z };
    (a, z)
}

#[cfg(test)]
mod tests;
