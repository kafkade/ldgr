//! OFX/QFX file parser.
//!
//! Parses OFX 1.x (SGML) and 2.x (XML) formats to extract bank transactions.
//! OFX 1.x is NOT proper XML — leaf elements lack closing tags.

use super::profile::ImportCandidate;

/// A parsed OFX transaction.
#[derive(Debug, Clone)]
pub struct OfxTransaction {
    pub date: String,
    pub amount: String,
    pub fitid: String,
    pub name: String,
    pub memo: Option<String>,
    pub trntype: String,
}

/// Parse an OFX/QFX file, returning import candidates.
///
/// Extracts `STMTTRN` elements from the `BANKTRANLIST` or `CCSTMTTRNRS`
/// (credit card) sections.
pub fn parse_ofx(input: &str, default_account: &str) -> (Vec<ImportCandidate>, Vec<String>) {
    let mut candidates = Vec::new();
    let mut errors = Vec::new();

    // Strip OFX headers (lines before <OFX>)
    let body = if let Some(idx) = input.find("<OFX>") {
        &input[idx..]
    } else if let Some(idx) = input.find("<ofx>") {
        &input[idx..]
    } else {
        errors.push("not a valid OFX file: missing <OFX> tag".into());
        return (candidates, errors);
    };

    // Extract all STMTTRN blocks
    let mut pos = 0;
    let lower = body.to_lowercase();
    let bytes = lower.as_bytes();

    while pos < bytes.len() {
        if let Some(start) = find_tag(&lower, pos, "<stmttrn>") {
            let content_start = start + "<stmttrn>".len();
            let end = find_tag(&lower, content_start, "</stmttrn>").unwrap_or(lower.len());
            let block = &body[content_start..end.min(body.len())];

            match parse_stmttrn(block) {
                Ok(txn) => {
                    let description = if let Some(memo) = &txn.memo {
                        if memo == &txn.name {
                            txn.name.clone()
                        } else {
                            format!("{} — {memo}", txn.name)
                        }
                    } else {
                        txn.name.clone()
                    };

                    candidates.push(ImportCandidate {
                        date: txn.date,
                        description,
                        amount: txn.amount,
                        source_account: default_account.to_string(),
                        target_account: None,
                        source_row: candidates.len() + 1,
                        fitid: Some(txn.fitid),
                    });
                }
                Err(msg) => errors.push(msg),
            }

            pos = end + "</stmttrn>".len();
        } else {
            break;
        }
    }

    (candidates, errors)
}

/// Parse a single `<STMTTRN>` block.
fn parse_stmttrn(block: &str) -> Result<OfxTransaction, String> {
    let trntype = extract_value(block, "TRNTYPE").unwrap_or_default();
    let dtposted = extract_value(block, "DTPOSTED").ok_or("STMTTRN missing DTPOSTED")?;
    let trnamt = extract_value(block, "TRNAMT").ok_or("STMTTRN missing TRNAMT")?;
    let fitid = extract_value(block, "FITID").ok_or("STMTTRN missing FITID")?;
    let name = extract_value(block, "NAME").unwrap_or_default();
    let memo = extract_value(block, "MEMO");

    let date = parse_ofx_date(&dtposted)?;
    let amount = trnamt.trim().to_string();

    Ok(OfxTransaction {
        date,
        amount,
        fitid: fitid.trim().to_string(),
        name: name.trim().to_string(),
        memo: memo.map(|m| m.trim().to_string()),
        trntype: trntype.trim().to_string(),
    })
}

