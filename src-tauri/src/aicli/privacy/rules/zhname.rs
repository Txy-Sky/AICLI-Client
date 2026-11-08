//! 中文姓名检测（Magpie `internal/redact/zhname.go`，@f11bf27）。
//!
//! 模型漏掉很多中文姓名：张伟在「写封信给张伟，他住在…」、李娜、客户后的
//! 陈建国都不带标签，且模型找到的常常连着别的字（「王芳，她下周一」「张伟
//! 先生」）。这里的做法与读者找姓名一致：一个姓氏、后面一到两个字、周围
//! 有说明这是人的词（客户、我叫、给…发、…先生、…说）。周围什么都没有的
//! 姓氏不动：「王国的军队」「张开的括号」不是人。
//!
//! 我方没有模型（E-03 决定不移植 model.go）：开关打开时等价于
//! `zh_name_spans(s, person = [], nicks = false)`，只在 personal 类别开启时
//! 生效（对应 Magpie 的 `o.covers(catPersonal)`）。`zh_trim` 只用于模型
//! 找到的姓名，未移植。占位符类别 `PERSON`。

use std::collections::HashSet;
use std::sync::OnceLock;

use super::{FoundBy, Span};

/// 常见单字姓氏（Magpie `zhSurnames`）。
const ZH_SURNAMES: &str = "王李张刘陈杨黄赵吴周徐孙马朱胡郭何高林罗郑梁谢宋唐许韩冯邓曹彭曾肖萧田董袁潘于蒋蔡余杜叶程苏魏吕丁任沈姚卢姜崔钟谭陆汪范金石廖贾夏韦付傅方白邹孟熊秦邱江尹薛闫段雷侯龙史陶黎贺顾毛郝龚邵万钱严覃武戴莫孔向汤常温康施文牛樊葛邢安齐易乔伍庞颜倪庄聂章鲁岳翟殷詹申欧耿关兰焦俞左柳甘祝包宁尚符舒阮柯纪梅童凌毕单季裴霍涂成苗谷盛曲翁冉骆蓝路游辛靳管柴蒙鲍华喻祁蒲房滕屈饶解牟艾尤阳时穆农司卓古吉缪简车项连芦麦褚娄窦戚岑景党宫费卜冷晏席卫米柏宗瞿桂全佟应臧闵苟邬边卞姬师仇栾隋商刁沙荣巫寇桑郎甄丛仲虞敖巩佘池查麻苑迟邝";

/// 复姓（Magpie `zhCompound`）。
const ZH_COMPOUND: [&str; 26] = [
    "欧阳", "司马", "上官", "诸葛", "东方", "皇甫", "尉迟", "公孙", "慕容", "令狐", "司徒", "夏侯",
    "长孙", "宇文", "轩辕", "端木", "独孤", "南宫", "西门", "钟离", "呼延", "申屠", "闻人", "澹台",
    "拓跋", "万俟",
];

/// 小/老 可以前置的姓氏（小王、老李；Magpie `zhNickSurnames`）。
const ZH_NICK_SURNAMES: &str = "王李张刘陈杨黄赵吴周徐孙马朱胡郭何高林罗郑梁谢宋唐许韩冯邓曹彭曾肖萧田董袁潘蒋蔡杜叶程苏魏吕丁沈姚卢姜崔钟谭陆汪范金廖贾夏方邹孟熊秦江薛段雷侯史陶贺顾毛郝龚邵钱严武戴孔汤";

/// 不能做名字的字：姓名后句子接下去用的词（Magpie `zhNotGiven`）。
const ZH_NOT_GIVEN: &str = "的了在是和与跟同说他她它我你您们这那个不也都就还把被给到对从向为以等及或并而吗呢吧啊去来有没要会能让请已将再又很更最先后上下里外时年日号发打回写问讲叫找用做看想知道得地着过之其此每各某该所反表认提负";

