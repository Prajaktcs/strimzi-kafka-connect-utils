use std::collections::{BTreeMap, BTreeSet};

use super::MetricSample;
use crate::{Error, Result};

pub(super) const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SAMPLES: usize = 10_000;
const MAX_LINE_BYTES: usize = 16 * 1024;
const MAX_LABELS: usize = 32;
const MAX_TOTAL_LABELS: usize = 32_768;
const MAX_NAME_BYTES: usize = 256;
const MAX_LABEL_VALUE_BYTES: usize = 4096;

/// Parse Prometheus text exposition without aggregating any identities.
///
/// Supports escaped labels, scientific notation, nonfinite values, comments,
/// and optional integer timestamps. Nonfinite values are preserved as exported;
/// consumers must use `WorkerMetrics::lookup` or explicitly check finiteness.
/// Errors contain only a line number and a fixed reason, never response text.
pub fn parse_prometheus(text: &str) -> Result<Vec<MetricSample>> {
    if text.len() > MAX_RESPONSE_BYTES {
        return Err(parse_error(0, "response exceeds size limit"));
    }
    let mut samples = Vec::new();
    let mut label_count = 0;
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        if line.len() > MAX_LINE_BYTES {
            return Err(parse_error(number, "line exceeds size limit"));
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if samples.len() >= MAX_SAMPLES {
            return Err(parse_error(number, "too many samples"));
        }
        let sample = parse_line(line).map_err(|reason| parse_error(number, reason))?;
        label_count += sample.labels.len();
        if label_count > MAX_TOTAL_LABELS {
            return Err(parse_error(number, "too many label identities"));
        }
        samples.push(sample);
    }
    {
        // Borrow identities rather than copying metric names and label sets.
        let mut identities = BTreeSet::new();
        for sample in &samples {
            if !identities.insert((&sample.name, &sample.labels)) {
                return Err(parse_error(0, "duplicate sample identity"));
            }
        }
    }
    Ok(samples)
}

fn parse_error(line: usize, reason: &str) -> Error {
    Error::Parse {
        format: "Prometheus metrics",
        reason: format!("line {line}: {reason}"),
    }
}

fn parse_line(line: &str) -> std::result::Result<MetricSample, &'static str> {
    let name_end = line
        .bytes()
        .position(|byte| !name_byte(byte, true))
        .unwrap_or(line.len());
    let name = &line[..name_end];
    if name.is_empty() || name.len() > MAX_NAME_BYTES || !name_byte(name.as_bytes()[0], false) {
        return Err("invalid metric name");
    }
    let mut rest = &line[name_end..];
    let labels = if rest.starts_with('{') {
        let (labels, remaining) = parse_labels(&rest[1..])?;
        rest = remaining;
        labels
    } else {
        BTreeMap::new()
    };
    if !rest.starts_with([' ', '\t']) {
        return Err("missing sample value");
    }
    let mut fields = rest.split_ascii_whitespace();
    let value = fields.next().ok_or("missing sample value")?;
    let unsigned = value.strip_prefix(['+', '-']).unwrap_or(value);
    let special = unsigned.eq_ignore_ascii_case("nan")
        || unsigned.eq_ignore_ascii_case("inf")
        || unsigned.eq_ignore_ascii_case("infinity");
    if !special
        && (!value.bytes().any(|byte| byte.is_ascii_digit())
            || !value.bytes().all(|byte| {
                byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.' | b'e' | b'E')
            }))
    {
        return Err("invalid sample value");
    }
    let value = value.parse::<f64>().map_err(|_| "invalid sample value")?;
    if let Some(timestamp) = fields.next() {
        timestamp
            .parse::<i64>()
            .map_err(|_| "invalid sample timestamp")?;
    }
    if fields.next().is_some() {
        return Err("unexpected sample fields");
    }
    Ok(MetricSample {
        name: name.to_owned(),
        labels,
        value,
    })
}

fn name_byte(byte: u8, digit: bool) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b':') || (digit && byte.is_ascii_digit())
}

fn label_byte(byte: u8, digit: bool) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || (digit && byte.is_ascii_digit())
}

fn parse_labels(
    mut rest: &str,
) -> std::result::Result<(BTreeMap<String, String>, &str), &'static str> {
    let mut labels = BTreeMap::new();
    loop {
        rest = rest.trim_start_matches([' ', '\t']);
        if let Some(remaining) = rest.strip_prefix('}') {
            return Ok((labels, remaining));
        }
        if labels.len() >= MAX_LABELS {
            return Err("too many labels");
        }
        let name_end = rest
            .bytes()
            .position(|byte| !label_byte(byte, true))
            .unwrap_or(rest.len());
        let name = &rest[..name_end];
        if name.is_empty() || name.len() > MAX_NAME_BYTES || !label_byte(name.as_bytes()[0], false)
        {
            return Err("invalid label name");
        }
        rest = rest[name_end..].trim_start_matches([' ', '\t']);
        rest = rest.strip_prefix('=').ok_or("missing label assignment")?;
        rest = rest.trim_start_matches([' ', '\t']);
        let (value, remaining) = parse_label_value(rest)?;
        if labels.insert(name.to_owned(), value).is_some() {
            return Err("duplicate label name");
        }
        rest = remaining.trim_start_matches([' ', '\t']);
        if let Some(remaining) = rest.strip_prefix(',') {
            rest = remaining;
        } else if let Some(remaining) = rest.strip_prefix('}') {
            return Ok((labels, remaining));
        } else {
            return Err("invalid label separator");
        }
    }
}

