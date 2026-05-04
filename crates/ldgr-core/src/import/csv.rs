//! Minimal CSV parser with auto-delimiter detection, quoted field support,
//! and BOM handling. No external dependencies.

/// Parse CSV text into rows of fields.
///
/// Features:
/// - Auto-detects delimiter (comma, semicolon, tab) from the first line
/// - Handles double-quoted fields with escaped quotes (`""`)
/// - Strips UTF-8 BOM if present
/// - Skips empty lines
pub fn parse_csv(input: &str, delimiter: Option<char>) -> Vec<Vec<String>> {
    let input = strip_bom(input);

    let delim = delimiter.unwrap_or_else(|| detect_delimiter(input));

    let mut rows = Vec::new();
    for line in input.lines() {
        if line.trim().is_empty() {
            continue;
        }
        rows.push(parse_row(line, delim));
    }
    rows
}

/// Auto-detect the delimiter from the first line.
///
/// Checks for tab, semicolon, then comma (in order of specificity).
fn detect_delimiter(input: &str) -> char {
    let first_line = input.lines().next().unwrap_or("");
    if first_line.contains('\t') {
        '\t'
    } else if first_line.contains(';') {
        ';'
    } else {
        ','
    }
}

/// Strip UTF-8 BOM (EF BB BF) if present.
fn strip_bom(input: &str) -> &str {
    input.strip_prefix('\u{FEFF}').unwrap_or(input)
}

/// Parse a single CSV row, handling quoted fields.
fn parse_row(line: &str, delimiter: char) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    // Escaped quote
                    chars.next();
                    current.push('"');
                } else {
                    // End of quoted field
                    in_quotes = false;
                }
            } else {
                current.push(ch);
            }
        } else if ch == '"' && current.is_empty() {
            in_quotes = true;
        } else if ch == delimiter {
            fields.push(current.trim().to_string());
            current = String::new();
        } else {
            current.push(ch);
        }
    }
    fields.push(current.trim().to_string());
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_csv() {
        let input = "a,b,c\n1,2,3\n";
        let rows = parse_csv(input, None);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec!["a", "b", "c"]);
        assert_eq!(rows[1], vec!["1", "2", "3"]);
    }

    #[test]
    fn parse_quoted_fields() {
        let input = r#""hello, world",42,"say ""hi""" "#;
        let rows = parse_csv(input, Some(','));
        assert_eq!(rows[0][0], "hello, world");
        assert_eq!(rows[0][1], "42");
        assert_eq!(rows[0][2], r#"say "hi""#);
    }

    #[test]
    fn auto_detect_tab_delimiter() {
        let input = "a\tb\tc\n1\t2\t3\n";
        let rows = parse_csv(input, None);
        assert_eq!(rows[0], vec!["a", "b", "c"]);
    }

    #[test]
    fn auto_detect_semicolon_delimiter() {
        let input = "a;b;c\n1;2;3\n";
        let rows = parse_csv(input, None);
        assert_eq!(rows[0], vec!["a", "b", "c"]);
    }

    #[test]
    fn strip_utf8_bom() {
        let input = "\u{FEFF}a,b,c\n";
        let rows = parse_csv(input, None);
        assert_eq!(rows[0][0], "a");
    }

    #[test]
    fn skip_empty_lines() {
        let input = "a,b\n\n\n1,2\n";
        let rows = parse_csv(input, None);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn handles_trailing_comma() {
        let input = "a,b,c,\n";
        let rows = parse_csv(input, None);
        assert_eq!(rows[0].len(), 4);
        assert_eq!(rows[0][3], "");
    }

    #[test]
    fn parse_bank_style_csv() {
        let input = "\
Date,Description,Amount,Balance
01/15/2024,\"WHOLE FOODS #123\",-42.50,1457.50
01/16/2024,DIRECT DEPOSIT,2500.00,3957.50
01/17/2024,\"AMZN MKTP US*AB1CD2EF\",-29.99,3927.51
";
        let rows = parse_csv(input, None);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1][1], "WHOLE FOODS #123");
        assert_eq!(rows[1][2], "-42.50");
    }
}
