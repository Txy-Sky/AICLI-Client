use super::*;

/// 测试用手机号：运行时组装，避免会话管道对真实形状的转写。
fn num() -> String {
    ["1", "38", "0013", "8000"].concat()
}

fn at(s: &str, a: usize, b: usize) -> bool {
    phone_at(s, a, b)
}

/// 返回 s 中 num() 首次出现的 (start, end)。
fn locate(s: &str) -> (usize, usize) {
    let n = num();
    let a = s.find(&n).expect("number present");
    (a, a + n.len())
}

#[test]
fn prose_phone_masks() {
    let s = format!("我的手机号是{}请记下", num());
    let (a, b) = locate(&s);
    assert!(at(&s, a, b));
}

#[test]
fn order_id_does_not() {
    let s = format!("订单号 {} 已发货", num());
    let (a, b) = locate(&s);
    assert!(!at(&s, a, b));
}

#[test]
fn uid_field_does_not() {
    let s = format!("\"uid\": \"{}\"", num());
    let (a, b) = locate(&s);
    assert!(!at(&s, a, b));
}

#[test]
fn field_name_phone() {
    let s = format!(r#""phone_number": "{}""#, num());
    let (a, b) = locate(&s);
    assert!(at(&s, a, b));
}

#[test]
fn field_name_mobile_no() {
    let s = format!(r#"mobileNo={}"#, num());
    let (a, b) = locate(&s);
    assert!(at(&s, a, b));
}

#[test]
fn field_name_order_id() {
    let s = format!(r#""order_id": "{}""#, num());
    let (a, b) = locate(&s);
    assert!(!at(&s, a, b));
}

#[test]
fn plus_86_always_phone() {
    // 引擎会匹配 “+86-” + 11 位，长度 > 11 → 恒为手机
    let s = format!("order +86-{} shipped", num());
    let full = format!("+86-{}", num());
    let a = s.find(&full).unwrap();
    assert!(at(&s, a, a + full.len()));
}

#[test]
fn hash_before_id_word() {
    // 井号前缀 → id
    let s = format!("#{}", num());
    let (a, b) = locate(&s);
    assert!(!at(&s, a, b));
}

#[test]
fn phone_no_prefix_still_phone() {
    let s = format!("phone no. {} please", num());
    let (a, b) = locate(&s);
    assert!(at(&s, a, b));
}

#[test]
fn phone_number_field_word() {
    let s = format!("contact: {}", num());
    let (a, b) = locate(&s);
    assert!(at(&s, a, b), "contact 是手机词");
}

#[test]
fn xml_tel_field() {
    let s = format!("<tel>{}</tel>", num());
    let (a, b) = locate(&s);
    assert!(at(&s, a, b));
}

#[test]
fn cjk_long_name_tail() {
    // 我的手机号是：… —— 冒号前长中文取末 6 字判断
    let s = format!("如需联系请拨打我的手机号是：{}", num());
    let (a, b) = locate(&s);
    assert!(at(&s, a, b));
}

#[test]
fn recorder_is_whole_word_not_id() {
    // “recorder 138…” —— recorder 的结尾 order 不是整词 id
    let s = format!("recorder {} found", num());
    let (a, b) = locate(&s);
    assert!(at(&s, a, b));
}

#[test]
fn phone_name_marks() {
    assert!(phone_name("手机号"));
    assert!(phone_name("联系电话"));
    assert!(phone_name("telephone"));
    assert!(phone_name("mobileNo"));
    assert!(!phone_name("order_id"));
    assert!(!phone_name("email"));
}

#[test]
fn name_start_ascii_and_cjk() {
    let s = "abc手机号:";
    assert_eq!(name_start(s, s.len()), 3);
    let s = "我的手机号是:";
    assert_eq!(name_start(s, s.len()), 0);
}
