//! CSV column mapping profiles for bank imports.
//!
//! A profile maps CSV columns to transaction fields, allowing reuse
//! across multiple imports from the same bank.

use serde::{Deserialize, Serialize};

/// A reusable CSV column mapping profile.
///
/// Column indices are 0-based. The profile defines how to extract
/// transaction data from each row of a CSV file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsvProfile {
    /// Profile name (e.g., "chase-checking").
    pub name: String,
    /// Column index for the transaction date.
    pub date_column: usize,
    /// Date format string (e.g., `"%m/%d/%Y"`, `"%Y-%m-%d"`).
    pub date_format: String,
    /// Column index for the description/payee.
    pub description_column: usize,
    /// Column index for the amount.
    pub amount_column: usize,
    /// Whether the first row is a header (skip it).
    pub skip_header: bool,
    /// The account that owns this CSV (e.g., `"Assets:Checking:Chase"`).
    pub default_account: String,
    /// Override delimiter (None = auto-detect).
    pub delimiter: Option<char>,
    /// Negate amounts (some banks use positive for debits).
    pub negate_amounts: bool,
}

impl CsvProfile {
    /// Default profile suitable for many US banks.
    pub fn default_us_bank(name: &str, account: &str) -> Self {
        Self {
            name: name.to_string(),
            date_column: 0,
            date_format: "%m/%d/%Y".to_string(),
            description_column: 1,
            amount_column: 2,
            skip_header: true,
            default_account: account.to_string(),
            delimiter: None,
            negate_amounts: false,
        }
    }
}

/// An import candidate produced by applying a profile to a CSV row.
#[derive(Debug, Clone)]
pub struct ImportCandidate {
    /// Transaction date (already formatted as YYYY-MM-DD).
    pub date: String,
    /// Description/payee from the CSV.
    pub description: String,
    /// Amount as a string (preserving decimal precision).
    pub amount: String,
    /// The source account (from the profile's `default_account`).
    pub source_account: String,
    /// The target account (set by rules engine, or None for user to assign).
    pub target_account: Option<String>,
    /// 1-based row number in the CSV file.
    pub source_row: usize,
    /// Financial institution transaction ID (OFX FITID) for exact dedup.
    pub fitid: Option<String>,
}

/// Apply a profile to parsed CSV rows, producing import candidates.
///
/// Skips the header row if `profile.skip_header` is true. Returns errors
/// for rows that can't be parsed (missing columns, invalid dates).
pub fn apply_profile(
    rows: &[Vec<String>],
    profile: &CsvProfile,
) -> (Vec<ImportCandidate>, Vec<String>) {
    let mut candidates = Vec::new();
    let mut errors = Vec::new();

    let start = usize::from(profile.skip_header);

    for (i, row) in rows.iter().enumerate().skip(start) {
        let row_num = i + 1;

        let date_raw = match row.get(profile.date_column) {
            Some(d) if !d.is_empty() => d,
            _ => {
                errors.push(format!("row {row_num}: missing date column"));
                continue;
            }
        };

        let Some(description) = row.get(profile.description_column) else {
            errors.push(format!("row {row_num}: missing description column"));
            continue;
        };
        let description = description.trim().to_string();

        let amount_raw = match row.get(profile.amount_column) {
            Some(a) if !a.is_empty() => a.trim().to_string(),
            _ => {
                errors.push(format!("row {row_num}: missing amount column"));
                continue;
            }
        };

        // Parse and reformat date
        let Some(date) = reformat_date(date_raw, &profile.date_format) else {
            errors.push(format!(
                "row {row_num}: invalid date '{date_raw}' (expected format: {})",
                profile.date_format
            ));
            continue;
        };

        // Clean amount (remove currency symbols, commas)
        let amount = clean_amount(&amount_raw, profile.negate_amounts);

        candidates.push(ImportCandidate {
            date,
            description,
            amount,
            source_account: profile.default_account.clone(),
            target_account: None,
            source_row: row_num,
            fitid: None,
        });
    }

    (candidates, errors)
}

/// Reformat a date string from the profile's format to ISO 8601 (YYYY-MM-DD).
fn reformat_date(raw: &str, format: &str) -> Option<String> {
    let parsed = chrono::NaiveDate::parse_from_str(raw.trim(), format).ok()?;
    Some(parsed.format("%Y-%m-%d").to_string())
}

/// Clean an amount string: remove `$`, commas, whitespace. Optionally negate.
fn clean_amount(raw: &str, negate: bool) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();

    if negate && !cleaned.is_empty() {
        if let Some(stripped) = cleaned.strip_prefix('-') {
            return stripped.to_string();
        }
        return format!("-{cleaned}");
    }

    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_profile() -> CsvProfile {
        CsvProfile {
            name: "test".into(),
            date_column: 0,
            date_format: "%m/%d/%Y".into(),
            description_column: 1,
            amount_column: 2,
            skip_header: true,
            default_account: "Assets:Checking".into(),
            delimiter: None,
            negate_amounts: false,
        }
    }

    #[test]
    fn apply_profile_to_bank_csv() {
        let rows = vec![
            vec!["Date".into(), "Description".into(), "Amount".into()],
            vec!["01/15/2024".into(), "WHOLE FOODS".into(), "-42.50".into()],
            vec![
                "01/16/2024".into(),
                "DIRECT DEPOSIT".into(),
                "2500.00".into(),
            ],
        ];

        let (candidates, errors) = apply_profile(&rows, &test_profile());
        assert!(errors.is_empty());
        assert_eq!(candidates.len(), 2);

        assert_eq!(candidates[0].date, "2024-01-15");
        assert_eq!(candidates[0].description, "WHOLE FOODS");
        assert_eq!(candidates[0].amount, "-42.50");
        assert_eq!(candidates[0].source_account, "Assets:Checking");
    }

    #[test]
    fn negate_amounts() {
        let rows = vec![
            vec!["Header".into(), "H".into(), "H".into()],
            vec!["01/15/2024".into(), "Purchase".into(), "42.50".into()],
        ];

        let mut profile = test_profile();
        profile.negate_amounts = true;

        let (candidates, _) = apply_profile(&rows, &profile);
        assert_eq!(candidates[0].amount, "-42.50");
    }

    #[test]
    fn clean_amount_removes_symbols() {
        assert_eq!(clean_amount("$1,234.56", false), "1234.56");
        assert_eq!(clean_amount("-$42.50", false), "-42.50");
    }

    #[test]
    fn invalid_date_produces_error() {
        let rows = vec![
            vec!["Header".into(), "H".into(), "H".into()],
            vec!["not-a-date".into(), "Test".into(), "10".into()],
        ];
        let (candidates, errors) = apply_profile(&rows, &test_profile());
        assert!(candidates.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("invalid date"));
    }

    #[test]
    fn missing_column_produces_error() {
        let rows = vec![
            vec!["Header".into(), "H".into(), "H".into()],
            vec!["01/15/2024".into()], // missing desc + amount
        ];
        let (candidates, errors) = apply_profile(&rows, &test_profile());
        assert!(candidates.is_empty());
        assert!(!errors.is_empty());
    }

    #[test]
    fn iso_date_format() {
        let rows = vec![
            vec!["Header".into(), "H".into(), "H".into()],
            vec!["2024-01-15".into(), "Test".into(), "10".into()],
        ];
        let mut profile = test_profile();
        profile.date_format = "%Y-%m-%d".into();

        let (candidates, errors) = apply_profile(&rows, &profile);
        assert!(errors.is_empty());
        assert_eq!(candidates[0].date, "2024-01-15");
    }
}
