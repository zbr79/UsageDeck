//! API-equivalent prices for Claude models, in dollars per million tokens.
//!
//! Source: the claude-api model reference, cached 2026-11-03, for input / output / cache read;
//! `shared/prompt-caching.md` for the 1.25x (5-minute) / 2x (1-hour) cache-write multipliers.
//! A model missing from the table is never priced as zero: the caller counts it as unpriced.

use serde::{Deserialize, Serialize};

/// Dollars per million tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub cache_read: f64,
}

/// Token counts of one reply (or a sum of replies). `output` already includes thinking tokens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
}

impl Tokens {
    pub fn add(&mut self, other: &Tokens) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write_5m += other.cache_write_5m;
        self.cache_write_1h += other.cache_write_1h;
    }

    /// The tokens of the prompt: uncached input plus cache reads and writes.
    pub fn prompt(&self) -> u64 {
        self.input + self.cache_read + self.cache_write_5m + self.cache_write_1h
    }

    pub fn is_empty(&self) -> bool {
        *self == Tokens::default()
    }
}

/// Which of a model's prices one request pays. Only a model with a long-prompt price has more
/// than `Standard`; it is chosen per request, from that request's prompt size.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "camelCase")]
pub enum Tier {
    #[default]
    Standard,
    /// The prompt was longer than [`LONG_PROMPT_TOKENS`].
    LongPrompt,
}

/// Prompts longer than this pay a model's long-prompt price (Haiku 5.5: "up to 100K tokens").
pub const LONG_PROMPT_TOKENS: u64 = 100_000;

const fn row(input: f64, output: f64, cache_read: f64) -> Price {
    Price {
        input,
        output,
        cache_write_5m: input * 1.25,
        cache_write_1h: input * 2.0,
        cache_read,
    }
}

struct Row {
    model: &'static str,
    standard: Price,
    long_prompt: Option<Price>,
}

const fn flat(model: &'static str, price: Price) -> Row {
    Row {
        model,
        standard: price,
        long_prompt: None,
    }
}

/// Model ids are matched after dropping a trailing `-YYYYMMDD` date.
const TABLE: &[Row] = &[
    flat("claude-fable-5-2", row(8.0, 40.0, 0.20)),
    flat("claude-mythos-5-2", row(8.0, 40.0, 0.20)),
    flat("claude-fable-5-1", row(10.0, 50.0, 0.25)),
    flat("claude-mythos-5-1", row(10.0, 50.0, 0.25)),
    // Provisional cache read: the reference gives none for Fable 5; Fable 5.1's, at the same
    // input price.
    flat("claude-fable-5", row(10.0, 50.0, 0.25)),
    flat("claude-opus-5-5", row(4.0, 20.0, 0.20)),
    flat("claude-opus-5", row(5.0, 25.0, 0.50)),
    flat("claude-opus-4-8", row(5.0, 25.0, 0.50)),
    flat("claude-opus-4-7", row(5.0, 25.0, 0.50)),
    flat("claude-opus-4-6", row(5.0, 25.0, 0.50)),
    flat("claude-sonnet-5-5", row(2.0, 10.0, 0.10)),
    flat("claude-sonnet-5", row(2.0, 10.0, 0.20)),
    flat("claude-sonnet-4-6", row(3.0, 15.0, 0.30)),
    // Provisional cache reads: the reference lists none for Haiku 5.5; 0.1x input, as the probe
    // used. Re-open when the reference gives a Haiku cache-read price.
    Row {
        model: "claude-haiku-5-5",
        standard: row(0.10, 0.50, 0.01),
        long_prompt: Some(row(0.50, 2.50, 0.05)),
    },
    flat("claude-haiku-4-5", row(1.0, 5.0, 0.10)),
];

