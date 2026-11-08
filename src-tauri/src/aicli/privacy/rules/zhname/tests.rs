//! E-08 测试：zhname 移植 + 接线 + 语料评估。
//!
//! Magpie `zhname_test.go` 的移植说明：
//! - `TestChineseNames` / `TestChineseWordsAreNoNames` / `TestMaoliIsAMargin`
//!   以 `person=[]`（无模型、开关打开）移植；
//! - 依赖模型的用例**未移植**，原因见 `model_only_cases_are_left`：
//!   `TestChineseNamesByModelOnlyNicks`（NamesByModel 路径，我方无模型）、
//!   `TestModelWordsAreNoNames` 与 `TestModelNamesBesideWords`（断言的是
//!   「模型把词当姓名/找到的姓名在词旁」时的行为，我方没有模型输入）、
//!   `TestChineseNamesNeedModel` 中「无模型不遮」半句（我方语义是开关打开
//!   就遮，`off_by_default_and_personal_required` 覆盖对应行为）。

use crate::aicli::privacy::placeholder::PlaceholderEngine;
use crate::aicli::privacy::rules::redact::{mask, MaskOptions};

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
        detect_cjk_names: true,
        ..Default::default()
    }
}

/// TestChineseNames 的无模型移植：遮盖这些姓名（keep 部分不动）。
#[test]
fn chinese_names_by_cues() {
    let mut e = engine();
    let cases: &[(&str, &[&str], &[&str])] = &[
        (
            "帮我写封信给张伟，他住在北京市朝阳区建国路88号",
            &["张伟"],
            &["，他住在", "建国路"],
        ),
        (
            "请把合同发给王芳，她下周一到上海。",
            &["王芳"],
            &["，她下周一到上海"],
        ),
        (
            "李娜和刘洋昨天开会讨论了项目进度。",
            &["李娜", "刘洋"],
            &["和", "昨天开会"],
        ),
        (
            "客户陈建国反馈导出功能在他电脑上失败了。",
            &["陈建国"],
            &["反馈导出"],
        ),
        ("客户张三反馈导出失败", &["张三"], &["反馈"]),
        ("联系人：赵丽颖，手机", &["赵丽颖"], &[]),
        (
            "提交者是欧阳娜娜，审核人是司马光。",
            &["欧阳娜娜", "司马光"],
            &["审核人是"],
        ),
        ("张伟先生，您好！感谢您的来信。", &["张伟"], &["先生，您好"]),
        ("我叫黄晓明，是这个仓库的维护者。", &["黄晓明"], &["我叫"]),
        (
            "给小王发个消息，说老李明天不来了。",
            &["小王", "老李"],
            &["发个消息", "明天不来了"],
        ),
        ("马云说过，今天很残酷。", &["马云"], &["说过"]),
        (
            "王老板说这个需求下周要上线，邓总明天要来看演示。",
            &["王老板", "邓总"],
            &["明天要来"],
        ),
        ("请帮我回复一下孙经理的邮件", &["孙经理"], &["的邮件"]),
        ("抄送给吴敏和郑浩。", &["吴敏", "郑浩"], &[]),
        ("负责人：许文强", &["许文强"], &[]),
        ("我是郭靖，想咨询一下退款的问题。", &["郭靖"], &["想咨询"]),
        ("问一下张伟，这段谁来写", &["张伟"], &["问一下"]),
        ("张伟说明天来改。", &["张伟"], &["说明天"]),
        ("客户张伟已经到了。", &["张伟"], &["已经到了"]),
        ("这张图发给张伟。", &["张伟"], &["发给"]),
        ("### 张伟发了个消息", &["张伟"], &["发了个消息"]),
        ("我和张伟，还有他。", &["张伟"], &["我和"]),
        ("这件事。 李娜说过。", &["李娜"], &["说过"]),
        ("客户方圆反馈导出失败", &["方圆"], &["反馈导出"]),
        ("联系人：高歌", &["高歌"], &[]),
    ];
    for (text, names, keep) in cases {
        let (out, count) = mask(text, &opts(), &mut e);
        assert!(count >= names.len(), "{text}: {out}");
        for n in *names {
            assert!(!out.contains(n), "{text}: {n} 未遮: {out}");
        }
        for k in *keep {
            assert!(out.contains(k), "{text}: {k} 被遮了: {out}");
        }
    }
}

