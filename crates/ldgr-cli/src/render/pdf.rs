//! Render a layout-agnostic [`ReportDocument`] to PDF bytes.
//!
//! Uses `pdf-writer` with the built-in Standard-14 (base-14) Type1 fonts
//! (Helvetica / Courier), so no font files are bundled. Proportional Helvetica
//! is used for labels/headings; monospace Courier is used for amounts so the
//! right-aligned money column can be measured from a fixed glyph advance (no
//! font-metrics table required).
//!
//! Layout is a simple top-down cursor with automatic page breaks, giving a
//! paginated, styled document: title, period subtitle, section headings, an
//! indented account hierarchy, and emphasized totals.
//!
//! `pdf-writer` is a low-level PDF writer: this module builds the document
//! catalog, the page tree, per-page content streams and the base-14 font
//! dictionaries by hand. Coordinates are PDF points with a bottom-left origin
//! (y grows upward).

use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str};

use ldgr_core::accounting::report_document::{ReportDocument, ReportRow};

// Page geometry (A4, PDF points; 1pt = 1/72 inch).
const PAGE_W: f32 = 595.28;
const PAGE_H: f32 = 841.89;
const MARGIN_L: f32 = 56.0;
const MARGIN_R: f32 = 56.0;
const MARGIN_T: f32 = 56.0;
const MARGIN_B: f32 = 56.0;

// Font sizes (points).
const SIZE_TITLE: f32 = 20.0;
const SIZE_PERIOD: f32 = 11.0;
const SIZE_HEADING: f32 = 13.0;
const SIZE_BODY: f32 = 10.0;
const SIZE_FOOTER: f32 = 8.0;

// Courier advance width is a fixed 600/1000 em.
const COURIER_ADVANCE_EM: f32 = 0.6;
// Indentation applied per nesting level (points).
const INDENT_STEP: f32 = 14.0;

// Resource names for the base-14 fonts referenced from content streams.
const FONT_REGULAR: &[u8] = b"F1";
const FONT_BOLD: &[u8] = b"F2";
const FONT_MONO: &[u8] = b"F3";
const FONT_MONO_BOLD: &[u8] = b"F4";

/// Line advance for a given font size, including leading.
fn line_height(size_pt: f32) -> f32 {
    size_pt * 1.45
}

/// Width (points) of a monospace Courier string at the given point size.
#[allow(clippy::cast_precision_loss)] // char counts are tiny; f32 is exact here
fn courier_width_pt(text: &str, size_pt: f32) -> f32 {
    text.chars().count() as f32 * size_pt * COURIER_ADVANCE_EM
}

fn content_right() -> f32 {
    PAGE_W - MARGIN_R
}

/// Encode a Rust UTF-8 string to WinAnsi/Latin-1 bytes for a base-14 font,
/// substituting `?` for any code point outside the single-byte range. This is a
/// best-effort transcoding sufficient for financial reports (ASCII plus common
/// Latin-1 currency/letter glyphs).
fn encode(s: &str) -> Vec<u8> {
    // Any code point outside 0x00..=0xFF cannot be a single WinAnsi byte.
    s.chars()
        .map(|c| u8::try_from(c as u32).unwrap_or(b'?'))
        .collect()
}

/// Object references allocated for the four base-14 fonts.
#[derive(Clone, Copy)]
struct Fonts {
    regular: Ref,
    bold: Ref,
    mono: Ref,
    mono_bold: Ref,
}

/// One page: its object id, its content-stream id, and the (still open)
/// content builder we append drawing operators to.
struct PageEntry {
    page_id: Ref,
    content_id: Ref,
    content: Content,
}

struct Painter {
    alloc: Ref,
    catalog: Ref,
    tree: Ref,
    fonts: Fonts,
    pages: Vec<PageEntry>,
    /// Baseline y (points from bottom) for the next line of text.
    cursor_y: f32,
    footer: String,
}

impl Painter {
    fn new(title: &str) -> Self {
        let mut alloc = Ref::new(1);
        let catalog = alloc.bump();
        let tree = alloc.bump();
        let fonts = Fonts {
            regular: alloc.bump(),
            bold: alloc.bump(),
            mono: alloc.bump(),
            mono_bold: alloc.bump(),
        };
        let mut painter = Self {
            alloc,
            catalog,
            tree,
            fonts,
            pages: Vec::new(),
            cursor_y: PAGE_H - MARGIN_T,
            footer: title.to_string(),
        };
        painter.start_page();
        painter
    }

