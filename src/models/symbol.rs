use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolAliasKind {
    CaseInsensitive,
    CentAlias,
    UniqueAffix,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolMatch {
    Exact(String),
    Alias {
        resolved: String,
        kind: SymbolAliasKind,
    },
    Ambiguous(Vec<String>),
    NoMatch,
}

impl SymbolMatch {
    pub fn resolved(&self) -> Option<&str> {
        match self {
            Self::Exact(symbol)
            | Self::Alias {
                resolved: symbol, ..
            } => Some(symbol),
            Self::Ambiguous(_) | Self::NoMatch => None,
        }
    }

    pub fn candidates(&self) -> &[String] {
        match self {
            Self::Ambiguous(candidates) => candidates,
            _ => &[],
        }
    }
}

pub fn resolve_symbol(requested: &str, available: &[String]) -> SymbolMatch {
    let requested = requested.trim();
    if requested.is_empty() || available.is_empty() {
        return SymbolMatch::NoMatch;
    }

    let mut catalog = available.to_vec();
    catalog.sort();
    catalog.dedup();

    if catalog.iter().any(|symbol| symbol == requested) {
        return SymbolMatch::Exact(requested.to_string());
    }

    let requested_lower = requested.to_ascii_lowercase();
    let case_matches = matching(&catalog, |symbol| {
        symbol.to_ascii_lowercase() == requested_lower
    });
    if let Some(result) = unique_or_ambiguous(case_matches, SymbolAliasKind::CaseInsensitive) {
        return result;
    }

    let requested_cent_base = cent_base(requested);
    let cent_matches = matching(&catalog, |symbol| cent_base(symbol) == requested_cent_base);
    if let Some(result) = unique_or_ambiguous(cent_matches, SymbolAliasKind::CentAlias) {
        return result;
    }

    let affix_matches = matching(&catalog, |symbol| {
        let symbol_lower = symbol.to_ascii_lowercase();
        symbol_lower.len() > requested_lower.len()
            && (symbol_lower.starts_with(&requested_lower)
                || symbol_lower.ends_with(&requested_lower))
    });
    unique_or_ambiguous(affix_matches, SymbolAliasKind::UniqueAffix).unwrap_or(SymbolMatch::NoMatch)
}

fn matching<F>(catalog: &[String], predicate: F) -> Vec<String>
where
    F: Fn(&str) -> bool,
{
    catalog
        .iter()
        .filter(|symbol| predicate(symbol))
        .cloned()
        .collect()
}

fn unique_or_ambiguous(candidates: Vec<String>, kind: SymbolAliasKind) -> Option<SymbolMatch> {
    match candidates.as_slice() {
        [] => None,
        [resolved] => Some(SymbolMatch::Alias {
            resolved: resolved.clone(),
            kind,
        }),
        _ => Some(SymbolMatch::Ambiguous(candidates)),
    }
}

fn cent_base(symbol: &str) -> String {
    let symbol_lower = symbol.to_ascii_lowercase();
    for suffix in [".cent", ".c"] {
        if symbol_lower.ends_with(suffix) {
            return symbol[..symbol.len() - suffix.len()].to_ascii_lowercase();
        }
    }

    if let Some(base) = symbol.strip_suffix('c') {
        if !base.is_empty()
            && base
                .chars()
                .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
        {
            return base.to_ascii_lowercase();
        }
    }
    symbol.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbols(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn resolves_each_priority_class() {
        assert_eq!(
            resolve_symbol("EURUSD", &symbols(&["EURUSD", "EURUSDm"])),
            SymbolMatch::Exact("EURUSD".into())
        );
        assert_eq!(
            resolve_symbol("eurusd", &symbols(&["EURUSD"])),
            SymbolMatch::Alias {
                resolved: "EURUSD".into(),
                kind: SymbolAliasKind::CaseInsensitive,
            }
        );
        assert_eq!(
            resolve_symbol("XAUUSD", &symbols(&["XAUUSD.cent"])),
            SymbolMatch::Alias {
                resolved: "XAUUSD.cent".into(),
                kind: SymbolAliasKind::CentAlias,
            }
        );
        assert_eq!(
            resolve_symbol("EURUSD", &symbols(&["EURUSDm"])),
            SymbolMatch::Alias {
                resolved: "EURUSDm".into(),
                kind: SymbolAliasKind::UniqueAffix,
            }
        );
        assert_eq!(
            resolve_symbol("EURUSD", &symbols(&["fxEURUSD"])),
            SymbolMatch::Alias {
                resolved: "fxEURUSD".into(),
                kind: SymbolAliasKind::UniqueAffix,
            }
        );
    }

    #[test]
    fn ambiguity_is_deterministic_and_never_selects_first() {
        assert_eq!(
            resolve_symbol("EURUSD", &symbols(&["EURUSDz", "EURUSDm", "EURUSDm"])),
            SymbolMatch::Ambiguous(vec!["EURUSDm".into(), "EURUSDz".into()])
        );
    }

    #[test]
    fn no_match_covers_empty_and_unrelated_catalogs() {
        assert_eq!(resolve_symbol("EURUSD", &[]), SymbolMatch::NoMatch);
        assert_eq!(
            resolve_symbol("EURUSD", &symbols(&["GBPJPY"])),
            SymbolMatch::NoMatch
        );
        assert_eq!(
            resolve_symbol("BTCUSD", &symbols(&["B", "BTC", "USD", "SD"])),
            SymbolMatch::NoMatch
        );
    }

    #[test]
    fn uppercase_c_is_not_treated_as_a_cent_suffix() {
        assert_eq!(
            resolve_symbol("USDC", &symbols(&["USD"])),
            SymbolMatch::NoMatch
        );
        assert_eq!(
            resolve_symbol("XAUUSDc", &symbols(&["XAUUSD.cent"])),
            SymbolMatch::Alias {
                resolved: "XAUUSD.cent".into(),
                kind: SymbolAliasKind::CentAlias,
            }
        );
    }

    #[test]
    fn dotted_cent_suffixes_remain_case_insensitive() {
        assert_eq!(
            resolve_symbol("XAUUSD", &symbols(&["XAUUSD.C", "XAUUSDm"])),
            SymbolMatch::Alias {
                resolved: "XAUUSD.C".into(),
                kind: SymbolAliasKind::CentAlias,
            }
        );
        assert_eq!(
            resolve_symbol("XAUUSD", &symbols(&["XAUUSD.CENT", "XAUUSDm"])),
            SymbolMatch::Alias {
                resolved: "XAUUSD.CENT".into(),
                kind: SymbolAliasKind::CentAlias,
            }
        );
    }
}