/// Extract the value of a tag like `<TRNAMT>-42.50` (OFX 1.x SGML style).
///
/// Handles both `<TAG>value` (no closing tag) and `<TAG>value</TAG>`.
fn extract_value(block: &str, tag: &str) -> Option<String> {
    let lower_block = block.to_lowercase();
    let open = format!("<{}>", tag.to_lowercase());

    let start = lower_block.find(&open)?;
    let value_start = start + open.len();
    let rest = &block[value_start..];

    // Value ends at next `<` or end of string
    let end = rest.find('<').unwrap_or(rest.len());
    let value = rest[..end].trim();

    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Parse OFX date format `YYYYMMDD[HHmmss]` to `YYYY-MM-DD`.
fn parse_ofx_date(s: &str) -> Result<String, String> {
    let s = s.trim();
    if s.len() < 8 {
        return Err(format!("invalid OFX date: '{s}'"));
    }

    let year = &s[0..4];
    let month = &s[4..6];
    let day = &s[6..8];

    // Basic validation
    if !year.chars().all(|c| c.is_ascii_digit())
        || !month.chars().all(|c| c.is_ascii_digit())
        || !day.chars().all(|c| c.is_ascii_digit())
    {
        return Err(format!("invalid OFX date: '{s}'"));
    }

    Ok(format!("{year}-{month}-{day}"))
}

fn find_tag(haystack: &str, start: usize, tag: &str) -> Option<usize> {
    haystack[start..].find(tag).map(|i| start + i)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_OFX: &str = r"OFXHEADER:100
DATA:OFXSGML

<OFX>
<SIGNONMSGSRSV1>
<SONRS>
<STATUS><CODE>0<SEVERITY>INFO</STATUS>
</SONRS>
</SIGNONMSGSRSV1>
<BANKMSGSRSV1>
<STMTTRNRS>
<STMTRS>
<BANKTRANLIST>
<STMTTRN>
<TRNTYPE>DEBIT
<DTPOSTED>20240115
<TRNAMT>-42.50
<FITID>20240115001
<NAME>WHOLE FOODS
<MEMO>WHOLE FOODS #10234
</STMTTRN>
<STMTTRN>
<TRNTYPE>CREDIT
<DTPOSTED>20240116
<TRNAMT>2500.00
<FITID>20240116001
<NAME>DIRECT DEPOSIT
</STMTTRN>
<STMTTRN>
<TRNTYPE>DEBIT
<DTPOSTED>20240117120000
<TRNAMT>-29.99
<FITID>20240117001
<NAME>AMZN MKTP US
<MEMO>AMZN MKTP US*AB1CD2EF
</STMTTRN>
</BANKTRANLIST>
</STMTRS>
</STMTTRNRS>
</BANKMSGSRSV1>
</OFX>";

    #[test]
    fn parse_ofx_basic() {
        let (candidates, errors) = parse_ofx(SAMPLE_OFX, "Assets:Checking");
        assert!(errors.is_empty(), "errors: {errors:?}");
        assert_eq!(candidates.len(), 3);
    }

    #[test]
    fn parse_ofx_dates_correct() {
        let (candidates, _) = parse_ofx(SAMPLE_OFX, "Assets:Checking");
        assert_eq!(candidates[0].date, "2024-01-15");
        assert_eq!(candidates[1].date, "2024-01-16");
        assert_eq!(candidates[2].date, "2024-01-17"); // strips time
    }

    #[test]
    fn parse_ofx_amounts_correct() {
        let (candidates, _) = parse_ofx(SAMPLE_OFX, "Assets:Checking");
        assert_eq!(candidates[0].amount, "-42.50");
        assert_eq!(candidates[1].amount, "2500.00");
        assert_eq!(candidates[2].amount, "-29.99");
    }

    #[test]
    fn parse_ofx_fitids_extracted() {
        let (candidates, _) = parse_ofx(SAMPLE_OFX, "Assets:Checking");
        assert_eq!(candidates[0].fitid.as_deref(), Some("20240115001"));
        assert_eq!(candidates[1].fitid.as_deref(), Some("20240116001"));
    }

    #[test]
    fn parse_ofx_description_with_memo() {
        let (candidates, _) = parse_ofx(SAMPLE_OFX, "Assets:Checking");
        // When memo differs from name, combine them
        assert!(candidates[0].description.contains("WHOLE FOODS #10234"));
        // When no memo, just name
        assert_eq!(candidates[1].description, "DIRECT DEPOSIT");
    }

    #[test]
    fn parse_ofx_source_account() {
        let (candidates, _) = parse_ofx(SAMPLE_OFX, "Assets:Checking:Chase");
        assert_eq!(candidates[0].source_account, "Assets:Checking:Chase");
    }

    #[test]
    fn invalid_ofx_returns_error() {
        let (_, errors) = parse_ofx("not an OFX file", "Assets:Checking");
        assert!(!errors.is_empty());
    }

    #[test]
    fn parse_ofx_date_formats() {
        assert_eq!(parse_ofx_date("20240115").unwrap(), "2024-01-15");
        assert_eq!(parse_ofx_date("20240115120000").unwrap(), "2024-01-15");
        assert!(parse_ofx_date("2024").is_err());
    }
}