    fn start_page(&mut self) {
        let page_id = self.alloc.bump();
        let content_id = self.alloc.bump();
        self.pages.push(PageEntry {
            page_id,
            content_id,
            content: Content::new(),
        });
        self.cursor_y = PAGE_H - MARGIN_T;
    }

    /// Ensure `needed` points of vertical space remain, else start a new page.
    fn ensure_space(&mut self, needed: f32) {
        if self.cursor_y - needed < MARGIN_B {
            self.start_page();
        }
    }

    fn advance(&mut self, dy: f32) {
        self.cursor_y -= dy;
    }

    /// Draw a line of text on the current page at an absolute baseline.
    fn text(&mut self, x: f32, y: f32, size: f32, font: &'static [u8], s: &str) {
        let bytes = encode(s);
        let content = &mut self.pages.last_mut().expect("a page exists").content;
        content.begin_text();
        content.set_font(Name(font), size);
        content.set_text_matrix([1.0, 0.0, 0.0, 1.0, x, y]);
        content.show(Str(&bytes));
        content.end_text();
    }

    /// Draw a light hairline rule across the content width.
    fn rule(&mut self, y: f32) {
        let content = &mut self.pages.last_mut().expect("a page exists").content;
        content.set_line_width(0.5);
        content.set_stroke_gray(0.65);
        content.move_to(MARGIN_L, y);
        content.line_to(content_right(), y);
        content.stroke();
    }

    fn header(&mut self, doc: &ReportDocument) {
        self.advance(line_height(SIZE_TITLE));
        self.text(MARGIN_L, self.cursor_y, SIZE_TITLE, FONT_BOLD, &doc.title);

        if let Some(period) = &doc.period {
            self.advance(line_height(SIZE_PERIOD));
            self.text(MARGIN_L, self.cursor_y, SIZE_PERIOD, FONT_REGULAR, period);
        }

        self.advance(7.0);
        self.rule(self.cursor_y);
        self.advance(11.0);
    }

    fn heading(&mut self, text: &str) {
        // Extra breathing room before each section.
        self.ensure_space(line_height(SIZE_HEADING) + 34.0);
        self.advance(11.0);
        self.advance(line_height(SIZE_HEADING));
        self.text(MARGIN_L, self.cursor_y, SIZE_HEADING, FONT_BOLD, text);
        self.advance(6.0);
        self.rule(self.cursor_y);
        self.advance(6.0);
    }

    fn text_label(&mut self, x: f32, y: f32, size: f32, bold: bool, s: &str) {
        let font = if bold { FONT_BOLD } else { FONT_REGULAR };
        self.text(x, y, size, font, s);
    }

    fn text_amount(&mut self, x: f32, y: f32, size: f32, bold: bool, s: &str) {
        let font = if bold { FONT_MONO_BOLD } else { FONT_MONO };
        self.text(x, y, size, font, s);
    }

    fn row(&mut self, row: &ReportRow) {
        let emphasis = row.emphasis;
        #[allow(clippy::cast_precision_loss)] // nesting depth is a small integer
        let label_x = MARGIN_L + row.depth as f32 * INDENT_STEP;

        // Emphasized rows (totals) get a hairline above them.
        if emphasis {
            self.ensure_space(line_height(SIZE_BODY) + 6.0);
            self.advance(4.0);
            self.rule(self.cursor_y);
            self.advance(6.0);
        }

        let amount_lines: Vec<String> = if row.amounts.is_empty() {
            vec![String::new()]
        } else {
            row.amounts.iter().map(amount_display).collect()
        };

        for (i, amount) in amount_lines.iter().enumerate() {
            let lh = line_height(SIZE_BODY);
            self.ensure_space(lh);
            self.advance(lh);
            let y = self.cursor_y;
            if i == 0 {
                self.text_label(label_x, y, SIZE_BODY, emphasis, &row.label);
            }
            if !amount.is_empty() {
                let w = courier_width_pt(amount, SIZE_BODY);
                let x = content_right() - w;
                self.text_amount(x, y, SIZE_BODY, emphasis, amount);
            }
        }
    }