/// 以姓氏开头的双字词，是词不是姓名（Magpie `zhWords`）。
const ZH_WORDS: &[&str] = &[
    "张量", "张开", "张贴", "高度", "高效", "高级", "高速", "高亮", "高手", "高兴", "王国", "王者",
    "黄河", "黄色", "方法", "方向", "方式", "方案", "方便", "方面", "程序", "程度", "温度", "白色",
    "金额", "金融", "江湖", "石头", "马上", "周期", "周末", "周围", "周边", "文件", "文本", "文档",
    "文字", "文章", "余额", "安全", "安装", "安排", "钱包", "叶子", "毛病", "毛利", "毛重", "毛坯",
    "毛巾", "毛衣", "万一", "雷达", "宋体", "常量", "常用", "常见", "向量", "向上", "全部", "全局",
    "全面", "史上", "易用", "武器", "时间", "时候", "明天", "明确", "明白", "任何", "任务", "任意",
    "华为", "管理", "简单", "简介", "范围", "许多", "许可", "包括", "包含", "关于", "关系", "关闭",
    "连接", "成功", "成为", "应用", "应该", "单位", "单元", "单独", "司机", "车辆", "车子", "路径",
    "路由", "林业", "龙头", "柏林", "米饭", "宁可", "师傅", "老师", "老板", "老婆", "老公", "老家",
    "小时", "小心", "小说", "小姐", "小孩", "小组", "周一", "周二", "周三", "周四", "周五", "周六",
    "周日", "周天", "文案", "文风", "文化", "文学", "文明", "文献", "文法", "文具", "方块", "方针",
    "方程", "方框", "关键", "关注", "关联", "关心", "关掉", "项目", "成本", "成员", "成果", "成绩",
    "成熟", "成就", "解决", "解析", "解释", "解码", "段落", "章节", "颜色", "蓝色", "查询", "查看",
    "查找", "单词", "单个", "单击", "包装", "包名", "时区", "时长", "时段", "时刻", "时代", "季度",
    "严格", "严重", "曾经", "麻烦", "安卓", "谷歌", "常规", "常数", "边框", "边界", "边缘", "付费",
    "付款", "路线", "简洁", "简历", "简化", "范例", "游戏", "游客", "商品", "商店", "商业", "左边",
    "左侧", "焦点", "毕竟", "尚未", "陆续", "连续", "连线", "林间", "林区", "谢谢", "顾客", "顾问",
    "房间", "房子", "高端", "高层", "古代", "古典", "农业", "司法", "施工", "卫生", "荣誉", "宗旨",
    "齐全", "凌晨", "冷却", "蒙版", "曲线", "黄金", "白天", "白板", "金钱", "石油", "夏天", "夏季",
    "唐朝", "宋朝", "秦朝", "申请", "符号", "景色", "费用", "舒服", "辛苦", "纪律", "席位", "车间",
    "管道", "何时", "何况", "于是", "余下", "杜绝", "罗列", "程式", "叶片", "涂层", "沙发", "池子",
    "巩固", "迟到", "仲裁", "钟点", "钟表", "向导", "史诗", "易懂", "马路", "马虎", "汤汁", "朱红",
    "阳光", "任职", "董事", "边距", "边栏", "尚需",
];

/// 词典里约 1500 个同类词（jieba v0.42.1，MIT；来源见 `zhdict.txt` 文件头
/// 与 THIRD_PARTY_NOTICES.md）。
const ZH_DICT_TEXT: &str = include_str!("zhdict.txt");

/// 颜色字：全是颜色字的词（黄绿）或以色结尾的词（黄绿色）是颜色，不是姓名。
/// 青不算——它做名字的机会一样多（王青）。
const ZH_COLOURS: &str = "红橙黄绿蓝紫黑白灰褐棕粉赤";

/// 让词变更长的字（关键→关键词、毛利→毛利率）：没有名字以这些结尾。
const ZH_NOUN_ENDS: &str =
    "词字性率值码型键框栏库器类表量度点线层期区图版式页数论员价计费件名夹帧戳";

