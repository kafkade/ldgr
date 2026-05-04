# ldgr Journal Subset Specification v1.0

ldgr supports a **strict subset** of hledger's journal format for import and
export. This document defines exactly what is supported, what is not, and how
ldgr's behavior differs from hledger.

## Supported Features

### Transactions

A transaction starts with a date line followed by one or more indented posting
lines. A blank line (or end of file) terminates the transaction.

```journal
2024-01-15 * (1001) Whole Foods  ; weekly groceries
    Expenses:Food:Groceries          42.50 USD
    Assets:Checking                 -42.50 USD
```

**Date formats**: `YYYY-MM-DD` or `YYYY/MM/DD`

**Status markers** (optional, between date and description):

- `*` — cleared
- `!` — pending
- (none) — unmarked

**Transaction code** (optional, in parentheses): `(1001)`

**Description**: free-form text after the status/code, up to a comment or end of
line.

### Postings

Each posting is an indented line (at least 2 spaces or 1 tab) containing an
account name and an optional amount.

```journal
2024-01-15 Groceries
    Expenses:Food       42.50 USD
    Assets:Checking
```

- **Account names**: colon-separated hierarchy (e.g., `Assets:Checking:Chase`).
  May contain letters, digits, spaces, hyphens, underscores, and colons.
- **Amount-less postings**: at most one posting per transaction may omit the
  amount. ldgr infers the balancing amount.
- **Amount separation**: at least 2 spaces between account name and amount.

### Amounts

```journal
    Expenses:Food       42.50 USD    ; postfix commodity
    Expenses:Food      $42.50        ; prefix commodity (single-char symbols)
    Expenses:Food       42.50        ; no commodity (uses account default)
    Expenses:Food      -42.50 USD    ; negative
```

- **Decimal point**: `.` (period)
- **Negative amounts**: `-` prefix on quantity
- **Commodity position**: postfix (`42.50 USD`) or prefix for single-character
  symbols (`$42.50`, `€100`, `£50`)

### Balance Assertions

A posting may include a balance assertion after the amount, introduced by `=`:

```journal
    Assets:Checking    -42.50 USD = 1457.50 USD
```

The assertion states the expected account balance *after* this posting. Only
single-commodity assertions are supported.

### Comments and Tags

```journal
; file-level comment
# also a comment

2024-01-15 Description  ; transaction comment, tag:value
    Account    42.50 USD  ; posting comment, project:alpha
```

- **Line comments**: lines starting with `;` or `#` (at any indentation level)
- **Inline comments**: `;` after the description or amount
- **Tags**: `key:value` pairs inside comments. The key is everything before the
  first colon; the value is everything after (trimmed). Multiple tags are
  separated by commas.

### Account Declarations

```journal
account Assets:Checking
account Expenses:Food:Groceries
```

Declares an account name. Used for validation and auto-completion.

### Commodity Declarations

```journal
commodity USD
commodity EUR
```

Declares a commodity symbol.

### Price Directives

```journal
P 2024-01-15 AAPL 185.50 USD
P 2024-01-15 EUR 1.08 USD
```

Records a market price: on `DATE`, one unit of `COMMODITY` is worth `AMOUNT`.

## Unsupported Features

The following hledger features are **not supported**. Attempting to import a
journal containing these features produces a clear error with the line number.

| Feature | Syntax | Workaround |
| --- | --- | --- |
| Include directives | `include other.journal` | Flatten with `hledger print -f main.journal > combined.journal` |
| Automated transactions | `= expr` | Create transactions manually |
| Periodic transactions | `~ monthly` | Use ldgr's budgeting module instead |
| Multi-commodity balance assertions | `= 100 USD, 0.5 BTC` | Use single-commodity assertions |
| Lot/cost notation | `10 AAPL {185.50 USD}` | Use ldgr's native lot tracking |
| Total cost notation | `10 AAPL @@ 1855 USD` | Use ldgr's native lot tracking |
| Valuation expressions | `V`, `--value` | Use ldgr's market data module |
| Inline math | `($10 + $20)` | Pre-compute amounts |
| Timedot format | `.journal.timedot` | Convert to standard journal format |
| Payee directive | `payee Name` | Not needed for ldgr import |
| Tag directive | `tag name` | Not needed for ldgr import |
| Apply account | `apply account Assets` | Use full account names |
| Aliases | `alias old = new` | Rename accounts after import |
| Default commodity | `D $1000.00` | Specify commodities explicitly |

## Differences from hledger

| Behavior | hledger | ldgr |
| --- | --- | --- |
| Canonical store | `.journal` text files | Encrypted vault (SQLite) |
| Round-trip fidelity | Full (text files are source of truth) | Lossy (formatting, comments, whitespace not preserved) |
| Include structure | Preserved | Flattened on import |
| Auto-balance | Tolerant of floating-point rounding | Exact `Decimal` arithmetic; no tolerance |
| Date separators | `-` or `/` accepted | Same |
| Multiple amount-less postings | Allowed with virtual postings | Error: at most one per transaction |

## Grammar (Informal BNF)

```text
journal     = { line }
line        = blank | comment | directive | txn_header
blank       = NEWLINE
comment     = [ INDENT ] (";" | "#") TEXT NEWLINE

directive   = account_decl | commodity_decl | price_dir | unsupported_dir
account_decl   = "account" SPACE account_name NEWLINE
commodity_decl = "commodity" SPACE symbol NEWLINE
price_dir      = "P" SPACE date SPACE symbol SPACE amount NEWLINE
unsupported_dir = ("include" | "=" | "~" | "payee" | "tag" | ...) TEXT NEWLINE → ERROR

txn_header  = date [ SPACE status ] [ SPACE code ] SPACE description
              [ SPACE ";" txn_comment ] NEWLINE
              posting { posting }
posting     = INDENT account_name [ SEP amount ] [ SPACE "=" SPACE amount ]
              [ SPACE ";" posting_comment ] NEWLINE

date        = YYYY ("-" | "/") MM ("-" | "/") DD
status      = "*" | "!"
code        = "(" TEXT ")"
description = TEXT  (up to ";" or NEWLINE)
account_name = WORD { ":" WORD }     (letters, digits, spaces, hyphens, underscores)
amount      = [ "-" ] quantity [ SPACE ] commodity
            | commodity quantity
quantity    = DIGIT { DIGIT } [ "." DIGIT { DIGIT } ]
commodity   = symbol
symbol      = LETTER { LETTER | DIGIT }   (e.g., USD, EUR, BTC, AAPL)
            | "$" | "€" | "£" | "¥"       (single-char prefix symbols)

SEP         = SPACE SPACE { SPACE }        (2+ spaces between account and amount)
INDENT      = SPACE SPACE { SPACE } | TAB  (2+ spaces or tab)
```

## Migration Guide

### Importing from hledger

```sh
# Validate before import
ldgr validate my-ledger.journal

# If the journal uses unsupported features, flatten first:
hledger print -f my-ledger.journal > flat.journal
ldgr validate flat.journal

# Import
ldgr import flat.journal
```

### Exporting to hledger

```sh
# One-way export for use with hledger reporting
ldgr export --format hledger > report.journal
hledger balance -f report.journal
```

Note: export is one-way. The vault is the source of truth. Formatting,
comments, and file structure from the original import are not preserved.