/// Drop a trailing `-YYYYMMDD` release date: `claude-opus-5-5-20261001` -> `claude-opus-5-5`.
fn base_model(model: &str) -> &str {
    match model.rsplit_once('-') {
        Some((head, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => model,
    }
}

fn row_of(model: &str) -> Option<&'static Row> {
    let base = base_model(model.trim());
    TABLE.iter().find(|r| r.model == base)
}

/// The price tier one request of `model` with `tokens` pays.
pub fn tier_of(model: &str, tokens: &Tokens) -> Tier {
    match row_of(model) {
        Some(Row {
            long_prompt: Some(_),
            ..
        }) if tokens.prompt() > LONG_PROMPT_TOKENS => Tier::LongPrompt,
        _ => Tier::Standard,
    }
}

/// The price of `model` at `tier`, or None for a model missing from the table (including
/// `<synthetic>`). A model without a long-prompt price has one price for every tier.
pub fn price_of(model: &str, tier: Tier) -> Option<Price> {
    let row = row_of(model)?;
    Some(match tier {
        Tier::LongPrompt => row.long_prompt.unwrap_or(row.standard),
        Tier::Standard => row.standard,
    })
}

/// The API-equivalent cost of `tokens` in dollars.
pub fn cost(price: &Price, tokens: &Tokens) -> f64 {
    (tokens.input as f64 * price.input
        + tokens.output as f64 * price.output
        + tokens.cache_read as f64 * price.cache_read
        + tokens.cache_write_5m as f64 * price.cache_write_5m
        + tokens.cache_write_1h as f64 * price.cache_write_1h)
        / 1e6
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standard(model: &str) -> Price {
        price_of(model, Tier::Standard).unwrap_or_else(|| panic!("{model} is unpriced"))
    }

    #[test]
    fn every_row_resolves_with_and_without_a_release_date() {
        let rows = [
            ("claude-fable-5-2", 8.0, 40.0, 0.20),
            ("claude-mythos-5-2", 8.0, 40.0, 0.20),
            ("claude-fable-5-1", 10.0, 50.0, 0.25),
            ("claude-mythos-5-1", 10.0, 50.0, 0.25),
            ("claude-fable-5", 10.0, 50.0, 0.25),
            ("claude-opus-5-5", 4.0, 20.0, 0.20),
            ("claude-opus-5", 5.0, 25.0, 0.50),
            ("claude-opus-4-8", 5.0, 25.0, 0.50),
            ("claude-opus-4-7", 5.0, 25.0, 0.50),
            ("claude-opus-4-6", 5.0, 25.0, 0.50),
            ("claude-sonnet-5-5", 2.0, 10.0, 0.10),
            ("claude-sonnet-5", 2.0, 10.0, 0.20),
            ("claude-sonnet-4-6", 3.0, 15.0, 0.30),
            ("claude-haiku-5-5", 0.10, 0.50, 0.01),
            ("claude-haiku-4-5", 1.0, 5.0, 0.10),
        ];
        assert_eq!(rows.len(), TABLE.len(), "every table row is checked");
        for (model, input, output, cache_read) in rows {
            let p = standard(model);
            assert_eq!(
                (p.input, p.output, p.cache_read),
                (input, output, cache_read),
                "{model}"
            );
            assert_eq!(
                (p.cache_write_5m, p.cache_write_1h),
                (input * 1.25, input * 2.0),
                "{model}"
            );
            assert_eq!(
                price_of(&format!("{model}-20261101"), Tier::Standard),
                Some(p),
                "{model} with a date"
            );
        }
    }

    #[test]
    fn cost_cross_check_fable_5_2() {
        // by hand, in dollars: 1 000 x 8 + 50 000 x 40 + 2 000 000 x 0.20 + 100 000 x 10 (5m
        // writes) + 10 000 x 16 (1h writes), per million = 0.008 + 2.0 + 0.4 + 1.0 + 0.16
        let tokens = Tokens {
            input: 1_000,
            output: 50_000,
            cache_read: 2_000_000,
            cache_write_5m: 100_000,
            cache_write_1h: 10_000,
        };
        let usd = cost(&standard("claude-fable-5-2-20261020"), &tokens);
        assert!((usd - 3.568).abs() < 1e-9, "{usd}");
    }

    #[test]
    fn cost_state_cross_check_opus_5_5() {
        // One real Claude Code `cost-state` record (redacted to its token counts) reported
        // costUSD 4.2579004 for claude-opus-5-5.
        let tokens = Tokens {
            input: 58,
            output: 74_371,
            cache_read: 5_619_482,
            cache_write_5m: 0,
            cache_write_1h: 205_794,
        };
        let usd = cost(&standard("claude-opus-5-5"), &tokens);
        assert!((usd - 4.2579004).abs() < 1e-6, "{usd}");
        assert_eq!(format!("{usd:.4}"), "4.2579");
    }

    #[test]
    fn haiku_5_5_prompts_over_100k_tokens_pay_the_long_prompt_price() {
        let prompt = |input: u64, cache_read: u64| Tokens {
            input,
            cache_read,
            output: 1_000,
            ..Tokens::default()
        };
        let at_limit = prompt(40_000, 60_000);
        let over = prompt(40_001, 60_000);
        assert_eq!(tier_of("claude-haiku-5-5", &at_limit), Tier::Standard);
        assert_eq!(
            tier_of("claude-haiku-5-5-20261001", &over),
            Tier::LongPrompt
        );
        // cache writes count toward the prompt too
        let writes = Tokens {
            cache_write_1h: 100_001,
            ..Tokens::default()
        };
        assert_eq!(tier_of("claude-haiku-5-5", &writes), Tier::LongPrompt);

        let long = price_of("claude-haiku-5-5", Tier::LongPrompt).unwrap();
        assert_eq!(
            (
                long.input,
                long.output,
                long.cache_read,
                long.cache_write_5m
            ),
            (0.50, 2.50, 0.05, 0.625)
        );
        // 40 001 x $0.50 + 60 000 x $0.05 + 1 000 x $2.50, per million
        let usd = cost(&long, &over);
        assert!((usd - 0.0255005).abs() < 1e-12, "{usd}");

        // models without a long-prompt price: one price whatever the size
        let big = prompt(150_000, 0);
        assert_eq!(tier_of("claude-haiku-4-5", &big), Tier::Standard);
        assert_eq!(tier_of("claude-opus-5-5", &big), Tier::Standard);
        assert_eq!(
            price_of("claude-opus-5-5", Tier::LongPrompt),
            price_of("claude-opus-5-5", Tier::Standard)
        );
    }

    #[test]
    fn unknown_models_are_unpriced() {
        for model in ["<synthetic>", "claude-mystery-9", "", "claude-opus"] {
            assert!(price_of(model, Tier::Standard).is_none(), "{model}");
            assert_eq!(tier_of(model, &Tokens::default()), Tier::Standard);
        }
    }

    #[test]
    fn five_minute_writes_are_cheaper_than_one_hour() {
        let p = standard("claude-opus-5-5");
        let five = Tokens {
            cache_write_5m: 1_000_000,
            ..Tokens::default()
        };
        let hour = Tokens {
            cache_write_1h: 1_000_000,
            ..Tokens::default()
        };
        assert_eq!((cost(&p, &five), cost(&p, &hour)), (5.0, 8.0));
    }
}