/// 姓名前的角色词（客户陈建国、联系人：赵丽颖、我叫黄晓明；Magpie `zhRoles`）。
const ZH_ROLES: &[&str] = &[
    "客户",
    "联系人",
    "作者",
    "提交者",
    "审核人",
    "审批人",
    "负责人",
    "经办人",
    "收件人",
    "发件人",
    "收货人",
    "申请人",
    "报告人",
    "同事",
    "姓名",
    "名字",
    "署名",
    "签名",
    "我叫",
    "他叫",
    "她叫",
    "名叫",
    "叫做",
    "尊敬的",
    "亲爱的",
    "致",
    "@",
];

/// 后面接人的动词（给张伟、问李娜；Magpie `zhBefore`）。
const ZH_BEFORE: &[&str] = &[
    "给", "找", "问", "让", "告诉", "通知", "联系", "抄送", "转给", "交给", "一下", "叫", "和",
    "与", "跟", "说", "请", "我是", "他是", "她是",
];

/// 跟在人后面的词（张伟先生、马云说；Magpie `zhAfter`）。
const ZH_AFTER: &[&str] = &[
    "先生", "女士", "小姐", "老师", "同学", "同志", "经理", "总监", "主任", "师傅", "医生", "律师",
    "教授", "博士", "说", "讲", "表示", "认为", "反馈", "提到", "提交", "负责", "告诉", "回复",
    "发", "打", "明天", "今天", "昨天", "已经", "他", "她",
];

/// zhAfter 里任何主语（物也一样）后面都能接的（文案已经改好了）。
const ZH_ANY_AFTER: &[&str] = &[
    "提交", "负责", "回复", "明天", "今天", "昨天", "已经", "他", "她",
];

/// 「一下」前要接人的动词（问一下张伟）；其它动词后的「一下」接的是物
///（改一下文案）。
const ZH_ASK_VERBS: &[&str] = &[
    "问", "找", "叫", "喊", "催", "请", "联系", "通知", "告诉", "提醒", "回复", "答复", "拜访",
    "采访", "抄送", "转告", "感谢", "谢谢", "艾特", "@",
];

/// 称谓（Magpie `zhTitles`）。
const ZH_TITLES: &[&str] = &[
    "先生", "女士", "小姐", "老师", "同学", "同志", "经理", "总监", "主任", "师傅", "医生", "律师",
    "教授", "博士",
];

// ---- 集合与词典 ----

fn char_set(s: &str) -> HashSet<char> {
    s.chars().collect()
}

fn zh_dict() -> &'static HashSet<&'static str> {
    static DICT: OnceLock<HashSet<&'static str>> = OnceLock::new();
    DICT.get_or_init(|| {
        let mut set = HashSet::new();
        for line in ZH_DICT_TEXT.split('\n') {
            if line.starts_with('#') {
                continue;
            }
            for word in line.split_whitespace() {
                set.insert(word);
            }
        }
        set
    })
}

fn zh_words() -> &'static HashSet<&'static str> {
    static WORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    WORDS.get_or_init(|| ZH_WORDS.iter().copied().collect())
}

/// 姓名可以以哪些字开头（姓氏 + 复姓首字 + 小/老）。
fn may_start() -> &'static HashSet<char> {
    static MAY: OnceLock<HashSet<char>> = OnceLock::new();
    MAY.get_or_init(|| {
        let mut set = char_set(ZH_SURNAMES);
        for c in ZH_COMPOUND {
            set.insert(c.chars().next().unwrap());
        }
        set.insert('小');
        set.insert('老');
        set
    })
}

// ---- 词形判断 ----

/// zhWordShaped（Magpie）：v 这个「姓名」是普通词——颜色（黄绿色）、以姓氏
/// 开头的词（zhWords/zhDict：边界、尚可）、或这些词加名词尾（关键词、应用层）。
/// 姓氏本身留着：黄既是黄绿色的黄，也是黄晓明的黄。
pub(crate) fn zh_word_shaped(v: &str) -> bool {
    let rs: Vec<char> = v.chars().collect();
    if rs.len() < 2 {
        return false;
    }
    let mut colour = true;
    for r in &rs {
        if !is_han(*r) {
            return false;
        }
        colour = colour && ZH_COLOURS.contains(*r);
    }
    if colour || rs[rs.len() - 1] == '色' {
        return true;
    }
    let two: String = rs[..2].iter().collect();
    match rs.len() {
        2 => zh_words().contains(two.as_str()) || zh_dict().contains(two.as_str()),
        3 => {
            (zh_words().contains(two.as_str()) || zh_dict().contains(two.as_str()))
                && ZH_NOUN_ENDS.contains(rs[2])
        }
        _ => false,
    }
}

