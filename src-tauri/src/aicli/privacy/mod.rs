//! E 阶段隐私引擎：占位符、检测规则、JSON 遍历、流式还原、中间件。
pub mod events;
pub mod layer;
pub mod placeholder;
pub mod rules;
pub mod stream;
pub mod walker;

#[cfg(test)]
mod fuzz_tests;
