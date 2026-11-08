# -*- coding: utf-8 -*-
"""E-08 中文姓名语料生成器。

姓氏 × 名字用字组合生成正例（避免手写具体人名），配提示词模板与反例词表。
固定种子，可复现。生成后自检：文件不含 {{ / TAG_（本地网关会把敏感原文
替换成占位符污染语料——E-03 与 Magpie 源参考都被污染过）。

用法：python scripts/aicli-poc/gen_zhname_corpus.py
输出：src-tauri/tests/fixtures/privacy/zhname_corpus.jsonl
"""

import json
import io
import random

OUT = "src-tauri/tests/fixtures/privacy/zhname_corpus.jsonl"

SURNAMES = list("王李张刘陈杨黄赵吴周徐孙马朱胡郭何高林罗郑梁谢宋唐许韩冯邓曹彭曾田董袁潘蒋蔡杜叶程苏魏吕丁任沈姚卢崔谭陆汪范金石廖贾夏韦付白邹孟熊邱江尹薛雷侯龙史陶贺顾毛郝龚邵万钱严武戴莫向汤温康施牛樊邢安齐易乔颜庄聂章岳申欧关兰焦左柳甘包宁尚舒阮柯纪童凌毕裴霍涂成谷盛曲翁管柴华喻房滕屈饶艾阳时司卓古简车连麦窦戚景党宫费卫米柏宗桂全应边卞仇隋沙荣巫寇郎甄仲虞敖巩池麻苑")

GIVEN = list("伟芳娜秀敏静丽强磊军勇艳杰娟涛明超兰霞平刚华健玉兰凤梅琳晶欣怡婷予昊宇泽博安宁欣月云飞翔_build".replace("_build", ""))  # 名字常用字
GIVEN = [c for c in GIVEN if c.strip()]

NICK_SURNAMES = list("王李张刘陈杨黄赵吴周徐孙马朱胡郭何高林罗郑梁谢宋唐许韩冯邓曹彭曾田董袁潘蒋蔡杜叶程苏魏吕丁沈姚卢崔谭陆汪范金廖贾夏方邹孟熊秦江薛段雷侯史陶贺顾毛郝龚邵钱严武戴孔汤")

COMPOUND = ["欧阳", "司马", "上官", "诸葛", "东方", "公孙", "慕容", "司徒", "宇文", "南宫"]

WORDS = [
    "王国", "张开", "文案", "毛利率", "黄绿色", "关键词", "应用层", "高兴", "边界", "建国路",
    "江山", "方圆", "常用", "林业", "高度", "方法", "方向", "安全", "程序", "周期", "文件",
    "文档", "余额", "马上", "项目", "成本", "颜色", "查询", "严格", "曾经", "谢谢", "付费",
    "应该", "尚可", "方案", "方面", "文本", "文章", "安装", "安排", "钱包", "毛病", "雷达",
    "常量", "向量", "全部", "全局", "时间", "明天", "任务", "管理", "简单", "范围", "许可",
    "包括", "关系", "关闭", "连接", "成功", "成为", "单位", "单元", "司机", "车辆", "路径",
    "路由", "龙头", "柏林", "宁可", "师傅", "老师", "老板", "老家", "小时", "小心", "小说",
    "小姐", "小孩", "小组", "周一", "周五", "周日", "文化", "文学", "文明", "文献", "方块",
    "方针", "方程", "关注", "关联", "关心", "关掉", "成员", "成果", "成绩", "成熟", "成就",
    "解决", "解析", "解释", "解码", "段落", "章节", "蓝色", "查看", "查找", "单词", "单击",
    "时区", "时长", "时刻", "时代", "季度", "严重", "麻烦", "安卓", "谷歌", "常规", "常数",
    "边框", "边缘", "路线", "简洁", "简历", "简化", "游戏", "游客", "商品", "商店", "商业",
    "焦点", "毕竟", "尚未", "连续", "连线", "林间", "顾客", "顾问", "房间", "房子", "高端",
    "高层", "古代", "古典", "农业", "司法", "施工", "卫生", "荣誉", "宗旨", "齐全", "凌晨",
    "冷却", "蒙版", "曲线", "黄金", "白天", "白板", "金钱", "石油", "夏天", "申请", "符号",
    "景色", "费用", "舒服", "辛苦", "纪律", "席位", "管道", "何时", "何况", "于是", "杜绝",
    "罗列", "叶片", "涂层", "沙发", "池子", "巩固", "迟到", "仲裁", "向导", "马路", "马虎",
    "阳光", "任职", "董事", "边距", "边栏", "尚需", "易触发", "关键发现",
]

