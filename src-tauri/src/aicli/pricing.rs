//! 按服务端（new-api rc.34）的口径估算 AICLI 请求的成本。
//!
//! 价格来自缓存的 `/api/pricing` 行，三种计费方式与 new-api 一致：
//! - 按倍率（`quota_type=0`）：每百万输入 = `model_ratio × 2` 美元（QuotaPerUnit=500000
//!   时倍率 1 = $0.002/1K）；输出乘 `completion_ratio`；缓存命中乘 `cache_ratio`
//!   （未配置按 1，`ratio_setting.GetCacheRatio`）；缓存写入乘 `create_cache_ratio`
//!   （未配置按 1.25，`GetCreateCacheRatio`）。见 `service/text_quota.go`。
//! - 按次（`quota_type=1`）：`model_price` 美元/次。
//! - 分段表达式（`billing_mode=tiered_expr`）：系数即美元/百万 tokens，按 `len`（完整输入
//!   长度）选档；表达式没单独计价的缓存 token 留在 `p` 里按输入价。见 `pkg/billingexpr/expr.md`。
//!
//! 三种都再乘分组倍率（`quota = 成本 × QuotaPerUnit × groupRatio`）。这里只做本地预估，
//! 服务端账单回填后以实扣为准（[`super::usage`]）。

use rust_decimal::Decimal;
use serde_json::Value;
use std::str::FromStr;

use crate::proxy::usage::{
    calculator::{CostBreakdown, CostCalculator, ModelPricing},
    parser::TokenUsage,
};

/// 一次 AICLI 请求的计价依据：所用模型的服务端价格行和预估分组的倍率。
/// 分组取路由的第一个分组；实际分组以服务端账单回填为准。
#[derive(Debug, Clone, PartialEq)]
pub struct UsageBilling {
    pub pricing_row: Value,
    pub group: String,
    pub group_ratio: Decimal,
}

/// 按服务端口径估算成本：各项为站点单价（未乘分组倍率），合计乘分组倍率，
/// 与本地成本明细「各项 → 成本倍数 → 合计」的结构一致。返回 (成本, 成本倍数)。
/// 价格认不出时返回 None，该行记为未计价，等服务端账单回填。
pub fn estimate(
    app_type: &str,
    usage: &TokenUsage,
    billing: &UsageBilling,
) -> Option<(CostBreakdown, Decimal)> {
    let inclusive = crate::services::sql_helpers::is_cache_inclusive_app(app_type);
    let len = if inclusive {
        u64::from(usage.input_tokens)
    } else {
        u64::from(usage.input_tokens)
            + u64::from(usage.cache_read_tokens)
            + u64::from(usage.cache_creation_tokens)
    };
    let ratio = billing.group_ratio;
    let cost = match server_price(&billing.pricing_row, len)? {
        ServerPrice::PerToken(pricing) => {
            CostCalculator::calculate_for_app(app_type, usage, &pricing, ratio)
        }
        ServerPrice::PerCall(price) => CostBreakdown {
            input_cost: Decimal::ZERO,
            output_cost: Decimal::ZERO,
            cache_read_cost: Decimal::ZERO,
            cache_creation_cost: Decimal::ZERO,
            total_cost: price * ratio,
        },
    };
    Some((cost, ratio))
}

/// 一行服务端价格，按某次请求的输入长度选好档后的结果。
#[derive(Debug, Clone, PartialEq)]
pub enum ServerPrice {
    /// 美元 / 百万 tokens（未乘分组倍率）。
    PerToken(ModelPricing),
    /// 美元 / 次（未乘分组倍率）。
    PerCall(Decimal),
}

fn decimal(value: f64) -> Option<Decimal> {
    value
        .is_finite()
        .then(|| Decimal::from_f64_retain(value))
        .flatten()
        .map(|value| value.round_dp(12).normalize())
}

fn number(row: &Value, key: &str) -> Option<f64> {
    row.get(key)
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite())
}

/// `len` 是请求的完整输入长度（含缓存命中与写入），只用于选档。
/// 认不出的计费方式或表达式返回 None：宁可不估，也不给出错误的价格。
pub fn server_price(row: &Value, len: u64) -> Option<ServerPrice> {
    if row.get("billing_mode").and_then(Value::as_str) == Some("tiered_expr") {
        let expr = row.get("billing_expr").and_then(Value::as_str)?;
        return tiered_price(expr, len).map(ServerPrice::PerToken);
    }
    match row.get("quota_type").and_then(Value::as_i64).unwrap_or(0) {
        1 => decimal(number(row, "model_price")?).map(ServerPrice::PerCall),
        0 => {
            let input = decimal(number(row, "model_ratio")? * 2.0)?;
            let ratio = |key: &str, default: f64| {
                decimal(number(row, key).unwrap_or(default)).map(|r| (input * r).normalize())
            };
            Some(ServerPrice::PerToken(ModelPricing {
                input_cost_per_million: input,
                output_cost_per_million: ratio("completion_ratio", 1.0)?,
                cache_read_cost_per_million: ratio("cache_ratio", 1.0)?,
                cache_creation_cost_per_million: ratio("create_cache_ratio", 1.25)?,
            }))
        }
        _ => None,
    }
}

