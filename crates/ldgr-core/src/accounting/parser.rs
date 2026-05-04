//! Line-oriented parser for the hledger journal subset.
//!
//! See `docs/journal-subset.md` for the full specification of supported syntax.
//! Unsupported features produce clear errors with line numbers.

use std::collections::HashMap;

use rust_decimal::Decimal;

use super::types::{
    AccountDeclaration, Amount, CommodityDeclaration, Journal, Posting, PriceDirective, Status,
    Transaction,
};

/// A parse error with source location and context.
#[derive(Debug, Clone)]
pub struct ParseError {
    /// 1-based line number.
    pub line: usize,
    pub message: String,
    /// The source line that caused the error.
    pub source_line: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)?;
        if !self.source_line.is_empty() {
            write!(f, "\n  | {}", self.source_line)?;
        }
        Ok(())
    }
}

impl std::error::Error for ParseError {}

// ── Unsupported directive keywords ─────────────────────────────────────────────

const UNSUPPORTED_DIRECTIVES: &[(&str, &str)] = &[
    ("include", "Flatten with `hledger print` before importing"),
    ("payee", "`payee` directives are not supported"),
    ("tag", "`tag` directives are not supported"),
    (
        "apply account",
        "`apply account` is not supported; use full account names",
    ),
    (
        "alias",
        "`alias` is not supported; rename accounts after import",
    ),
    (
        "D ",
        "`D` (default commodity) is not supported; specify commodities explicitly",
    ),
];

/// Single-character currency symbols that appear as prefixes (e.g., `$42.50`).
const PREFIX_SYMBOLS: &[char] = &['$', '€', '£', '¥', '₹', '₽', '₿'];

// ── Public API ─────────────────────────────────────────────────────────────────

/// Parse a complete journal from source text.
///
/// Returns a [`Journal`] on success, or a list of [`ParseError`]s if any
/// lines could not be parsed. The parser is lenient: it collects all errors
/// rather than stopping at the first one.
#[allow(clippy::too_many_lines)]
pub fn parse_journal(input: &str) -> Result<Journal, Vec<ParseError>> {
    let mut journal = Journal::default();
    let mut errors = Vec::new();
    let lines: Vec<&str> = input.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let line_num = i + 1;

        // Blank lines
        if line.trim().is_empty() {
            i += 1;
            continue;
        }

        // Comments (possibly indented)
        if is_comment_line(line) {
            i += 1;
            continue;
        }

        // Unsupported directives — produce clear errors
        if let Some(msg) = check_unsupported(line) {
            errors.push(ParseError {
                line: line_num,
                message: msg,
                source_line: line.to_string(),
            });
            i += 1;
            continue;
        }

        // Automated transactions (=)
        let trimmed = line.trim_start();
        if trimmed.starts_with("= ") || trimmed == "=" {
            errors.push(ParseError {
                line: line_num,
                message: "automated transactions (`=`) are not supported".into(),
                source_line: line.to_string(),
            });
            // Skip the auto-txn body (indented lines)
            i += 1;
            while i < lines.len() && is_indented(lines[i]) {
                i += 1;
            }
            continue;
        }

        // Periodic transactions (~)
        if trimmed.starts_with("~ ") || trimmed == "~" {
            errors.push(ParseError {
                line: line_num,
                message:
                    "periodic transactions (`~`) are not supported; use ldgr's budgeting module"
                        .into(),
                source_line: line.to_string(),
            });
            i += 1;
            while i < lines.len() && is_indented(lines[i]) {
                i += 1;
            }
            continue;
        }

        // Account declaration
        if let Some(rest) = line.strip_prefix("account ") {
            let name = rest.trim();
            if name.is_empty() {
                errors.push(ParseError {
                    line: line_num,
                    message: "account declaration missing name".into(),
                    source_line: line.to_string(),
                });
            } else {
                journal.account_declarations.push(AccountDeclaration {
                    name: name.to_string(),
                    source_line: line_num,
                });
            }
            i += 1;
            continue;
        }

        // Commodity declaration
        if let Some(rest) = line.strip_prefix("commodity ") {
            let symbol = rest.trim();
            if symbol.is_empty() {
                errors.push(ParseError {
                    line: line_num,
                    message: "commodity declaration missing symbol".into(),
                    source_line: line.to_string(),
                });
            } else {
                journal.commodity_declarations.push(CommodityDeclaration {
                    symbol: symbol.to_string(),
                    source_line: line_num,
                });
            }
            i += 1;
            continue;
        }

        // Price directive
        if let Some(rest) = line.strip_prefix("P ") {
            match parse_price_directive(rest.trim(), line_num) {
                Ok(pd) => journal.price_directives.push(pd),
                Err(e) => errors.push(e),
            }
            i += 1;
            continue;
        }

        // Transaction (starts with a digit = date)
        if trimmed.starts_with(|c: char| c.is_ascii_digit()) {
            match parse_transaction(&lines, &mut i) {
                Ok(txn) => journal.transactions.push(txn),
                Err(mut errs) => errors.append(&mut errs),
            }
            continue; // parse_transaction advances i
        }

        // Unknown line
        errors.push(ParseError {
            line: line_num,
            message: "unrecognized journal syntax".into(),
            source_line: line.to_string(),
        });
        i += 1;
    }

    if errors.is_empty() {
        Ok(journal)
    } else {
        Err(errors)
    }
}