/// TestChineseNames 的反例部分 + TestChineseWordsAreNoNames：这些词不是姓名。
#[test]
fn chinese_words_are_no_names() {
    let mut e = engine();
    let words = [
        "文案",
        "文档",
        "文风",
        "方案",
        "高度",
        "安全",
        "马上",
        "程序",
        "林间",
        "关键",
        "项目",
        "成本",
        "段落",
        "颜色",
        "查询",
        "时区",
        "季度",
        "边框",
        "简历",
        "游戏",
        "严格",
        "曾经",
        "谢谢",
        "解析",
        "付费",
        "边界",
        "应该",
        "尚可",
        "毛利率",
        "应用层",
        "关键词",
        "黄绿色",
    ];
    let shapes = [
        "改一下{}",
        "检查一下{}。",
        "{}已经好了",
        "{}明天给你",
        "{}说明",
        "{}他们定",
        "请{}",
        "发给{}",
        "{}：",
        "把{}换成英文",
        "提取这段文本的{}",
    ];
    for w in words {
        for sh in shapes {
            let text = sh.replace("{}", w);
            let (out, count) = mask(&text, &opts(), &mut e);
            assert_eq!(count, 0, "{text}: {out}");
        }
    }
    let cases: &[(&str, &[&str])] = &[
        ("这个函数的作用是把高度转换成像素。", &["高度"]),
        ("王国的军队在黄河边扎营。", &["王国", "黄河"]),
        ("我们需要一个更高效的排序算法。", &["高效"]),
        ("请帮我修复这个 bug，张开的括号没有闭合。", &["张开"]),
        ("给张量赋值之前先检查形状。", &["张量"]),
        ("在建国路上，路边有一家店。", &["建国路", "路边"]),
        ("把方法和文件名都改一下", &["方法", "文件"]),
        ("给老师发个消息", &["老师"]),
        ("和平时一样，周五发版。", &["周五", "和平"]),
        (
            "方总结一下：说明文档要写清楚安装步骤。",
            &["方总结", "说明"],
        ),
        (
            "运行测试的时候，内存一直在涨。签名和工具调用都会保留。",
            &["签名和工具"],
        ),
        ("### 关键发现：5 组 bot/人类重复 PR", &["关键发现"]),
        ("`gh pr view` 易触发 GraphQL rate limit", &["易触发"]),
        ("\n\n先读判断和边界，仅当来源", &["和边界，"]),
        ("打开文件之前，边界检查失败。", &["边界检查"]),
        ("帮我改一下这段文案", &["文案"]),
        ("润色一下文案。", &["文案"]),
        ("检查一下文案，看看有没有错别字", &["文案"]),
        ("文案：首页标题换成「更快的网关」", &["文案"]),
        ("把首页文案换成英文", &["文案"]),
        ("### 文案\n\n- 文案和图片都要改", &["文案"]),
        ("文案已经改好了，你再看看。", &["文案"]),
        ("文案明天给你，图片今天先发。", &["文案"]),
        ("文案说明要简短，不超过两行。", &["文案说明"]),
        ("文案他们说下午定稿", &["文案他们"]),
        ("这张图发给文案。", &["文案"]),
        ("天下方圆百里", &["方圆"]),
        ("江山如画", &["江山"]),
    ];
    for (text, keep) in cases {
        let (out, count) = mask(text, &opts(), &mut e);
        assert_eq!(count, 0, "{text}: {out}");
        for k in *keep {
            assert!(out.contains(k), "{text}: {k} 被遮了: {out}");
        }
    }
}