/// zhWordNotName（Magpie）：s[a:b] 这个候选是普通词——除非前面有角色词或
/// 后面有称谓说明这是人（客户方圆、江山老师）。
fn zh_word_not_name(s: &str, a: usize, b: usize) -> bool {
    zh_word_shaped(&s[a..b])
        && !ends_any(&s[..a], ZH_ROLES, true)
        && !starts_any(&s[b..], ZH_TITLES)
}

/// zhWord（Magpie）：s 以 zhWords 里的双字词开头。
fn zh_word(s: &str) -> bool {
    let two: String = s.chars().take(2).collect();
    zh_words().contains(two.as_str())
}

// ---- 位置判断 ----

/// endsAny（Magpie）：s 以 ws 之一结尾；role 时允许后面跟 是/为/冒号/空格。
fn ends_any(s: &str, ws: &[&str], role: bool) -> bool {
    if role {
        let mut s = s.trim_end_matches([' ', '\t']);
        for sep in ["：", ":", "是", "为"] {
            if let Some(t) = s.strip_suffix(sep) {
                s = t.trim_end_matches([' ', '\t']);
                break;
            }
        }
        return ws.iter().any(|w| s.ends_with(w));
    }
    ws.iter().any(|w| s.ends_with(w))
}

fn starts_any(s: &str, ws: &[&str]) -> bool {
    ws.iter().any(|w| s.starts_with(w))
}

/// endsWord（Magpie）：词在 s 开头处结束——s 为空，或开头不是汉字。
fn ends_word(s: &str) -> bool {
    match s.chars().next() {
        None => true,
        Some(r) => !is_han(r),
    }
}

/// sentenceStart（Magpie）：姓名可以在 s 之后开始——s 为空，或以标点/空白
/// 结尾。词或代码后的空格在句中（`gh pr view` 易触发）。
fn sentence_start(s: &str) -> bool {
    let s = s.trim_end_matches([' ', '\t', '\u{3000}']);
    if s.is_empty() {
        return true;
    }
    s.chars()
        .next_back()
        .is_some_and(|r| is_punct(r) || r.is_whitespace())
}

/// wordAfter（Magpie）：s 开头的 发/打 开始的是词（发现、打开），不是人做的
/// 动作（发了个消息）；说明/讲解 也一样，但说明天除外（张伟说明天来）。
fn word_after(s: &str) -> bool {
    for w in ["说明", "说法", "讲解", "讲座", "讲义", "讲究"] {
        if s.starts_with(w) {
            return !s.starts_with("说明天");
        }
    }
    for v in ["发", "打"] {
        if let Some(t) = s.strip_prefix(v) {
            let Some(r) = t.chars().next() else {
                return false;
            };
            return is_han(r) && !"了个给过一".contains(r);
        }
    }
    false
}

/// personBefore（Magpie）：s 以代词或可能是姓名的字结尾——一个姓氏加一到
/// 三个字，且不是 zhWords 里的词。
fn person_before(s: &str) -> bool {
    if let Some(r) = s.chars().next_back() {
        if "我你您他她们".contains(r) {
            return true;
        }
    }
    let rs: Vec<char> = s.chars().collect();
    let may = may_start();
    for n in 2..=4.min(rs.len()) {
        let first = rs[rs.len() - n];
        let u: String = rs[rs.len() - n..].iter().collect();
        if !may.contains(&first) || !has_han(&u) {
            continue;
        }
        if !zh_word(&u) {
            return true;
        }
    }
    false
}

/// zhTotal（Magpie）：总 在 s 前面是词的总（总是、总结、总共），不是称谓。
fn zh_total(s: &str) -> bool {
    s.chars()
        .next()
        .is_some_and(|r| "是结共体之数计线量部归算会得要".contains(r))
}