// ── Internal parsers ───────────────────────────────────────────────────────────

fn is_comment_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with(';') || trimmed.starts_with('#')
}

fn is_indented(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }
    let first = line.as_bytes()[0];
    first == b' ' || first == b'\t'
}

fn check_unsupported(line: &str) -> Option<String> {
    for (keyword, msg) in UNSUPPORTED_DIRECTIVES {
        if line.starts_with(keyword) {
            return Some(msg.to_string());
        }
    }
    None
}

/// Parse a transaction: date line + following indented posting lines.
/// Advances `i` past all consumed lines.
fn parse_transaction(lines: &[&str], i: &mut usize) -> Result<Transaction, Vec<ParseError>> {
    let header_line = lines[*i];
    let header_line_num = *i + 1;
    let mut errors = Vec::new();

    // Parse the date line
    let header = match parse_txn_header(header_line) {
        Ok(h) => h,
        Err(msg) => {
            errors.push(ParseError {
                line: header_line_num,
                message: msg,
                source_line: header_line.to_string(),
            });
            *i += 1;
            // Skip indented lines
            while *i < lines.len() && is_indented(lines[*i]) {
                *i += 1;
            }
            return Err(errors);
        }
    };

    // Extract tags from transaction comment
    let tags = header
        .comment
        .as_deref()
        .map(extract_tags)
        .unwrap_or_default();

    *i += 1; // advance past header

    // Parse postings (indented lines)
    let mut postings = Vec::new();
    while *i < lines.len() {
        let pline = lines[*i];
        if pline.trim().is_empty() {
            *i += 1;
            break; // blank line ends transaction
        }
        if !is_indented(pline) {
            break; // non-indented line = next directive/transaction
        }

        let pline_num = *i + 1;

        // Skip comment-only posting lines
        if is_comment_line(pline) {
            *i += 1;
            continue;
        }

        match parse_posting(pline.trim()) {
            Ok(posting) => postings.push(posting),
            Err(msg) => errors.push(ParseError {
                line: pline_num,
                message: msg,
                source_line: pline.to_string(),
            }),
        }
        *i += 1;
    }

    if postings.is_empty() && errors.is_empty() {
        errors.push(ParseError {
            line: header_line_num,
            message: "transaction has no postings".into(),
            source_line: header_line.to_string(),
        });
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(Transaction {
        date: header.date,
        status: header.status,
        code: header.code,
        description: header.description,
        postings,
        tags,
        comment: header.comment,
        source_line: header_line_num,
    })
}

/// Parsed parts of a transaction header line.
struct TxnHeader {
    date: String,
    status: Status,
    code: Option<String>,
    description: String,
    comment: Option<String>,
}

/// Parse the transaction header line: `DATE [STATUS] [(CODE)] DESCRIPTION [; COMMENT]`
fn parse_txn_header(line: &str) -> Result<TxnHeader, String> {
    let mut rest = line;

    // Date
    let date = take_date(&mut rest)?;
    skip_spaces(&mut rest);

    // Status (optional)
    let status = take_status(&mut rest);
    skip_spaces(&mut rest);

    // Code (optional)
    let code = take_code(&mut rest);
    skip_spaces(&mut rest);

    // Split description and comment at first unquoted `;`
    let (description, comment) = split_at_comment(rest);
    let description = description.trim().to_string();

    if description.is_empty() {
        return Err("transaction missing description".into());
    }

    Ok(TxnHeader {
        date,
        status,
        code,
        description,
        comment,
    })
}

/// Parse a posting line (already trimmed of leading whitespace).
fn parse_posting(line: &str) -> Result<Posting, String> {
    // Split at inline comment
    let (content, comment) = split_at_comment(line);
    let tags = comment.as_deref().map(extract_tags).unwrap_or_default();

    // Check for posting-level status
    let mut rest = content.trim();
    let status = take_status(&mut rest);
    rest = rest.trim_start();

    // Find the account name: everything up to 2+ spaces or end of content
    let (account, remainder) = split_account_amount(rest);
    let account = account.trim().to_string();

    if account.is_empty() {
        return Err("posting missing account name".into());
    }

    let remainder = remainder.trim();

    if remainder.is_empty() {
        // Amount-less posting
        return Ok(Posting {
            account,
            amount: None,
            balance_assertion: None,
            status,
            comment,
            tags,
        });
    }

    // Parse amount and optional balance assertion (separated by ` = `)
    let (amount_str, assertion_str) = if let Some(idx) = remainder.find(" = ") {
        (&remainder[..idx], Some(remainder[idx + 3..].trim()))
    } else if let Some(stripped) = remainder.strip_prefix("= ") {
        // No amount, just assertion
        ("", Some(stripped.trim()))
    } else {
        (remainder, None)
    };

    let amount = if amount_str.trim().is_empty() {
        None
    } else {
        Some(parse_amount(amount_str.trim())?)
    };

    let balance_assertion = match assertion_str {
        Some(s) if !s.is_empty() => Some(parse_amount(s)?),
        _ => None,
    };

    Ok(Posting {
        account,
        amount,
        balance_assertion,
        status,
        comment,
        tags,
    })
}

/// Parse a price directive: `DATE COMMODITY AMOUNT`
fn parse_price_directive(text: &str, line_num: usize) -> Result<PriceDirective, ParseError> {
    let mut rest = text;

    let date = take_date(&mut rest).map_err(|msg| ParseError {
        line: line_num,
        message: format!("invalid price directive: {msg}"),
        source_line: format!("P {text}"),
    })?;
    skip_spaces(&mut rest);

    // Commodity symbol
    let commodity = take_word(&mut rest).ok_or_else(|| ParseError {
        line: line_num,
        message: "price directive missing commodity".into(),
        source_line: format!("P {text}"),
    })?;
    skip_spaces(&mut rest);

    // Price amount
    let price = parse_amount(rest.trim()).map_err(|msg| ParseError {
        line: line_num,
        message: format!("invalid price amount: {msg}"),
        source_line: format!("P {text}"),
    })?;

    Ok(PriceDirective {
        date,
        commodity,
        price,
        source_line: line_num,
    })
}

// ── Amount parsing ─────────────────────────────────────────────────────────────

/// Parse an amount string like `42.50 USD`, `$42.50`, `-100`, or `EUR 100`.
fn parse_amount(s: &str) -> Result<Amount, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty amount".into());
    }

    // Check for prefix currency symbol ($42.50, -$42.50)
    let (neg, rest) = if let Some(stripped) = s.strip_prefix('-') {
        (true, stripped.trim_start())
    } else {
        (false, s)
    };

    if let Some(&sym) = PREFIX_SYMBOLS.iter().find(|&&sym| rest.starts_with(sym)) {
        let after_sym = rest[sym.len_utf8()..].trim_start();
        let quantity = parse_decimal(after_sym)?;
        let quantity = if neg { -quantity } else { quantity };
        return Ok(Amount {
            quantity,
            commodity: sym.to_string(),
        });
    }

    // Check for postfix: `42.50 USD` or `42.50` or `-42.50 USD`
    // Or prefix word commodity: `USD 42.50`
    let parts: Vec<&str> = s.split_whitespace().collect();

    match parts.len() {
        1 => {
            // Just a number (no commodity)
            let quantity = parse_decimal(parts[0])?;
            Ok(Amount {
                quantity,
                commodity: String::new(),
            })
        }
        2 => {
            // Either `42.50 USD` or `USD 42.50`
            if let Ok(q) = parse_decimal(parts[0]) {
                // `42.50 USD` — postfix
                Ok(Amount {
                    quantity: q,
                    commodity: parts[1].to_string(),
                })
            } else if let Ok(q) = parse_decimal(parts[1]) {
                // `USD 42.50` — prefix word commodity
                Ok(Amount {
                    quantity: q,
                    commodity: parts[0].to_string(),
                })
            } else {
                Err(format!("cannot parse amount: {s}"))
            }
        }
        _ => Err(format!("cannot parse amount: {s}")),
    }
}

