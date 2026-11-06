# Third-Party Notices

本文件汇总随源码分发（非通过 Cargo/npm 依赖引入）的第三方材料声明。

## Magpie（MIT）

AICLI Client 的隐私引擎（阶段 E）以 Magpie 的脱敏模块为参考实现基础：
确定性占位符（HMAC-SHA256、base32）、检测规则、JSON 遍历与协议保护、
流式事件级扣留等算法的来源与移植依据。来源仅作开发参考（`source-reference/`
不入库），移植后的代码在对应文件头注明来源与 commit。

来源：https://github.com/yetone/magpie @ `f11bf27c1f3f0f7d5284dffa2cb8b066debaed84`

```text
MIT License

Copyright (c) 2026 yetone

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## jieba 词典（MIT，经 Magpie 转载）

中文姓名检测的排除词典（`zhdict.txt`，约 1550 个以姓氏开头的常用双字词）取自
jieba 的 `dict.txt`，其许可声明如下（摘自 Magpie 自身的 `THIRD_PARTY_NOTICES`
「jieba's dictionary (a word list)」一节）。是否在 v1 启用中文姓名检测由
设置项 `privacy_detect_cjk_names`（默认关闭）决定（E-08）。

上游：jieba v0.42.1，https://github.com/fxsjy/jieba

```text
The MIT License (MIT)

Copyright (c) 2013 Sun Junyi

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