fn parse_label_value(rest: &str) -> std::result::Result<(String, &str), &'static str> {
    let rest = rest.strip_prefix('"').ok_or("unquoted label value")?;
    let mut value = String::new();
    let mut chars = rest.char_indices();
    while let Some((index, character)) = chars.next() {
        match character {
            '"' => return Ok((value, &rest[index + 1..])),
            '\\' => {
                let (_, escaped) = chars.next().ok_or("unfinished label escape")?;
                value.push(match escaped {
                    '\\' => '\\',
                    '"' => '"',
                    'n' => '\n',
                    _ => return Err("invalid label escape"),
                });
            }
            '\n' | '\r' => return Err("invalid label value"),
            character => value.push(character),
        }
        if value.len() > MAX_LABEL_VALUE_BYTES {
            return Err("label value exceeds size limit");
        }
    }
    Err("unterminated label value")
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use super::*;

    #[test]
    fn parses_exported_task_labels_escapes_and_timestamps_without_merging() {
        let text = r#"# HELP kafka_connect_source_task_source_record_poll_total Real counter
# TYPE kafka_connect_source_task_source_record_poll_total counter
kafka_connect_source_task_source_record_poll_total{connector="orders",task="0",note="quote\" slash\\ line\n雪"} 1.25e+3 1720000000000
kafka_connect_source_task_source_record_poll_total{task="1",connector="orders"} -2.5E-1
"#;
        let samples = parse_prometheus(text).unwrap();
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].value.to_bits(), 1250.0_f64.to_bits());
        assert_eq!(samples[1].value.to_bits(), (-0.25_f64).to_bits());
        assert_eq!(samples[0].labels["task"], "0");
        assert_eq!(samples[1].labels["task"], "1");
        assert_eq!(samples[0].labels["note"], "quote\" slash\\ line\n雪");
    }

    #[test]
    fn rejects_ambiguous_duplicate_identities_even_when_label_order_differs() {
        let text =
            "metric{connector=\"orders\",task=\"0\"} 1\nmetric{task=\"0\",connector=\"orders\"} 2";
        let error = parse_prometheus(text).unwrap_err();
        assert!(error.to_string().contains("duplicate sample identity"));
        assert!(parse_prometheus("metric{task=\"0\",task=\"1\"} 3").is_err());
    }

    #[test]
    fn rejects_malformed_samples_without_echoing_response_secrets() {
        for text in [
            "metric{token=\"secret\"} secret-token",
            "metric{token=\"secret\\q\"} 1",
            "metric{token=\"secret\" 1",
            "metric{token=secret} 1",
            "metric{token=\"secret\"} 1 nope",
            "metric{token=\"secret\"} 1 123 extra",
            "9metric 1",
            "metric 1 # inline-comment",
        ] {
            let error = parse_prometheus(text).unwrap_err().to_string();
            assert!(!error.contains("secret"), "{error}");
        }
    }

    #[test]
    fn preserves_real_zero_and_nonfinite_values_without_substitution() {
        let samples = parse_prometheus(
            "zero 0\nnan NaN\npos +Inf\nneg -Inf\noverflow 1e999\nupper Inf\nlower -infinity\nsignednan +nan",
        )
        .unwrap();
        assert_eq!(samples[0].value.to_bits(), 0.0_f64.to_bits());
        assert!(samples[1].value.is_nan());
        assert_eq!(samples[2].value.to_bits(), f64::INFINITY.to_bits());
        assert_eq!(samples[3].value.to_bits(), f64::NEG_INFINITY.to_bits());
        assert!(!samples[4].value.is_finite());
        assert_eq!(samples[5].value.to_bits(), f64::INFINITY.to_bits());
        assert_eq!(samples[6].value.to_bits(), f64::NEG_INFINITY.to_bits());
        assert!(samples[7].value.is_nan());
    }

    #[test]
    fn rejects_bounded_resources_and_accepts_comment_only_exporter() {
        assert!(parse_prometheus("# HELP no_samples unavailable\n\n")
            .unwrap()
            .is_empty());
        assert!(parse_prometheus(&"x".repeat(MAX_RESPONSE_BYTES + 1)).is_err());
        assert!(parse_prometheus(&format!(
            "metric{{label=\"{}\"}} 1",
            "x".repeat(MAX_LABEL_VALUE_BYTES + 1)
        ))
        .is_err());
        let mut excessive = String::new();
        for index in 0..=MAX_SAMPLES {
            writeln!(excessive, "metric{{task=\"{index}\"}} 1").unwrap();
        }
        assert!(parse_prometheus(&excessive).is_err());
        let mut excessive_labels = String::new();
        for sample in 0..=(MAX_TOTAL_LABELS / MAX_LABELS) {
            excessive_labels.push_str("metric{");
            for label in 0..MAX_LABELS {
                if label > 0 {
                    excessive_labels.push(',');
                }
                write!(excessive_labels, "label_{label}=\"{sample}\"").unwrap();
            }
            excessive_labels.push_str("} 1\n");
        }
        assert!(parse_prometheus(&excessive_labels)
            .unwrap_err()
            .to_string()
            .contains("too many label identities"));
    }
}