fn parse_decimal(s: &str) -> Result<Decimal, String> {
    s.parse::<Decimal>()
        .map_err(|e| format!("invalid number '{s}': {e}"))
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Consume a date (YYYY-MM-DD or YYYY/MM/DD) from the front of `rest`.
fn take_date(rest: &mut &str) -> Result<String, String> {
    // Date is at least 10 chars: YYYY-MM-DD
    if rest.len() < 10 {
        return Err("expected date in YYYY-MM-DD format".into());
    }

    let date_str = &rest[..10];
    // Validate format
    let bytes = date_str.as_bytes();
    let sep = bytes[4];
    if sep != b'-' && sep != b'/' {
        return Err(format!("invalid date separator in '{date_str}'"));
    }
    if bytes[7] != sep {
        return Err(format!("inconsistent date separators in '{date_str}'"));
    }

    // Validate digits
    for &pos in &[0, 1, 2, 3, 5, 6, 8, 9] {
        if !bytes[pos].is_ascii_digit() {
            return Err(format!("invalid date '{date_str}'"));
        }
    }

    // Normalize to YYYY-MM-DD
    let normalized = date_str.replace('/', "-");

    // Basic range validation
    let month = &normalized[5..7];
    let day = &normalized[8..10];
    let m: u32 = month.parse().unwrap_or(0);
    let d: u32 = day.parse().unwrap_or(0);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return Err(format!("invalid date '{date_str}'"));
    }

    *rest = &rest[10..];
    Ok(normalized)
}