/// TestMaoliIsAMargin：毛利是毛利率的毛利，不是人（模型路径除外）。
#[test]
fn maoli_is_a_margin() {
    let mut e = engine();
    for text in [
        "毛利低",
        "毛利低说",
        "我是毛利低",
        "这个产品毛利低说明定价有问题",
        "我是毛利低的那个品类负责人",
        "毛利低说明成本高",
        "告诉毛利低的门店先别补货",
        "毛利率下降了",
        "毛利额是多少",
        "看一下毛利润",
    ] {
        let (out, count) = mask(text, &opts(), &mut e);
        assert_eq!(count, 0, "{text}: {out}");
    }
}

/// 依赖模型的 Magpie 用例未移植——这些是它们所断言的「无提示词时保留」
/// 行为（我方语义：没有提示词就没有姓名；模型标记的那部分不存在）。
#[test]
fn model_only_cases_are_left() {
    let mut e = engine();
    // 田中太郎 / 单独的「刘洋」/「王芳，她下周一」/ 空格分开的「张 伟」：
    // 在 Magpie 里由模型标记；无模型、无提示词时按规则保留
    for text in [
        "田中太郎さんに連絡してください。",
        "刘洋",
        "王芳，她下周一",
        "转交张 伟处理",
        "// author: 周杰伦",
        "感谢李老师的帮助",
        "感谢江山老师的帮助",
    ] {
        let (out, count) = mask(text, &opts(), &mut e);
        assert_eq!(count, 0, "{text}: {out}");
    }
}

/// 接线：默认关闭、personal 关闭时不生效；开启后遮为 PERSON。
#[test]
fn off_by_default_and_personal_required() {
    let mut e = engine();
    let text = "客户陈建国反馈导出失败";
    let off = MaskOptions {
        secrets: true,
        personal: true,
        detect_cjk_names: false,
        ..Default::default()
    };
    let (out, count) = mask(text, &off, &mut e);
    assert_eq!(count, 0, "{out}");
    let no_personal = MaskOptions {
        secrets: true,
        personal: false,
        detect_cjk_names: true,
        ..Default::default()
    };
    let (out, count) = mask(text, &no_personal, &mut e);
    assert_eq!(count, 0, "{out}");
    let (out, count) = mask(text, &opts(), &mut e);
    assert_eq!(count, 1, "{out}");
    assert!(out.contains("{{PERSON_"), "{out}");
    assert_eq!(out.matches("{{PERSON_").count(), 1, "{out}");
    // kinds 包含 PERSON：流式 word_tail 识别 {{PERSON_ 半截占位符的前提
    assert!(e.kinds().contains("PERSON"), "kinds has PERSON");
    // 还原闭环
    let restored = crate::aicli::privacy::walker::restore_json(&out, &e);
    assert_eq!(restored, text, "roundtrip");
}

/// 流式：姓名占位符切在两个事件之间，客户端最后收到原名。
#[test]
fn person_placeholder_split_across_events() {
    use crate::aicli::privacy::stream::StreamRestorer;
    let mut e = engine();
    let _ = mask("客户陈建国反馈导出失败", &opts(), &mut e);
    let p = e.mask("PERSON", "陈建国");
    let (head, tail) = p.split_at(p.len() / 2);
    let ev1 = format!("data: {{\"delta\":{{\"content\":\"客户 {head}\"}}}}\n\n");
    let ev2 = format!("data: {{\"delta\":{{\"content\":\"{tail} 反馈\"}}}}\n\n");
    let mut r = StreamRestorer::new(&e);
    let mut out = r.push(ev1.as_bytes());
    assert!(out.is_empty(), "held: {:?}", String::from_utf8_lossy(&out));
    out.extend_from_slice(&r.push(ev2.as_bytes()));
    out.extend_from_slice(&r.finish());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("陈建国"), "{text}");
    assert!(!text.contains("{{PERSON_"), "{text}");
}

// ---- 语料评估（两种方式；[验收] 以较低值为准） ----

#[derive(serde::Deserialize)]
struct CorpusLine {
    text: String,
    /// 应被遮的姓名（可为空）。
    names: Vec<String>,
}

fn load_corpus() -> Vec<CorpusLine> {
    let raw = include_str!("../../../../../tests/fixtures/privacy/zhname_corpus.jsonl");
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("corpus line parses"))
        .collect()
}