/// zhNick（Magpie）：小或老加一个姓氏（小王、老李）。
fn zh_nick(name: &str) -> bool {
    let mut chars = name.chars();
    let (Some(r), Some(r2)) = (chars.next(), chars.next()) else {
        return false;
    };
    (r == '小' || r == '老')
        && char_set(ZH_NICK_SURNAMES).contains(&r2)
        && r.len_utf8() + r2.len_utf8() == name.len()
}

fn is_han(r: char) -> bool {
    ('\u{3400}'..='\u{9fff}').contains(&r)
        || ('\u{f900}'..='\u{faff}').contains(&r)
        || ('\u{20000}'..='\u{2ffff}').contains(&r)
}

fn has_han(s: &str) -> bool {
    s.chars().any(|r| r >= '\u{3400}' && is_han(r))
}

fn is_punct(r: char) -> bool {
    // 对齐 Go unicode.IsPunct（P 类）：ASCII 标点不含反引号、$、+、<、=、>、^、|、~
    //（S 类），否则 ``gh pr view` 易触发` 的反引号会当句读、易触当姓名
    matches!(
        r,
        '!' | '#'
            | '%'
            | '&'
            | '\''
            | '('
            | ')'
            | '*'
            | ','
            | '-'
            | '.'
            | '/'
            | ':'
            | ';'
            | '?'
            | '@'
            | '['
            | ']'
            | '_'
            | '{'
            | '}'
            | '\\'
            | '"'
    ) || matches!(r, '\u{3000}'..='\u{303f}' | '\u{ff00}'..='\u{ffef}' | '\u{2010}'..='\u{2027}')
}

// ---- 姓名查找 ----

/// zhNameSpans（Magpie）：s 里的中文姓名（模型没找到的那些），PERSON 片段；
/// person 是模型找到的（我方为空）。nicks=true 时只找昵称（小王、老李）。
pub(crate) fn zh_name_spans(s: &str, person: &[Span], nicks: bool) -> Vec<Span> {
    if !has_han(s) {
        return Vec::new();
    }
    // 候选：kind 有值的才是姓名
    struct Candidate {
        span: Span,
        is_name: bool,
    }
    let mut cs: Vec<Candidate> = Vec::new();
    let mut i = 0usize;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        let Some((a, b, ok)) = zh_name(s, i) else {
            // 此处没有姓名可开始：前进一个字符
            let step = s[i..].chars().next().map_or(1, char::len_utf8);
            i += step;
            continue;
        };
        i = b;
        let is_word_not_name = zh_word_not_name(s, a, b);
        let drop_it = if nicks && !zh_nick(&s[a..b]) {
            true
        } else {
            is_word_not_name
        };
        if drop_it {
            continue; // 是词，或没接到姓名上：张伟和边界
        }
        cs.push(Candidate {
            span: Span {
                start: a,
                end: b,
                kind: "PERSON",
                by: FoundBy::Model,
                bare: false,
            },
            is_name: ok,
        });
    }
    // 和/与/跟/及/、 连到已确定姓名上的候选也是姓名：李娜和刘洋
    let mut starts: HashSet<usize> = person.iter().map(|p| p.start).collect();
    let mut ends: HashSet<usize> = person.iter().map(|p| p.end).collect();
    for c in &cs {
        if c.is_name {
            starts.insert(c.span.start);
            ends.insert(c.span.end);
        }
    }
    let mut grew = true;
    while grew {
        grew = false;
        for c in &mut cs {
            if !c.is_name && joined(s, &c.span, &starts, &ends) {
                c.is_name = true;
                starts.insert(c.span.start);
                ends.insert(c.span.end);
                grew = true;
            }
        }
    }
    cs.into_iter()
        .filter(|c| c.is_name)
        .map(|c| c.span)
        .collect()
}

