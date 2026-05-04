//! Import deduplication with exact and fuzzy matching.
//!
//! Three match levels:
//! - **Exact**: same FITID → auto-skip
//! - **Strong**: same date + amount + payee similarity ≥ 0.85 → flag
//! - **Weak**: date ±2 days + same amount → flag for review

use super::profile::ImportCandidate;

/// Result of deduplication check for a single candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DedupResult {
    /// No match — safe to import.
    New,
    /// Exact duplicate (same FITID) — auto-skip.
    ExactDuplicate { existing_index: usize },
    /// Strong match (same date + amount + similar payee) — needs review.
    StrongMatch {
        existing_index: usize,
        similarity: u32,
    },
    /// Weak match (nearby date + same amount) — needs review.
    WeakMatch { existing_index: usize },
}

/// An existing transaction summary for dedup comparison.
#[derive(Debug, Clone)]
pub struct ExistingTransaction {
    pub date: String,
    pub description: String,
    pub amount: String,
    pub fitid: Option<String>,
}

/// Check a candidate against existing transactions for duplicates.
pub fn check_duplicate(
    candidate: &ImportCandidate,
    existing: &[ExistingTransaction],
) -> DedupResult {
    // 1. Exact FITID match
    if let Some(fitid) = &candidate.fitid {
        if !fitid.is_empty() {
            for (i, ex) in existing.iter().enumerate() {
                if ex.fitid.as_deref() == Some(fitid.as_str()) {
                    return DedupResult::ExactDuplicate { existing_index: i };
                }
            }
        }
    }

    let cand_amount = candidate.amount.trim();

    // 2. Strong match: same date + same amount + payee similarity ≥ 85%
    for (i, ex) in existing.iter().enumerate() {
        if ex.date == candidate.date && ex.amount.trim() == cand_amount {
            let sim = jaro_winkler(
                &normalize_payee(&candidate.description),
                &normalize_payee(&ex.description),
            );
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let sim_pct = (sim * 100.0) as u32;
            if sim_pct >= 85 {
                return DedupResult::StrongMatch {
                    existing_index: i,
                    similarity: sim_pct,
                };
            }
        }
    }

    // 3. Weak match: date ±2 days + same amount
    for (i, ex) in existing.iter().enumerate() {
        if ex.amount.trim() == cand_amount && dates_within_days(&candidate.date, &ex.date, 2) {
            return DedupResult::WeakMatch { existing_index: i };
        }
    }

    DedupResult::New
}

/// Run dedup on all candidates, returning results aligned by index.
pub fn deduplicate(
    candidates: &[ImportCandidate],
    existing: &[ExistingTransaction],
) -> Vec<DedupResult> {
    candidates
        .iter()
        .map(|c| check_duplicate(c, existing))
        .collect()
}

// ── Jaro-Winkler string similarity ────────────────────────────────────────────

/// Jaro-Winkler similarity between two strings (0.0 to 1.0).
#[allow(clippy::cast_precision_loss)] // Jaro similarity — precision loss is negligible for string lengths
fn jaro_winkler(s1: &str, s2: &str) -> f64 {
    let jaro = jaro_similarity(s1, s2);
    // Winkler prefix bonus (up to 4 chars)
    let prefix_len = s1
        .chars()
        .zip(s2.chars())
        .take(4)
        .take_while(|(a, b)| a == b)
        .count();
    let p = 0.1;
    jaro + (prefix_len as f64) * p * (1.0 - jaro)
}