/// 一条语料的 PERSON 片段与标注比对：TP=范围一致；FP=多遮；范围不一致的
/// 标注记 FN。
fn score_line(text: &str, names: &[String], spans: &[(usize, usize)]) -> (u32, u32, u32) {
    let mut labeled: Vec<(usize, usize)> = Vec::new();
    for name in names {
        if name.is_empty() {
            continue;
        }
        let mut from = 0usize;
        while let Some(i) = text[from..].find(name.as_str()) {
            labeled.push((from + i, from + i + name.len()));
            from += i + name.len();
        }
    }
    let mut tp = 0u32;
    let mut fp = 0u32;
    for span in spans {
        if labeled.contains(span) {
            tp += 1;
        } else {
            fp += 1;
        }
    }
    let missed = labeled.iter().filter(|l| !spans.contains(*l)).count() as u32;
    (tp, fp, missed)
}

/// 该文本里被遮成 PERSON 的原文片段（字节范围）。
fn person_spans(text: &str, e: &mut PlaceholderEngine) -> Vec<(usize, usize)> {
    let (out, _) = mask(text, &opts(), e);
    let mut spans = Vec::new();
    let mut used: Vec<usize> = Vec::new(); // 原文中已对位的起点
    let mut rest = out.as_str();
    while let Some(i) = rest.find("{{PERSON_") {
        let end = rest[i..].find("}}").unwrap() + i + 2;
        let placeholder = &rest[i..end];
        let Some(value) = e.restore(placeholder) else {
            rest = &rest[end..];
            continue;
        };
        // 在原文中按序对位（遮盖不改变未遮部分的顺序）
        let mut from = 0usize;
        while let Some(j) = text[from..].find(value) {
            let at = from + j;
            if !used.contains(&at) {
                used.push(at);
                spans.push((at, at + value.len()));
                break;
            }
            from = at + value.len();
        }
        rest = &rest[end..];
    }
    spans.sort_unstable();
    spans
}

/// 两种方式评估：每条新引擎 / 共用一个引擎按顺序（known 记忆会把误报
/// 固化到后续请求里）。断言较低值 ≥ 95%。
#[test]
fn zhname_corpus_precision() {
    let corpus = load_corpus();
    assert!(corpus.len() >= 200, "corpus has {} lines", corpus.len());

    // 方式一：每条语料一个新引擎（单条精确率）
    let (mut tp1, mut fp1, mut fn1) = (0u32, 0u32, 0u32);
    for line in &corpus {
        let mut e = engine();
        let spans = person_spans(&line.text, &mut e);
        let (tp, fp, f) = score_line(&line.text, &line.names, &spans);
        tp1 += tp;
        fp1 += fp;
        fn1 += f;
    }
    let p1 = tp1 as f64 / (tp1 + fp1) as f64;
    let r1 = tp1 as f64 / (tp1 + fn1) as f64;

    // 方式二：共用一个引擎按顺序（真实会话的累积效应）
    let mut shared = engine();
    let (mut tp2, mut fp2, mut fn2) = (0u32, 0u32, 0u32);
    for line in &corpus {
        let spans = person_spans(&line.text, &mut shared);
        let (tp, fp, f) = score_line(&line.text, &line.names, &spans);
        tp2 += tp;
        fp2 += fp;
        fn2 += f;
    }
    let p2 = tp2 as f64 / (tp2 + fp2) as f64;
    let r2 = tp2 as f64 / (tp2 + fn2) as f64;

    eprintln!(
        "zhname 语料 {} 条：方式一 P={p1:.4} R={r1:.4}（TP={tp1} FP={fp1} FN={fn1}）；方式二（共用引擎）P={p2:.4} R={r2:.4}（TP={tp2} FP={fp2} FN={fn2}）",
        corpus.len()
    );
    let lower = p1.min(p2);
    assert!(
        lower >= 0.95,
        "精确率 {lower:.4}（方式一 {p1:.4}，方式二 {p2:.4}）低于 95%：按验收要求需隐藏开关"
    );
}
