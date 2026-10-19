//! API-equivalent prices for Claude models, in dollars per million tokens.
//!
//! Source: docs/plans/claude-spend-and-pace.md "Prices" (the claude-api reference, cached
//! 2026-10-06, for input / output / cache read; `shared/prompt-caching.md` for the 1.25x / 2x
//! cache-write multipliers). The older rows (Opus 5, Sonnet 5) come from the read-only probe the
//! plan's numbers were measured with. A model missing from the table is never priced as zero: the
//! caller counts it as unpriced.

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
}

const fn row(input: f64, output: f64, cache_read: f64) -> Price {
    Price {
        input,
        output,
        cache_write_5m: input * 1.25,
        cache_write_1h: input * 2.0,
        cache_read,
    }
}

/// (model id, price). Model ids are matched after dropping a trailing `-YYYYMMDD` date.
const TABLE: [(&str, Price); 6] = [
    ("claude-opus-5-5", row(4.0, 20.0, 0.20)),
    ("claude-sonnet-5-5", row(2.0, 10.0, 0.20)),
    ("claude-fable-5-1", row(10.0, 50.0, 0.25)),
    // Provisional cache read: the reference lists none for Haiku 5.5; 0.1x input, as the probe
    // used. Re-open when the reference gives a Haiku cache-read price.
    ("claude-haiku-5-5", row(0.10, 0.50, 0.01)),
    ("claude-opus-5", row(5.0, 25.0, 0.50)),
    ("claude-sonnet-5", row(2.0, 10.0, 0.20)),
];

/// Drop a trailing `-YYYYMMDD` release date: `claude-opus-5-5-20261001` -> `claude-opus-5-5`.
fn base_model(model: &str) -> &str {
    match model.rsplit_once('-') {
        Some((head, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => model,
    }
}

/// The price of `model`, or None for a model missing from the table (including `<synthetic>`).
pub fn price_of(model: &str) -> Option<Price> {
    let base = base_model(model.trim());
    TABLE.iter().find(|(id, _)| *id == base).map(|(_, p)| *p)
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
        let usd = cost(&price_of("claude-opus-5-5").unwrap(), &tokens);
        assert!((usd - 4.2579004).abs() < 1e-6, "{usd}");
        assert_eq!(format!("{usd:.4}"), "4.2579");
    }

    #[test]
    fn table_rows_and_multipliers() {
        let opus = price_of("claude-opus-5-5").unwrap();
        assert_eq!(
            (opus.cache_write_5m, opus.cache_write_1h, opus.cache_read),
            (5.0, 8.0, 0.20)
        );
        let fable = price_of("claude-fable-5-1").unwrap();
        assert_eq!((fable.cache_write_5m, fable.cache_write_1h), (12.5, 20.0));
        let haiku = price_of("claude-haiku-5-5").unwrap();
        assert_eq!((haiku.cache_write_5m, haiku.cache_write_1h), (0.125, 0.2));
        assert_eq!(
            price_of("claude-sonnet-5-5-20260901"),
            price_of("claude-sonnet-5-5")
        );
        assert!(price_of("claude-opus-5").is_some());
    }

    #[test]
    fn unknown_models_are_unpriced() {
        assert!(price_of("<synthetic>").is_none());
        assert!(price_of("claude-mystery-9").is_none());
        assert!(price_of("").is_none());
    }

    #[test]
    fn five_minute_writes_are_cheaper_than_one_hour() {
        let p = price_of("claude-opus-5-5").unwrap();
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
