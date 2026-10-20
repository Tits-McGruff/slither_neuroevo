//! Streaming resource preflight for legacy population JSON before materialization.
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::fmt;
use std::io::{self, Read};

#[derive(Clone, Copy)]
enum Context {
    Root,
    Genomes,
    Genome,
    Weights,
    WeightScalar,
    Other,
}
struct Budget {
    owned: usize,
    scratch: usize,
    limit: usize,
}
impl Budget {
    fn charge<E: serde::de::Error>(&mut self, bytes: usize) -> Result<(), E> {
        self.owned = self
            .owned
            .checked_add(bytes)
            .ok_or_else(|| E::custom("legacy resource estimate overflow"))?;
        self.check()
    }
    fn string<E: serde::de::Error>(&mut self, bytes: usize) -> Result<(), E> {
        let bound = bytes
            .checked_mul(2)
            .ok_or_else(|| E::custom("legacy string estimate overflow"))?;
        self.scratch = self.scratch.max(bound);
        self.charge(bound)
    }
    fn check<E: serde::de::Error>(&self) -> Result<(), E> {
        let peak = self
            .owned
            .checked_add(self.scratch)
            .ok_or_else(|| E::custom("legacy resource peak overflow"))?;
        if peak > self.limit {
            return Err(E::custom(format!(
                "legacy JSON needs at least {peak} decoded/work bytes, budget {}",
                self.limit
            )));
        }
        Ok(())
    }
}
struct Estimate<'a> {
    budget: &'a mut Budget,
    context: Context,
}
impl<'de> DeserializeSeed<'de> for Estimate<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<(), D::Error> {
        decoder.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Estimate<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("bounded legacy population JSON")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        // Covers map/struct storage and allocator bookkeeping, including settings Values.
        self.budget.charge(256)?;
        while let Some(key) = map.next_key::<String>()? {
            self.budget.charge(192)?;
            self.budget.string(key.len())?;
            let context = match (self.context, key.as_str()) {
                (Context::Root, "genomes") => Context::Genomes,
                (Context::Genome, "weights") => Context::Weights,
                _ => Context::Other,
            };
            map.next_value_seed(Estimate {
                budget: self.budget,
                context,
            })?;
        }
        Ok(())
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        self.budget.charge(64)?;
        let context = match self.context {
            Context::Genomes => Context::Genome,
            Context::Weights => Context::WeightScalar,
            _ => Context::Other,
        };
        while seq
            .next_element_seed(Estimate {
                budget: self.budget,
                context,
            })?
            .is_some()
        {}
        Ok(())
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<(), E> {
        self.budget.charge(128)?;
        self.budget.string(value.len())
    }
    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<(), E> {
        self.visit_str(&value)
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        self.budget.charge(128)
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        self.budget.charge(128)
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        self.number()
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        self.number()
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        self.number()
    }
}
impl Estimate<'_> {
    fn number<E: serde::de::Error>(self) -> Result<(), E> {
        // Only the exact genomes[].weights path materializes Float32 vectors.
        // Two times scalar bytes covers geometric Vec capacity; other numbers
        // may be serde_json::Value entries with much larger ownership overhead.
        self.budget
            .charge(if matches!(self.context, Context::WeightScalar) {
                8
            } else {
                128
            })
    }
}

/// Stop an oversized string before serde can grow its decoding scratch buffer.
struct TokenBound<R> {
    input: R,
    in_string: bool,
    escaped: bool,
    bytes: usize,
    maximum: usize,
}
impl<R: Read> Read for TokenBound<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let count = self.input.read(output)?;
        for byte in &output[..count] {
            if self.in_string {
                self.bytes = self
                    .bytes
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("legacy string length overflow"))?;
                if self.bytes > self.maximum {
                    return Err(io::Error::other(
                        "legacy JSON string exceeds its resource budget",
                    ));
                }
                if self.escaped {
                    self.escaped = false
                } else if *byte == b'\\' {
                    self.escaped = true
                } else if *byte == b'"' {
                    self.in_string = false
                }
            } else if *byte == b'"' {
                self.in_string = true;
                self.bytes = 0
            }
        }
        super::work_progress::advance(count);
        Ok(count)
    }
}

/// Price materialized values separately from decoder scratch, without owning a
/// population or settings tree. Caller subtracts returned ownership while constructing
/// the new boundary, then may reclaim its selected budget after buffers retire.
pub(crate) fn estimate<R: Read>(input: R, limit: usize) -> Result<usize, String> {
    const BASE: usize = 64 * 1024;
    if limit < BASE {
        return Err(format!(
            "legacy JSON requires at least {BASE} work bytes, budget {limit}"
        ));
    }
    let reader = TokenBound {
        input,
        in_string: false,
        escaped: false,
        bytes: 0,
        maximum: (limit - BASE) / 4,
    };
    let mut decoder = serde_json::Deserializer::from_reader(std::io::BufReader::new(reader));
    let mut budget = Budget {
        owned: BASE,
        scratch: 0,
        limit,
    };
    Estimate {
        budget: &mut budget,
        context: Context::Root,
    }
    .deserialize(&mut decoder)
    .map_err(|e| e.to_string())?;
    decoder.end().map_err(|e| e.to_string())?;
    Ok(budget.owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_weight_path_is_priced_as_f32_but_settings_cannot_spoof_it() {
        let weights = (0..1000).map(|_| "0").collect::<Vec<_>>().join(",");
        let population = format!("{{\"genomes\":[{{\"weights\":[{weights}]}}]}}");
        let settings = format!("{{\"settings\":{{\"genomes\":[{{\"weights\":[{weights}]}}]}}}}");
        let small = estimate(population.as_bytes(), 96 * 1024).unwrap();
        assert!(small < 80 * 1024);
        assert!(estimate(settings.as_bytes(), 96 * 1024).is_err());
        assert!(estimate(population.as_bytes(), 64 * 1024).is_err());
    }
    #[test]
    fn oversized_escaped_strings_and_trailing_data_reject_before_materialization() {
        let long = format!("{{\"settings\":\"{}\"}}", "a".repeat(40 * 1024));
        assert!(estimate(long.as_bytes(), 128 * 1024).is_err());
        assert!(estimate(
            br#"{"runId":"escaped\\\"quote"} trailing"#.as_slice(),
            128 * 1024
        )
        .is_err());
        assert!(estimate(br#"{"runId":"escaped\\\"quote"}"#.as_slice(), 128 * 1024).is_ok());
    }
}