WORD_SHAPES = [
    "帮我改一下这段{w}",
    "检查一下{w}，看看有没有问题",
    "{w}已经好了，你再看看",
    "{w}明天给你，今天先发",
    "{w}说明要简短",
    "把{w}换成英文",
    "请{w}",
    "发给{w}",
    "{w}：首页标题换成新方案",
    "提取这段文本的{w}",
]

NAME_SHAPES = [
    ("客户{n}反馈导出失败", ["n"]),
    ("联系人：{n}，请尽快确认", ["n"]),
    ("我叫{n}，是这个项目的负责人", ["n"]),
    ("帮我写封信给{n}，他明天到北京", ["n"]),
    ("问一下{n}，这段代码谁来审", ["n"]),
    ("{n}先生，您的订单已发货", ["n"]),
    ("{n}老师说要注意这个问题", ["n"]),
    ("{n}说过这个方案可行", ["n"]),
    ("请把合同发给{n}，她下周一到上海", ["n"]),
    ("抄送给{n}，务必今天确认", ["n"]),
    ("负责人：{n}", ["n"]),
    ("我是{n}，想咨询一下退款的问题", ["n"]),
    ("审核人是{n}，请他尽快看", ["n"]),
    ("请回复一下{n}经理的邮件", ["n"]),
    ("转给{n}，会议改到下午", ["n"]),
    ("{a}和{b}昨天开会讨论了进度", ["a", "b"]),
    ("{a}和{b}昨天一起评审了这份文档", ["a", "b"]),
]


def gen_name(rng):
    surname = rng.choice(SURNAMES)
    given = "".join(rng.sample(GIVEN, rng.choice([1, 2])))
    return surname + given


def gen_nick(rng):
    return rng.choice(["小", "老"]) + rng.choice(NICK_SURNAMES)


def main():
    rng = random.Random(20261108)
    lines = []
    # 正例：姓名模板
    for i in range(105):
        shape, slots = NAME_SHAPES[i % len(NAME_SHAPES)]
        text = shape
        names = []
        for slot in slots:
            name = gen_name(rng)
            text = text.replace("{" + slot + "}", name)
            names.append(name)
        # 姓名必须真的在文本里
        assert all(n in text for n in names)
        lines.append({"text": text, "names": names})
    # 正例：复姓
    for i in range(10):
        compound = COMPOUND[i % len(COMPOUND)]
        given = "".join(rng.sample(GIVEN, rng.choice([1, 2])))
        name = compound + given
        shape = NAME_SHAPES[i % 8][0]
        text = shape.replace("{n}", name)
        lines.append({"text": text, "names": [name]})
    # 正例：昵称（小王、老李）
    for i in range(10):
        nick = gen_nick(rng)
        shape = ["给{p}发个消息，说明天不来了", "{p}说数据还没跑完", "问一下{p}，这个参数怎么配"][i % 3]
        text = shape.replace("{p}", nick)
        lines.append({"text": text, "names": [nick]})
    # 反例：词 × 句式
    negatives = 0
    for i, word in enumerate(WORDS):
        shape = WORD_SHAPES[i % len(WORD_SHAPES)]
        text = shape.replace("{w}", word)
        lines.append({"text": text, "names": []})
        negatives += 1
    # 反例：整句
    for text in [
        "王国的军队在黄河边扎营。",
        "请帮我修复这个 bug，张开的括号没有闭合。",
        "在建国路上，路边有一家店。",
        "把方法和文件名都改一下",
        "毛利率下降了三个点",
        "黄绿色配红色好看吗",
        "关键词提取失败了",
        "应用层协议不对",
        "和平时一样，周五发版。",
        "方总结一下：说明文档要写清楚步骤。",
        "打开文件之前，边界检查失败。",
        "这个函数把高度转换成像素。",
        "给张量赋值之前先检查形状。",
        "运行测试时内存一直在涨。签名和工具调用都会保留。",
        "毛利低说明成本高",
        "看一下毛利润和毛利额",
        "天下方圆百里",
        "江山如画",
        "这个产品毛利低说明定价有问题",
    ]:
        lines.append({"text": text, "names": []})
        negatives += 1

    # 打乱顺序（正反混合，共用引擎评估时正反交错）
    rng.shuffle(lines)

    with io.open(OUT, "w", encoding="utf-8", newline="\n") as f:
        for line in lines:
            f.write(json.dumps(line, ensure_ascii=False) + "\n")

    # 自检：没有占位符污染、没有手写人名泄漏形态
    content = io.open(OUT, encoding="utf-8").read()
    assert "{{" not in content, "placeholder leaked into corpus"
    assert "TAG_" not in content, "gateway tag leaked into corpus"

    positives = sum(1 for l in lines if l["names"])
    print(
        f"wrote {len(lines)} lines to {OUT}: {positives} positive, "
        f"{negatives} negative ({positives / len(lines):.0%}/{negatives / len(lines):.0%})"
    )


if __name__ == "__main__":
    main()