    /// Draw the per-page footer (report title left, pagination right) on every
    /// page now that the total page count is known.
    fn draw_footers(&mut self) {
        let total = self.pages.len();
        let footer = self.footer.clone();
        let y = MARGIN_B * 0.5;
        for i in 0..total {
            let title_bytes = encode(&footer);
            let page_str = format!("Page {} of {}", i + 1, total);
            let page_bytes = encode(&page_str);
            let page_x = content_right() - courier_width_pt(&page_str, SIZE_FOOTER);
            let content = &mut self.pages[i].content;

            content.begin_text();
            content.set_font(Name(FONT_REGULAR), SIZE_FOOTER);
            content.set_text_matrix([1.0, 0.0, 0.0, 1.0, MARGIN_L, y]);
            content.show(Str(&title_bytes));
            content.end_text();

            content.begin_text();
            content.set_font(Name(FONT_MONO), SIZE_FOOTER);
            content.set_text_matrix([1.0, 0.0, 0.0, 1.0, page_x, y]);
            content.show(Str(&page_bytes));
            content.end_text();
        }
    }

    /// Finalize the document into PDF bytes.
    fn into_bytes(mut self) -> Vec<u8> {
        self.draw_footers();

        let catalog = self.catalog;
        let tree = self.tree;
        let fonts = self.fonts;

        // Serialize each page's content stream (consumes the builders).
        let pages: Vec<(Ref, Ref, Vec<u8>)> = self
            .pages
            .into_iter()
            .map(|e| (e.page_id, e.content_id, e.content.finish().to_vec()))
            .collect();

        let mut pdf = Pdf::new();
        pdf.catalog(catalog).pages(tree);

        {
            let count = i32::try_from(pages.len()).unwrap_or(i32::MAX);
            let mut tree_dict = pdf.pages(tree);
            tree_dict.count(count);
            tree_dict.kids(pages.iter().map(|(p, _, _)| *p));
            tree_dict.finish();
        }

        for (page_id, content_id, bytes) in &pages {
            {
                let mut page = pdf.page(*page_id);
                page.parent(tree);
                page.media_box(Rect::new(0.0, 0.0, PAGE_W, PAGE_H));
                page.contents(*content_id);
                let mut res = page.resources();
                let mut font_dict = res.fonts();
                font_dict.pair(Name(FONT_REGULAR), fonts.regular);
                font_dict.pair(Name(FONT_BOLD), fonts.bold);
                font_dict.pair(Name(FONT_MONO), fonts.mono);
                font_dict.pair(Name(FONT_MONO_BOLD), fonts.mono_bold);
                font_dict.finish();
                res.finish();
                page.finish();
            }
            pdf.stream(*content_id, bytes);
        }

        for (font_ref, base) in [
            (fonts.regular, &b"Helvetica"[..]),
            (fonts.bold, &b"Helvetica-Bold"[..]),
            (fonts.mono, &b"Courier"[..]),
            (fonts.mono_bold, &b"Courier-Bold"[..]),
        ] {
            pdf.type1_font(font_ref)
                .base_font(Name(base))
                .encoding_predefined(Name(b"WinAnsiEncoding"));
        }

        pdf.finish()
    }
}

/// Amount display kept local so tests can exercise formatting directly.
fn amount_display(amount: &ldgr_core::accounting::report_document::Amount) -> String {
    amount.display()
}