/// Consume optional status marker (* or !).
fn take_status(rest: &mut &str) -> Status {
    if rest.starts_with('*') {
        *rest = &rest[1..];
        Status::Cleared
    } else if rest.starts_with('!') {
        *rest = &rest[1..];
        Status::Pending
    } else {
        Status::Unmarked
    }
}

/// Consume optional transaction code in parentheses.
fn take_code(rest: &mut &str) -> Option<String> {
    if rest.starts_with('(') {
        if let Some(end) = rest.find(')') {
            let code = rest[1..end].to_string();
            *rest = &rest[end + 1..];
            return Some(code);
        }
    }
    None
}

/// Consume a word (non-whitespace sequence).
fn take_word(rest: &mut &str) -> Option<String> {
    let trimmed = rest.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let word = trimmed[..end].to_string();
    *rest = &trimmed[end..];
    Some(word)
}

fn skip_spaces(rest: &mut &str) {
    *rest = rest.trim_start_matches(' ');
}

/// Split at the first `;` that is not inside a string.
fn split_at_comment(s: &str) -> (String, Option<String>) {
    if let Some(idx) = s.find(';') {
        let content = s[..idx].to_string();
        let comment = s[idx + 1..].trim().to_string();
        (content, Some(comment))
    } else {
        (s.to_string(), None)
    }
}

