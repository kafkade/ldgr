//! Render a layout-agnostic [`ReportDocument`] to PDF bytes.
//!
//! Uses `printpdf` with the built-in Standard-14 fonts (Helvetica / Courier), so
//! no font files are bundled. Proportional Helvetica is used for labels/headings;
//! monospace Courier is used for amounts so the right-aligned money column can be
//! measured from a fixed glyph advance (no font-metrics table required).
//!
//! Layout is a simple top-down cursor with automatic page breaks, giving a
//! paginated, styled document: title, period subtitle, section headings, an
//! indented account hierarchy, and emphasized totals.

use printpdf::{
    BuiltinFont, Color, IndirectFontRef, Line, Mm, PdfDocument, PdfDocumentReference,
    PdfLayerIndex, PdfPageIndex, Point, Rgb,
};

use ldgr_core::accounting::report_document::{ReportDocument, ReportRow};

// Page geometry (A4, millimetres).
const PAGE_W: f32 = 210.0;
const PAGE_H: f32 = 297.0;
const MARGIN_L: f32 = 20.0;
const MARGIN_R: f32 = 20.0;
const MARGIN_T: f32 = 20.0;
const MARGIN_B: f32 = 20.0;

// Font sizes (points).
const SIZE_TITLE: f32 = 20.0;
const SIZE_PERIOD: f32 = 11.0;
const SIZE_HEADING: f32 = 13.0;
const SIZE_BODY: f32 = 10.0;
const SIZE_FOOTER: f32 = 8.0;

// Courier advance width is a fixed 600/1000 em.
const COURIER_ADVANCE_EM: f32 = 0.6;
// Indentation applied per nesting level (mm).
const INDENT_STEP: f32 = 5.0;

const PT_TO_MM: f32 = 25.4 / 72.0;

fn pt_to_mm(pt: f32) -> f32 {
    pt * PT_TO_MM
}

/// Line advance for a given font size, including leading.
fn line_height(size_pt: f32) -> f32 {
    pt_to_mm(size_pt) * 1.45
}

/// Width (mm) of a monospace Courier string at the given point size.
#[allow(clippy::cast_precision_loss)] // char counts are tiny; f32 is exact here
fn courier_width_mm(text: &str, size_pt: f32) -> f32 {
    text.chars().count() as f32 * size_pt * COURIER_ADVANCE_EM * PT_TO_MM
}

fn content_right() -> f32 {
    PAGE_W - MARGIN_R
}

struct Fonts {
    regular: IndirectFontRef,
    bold: IndirectFontRef,
    mono: IndirectFontRef,
    mono_bold: IndirectFontRef,
}

struct Painter {
    doc: PdfDocumentReference,
    fonts: Fonts,
    pages: Vec<(PdfPageIndex, PdfLayerIndex)>,
    current: usize,
    /// Baseline y (mm from bottom) for the next line of text.
    cursor_y: f32,
    footer: String,
}

impl Painter {
    fn new(title: &str) -> Self {
        let (doc, page1, layer1) = PdfDocument::new(title, Mm(PAGE_W), Mm(PAGE_H), "content");
        let fonts = Fonts {
            regular: doc.add_builtin_font(BuiltinFont::Helvetica).unwrap(),
            bold: doc.add_builtin_font(BuiltinFont::HelveticaBold).unwrap(),
            mono: doc.add_builtin_font(BuiltinFont::Courier).unwrap(),
            mono_bold: doc.add_builtin_font(BuiltinFont::CourierBold).unwrap(),
        };
        Self {
            doc,
            fonts,
            pages: vec![(page1, layer1)],
            current: 0,
            cursor_y: PAGE_H - MARGIN_T,
            footer: title.to_string(),
        }
    }

    fn layer(&self) -> printpdf::PdfLayerReference {
        let (page, layer) = self.pages[self.current];
        self.doc.get_page(page).get_layer(layer)
    }

    fn new_page(&mut self) {
        let (page, layer) = self.doc.add_page(Mm(PAGE_W), Mm(PAGE_H), "content");
        self.pages.push((page, layer));
        self.current = self.pages.len() - 1;
        self.cursor_y = PAGE_H - MARGIN_T;
    }

    /// Ensure `needed` mm of vertical space remains, else start a new page.
    fn ensure_space(&mut self, needed: f32) {
        if self.cursor_y - needed < MARGIN_B {
            self.new_page();
        }
    }

    fn text(&self, x: f32, y: f32, size: f32, font: &IndirectFontRef, s: &str) {
        self.layer().use_text(s, size, Mm(x), Mm(y), font);
    }

