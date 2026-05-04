//! Import rules engine for auto-categorization.
//!
//! Rules map payee/description patterns to target accounts. During import,
//! each transaction's description is matched against rules in priority order
//! (highest priority first). The first matching rule assigns the target account.

use serde::{Deserialize, Serialize};

use super::profile::ImportCandidate;

/// How the rule pattern matches against the description.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatchType {
    /// Case-insensitive substring match.
    Contains,
    /// Exact match (case-insensitive).
    Exact,
    /// Case-insensitive prefix match.
    StartsWith,
}

/// A single auto-categorization rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportRule {
    /// Unique identifier.
    pub id: String,
    /// Higher priority rules are checked first.
    pub priority: i64,
    /// The pattern to match against the transaction description.
    pub pattern: String,
    /// How to match the pattern.
    pub match_type: MatchType,
    /// The account to assign when the rule matches.
    pub target_account: String,
}

/// Apply rules to a list of import candidates, setting `target_account`
/// on each candidate that matches a rule.
///
/// Rules are applied in priority order (highest first). The first matching
/// rule wins for each candidate.
pub fn apply_rules(candidates: &mut [ImportCandidate], rules: &[ImportRule]) {
    // Sort rules by priority descending
    let mut sorted_rules: Vec<&ImportRule> = rules.iter().collect();
    sorted_rules.sort_by_key(|r| std::cmp::Reverse(r.priority));

    for candidate in candidates.iter_mut() {
        if candidate.target_account.is_some() {
            continue; // already assigned
        }

        for rule in &sorted_rules {
            if matches_rule(&candidate.description, rule) {
                candidate.target_account = Some(rule.target_account.clone());
                break;
            }
        }
    }
}

/// Test a single description against a single rule.
pub fn matches_rule(description: &str, rule: &ImportRule) -> bool {
    let desc_lower = description.to_lowercase();
    let pattern_lower = rule.pattern.to_lowercase();

    match rule.match_type {
        MatchType::Contains => desc_lower.contains(&pattern_lower),
        MatchType::Exact => desc_lower == pattern_lower,
        MatchType::StartsWith => desc_lower.starts_with(&pattern_lower),
    }
}

/// Test which rule (if any) would match a given description.
///
/// Returns the first matching rule in priority order.
pub fn test_rules<'a>(description: &str, rules: &'a [ImportRule]) -> Option<&'a ImportRule> {
    let mut sorted: Vec<&ImportRule> = rules.iter().collect();
    sorted.sort_by_key(|r| std::cmp::Reverse(r.priority));

    sorted
        .into_iter()
        .find(|rule| matches_rule(description, rule))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_rules() -> Vec<ImportRule> {
        vec![
            ImportRule {
                id: "r1".into(),
                priority: 10,
                pattern: "WHOLE FOODS".into(),
                match_type: MatchType::Contains,
                target_account: "Expenses:Food:Groceries".into(),
            },
            ImportRule {
                id: "r2".into(),
                priority: 5,
                pattern: "AMZN".into(),
                match_type: MatchType::Contains,
                target_account: "Expenses:Shopping:Online".into(),
            },
            ImportRule {
                id: "r3".into(),
                priority: 20,
                pattern: "DIRECT DEPOSIT".into(),
                match_type: MatchType::Exact,
                target_account: "Income:Salary".into(),
            },
        ]
    }

    fn candidate(desc: &str) -> ImportCandidate {
        ImportCandidate {
            date: "2024-01-15".into(),
            description: desc.into(),
            amount: "-42.50".into(),
            source_account: "Assets:Checking".into(),
            target_account: None,
            source_row: 1,
        }
    }

    #[test]
    fn contains_match() {
        let rules = sample_rules();
        assert!(matches_rule("WHOLE FOODS #123", &rules[0]));
        assert!(!matches_rule("TRADER JOES", &rules[0]));
    }

    #[test]
    fn exact_match() {
        let rules = sample_rules();
        assert!(matches_rule("DIRECT DEPOSIT", &rules[2]));
        assert!(!matches_rule("DIRECT DEPOSIT EXTRA", &rules[2]));
    }

    #[test]
    fn case_insensitive() {
        let rules = sample_rules();
        assert!(matches_rule("whole foods market", &rules[0]));
        assert!(matches_rule("direct deposit", &rules[2]));
    }

    #[test]
    fn starts_with_match() {
        let rule = ImportRule {
            id: "r".into(),
            priority: 1,
            pattern: "STARBUCKS".into(),
            match_type: MatchType::StartsWith,
            target_account: "Expenses:Food:Coffee".into(),
        };
        assert!(matches_rule("STARBUCKS #456", &rule));
        assert!(!matches_rule("AT STARBUCKS", &rule));
    }

    #[test]
    fn apply_rules_sets_target_account() {
        let rules = sample_rules();
        let mut candidates = vec![
            candidate("WHOLE FOODS #123"),
            candidate("AMZN MKTP US"),
            candidate("UNKNOWN VENDOR"),
        ];

        apply_rules(&mut candidates, &rules);

        assert_eq!(
            candidates[0].target_account.as_deref(),
            Some("Expenses:Food:Groceries")
        );
        assert_eq!(
            candidates[1].target_account.as_deref(),
            Some("Expenses:Shopping:Online")
        );
        assert!(candidates[2].target_account.is_none());
    }

    #[test]
    fn highest_priority_wins() {
        let rules = vec![
            ImportRule {
                id: "low".into(),
                priority: 1,
                pattern: "FOOD".into(),
                match_type: MatchType::Contains,
                target_account: "Expenses:Food".into(),
            },
            ImportRule {
                id: "high".into(),
                priority: 100,
                pattern: "FOOD".into(),
                match_type: MatchType::Contains,
                target_account: "Expenses:Food:Premium".into(),
            },
        ];

        let mut candidates = vec![candidate("FOOD MART")];
        apply_rules(&mut candidates, &rules);
        assert_eq!(
            candidates[0].target_account.as_deref(),
            Some("Expenses:Food:Premium")
        );
    }

    #[test]
    fn already_assigned_not_overwritten() {
        let rules = sample_rules();
        let mut candidates = vec![ImportCandidate {
            target_account: Some("Manual:Override".into()),
            ..candidate("WHOLE FOODS")
        }];

        apply_rules(&mut candidates, &rules);
        assert_eq!(
            candidates[0].target_account.as_deref(),
            Some("Manual:Override")
        );
    }

    #[test]
    fn test_rules_returns_match() {
        let rules = sample_rules();
        let result = test_rules("AMZN MKTP US*AB1CD", &rules);
        assert!(result.is_some());
        assert_eq!(result.unwrap().target_account, "Expenses:Shopping:Online");
    }

    #[test]
    fn test_rules_returns_none_for_no_match() {
        let rules = sample_rules();
        assert!(test_rules("COMPLETELY UNKNOWN", &rules).is_none());
    }
}