/// Split a posting line into account name and amount remainder.
///
/// The account name ends at the first occurrence of 2+ spaces (the separator).
/// If there are no 2+ spaces, the entire line is the account name.
fn split_account_amount(s: &str) -> (&str, &str) {
    // Find first run of 2+ spaces
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b' ' {
            let start = i;
            while i < bytes.len() && bytes[i] == b' ' {
                i += 1;
            }
            if i - start >= 2 {
                return (&s[..start], &s[i..]);
            }
        } else {
            i += 1;
        }
    }
    (s, "")
}

/// Extract `key:value` tags from a comment string.
fn extract_tags(comment: &str) -> HashMap<String, String> {
    let mut tags = HashMap::new();
    // Tags are comma-separated `key:value` pairs
    for part in comment.split(',') {
        let part = part.trim();
        if let Some(colon_pos) = part.find(':') {
            let key = part[..colon_pos].trim();
            let value = part[colon_pos + 1..].trim();
            if !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            {
                tags.insert(key.to_string(), value.to_string());
            }
        }
    }
    tags
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // --- Basic parsing ---

    #[test]
    fn parse_simple_transaction() {
        let input = "\
2024-01-15 Whole Foods
    Expenses:Food       42.50 USD
    Assets:Checking    -42.50 USD
";
        let journal = parse_journal(input).unwrap();
        assert_eq!(journal.transactions.len(), 1);

        let txn = &journal.transactions[0];
        assert_eq!(txn.date, "2024-01-15");
        assert_eq!(txn.description, "Whole Foods");
        assert_eq!(txn.status, Status::Unmarked);
        assert_eq!(txn.postings.len(), 2);

        assert_eq!(txn.postings[0].account, "Expenses:Food");
        let amt = txn.postings[0].amount.as_ref().unwrap();
        assert_eq!(amt.quantity, Decimal::new(4250, 2));
        assert_eq!(amt.commodity, "USD");
    }

    #[test]
    fn parse_transaction_with_status_and_code() {
        let input = "2024-01-15 * (1001) Grocery run\n    Expenses:Food  42 USD\n    Assets:Cash\n";
        let journal = parse_journal(input).unwrap();
        let txn = &journal.transactions[0];
        assert_eq!(txn.status, Status::Cleared);
        assert_eq!(txn.code.as_deref(), Some("1001"));
        assert_eq!(txn.description, "Grocery run");
    }

    #[test]
    fn parse_pending_status() {
        let input = "2024-01-15 ! Pending payment\n    Expenses:Food  10 USD\n    Assets:Cash\n";
        let journal = parse_journal(input).unwrap();
        assert_eq!(journal.transactions[0].status, Status::Pending);
    }

    #[test]
    fn parse_amountless_posting() {
        let input = "\
2024-01-15 Transfer
    Assets:Savings     1000 USD
    Assets:Checking
";
        let journal = parse_journal(input).unwrap();
        let txn = &journal.transactions[0];
        assert_eq!(txn.postings.len(), 2);
        assert!(txn.postings[1].amount.is_none());
    }

    // --- Amounts ---

    #[test]
    fn parse_prefix_currency_symbol() {
        let input = "2024-01-15 Coffee\n    Expenses:Food  $4.50\n    Assets:Cash\n";
        let journal = parse_journal(input).unwrap();
        let amt = journal.transactions[0].postings[0].amount.as_ref().unwrap();
        assert_eq!(amt.quantity, Decimal::new(450, 2));
        assert_eq!(amt.commodity, "$");
    }

    #[test]
    fn parse_negative_amount() {
        let input = "2024-01-15 Refund\n    Assets:Checking  -50.00 USD\n    Income:Refund\n";
        let journal = parse_journal(input).unwrap();
        let amt = journal.transactions[0].postings[0].amount.as_ref().unwrap();
        assert_eq!(amt.quantity, Decimal::new(-5000, 2));
    }

    #[test]
    fn parse_amount_no_commodity() {
        let input = "2024-01-15 Transfer\n    Assets:A  100\n    Assets:B\n";
        let journal = parse_journal(input).unwrap();
        let amt = journal.transactions[0].postings[0].amount.as_ref().unwrap();
        assert_eq!(amt.quantity, Decimal::new(100, 0));
        assert!(amt.commodity.is_empty());
    }

    // --- Balance assertions ---

    #[test]
    fn parse_balance_assertion() {
        let input = "\
2024-01-15 Deposit
    Assets:Checking    500 USD = 1500 USD
    Income:Salary
";
        let journal = parse_journal(input).unwrap();
        let posting = &journal.transactions[0].postings[0];
        let assertion = posting.balance_assertion.as_ref().unwrap();
        assert_eq!(assertion.quantity, Decimal::new(1500, 0));
        assert_eq!(assertion.commodity, "USD");
    }

    // --- Comments and tags ---

    #[test]
    fn parse_transaction_comment_and_tags() {
        let input = "2024-01-15 Lunch  ; project:alpha, category:meals\n    Expenses:Food  15 USD\n    Assets:Cash\n";
        let journal = parse_journal(input).unwrap();
        let txn = &journal.transactions[0];
        assert_eq!(txn.tags.get("project").map(String::as_str), Some("alpha"));
        assert_eq!(txn.tags.get("category").map(String::as_str), Some("meals"));
    }

    #[test]
    fn parse_posting_comment() {
        let input =
            "2024-01-15 Food\n    Expenses:Food  42 USD  ; organic produce\n    Assets:Cash\n";
        let journal = parse_journal(input).unwrap();
        assert_eq!(
            journal.transactions[0].postings[0].comment.as_deref(),
            Some("organic produce")
        );
    }

    #[test]
    fn parse_line_comments() {
        let input = "\
; this is a comment
# this too

2024-01-15 Test
    ; comment posting line (should be skipped)
    Expenses:Test  10 USD
    Assets:Cash
";
        let journal = parse_journal(input).unwrap();
        assert_eq!(journal.transactions.len(), 1);
        assert_eq!(journal.transactions[0].postings.len(), 2);
    }

    // --- Directives ---

    #[test]
    fn parse_account_declaration() {
        let input = "account Assets:Checking\naccount Expenses:Food\n";
        let journal = parse_journal(input).unwrap();
        assert_eq!(journal.account_declarations.len(), 2);
        assert_eq!(journal.account_declarations[0].name, "Assets:Checking");
    }

    #[test]
    fn parse_commodity_declaration() {
        let input = "commodity USD\ncommodity EUR\n";
        let journal = parse_journal(input).unwrap();
        assert_eq!(journal.commodity_declarations.len(), 2);
        assert_eq!(journal.commodity_declarations[0].symbol, "USD");
    }

    #[test]
    fn parse_price_directive() {
        let input = "P 2024-01-15 AAPL 185.50 USD\n";
        let journal = parse_journal(input).unwrap();
        assert_eq!(journal.price_directives.len(), 1);
        let pd = &journal.price_directives[0];
        assert_eq!(pd.date, "2024-01-15");
        assert_eq!(pd.commodity, "AAPL");
        assert_eq!(pd.price.quantity, Decimal::new(18550, 2));
        assert_eq!(pd.price.commodity, "USD");
    }

    // --- Date formats ---

    #[test]
    fn parse_slash_date() {
        let input = "2024/01/15 Test\n    Expenses:Test  10 USD\n    Assets:Cash\n";
        let journal = parse_journal(input).unwrap();
        assert_eq!(journal.transactions[0].date, "2024-01-15"); // normalized
    }

    // --- Unsupported features ---

    #[test]
    fn include_directive_errors() {
        let input = "include other.journal\n";
        let errors = parse_journal(input).unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("Flatten"));
        assert_eq!(errors[0].line, 1);
    }

    #[test]
    fn automated_transaction_errors() {
        let input = "= expenses:food\n    budget:food  *-1\n";
        let errors = parse_journal(input).unwrap_err();
        assert!(errors[0].message.contains("automated"));
    }

    #[test]
    fn periodic_transaction_errors() {
        let input = "~ monthly\n    Expenses:Rent  1500 USD\n    Assets:Checking\n";
        let errors = parse_journal(input).unwrap_err();
        assert!(errors[0].message.contains("periodic"));
    }

    // --- Multiple transactions ---

    #[test]
    fn parse_multiple_transactions() {
        let input = "\
2024-01-15 Groceries
    Expenses:Food       42.50 USD
    Assets:Checking

2024-01-16 * Gas
    Expenses:Transport  35.00 USD
    Assets:Checking

2024-01-17 ! Pending charge
    Expenses:Other      10.00 USD
    Assets:Credit
";
        let journal = parse_journal(input).unwrap();
        assert_eq!(journal.transactions.len(), 3);
        assert_eq!(journal.transactions[0].description, "Groceries");
        assert_eq!(journal.transactions[1].status, Status::Cleared);
        assert_eq!(journal.transactions[2].status, Status::Pending);
    }

    // --- Mixed journal ---

    #[test]
    fn parse_complete_journal() {
        let input = "\
; My ledger
commodity USD
commodity EUR

account Assets:Checking
account Expenses:Food

P 2024-01-01 EUR 1.08 USD

2024-01-15 * (1001) Whole Foods  ; weekly, store:wholefoods
    Expenses:Food:Groceries          42.50 USD
    Assets:Checking                 -42.50 USD = 957.50 USD

2024-01-16 Transfer
    Assets:Savings     500.00 USD
    Assets:Checking
";
        let journal = parse_journal(input).unwrap();
        assert_eq!(journal.commodity_declarations.len(), 2);
        assert_eq!(journal.account_declarations.len(), 2);
        assert_eq!(journal.price_directives.len(), 1);
        assert_eq!(journal.transactions.len(), 2);

        let txn1 = &journal.transactions[0];
        assert_eq!(txn1.status, Status::Cleared);
        assert_eq!(txn1.code.as_deref(), Some("1001"));
        assert_eq!(
            txn1.tags.get("store").map(String::as_str),
            Some("wholefoods")
        );
        assert!(txn1.postings[1].balance_assertion.is_some());
    }

    // --- Error context ---

    #[test]
    fn error_includes_line_number() {
        let input =
            "2024-01-15 OK\n    Expenses:Test  10 USD\n    Assets:Cash\n\ninclude bad.journal\n";
        let errors = parse_journal(input).unwrap_err();
        assert_eq!(errors[0].line, 5);
    }

    #[test]
    fn invalid_date_produces_error() {
        let input = "not-a-date Description\n    Expenses:Test  10 USD\n";
        let errors = parse_journal(input).unwrap_err();
        assert!(!errors.is_empty());
    }

    #[test]
    fn transaction_without_postings_errors() {
        let input =
            "2024-01-15 No postings\n\n2024-01-16 Next\n    Expenses:A  10 USD\n    Assets:B\n";
        let errors = parse_journal(input).unwrap_err();
        assert!(errors[0].message.contains("no postings"));
    }

    // --- Amount edge cases ---

    #[test]
    fn parse_integer_amount() {
        let result = parse_amount("100 USD").unwrap();
        assert_eq!(result.quantity, Decimal::new(100, 0));
        assert_eq!(result.commodity, "USD");
    }

    #[test]
    fn parse_prefix_word_commodity() {
        let result = parse_amount("EUR 42.50").unwrap();
        assert_eq!(result.quantity, Decimal::new(4250, 2));
        assert_eq!(result.commodity, "EUR");
    }

    #[test]
    fn parse_negative_prefix_symbol() {
        let result = parse_amount("-$42.50").unwrap();
        assert_eq!(result.quantity, Decimal::new(-4250, 2));
        assert_eq!(result.commodity, "$");
    }

    // --- Source line tracking ---

    #[test]
    fn transaction_source_line_tracked() {
        let input = "\n\n2024-01-15 Test\n    Expenses:A  10 USD\n    Assets:B\n";
        let journal = parse_journal(input).unwrap();
        assert_eq!(journal.transactions[0].source_line, 3);
    }
}