    fn rule(&self, y: f32) {
        let layer = self.layer();
        layer.set_outline_thickness(0.4);
        layer.set_outline_color(Color::Rgb(Rgb::new(0.65, 0.65, 0.65, None)));
        let line = Line {
            points: vec![
                (Point::new(Mm(MARGIN_L), Mm(y)), false),
                (Point::new(Mm(content_right()), Mm(y)), false),
            ],
            is_closed: false,
        };
        layer.add_line(line);
    }

    fn advance(&mut self, dy: f32) {
        self.cursor_y -= dy;
    }

    fn header(&mut self, doc: &ReportDocument) {
        let lh = line_height(SIZE_TITLE);
        self.advance(lh);
        self.text(
            MARGIN_L,
            self.cursor_y,
            SIZE_TITLE,
            &self.fonts.bold,
            &doc.title,
        );

        if let Some(period) = &doc.period {
            self.advance(line_height(SIZE_PERIOD));
            self.text(
                MARGIN_L,
                self.cursor_y,
                SIZE_PERIOD,
                &self.fonts.regular,
                period,
            );
        }

        self.advance(2.5);
        self.rule(self.cursor_y);
        self.advance(4.0);
    }

    fn heading(&mut self, text: &str) {
        // Extra breathing room before each section.
        self.ensure_space(line_height(SIZE_HEADING) + 12.0);
        self.advance(4.0);
        self.advance(line_height(SIZE_HEADING));
        self.text(
            MARGIN_L,
            self.cursor_y,
            SIZE_HEADING,
            &self.fonts.bold,
            text,
        );
        self.advance(2.0);
        self.rule(self.cursor_y);
        self.advance(2.0);
    }

    fn text_label(&self, x: f32, y: f32, size: f32, bold: bool, s: &str) {
        let font = if bold {
            &self.fonts.bold
        } else {
            &self.fonts.regular
        };
        self.text(x, y, size, font, s);
    }

    fn text_amount(&self, x: f32, y: f32, size: f32, bold: bool, s: &str) {
        let font = if bold {
            &self.fonts.mono_bold
        } else {
            &self.fonts.mono
        };
        self.text(x, y, size, font, s);
    }

    fn row(&mut self, row: &ReportRow) {
        let emphasis = row.emphasis;
        #[allow(clippy::cast_precision_loss)] // nesting depth is a small integer
        let label_x = MARGIN_L + row.depth as f32 * INDENT_STEP;

        // Emphasized rows (totals) get a hairline above them.
        if emphasis {
            self.ensure_space(line_height(SIZE_BODY) + 2.0);
            self.advance(1.5);
            self.rule(self.cursor_y);
            self.advance(2.0);
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
                let w = courier_width_mm(amount, SIZE_BODY);
                let x = content_right() - w;
                self.text_amount(x, y, SIZE_BODY, emphasis, amount);
            }
        }
    }

    fn footers(&self) {
        let total = self.pages.len();
        for (i, &(page, layer)) in self.pages.iter().enumerate() {
            let layer_ref = self.doc.get_page(page).get_layer(layer);
            let y = MARGIN_B * 0.5;
            layer_ref.use_text(
                &self.footer,
                SIZE_FOOTER,
                Mm(MARGIN_L),
                Mm(y),
                &self.fonts.regular,
            );
            let page_str = format!("Page {} of {}", i + 1, total);
            let w = courier_width_mm(&page_str, SIZE_FOOTER);
            layer_ref.use_text(
                &page_str,
                SIZE_FOOTER,
                Mm(content_right() - w),
                Mm(y),
                &self.fonts.mono,
            );
        }
    }
}

/// Amount display kept local so tests can exercise formatting directly.
fn amount_display(amount: &ldgr_core::accounting::report_document::Amount) -> String {
    amount.display()
}

/// Render a [`ReportDocument`] to PDF bytes.
///
/// # Errors
/// Returns an error if the underlying PDF serialization fails.
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
        painter.ensure_space(line_height(SIZE_FOOTER) + 6.0);
        painter.advance(6.0);
        painter.advance(line_height(SIZE_FOOTER));
        painter.text(
            MARGIN_L,
            painter.cursor_y,
            SIZE_FOOTER,
            &painter.fonts.regular,
            note,
        );
    }

    painter.footers();

    let bytes = painter
        .doc
        .save_to_bytes()
        .map_err(|e| anyhow::anyhow!("failed to serialize PDF: {e}"))?;
    Ok(bytes)
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
        let short = courier_width_mm("1", SIZE_BODY);
        let long = courier_width_mm("1000000", SIZE_BODY);
        assert!(long > short);
        // Seven characters should be exactly 7x a single character.
        assert!((long - short * 7.0).abs() < 1e-3);
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