/// Render a [`ReportDocument`] to PDF bytes.
///
/// # Errors
/// Currently infallible, but returns a `Result` so callers integrate cleanly
/// with fallible rendering backends and future validation.
#[allow(clippy::unnecessary_wraps)] // stable fallible signature for callers
pub fn render_report(doc: &ReportDocument) -> anyhow::Result<Vec<u8>> {
    let mut painter = Painter::new(&doc.title);
    painter.header(doc);

    for section in &doc.sections {
        painter.heading(&section.heading);
        for row in &section.rows {
            painter.row(row);
        }
        if let Some(total) = &section.total {
            painter.row(total);
        }
    }

    if let Some(note) = &doc.note {
        painter.ensure_space(line_height(SIZE_FOOTER) + 17.0);
        painter.advance(17.0);
        painter.advance(line_height(SIZE_FOOTER));
        painter.text(MARGIN_L, painter.cursor_y, SIZE_FOOTER, FONT_REGULAR, note);
    }

    Ok(painter.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ldgr_core::accounting::report_document::{Amount, ReportRow, ReportSection};
    use rust_decimal::Decimal;

    fn sample_doc() -> ReportDocument {
        ReportDocument {
            title: "Balance Sheet".to_string(),
            period: Some("2024-01-01 to 2024-12-31".to_string()),
            sections: vec![ReportSection {
                heading: "Assets".to_string(),
                rows: vec![ReportRow {
                    label: "Checking".to_string(),
                    depth: 2,
                    amounts: vec![
                        Amount {
                            commodity: "USD".to_string(),
                            value: Decimal::new(150_000, 2),
                        },
                        Amount {
                            commodity: "EUR".to_string(),
                            value: Decimal::new(20_000, 2),
                        },
                    ],
                    emphasis: false,
                }],
                total: Some(ReportRow {
                    label: "Total Assets".to_string(),
                    depth: 1,
                    amounts: vec![Amount {
                        commodity: "USD".to_string(),
                        value: Decimal::new(150_000, 2),
                    }],
                    emphasis: true,
                }),
            }],
            note: Some("Generated by ldgr".to_string()),
        }
    }

    #[test]
    fn renders_valid_pdf_bytes() {
        let bytes = render_report(&sample_doc()).expect("render");
        assert!(
            bytes.len() > 500,
            "expected non-trivial PDF, got {}",
            bytes.len()
        );
        assert_eq!(
            &bytes[..5],
            b"%PDF-",
            "must start with the PDF magic header"
        );
    }

    #[test]
    fn courier_width_scales_with_length() {
        let short = courier_width_pt("1", SIZE_BODY);
        let long = courier_width_pt("1000000", SIZE_BODY);
        assert!(long > short);
        // Seven characters should be exactly 7x a single character.
        assert!((long - short * 7.0).abs() < 1e-3);
    }

    #[test]
    fn encode_replaces_non_latin1() {
        assert_eq!(encode("USD"), b"USD");
        // Euro sign (U+20AC) is outside Latin-1 and becomes '?'.
        assert_eq!(encode("\u{20ac}5"), b"?5");
        // Latin-1 letters within 0x00..=0xFF pass through.
        assert_eq!(encode("caf\u{e9}"), b"caf\xe9");
    }

    #[test]
    fn amount_display_includes_commodity() {
        let a = Amount {
            commodity: "USD".to_string(),
            value: Decimal::new(150_000, 2),
        };
        assert_eq!(amount_display(&a), "1500.00 USD");
    }

    /// Full pipeline: parse a journal → compute each report → build the document
    /// → render PDF. Mirrors the CLI path and covers the three required reports.
    #[test]
    fn pipeline_renders_all_three_reports() {
        use ldgr_core::accounting::query::Query;
        use ldgr_core::accounting::reports::{
            compute_balance_sheet, compute_income_statement, compute_net_worth,
        };
        use ldgr_core::accounting::{
            balance_sheet_document, income_statement_document, net_worth_document, parse_journal,
        };

        let journal = parse_journal(
            "2024-01-15 Paycheck\n    Assets:Checking      3000.00 USD\n    Income:Salary\n\n\
             2024-02-01 Rent\n    Expenses:Rent        1200.00 USD\n    Assets:Checking\n\n\
             2024-03-01 Brokerage transfer\n    Assets:Investments:Brokerage   500.00 USD\n    Assets:Checking\n",
        )
        .expect("journal parses");
        let txns = journal.transactions;
        let query = Query::parse(&[]);

        let bs = balance_sheet_document(&compute_balance_sheet(&txns, &query), None);
        let is = income_statement_document(&compute_income_statement(&txns, &query), None);
        let nw = net_worth_document(&compute_net_worth(&txns, &query), None);

        for doc in [bs, is, nw] {
            let bytes = render_report(&doc).expect("render");
            assert_eq!(&bytes[..5], b"%PDF-", "{} must be a valid PDF", doc.title);
            assert!(bytes.len() > 500);
        }
    }
}
