//! 日期词的等价形式匹配（Magpie `dateword.go`，@f11bf27）。
//!
//! 被遮盖的词若是一个日期（1990-05-12），则同一天的所有写法（1990/5/12、
//! 1990年5月12日、19900512、May 12, 1990）都按词匹配为同值。

use std::collections::HashMap;
use std::sync::Mutex;

use regex::Regex;

use super::{FoundBy, Span};

/// 日期分隔（Magpie `dateSep`）。
const DATE_SEP: &str = r"(?:[ \t]*[-/.、／－][ \t]*|[ \t])";

const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// 月份名或其前三位（Magpie `monthOf`），0 表示不是。
fn month_of(name: &str) -> usize {
    let l = name.to_lowercase();
    if l == "sept" {
        return 9;
    }
    for (i, m) in MONTH_NAMES.iter().enumerate() {
        let ml = m.to_lowercase();
        if l == ml || (l.len() == 3 && ml.starts_with(&l)) {
            return i + 1;
        }
    }
    0
}

/// y-m-d 是否真实日历日（Magpie `realDate`）。无 chrono 依赖场景下手写。
fn days_in_month(y: i32, m: usize) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn real_date(y: i32, m: usize, d: usize) -> bool {
    (1000..=2999).contains(&y) && (1..=12).contains(&m) && d >= 1 && d as u32 <= days_in_month(y, m)
}

struct ParsedDate {
    y: i32,
    m: usize,
    d: usize,
}

/// 词 w 是否一个日期（Magpie `parseDateWord`）。
fn parse_date_word(w: &str) -> Option<ParsedDate> {
    let n = |s: &str| -> i32 { s.parse().unwrap_or(0) };

    let ymd = Regex::new(&format!(
        r"^(\d{{4}}){sep}(\d{{1,2}}){sep}(\d{{1,2}})\.?$",
        sep = DATE_SEP
    ))
    .unwrap();
    let cjk = Regex::new(
        r"^(\d{4})[ \t]*[年년][ \t]*(\d{1,2})[ \t]*[月월][ \t]*(\d{1,2})[ \t]*[日号號일]?$",
    )
    .unwrap();
    let dmy = Regex::new(&format!(
        r"^(\d{{1,2}}){sep}(\d{{1,2}}){sep}(\d{{4}})$",
        sep = DATE_SEP
    ))
    .unwrap();
    let name_f =
        Regex::new(r"^([A-Za-z]+)\.?[ \t]+(\d{1,2})(?:st|nd|rd|th)?,?[ \t]+(\d{4})$").unwrap();
    let name_b =
        Regex::new(r"^(\d{1,2})(?:st|nd|rd|th)?(?:[ \t]+|-)([A-Za-z]+)\.?,?(?:[ \t]+|-)(\d{4})$")
            .unwrap();

    if let Some(g) = ymd.captures(w) {
        let p = ParsedDate {
            y: n(&g[1]),
            m: n(&g[2]) as usize,
            d: n(&g[3]) as usize,
        };
        return real_date(p.y, p.m, p.d).then_some(p);
    }
    if let Some(g) = cjk.captures(w) {
        let p = ParsedDate {
            y: n(&g[1]),
            m: n(&g[2]) as usize,
            d: n(&g[3]) as usize,
        };
        return real_date(p.y, p.m, p.d).then_some(p);
    }
    if let Some(g) = dmy.captures(w) {
        let (p1, p2) = (n(&g[1]) as usize, n(&g[2]) as usize);
        let y = n(&g[3]);
        let (m, d) = match (p1, p2) {
            (a, b) if a == b => (a, b),
            (a, b) if a > 12 && b <= 12 => (b, a),
            (a, b) if b > 12 && a <= 12 => (a, b),
            _ => return None, // 05/12/1990 两读
        };
        return real_date(y, m, d).then_some(ParsedDate { y, m, d });
    }
    if let Some(g) = name_f.captures(w) {
        let m = month_of(&g[1]);
        let p = ParsedDate {
            y: n(&g[3]),
            m,
            d: n(&g[2]) as usize,
        };
        return (m > 0 && real_date(p.y, p.m, p.d)).then_some(p);
    }
    if let Some(g) = name_b.captures(w) {
        let m = month_of(&g[2]);
        let p = ParsedDate {
            y: n(&g[3]),
            m,
            d: n(&g[1]) as usize,
        };
        return (m > 0 && real_date(p.y, p.m, p.d)).then_some(p);
    }
    None
}

/// 日期 y-m-d 的全部等价写法（Magpie `dateForms`）。
fn date_forms(y: i32, m: usize, d: usize, w: &str) -> String {
    let num = |v: usize| -> String {
        if v < 10 {
            format!("0?{v}")
        } else {
            v.to_string()
        }
    };
    let (ys, ms, ds) = (y.to_string(), num(m), num(d));
    let full = MONTH_NAMES[m - 1];
    let abbr = if m == 9 {
        "Sept|Sep"
    } else {
        &MONTH_NAMES[m - 1][..3]
    };
    let name = format!("(?i:{full}|(?:{abbr})\\.?)");
    let ord = "(?:st|nd|rd|th)?";
    let mut forms = vec![
        format!("{ys}{sep}{ms}{sep}{ds}", sep = DATE_SEP),
        format!("{ys}[ \\t]*年[ \\t]*{ms}[ \\t]*月[ \\t]*{ds}(?:[ \\t]*[日号號])?"),
        format!("{ys}[ \\t]*년[ \\t]*{ms}[ \\t]*월[ \\t]*{ds}(?:[ \\t]*일)?"),
        format!("{ys}{}{}", 100 + m).replace("1", "") + &format!("{:02}", d),
        format!("{name}[ \\t]+{ds}{ord},?[ \\t]+{ys}"),
        format!("{ds}{ord}(?:[ \\t]+|-){name},?(?:[ \\t]+|-){ys}"),
    ];
    // 前日后月日仅当无歧义
    if d > 12 || d == m {
        forms.push(format!("{ds}{sep}{ms}{sep}{ys}", sep = DATE_SEP));
        if d != m {
            forms.push(format!("{ms}{sep}{ds}{sep}{ys}", sep = DATE_SEP));
        }
    }
    let quoted = regex::escape(w);
    format!("(?:{}|{quoted})", forms.join("|"))
}