/// joined（Magpie）：c 与一个已确定姓名之间只有 和/与/跟/及/、——
/// 对方在 ends 里结束，或在 starts 里开始。
fn joined(s: &str, c: &Span, starts: &HashSet<usize>, ends: &HashSet<usize>) -> bool {
    for j in ["和", "与", "跟", "及", "、"] {
        let jlen = j.len();
        if s[c.end..].starts_with(j) && starts.contains(&(c.end + jlen)) {
            return true;
        }
        if s[..c.start].ends_with(j) && c.start >= jlen && ends.contains(&(c.start - jlen)) {
            return true;
        }
    }
    false
}

/// zhName（Magpie）：可能从 i 开始的姓名——它的范围，以及周围是否说明这是
/// 人；None = 此处没有姓名可开始。
fn zh_name(s: &str, i: usize) -> Option<(usize, usize, bool)> {
    let rest = &s[i..];
    let first = rest.chars().next()?;
    if !may_start().contains(&first) {
        return None;
    }
    let mut sur = 0usize;
    for c in ZH_COMPOUND {
        if rest.starts_with(c) {
            sur = c.len();
            break;
        }
    }
    let mut nick = false;
    if sur == 0 {
        let n = first.len_utf8();
        if (first == '小' || first == '老') && rest.len() > n {
            let mut it = rest[n..].chars();
            if let Some(r2) = it.next() {
                if char_set(ZH_NICK_SURNAMES).contains(&r2) {
                    sur = n + r2.len_utf8();
                    nick = true;
                }
            }
        }
        if sur == 0 && char_set(ZH_SURNAMES).contains(&first) {
            sur = n;
        }
    }
    if sur == 0 {
        return None;
    }
    // 名：小王 后面没有，否则一到两个字
    let mut ends: Vec<usize> = Vec::new();
    if nick {
        ends.push(i + sur);
    } else {
        let mut at = i + sur;
        let not_given = char_set(ZH_NOT_GIVEN);
        for _ in 0..2 {
            let Some(r) = s[at..].chars().next() else {
                break;
            };
            if !is_han(r) || not_given.contains(&r) {
                break;
            }
            at += r.len_utf8();
            ends.push(at);
        }
    }
    if ends.is_empty() {
        return None;
    }
    if zh_word(rest) {
        return None;
    }
    // 前面有不是提示词的汉字说明它是词的一部分（建国路），除非提示词正好在那结束
    let before = &s[..i];
    let role = ends_any(before, ZH_ROLES, true);
    let mut verb = ends_any(before, ZH_BEFORE, false);
    // 和/与 同类相连：前面是人的后面才是人（吴敏和郑浩、我和张伟），
    // 不是 判断和边界
    for j in ["和", "与"] {
        if let Some(t) = before.strip_suffix(j) {
            if verb {
                verb = person_before(t);
            }
        }
    }
    // 一下 只在「问人」的动词后接人（问一下张伟），不是 改一下文案
    if let Some(t) = before.strip_suffix("一下") {
        if verb {
            verb = ends_any(t, ZH_ASK_VERBS, false);
        }
    }
    let start = sentence_start(before);
    if !role && !verb && !start {
        return None;
    }
    // 姓氏加称谓是人（孙经理、邓总）
    if !nick {
        let mut titles: Vec<&str> = ZH_TITLES.to_vec();
        titles.push("总");
        for t in titles {
            let after_sur = &s[i + sur..];
            if after_sur.starts_with(t) && !(t == "总" && zh_total(&after_sur[t.len()..])) {
                return Some((i, i + sur + t.len(), true));
            }
        }
    }
    // 从最长的开始：后面有词说明是人的那个结尾
    for k in (0..ends.len()).rev() {
        let e = ends[k];
        let after = &s[e..];
        if starts_any(after, ZH_AFTER)
            && !word_after(after)
            && (role || verb || !starts_any(after, ZH_ANY_AFTER))
        {
            return Some((i, e, true));
        }
        if (role || verb) && ends_word(after) {
            return Some((i, e, true));
        }
    }
    if role {
        return Some((i, *ends.last().unwrap(), true));
    }
    Some((i, *ends.last().unwrap(), false))
}

#[cfg(test)]
mod tests;