/// 解析 new-api 的分段表达式并按 `len` 取所选档的单价。只支持站点实际在用的形状：
/// `[v1:] tier("名称", 项 + 项 ...)` 或 `len <op> N ? 表达式 : 表达式`（可嵌套），
/// 项为 `变量 * 系数` 或 `系数 * 变量`。带请求规则（`|||`）或其他函数的表达式不认。
fn tiered_price(expr: &str, len: u64) -> Option<ModelPricing> {
    let expr = expr.trim();
    let expr = expr.strip_prefix("v1:").unwrap_or(expr);
    if expr.contains("|||") {
        return None;
    }
    let mut parser = Parser {
        src: expr.as_bytes(),
        pos: 0,
    };
    let terms = parser.expr(len)?;
    parser.skip_ws();
    if parser.pos != parser.src.len() {
        return None;
    }
    let coef = |name: &str| terms.iter().find(|(var, _)| var == name).map(|(_, c)| *c);
    let input = coef("p")?;
    Some(ModelPricing {
        input_cost_per_million: input,
        output_cost_per_million: coef("c")?,
        // 表达式没单独计价时，这部分 token 留在 p 里按输入价计
        cache_read_cost_per_million: coef("cr").unwrap_or(input),
        cache_creation_cost_per_million: coef("cc").unwrap_or(input),
    })
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

const VARIABLES: [&str; 9] = ["p", "c", "cr", "cc", "cc1h", "img", "ai", "ao", "img_o"];

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while self.src.get(self.pos).is_some_and(u8::is_ascii_whitespace) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, token: &str) -> bool {
        self.skip_ws();
        if self.src[self.pos..].starts_with(token.as_bytes()) {
            self.pos += token.len();
            true
        } else {
            false
        }
    }

    fn ident(&mut self) -> Option<String> {
        self.skip_ws();
        let start = self.pos;
        while self
            .src
            .get(self.pos)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            self.pos += 1;
        }
        let ident = std::str::from_utf8(&self.src[start..self.pos]).ok()?;
        (!ident.is_empty() && !ident.as_bytes()[0].is_ascii_digit()).then(|| ident.to_owned())
    }

    fn number(&mut self) -> Option<Decimal> {
        self.skip_ws();
        let start = self.pos;
        while self
            .src
            .get(self.pos)
            .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
        {
            self.pos += 1;
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).ok()?;
        Decimal::from_str(text).ok()
    }

    /// 表达式 → 所选档的 (变量, 系数) 列表。两个分支都要能解析，未选中的分支也校验形状。
    fn expr(&mut self, len: u64) -> Option<Vec<(String, Decimal)>> {
        self.skip_ws();
        if self.src[self.pos..].starts_with(b"len") {
            self.pos += 3;
            let op = ["<=", ">=", "<", ">", "=="]
                .into_iter()
                .find(|op| self.eat(op))?;
            let bound = self.number()?;
            if !self.eat("?") {
                return None;
            }
            let then = self.expr(len)?;
            if !self.eat(":") {
                return None;
            }
            let otherwise = self.expr(len)?;
            let len = Decimal::from(len);
            let take_then = match op {
                "<=" => len <= bound,
                ">=" => len >= bound,
                "<" => len < bound,
                ">" => len > bound,
                _ => len == bound,
            };
            return Some(if take_then { then } else { otherwise });
        }
        if self.eat("(") {
            let inner = self.expr(len)?;
            return self.eat(")").then_some(inner);
        }
        self.tier()
    }

    fn tier(&mut self) -> Option<Vec<(String, Decimal)>> {
        if !self.eat("tier") || !self.eat("(") || !self.eat("\"") {
            return None;
        }
        let close = self.src[self.pos..].iter().position(|b| *b == b'"')?;
        self.pos += close + 1;
        if !self.eat(",") {
            return None;
        }
        let mut terms: Vec<(String, Decimal)> = Vec::new();
        loop {
            let term = if let Some(coef) = self.number() {
                if !self.eat("*") {
                    return None;
                }
                (self.ident()?, coef)
            } else {
                let var = self.ident()?;
                if !self.eat("*") {
                    return None;
                }
                (var, self.number()?)
            };
            if !VARIABLES.contains(&term.0.as_str()) || terms.iter().any(|(v, _)| *v == term.0) {
                return None;
            }
            terms.push(term);
            if !self.eat("+") {
                break;
            }
        }
        self.eat(")").then_some(terms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn d(value: &str) -> Decimal {
        Decimal::from_str(value).unwrap()
    }

    fn per_token(input: &str, output: &str, read: &str, write: &str) -> ServerPrice {
        ServerPrice::PerToken(ModelPricing {
            input_cost_per_million: d(input),
            output_cost_per_million: d(output),
            cache_read_cost_per_million: d(read),
            cache_creation_cost_per_million: d(write),
        })
    }

    // 线上 gpt-5.6-sol 的价格行（2026-11-15 /api/pricing）
    fn sol() -> Value {
        json!({"model_name":"gpt-5.6-sol","quota_type":0,"model_ratio":37.5,"completion_ratio":6,
            "billing_mode":"tiered_expr",
            "billing_expr":"len < 272000 ? tier(\"标准上下文（＜ 272K 输入 tokens）\", p * 5 + c * 30 + cr * 0.5 + cc * 6.25) : tier(\"长上下文（≥ 272K 输入 tokens）\", p * 10 + c * 45 + cr * 1 + cc * 9.375)"})
    }

    #[test]
    fn tiered_expression_picks_the_tier_by_full_input_length() {
        assert_eq!(
            server_price(&sol(), 38_320),
            Some(per_token("5", "30", "0.5", "6.25"))
        );
        assert_eq!(
            server_price(&sol(), 271_999),
            Some(per_token("5", "30", "0.5", "6.25"))
        );
        assert_eq!(
            server_price(&sol(), 272_000),
            Some(per_token("10", "45", "1", "9.375"))
        );
    }

    #[test]
    fn tiered_expression_without_cache_terms_prices_cache_as_input() {
        let row = json!({"billing_mode":"tiered_expr","billing_expr":"v1:tier(\"base\", 2.5 * p + c * 15)"});
        assert_eq!(
            server_price(&row, 10),
            Some(per_token("2.5", "15", "2.5", "2.5"))
        );
        // 站点在用的其他形状：<=、cc1h、ai 也能解析（这两个子类别本地不单列，留在 p 里）
        let row = json!({"billing_mode":"tiered_expr","billing_expr":"len <= 200000 ? tier(\"a\", p * 3 + cr * 0.3 + cc * 3.75 + cc1h * 6 + c * 15) : tier(\"b\", p * 6 + cr * 0.6 + cc * 7.5 + cc1h * 12 + c * 22.5)"});
        assert_eq!(
            server_price(&row, 200_000),
            Some(per_token("3", "15", "0.3", "3.75"))
        );
        let row = json!({"billing_mode":"tiered_expr","billing_expr":"tier(\"base\", p * 0.3 + cr * 0.03 + ai * 1 + c * 2.5)"});
        assert_eq!(
            server_price(&row, 1),
            Some(per_token("0.3", "2.5", "0.03", "0.3"))
        );
    }

    #[test]
    fn unsupported_expressions_are_not_guessed() {
        for expr in [
            "tier(\"base\", p * 5 + c * 25)|||when(header(\"x\") has \"y\") * 6",
            "tier(\"base\", max(p, 1) * 5 + c * 25)",
            "hour(\"UTC\") < 8 ? tier(\"a\", p * 1 + c * 2) : tier(\"b\", p * 2 + c * 4)",
            "tier(\"base\", c * 25)",
            "tier(\"base\", p * 5 + p * 6 + c * 1)",
            "tier(\"base\", p * 5 + c * 25) + 1",
        ] {
            let row = json!({"billing_mode":"tiered_expr","billing_expr":expr});
            assert_eq!(server_price(&row, 1), None, "{expr}");
        }
    }

    #[test]
    fn ratio_rows_follow_new_api_defaults() {
        let row = json!({"quota_type":0,"model_ratio":2.5,"completion_ratio":6,"cache_ratio":0.1});
        assert_eq!(
            server_price(&row, 1),
            Some(per_token("5", "30", "0.5", "6.25"))
        );
        // 未配置缓存倍率：命中按 1、写入按 1.25；未配置补全倍率按 1
        let row = json!({"quota_type":0,"model_ratio":37.5});
        assert_eq!(
            server_price(&row, 1),
            Some(per_token("75", "75", "75", "93.75"))
        );
    }

    #[test]
    fn per_call_rows_use_the_model_price() {
        let row = json!({"quota_type":1,"model_ratio":0,"model_price":0.18});
        assert_eq!(server_price(&row, 0), Some(ServerPrice::PerCall(d("0.18"))));
        assert_eq!(server_price(&json!({"quota_type":2}), 0), None);
    }
}