/// 词 → 等价日期正则缓存（Magpie `dateWords`）；None 表示不是日期。
fn date_word(w: &str) -> Option<Regex> {
    static CACHE: Mutex<Option<HashMap<String, Option<Regex>>>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(hit) = cache.get(w) {
        return hit.clone();
    }
    let compiled = parse_date_word(w).map(|p| Regex::new(&date_forms(p.y, p.m, p.d, w)).unwrap());
    cache.insert(w.to_owned(), compiled.clone());
    compiled
}

/// 日期两侧是否无更长数字/版本号（Magpie `dateWhole`）。
fn date_whole(s: &[u8], a: usize, b: usize) -> bool {
    let is_digit = |c: u8| c.is_ascii_digit();
    if a > 0 {
        if is_digit(s[a - 1]) || s[a - 1] == b'.' && a > 1 && is_digit(s[a - 2]) {
            return false;
        }
        // 字母紧贴月份名（xMay）不是日期；CJK 文字后可以
        let prev = decode_last_char(&s[..a]);
        let this = s[a] as char;
        if this.is_ascii_alphabetic() && prev.is_alphabetic() && !is_cjk_like(prev) {
            return false;
        }
    }
    if b < s.len() && (is_digit(s[b]) || s[b] == b'.' && b + 1 < s.len() && is_digit(s[b + 1])) {
        return false;
    }
    true
}

fn decode_last_char(bytes: &[u8]) -> char {
    let mut start = bytes.len().saturating_sub(1);
    while start > 0 && bytes[start] & 0xC0 == 0x80 {
        start -= 1;
    }
    std::str::from_utf8(&bytes[start..])
        .ok()
        .and_then(|s| s.chars().next())
        .unwrap_or('\0')
}

fn is_cjk_like(c: char) -> bool {
    matches!(c,
        '\u{4e00}'..='\u{9fff}'   // Han
        | '\u{3040}'..='\u{309f}' // Hiragana
        | '\u{30a0}'..='\u{30ff}' // Katakana
        | '\u{ac00}'..='\u{d7af}' // Hangul
    )
}

/// s 中该日期词的所有等价出现（Magpie `dateSpans`）。
/// None 表示 w 不是日期。
pub(crate) fn date_spans(s: &str, w: &str) -> Option<Vec<Span>> {
    let re = date_word(w)?;
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    for m in re.find_iter(s) {
        if date_whole(bytes, m.start(), m.end()) {
            out.push(Span {
                start: m.start(),
                end: m.end(),
                kind: "TERM",
                by: FoundBy::Word,
                bare: false,
            });
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_date_words() {
        let cases = [
            ("1990-05-12", Some((1990, 5, 12))),
            ("1990/5/12", Some((1990, 5, 12))),
            ("1990.5.12", Some((1990, 5, 12))),
            ("1990年5月12日", Some((1990, 5, 12))),
            ("19900512", None), // 八位数字本身不算
            ("13/05/1990", Some((1990, 5, 13))),
            ("05/13/1990", Some((1990, 5, 13))),
            ("05/12/1990", None), // 两读
            ("May 12, 1990", Some((1990, 5, 12))),
            ("12 May 1990", Some((1990, 5, 12))),
            ("Sept 5, 2020", Some((2020, 9, 5))),
            ("1990-02-30", None), // 不是日历日
            ("1990-13-01", None),
        ];
        for (word, want) in cases {
            let got = parse_date_word(word).map(|p| (p.y, p.m, p.d));
            assert_eq!(got, want.map(|(y, m, d)| (y as i32, m, d)), "word={word}");
        }
    }

    #[test]
    fn equivalent_forms_found() {
        let word = "1990-05-12";
        let text = "born 1990/5/12, again 1990年5月12日, id 19900512, said May 12, 1990 ok";
        let spans = date_spans(text, word).expect("is a date");
        assert!(spans.len() >= 4, "found {} spans", spans.len());
        for span in &spans {
            assert_eq!(span.kind, "TERM");
        }
    }

    #[test]
    fn version_is_not_a_date_part() {
        let text = "version 1990.5.12.1 here";
        // 1990.5.12.1 中 1990.5.12 后跟 .1 → dateWhole 拒绝
        let word = "1990.5.12";
        let spans = date_spans(text, word).expect("is a date");
        assert!(spans.is_empty(), "date inside version must not match");
    }

    #[test]
    fn non_date_word() {
        assert!(date_spans("hello 1990-05-12", "hello").is_none());
    }
}
