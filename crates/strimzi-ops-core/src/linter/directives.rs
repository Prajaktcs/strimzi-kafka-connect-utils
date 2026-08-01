use std::collections::BTreeSet;
use std::sync::OnceLock;

use regex::Regex;

pub fn parse_lint_directives(text: &str) -> BTreeSet<String> {
    static PATTERNS: OnceLock<[Regex; 2]> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        [
            Regex::new(r"(?i)#[ \t]*lint-disable(?:-file)?:[ \t]*([\w ,-]+)")
                .expect("internal error: invalid lint-disable regex"),
            Regex::new(r"(?i)#[ \t]*lint-disable(?:-file)?[ \t]+([\w ,-]+)")
                .expect("internal error: invalid lint-disable regex"),
        ]
    });

    let mut disabled = BTreeSet::new();
    for pattern in patterns {
        for caps in pattern.captures_iter(text) {
            let Some(rules) = caps.get(1) else {
                continue;
            };
            for rule in rules.as_str().split(',') {
                let trimmed = rule.trim();
                if !trimmed.is_empty() {
                    disabled.insert(trimmed.to_owned());
                }
            }
        }
    }
    disabled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comma_separated_directives() {
        let text = "# lint-disable: naming-convention, sensitive-data\nname: x\n";
        let disabled = parse_lint_directives(text);
        assert!(disabled.contains("naming-convention"));
        assert!(disabled.contains("sensitive-data"));
    }
}
