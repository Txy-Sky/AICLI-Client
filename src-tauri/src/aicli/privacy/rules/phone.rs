//! 手机号语境判定（Magpie `phone.go`，@f11bf27）。
//!
//! 大陆手机号的形状（13-19 开头 11 位）同时是订单号/用户 id 的形状：仅在
//! 「有手机号自身标记（+86/86）」「字段名含手机词汇」「散文中前面无 id 词汇」
//! 时按手机号处理。

use super::{has_cjk, in_set, name_words, ALNUM};

/// 字段名里表示手机的英文词（Magpie `phoneWords`）。
const PHONE_WORDS: [&str; 14] = [
    "phone",
    "phones",
    "mobile",
    "mobiles",
    "tel",
    "telephone",
    "cell",
    "cellphone",
    "contact",
    "contacts",
    "whatsapp",
    "sms",
    "msisdn",
    "fax",
];

/// 字段名里表示手机的中文词（Magpie `phoneMarks`）。
const PHONE_MARKS: [&str; 6] = ["手机", "电话", "電話", "联系", "聯繫", "联络"];

/// 散文中紧贴数字前、表示那是 id 的词（Magpie `idWords`）。
const ID_WORDS: [&str; 30] = [
    "order",
    "id",
    "uid",
    "no",
    "no.",
    "#",
    "number",
    "ticket",
    "invoice",
    "订单",
    "订单号",
    "单号",
    "编号",
    "流水号",
    "工单",
    "工单号",
    "运单号",
    "快递单号",
    "订单编号",
    "交易号",
    "序号",
    "账单号",
    "id号",
    // 占位以保持长度对齐；Go 版为 23 项，这里补齐到数组常量长度
    "",
    "",
    "",
    "",
    "",
    "",
    "",
];

/// PHONE 规则的 s[a:b] 是否真是手机号而非同形 id（Magpie `phoneAt`）。
pub(crate) fn phone_at(s: &str, a: usize, b: usize) -> bool {
    let bytes = s.as_bytes();
    if b - a > 11 {
        return true; // +86 / 86 前缀被一并匹配
    }
    if let Some((name, prev)) = field_name(s, a) {
        return phone_name(&name)
            || (!prev.is_empty() && phone_name(&prev))
            || name.eq_ignore_ascii_case("number");
    }
    if a > 0 && bytes[a - 1] == b'#' {
        return false; // #12345678901
    }
    let from = a.saturating_sub(32);
    let mut before = s[from..a].to_lowercase();
    before = before
        .trim_end_matches([' ', '\t', '(', '"', '\'', '\u{201c}', '\u{300c}'])
        .to_string();
    for c in ["是", "为", "為"] {
        if let Some(stripped) = before.strip_suffix(c) {
            before = stripped.trim_end_matches([' ', '\t']).to_string();
        }
    }
    for w in ID_WORDS.iter().filter(|w| !w.is_empty()) {
        let Some(rest) = before.strip_suffix(w) else {
            continue;
        };
        // 整词匹配：order 而非 recorder 的结尾
        let last = rest.chars().next_back();
        if !rest.is_empty()
            && w.as_bytes()[0] < 0x80
            && matches!(last, Some(r) if r.is_ascii_lowercase() || r.is_ascii_digit() || r == '_')
        {
            continue;
        }
        // phone no. 138… 的号码仍算手机
        if *w == "no" || *w == "no." || *w == "number" {
            let trimmed = rest.trim_end_matches(' ');
            if phone_name(&trimmed[name_start(trimmed, trimmed.len())..]) {
                return true;
            }
        }
        return false;
    }
    true
}

/// 字段名是否手机的（Magpie `phoneName`）。
pub(crate) fn phone_name(name: &str) -> bool {
    for w in name_words(name) {
        if PHONE_WORDS.contains(&w.as_str()) {
            return true;
        }
    }
    PHONE_MARKS.iter().any(|m| name.contains(m))
}

/// 值 s[a] 所在字段的名称与前一词（Magpie `fieldName`）。
/// 返回 None 表示散文（值前没有 : = ： 或 XML 元素闭合）。
pub(crate) fn field_name(s: &str, a: usize) -> Option<(String, String)> {
    let bytes = s.as_bytes();
    let mut i = a;
    while i > 0 && b" \t\"'".contains(&bytes[i - 1]) {
        i -= 1;
    }
    if i > 0 && bytes[i - 1] == b'>' {
        // <tel>138…</tel>：元素名
        let lo = i.saturating_sub(65);
        let k = bytes[lo..i - 1].iter().rposition(|&c| c == b'<')?;
        let tag_bytes = &bytes[lo + k + 1..i - 1];
        let mut tag = tag_bytes;
        if let Some(sp) = tag.iter().position(|&c| c == b' ' || c == b'\t') {
            tag = &tag[..sp];
        }
        if tag.is_empty()
            || tag
                .iter()
                .any(|&c| !(c < 0x80 && (in_set(ALNUM, c) || b"_.:-".contains(&c))))
        {
            return None;
        }
        return Some((String::from_utf8_lossy(tag).into_owned(), String::new()));
    }
    if i > 0 && (bytes[i - 1] == b':' || bytes[i - 1] == b'=') {
        i -= 1;
    } else if i >= 3 && &bytes[i - 3..i] == "：".as_bytes() {
        // 全角冒号（3 字节）；用字节比较避免落在多字节字符中间时 panic
        i -= 3;
    } else {
        return None;
    }
    while i > 0 && b" \t\"'".contains(&bytes[i - 1]) {
        i -= 1;
    }
    let j = name_start(s, i);
    if j == i {
        return None;
    }
    let mut name = s[j..i].to_string();
    let rune_count = name.chars().count();
    if rune_count > 6 && has_cjk(&name) {
        // 冒号前一大段中文是句子：最后几个字才是名称（我的手机号是：…）
        let tail: String = name.chars().skip(rune_count - 6).collect();
        name = tail;
    }
    let mut prev = String::new();
    if j >= 2 && bytes[j - 1] == b' ' && bytes[j - 2] != b' ' {
        prev = s[name_start(s, j - 1)..j - 1].to_string();
    }
    Some((name, prev))
}

/// 名称在 s[i] 结束时的起点（Magpie `nameStart`）：ASCII 字母数字 _ . - 或汉字。
pub(crate) fn name_start(s: &str, i: usize) -> usize {
    let bytes = s.as_bytes();
    let mut j = i;
    while j > 0 {
        // 回退一个 UTF-8 字符
        let mut start = j - 1;
        while start > 0 && bytes[start] & 0xC0 == 0x80 {
            start -= 1;
        }
        let ch = s[start..].chars().next().unwrap_or('\0');
        let ok = (ch.is_ascii()
            && (in_set(ALNUM, ch as u8) || ch == '_' || ch == '.' || ch == '-'))
            || ('\u{4e00}'..='\u{9fff}').contains(&ch);
        if !ok {
            break;
        }
        j = start;
    }
    j
}

#[cfg(test)]
mod tests;