#[allow(clippy::cast_precision_loss)]
fn jaro_similarity(s1: &str, s2: &str) -> f64 {
    if s1.is_empty() && s2.is_empty() {
        return 1.0;
    }
    if s1.is_empty() || s2.is_empty() {
        return 0.0;
    }

    let s1_chars: Vec<char> = s1.chars().collect();
    let s2_chars: Vec<char> = s2.chars().collect();
    let len1 = s1_chars.len();
    let len2 = s2_chars.len();

    let match_distance = (len1.max(len2) / 2).saturating_sub(1);

    let mut s1_matches = vec![false; len1];
    let mut s2_matches = vec![false; len2];
    let mut matches = 0usize;
    let mut transpositions = 0usize;

    for i in 0..len1 {
        let start = i.saturating_sub(match_distance);
        let end = (i + match_distance + 1).min(len2);
        for j in start..end {
            if s2_matches[j] || s1_chars[i] != s2_chars[j] {
                continue;
            }
            s1_matches[i] = true;
            s2_matches[j] = true;
            matches += 1;
            break;
        }
    }

    if matches == 0 {
        return 0.0;
    }

    let mut k = 0;
    for i in 0..len1 {
        if !s1_matches[i] {
            continue;
        }
        while !s2_matches[k] {
            k += 1;
        }
        if s1_chars[i] != s2_chars[k] {
            transpositions += 1;
        }
        k += 1;
    }

    let m = matches as f64;
    (m / len1 as f64 + m / len2 as f64 + (m - transpositions as f64 / 2.0) / m) / 3.0
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Normalize a payee string for comparison: lowercase, strip common prefixes.
fn normalize_payee(s: &str) -> String {
    let lower = s.to_lowercase();
    let stripped = lower
        .trim_start_matches("pos ")
        .trim_start_matches("ach ")
        .trim_start_matches("check ")
        .trim_start_matches("wire ")
        .trim_start_matches("debit ")
        .trim_start_matches("credit ");
    stripped.trim().to_string()
}

/// Check if two ISO dates are within `days` of each other.
fn dates_within_days(d1: &str, d2: &str, days: i64) -> bool {
    let Ok(nd1) = chrono::NaiveDate::parse_from_str(d1, "%Y-%m-%d") else {
        return false;
    };
    let Ok(nd2) = chrono::NaiveDate::parse_from_str(d2, "%Y-%m-%d") else {
        return false;
    };
    (nd1 - nd2).num_days().abs() <= days
}

#[cfg(test)]
mod tests {
    use super::*;

    fn existing(date: &str, desc: &str, amount: &str, fitid: Option<&str>) -> ExistingTransaction {
        ExistingTransaction {
            date: date.into(),
            description: desc.into(),
            amount: amount.into(),
            fitid: fitid.map(String::from),
        }
    }

    fn candidate(date: &str, desc: &str, amount: &str, fitid: Option<&str>) -> ImportCandidate {
        ImportCandidate {
            date: date.into(),
            description: desc.into(),
            amount: amount.into(),
            source_account: "Assets:Checking".into(),
            target_account: None,
            source_row: 1,
            fitid: fitid.map(String::from),
        }
    }

    #[test]
    fn exact_fitid_duplicate() {
        let existing = vec![existing(
            "2024-01-15",
            "WHOLE FOODS",
            "-42.50",
            Some("FIT001"),
        )];
        let cand = candidate("2024-01-15", "WHOLE FOODS", "-42.50", Some("FIT001"));
        assert!(matches!(
            check_duplicate(&cand, &existing),
            DedupResult::ExactDuplicate { .. }
        ));
    }

    #[test]
    fn strong_match_same_date_amount_similar_payee() {
        let existing = vec![existing("2024-01-15", "WHOLE FOODS #123", "-42.50", None)];
        let cand = candidate("2024-01-15", "WHOLE FOODS MARKET", "-42.50", None);
        let result = check_duplicate(&cand, &existing);
        assert!(
            matches!(result, DedupResult::StrongMatch { .. }),
            "expected StrongMatch, got {result:?}"
        );
    }

    #[test]
    fn weak_match_nearby_date_same_amount() {
        let existing = vec![existing("2024-01-14", "GROCERY STORE", "-42.50", None)];
        let cand = candidate("2024-01-15", "DIFFERENT STORE", "-42.50", None);
        assert!(matches!(
            check_duplicate(&cand, &existing),
            DedupResult::WeakMatch { .. }
        ));
    }

    #[test]
    fn no_match_different_amount() {
        let existing = vec![existing("2024-01-15", "WHOLE FOODS", "-42.50", None)];
        let cand = candidate("2024-01-15", "WHOLE FOODS", "-99.99", None);
        assert_eq!(check_duplicate(&cand, &existing), DedupResult::New);
    }

    #[test]
    fn no_match_distant_date() {
        let existing = vec![existing("2024-01-10", "STORE", "-42.50", None)];
        let cand = candidate("2024-01-20", "STORE", "-42.50", None);
        assert_eq!(check_duplicate(&cand, &existing), DedupResult::New);
    }

    #[test]
    fn jaro_winkler_identical_strings() {
        let sim = jaro_winkler("hello", "hello");
        assert!((sim - 1.0).abs() < 0.001);
    }

    #[test]
    fn jaro_winkler_similar_strings() {
        let sim = jaro_winkler("whole foods", "whole foods market");
        assert!(sim > 0.85, "expected > 0.85, got {sim}");
    }

    #[test]
    fn jaro_winkler_different_strings() {
        let sim = jaro_winkler("whole foods", "gas station");
        assert!(sim < 0.7, "expected < 0.7, got {sim}");
    }

    #[test]
    fn normalize_strips_prefixes() {
        assert_eq!(normalize_payee("POS WHOLE FOODS"), "whole foods");
        assert_eq!(normalize_payee("ACH DIRECT DEPOSIT"), "direct deposit");
    }

    #[test]
    fn deduplicate_batch() {
        let existing = vec![
            existing("2024-01-15", "WHOLE FOODS", "-42.50", Some("FIT001")),
            existing("2024-01-16", "GAS STATION", "-35.00", None),
        ];
        let candidates = vec![
            candidate("2024-01-15", "WHOLE FOODS", "-42.50", Some("FIT001")), // exact
            candidate("2024-01-17", "NEW PURCHASE", "-100.00", None),         // new
        ];

        let results = deduplicate(&candidates, &existing);
        assert!(matches!(results[0], DedupResult::ExactDuplicate { .. }));
        assert_eq!(results[1], DedupResult::New);
    }

    #[test]
    fn dates_within_range() {
        assert!(dates_within_days("2024-01-15", "2024-01-17", 2));
        assert!(dates_within_days("2024-01-15", "2024-01-13", 2));
        assert!(!dates_within_days("2024-01-15", "2024-01-20", 2));
    }
}
