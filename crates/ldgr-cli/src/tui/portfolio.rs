//! Portfolio TUI: holdings view with market values, gain/loss, allocation.
//!
//! Requires an unlocked vault to read investment accounts and compute
//! holdings. Fetches current market prices for valuation.

use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use ldgr_core::market::Quote;

/// A single holding in the portfolio.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Holding {
    pub symbol: String,
    pub shares: Decimal,
    pub cost_basis: Decimal,
    pub cost_commodity: String,
    pub market_price: Option<Decimal>,
    pub market_value: Option<Decimal>,
    pub gain_loss: Option<Decimal>,
    pub gain_loss_pct: Option<Decimal>,
    pub allocation_pct: Option<Decimal>,
}

/// Portfolio UI mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortfolioMode {
    Normal,
    Chart,
}

/// Portfolio application state.
pub struct PortfolioApp {
    pub holdings: Vec<Holding>,
    pub table_state: TableState,
    pub mode: PortfolioMode,
    pub should_quit: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub last_refresh: Option<String>,
    pub total_value: Decimal,
    pub total_cost: Decimal,
    pub total_gain: Decimal,
}

impl PortfolioApp {
    /// Create a new portfolio app with the given holdings.
    pub fn new(holdings: Vec<Holding>) -> Self {
        let mut app = Self {
            holdings,
            table_state: TableState::default(),
            mode: PortfolioMode::Normal,
            should_quit: false,
            loading: true,
            error: None,
            last_refresh: None,
            total_value: Decimal::ZERO,
            total_cost: Decimal::ZERO,
            total_gain: Decimal::ZERO,
        };
        if !app.holdings.is_empty() {
            app.table_state.select(Some(0));
        }
        app
    }

    /// Get the currently selected holding's symbol.
    pub fn selected_symbol(&self) -> Option<&str> {
        self.table_state
            .selected()
            .and_then(|i| self.holdings.get(i))
            .map(|h| h.symbol.as_str())
    }

    /// Update holdings with fresh market quotes.
    pub fn update_quotes(&mut self, quotes: &[Quote]) {
        let quote_map: BTreeMap<String, &Quote> = quotes
            .iter()
            .map(|q| (q.symbol.to_uppercase(), q))
            .collect();

        let mut total_value = Decimal::ZERO;
        let mut total_cost = Decimal::ZERO;

        for holding in &mut self.holdings {
            if let Some(quote) = quote_map.get(&holding.symbol.to_uppercase()) {
                holding.market_price = Some(quote.price);
                let mv = holding.shares * quote.price;
                holding.market_value = Some(mv);
                let gl = mv - holding.cost_basis;
                holding.gain_loss = Some(gl);
                holding.gain_loss_pct = if holding.cost_basis.is_zero() {
                    None
                } else {
                    Some((gl / holding.cost_basis) * Decimal::new(100, 0))
                };
                total_value += mv;
            }
            total_cost += holding.cost_basis;
        }

        // Compute allocation percentages
        if !total_value.is_zero() {
            for holding in &mut self.holdings {
                if let Some(mv) = holding.market_value {
                    holding.allocation_pct = Some((mv / total_value) * Decimal::new(100, 0));
                }
            }
        }

        self.total_value = total_value;
        self.total_cost = total_cost;
        self.total_gain = total_value - total_cost;
        self.loading = false;
        self.last_refresh = Some(chrono::Local::now().format("%H:%M:%S").to_string());
    }

    /// Set an error state.
    pub fn set_error(&mut self, error: String) {
        self.loading = false;
        self.error = Some(error);
    }

    /// Handle a key event.
    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.holdings.len();
                if len > 0 {
                    let i = self.table_state.selected().map_or(0, |i| (i + 1) % len);
                    self.table_state.select(Some(i));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let len = self.holdings.len();
                if len > 0 {
                    let i = self
                        .table_state
                        .selected()
                        .map_or(0, |i| if i == 0 { len - 1 } else { i - 1 });
                    self.table_state.select(Some(i));
                }
            }
            KeyCode::Enter => {
                if self.selected_symbol().is_some() {
                    self.mode = PortfolioMode::Chart;
                }
            }
            KeyCode::Char('r') => {
                self.loading = true;
            }
            _ => {}
        }
    }

    /// Render the portfolio view.
    pub fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // summary
                Constraint::Min(5),    // table
                Constraint::Length(1), // status bar
            ])
            .split(area);

        self.render_summary(frame, chunks[0]);
        self.render_table(frame, chunks[1]);
        self.render_status_bar(frame, chunks[2]);
    }

    fn render_summary(&self, frame: &mut Frame, area: Rect) {
        let gain_color = if self.total_gain >= Decimal::ZERO {
            Color::Green
        } else {
            Color::Red
        };

        let gain_pct = if self.total_cost.is_zero() {
            Decimal::ZERO
        } else {
            (self.total_gain / self.total_cost) * Decimal::new(100, 0)
        };

        let refresh = self.last_refresh.as_deref().unwrap_or("—");

        let line1 = Line::from(vec![
            Span::styled(" 💼 Portfolio", Style::default().bold()),
            Span::raw(format!(
                "  Value: {}  Cost: {}  ",
                format_money(self.total_value),
                format_money(self.total_cost),
            )),
            Span::styled(
                format!(
                    "G/L: {} ({:.1}%)",
                    format_change(self.total_gain),
                    gain_pct.to_f64().unwrap_or(0.0)
                ),
                Style::default().fg(gain_color),
            ),
            Span::raw(format!("  Last: {refresh}")),
        ]);

        let block = Block::default().borders(Borders::BOTTOM);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(Paragraph::new(line1), inner);
    }

    fn render_table(&mut self, frame: &mut Frame, area: Rect) {
        if self.loading {
            let msg = Paragraph::new(" Loading portfolio data…")
                .block(Block::default().borders(Borders::ALL).title("Holdings"));
            frame.render_widget(msg, area);
            return;
        }

        if let Some(ref err) = self.error {
            let msg = Paragraph::new(format!(" Error: {err}"))
                .style(Style::default().fg(Color::Red))
                .block(Block::default().borders(Borders::ALL).title("Holdings"));
            frame.render_widget(msg, area);
            return;
        }

        if self.holdings.is_empty() {
            let msg = Paragraph::new(" No investment holdings found.\n Use investment accounts (e.g., Assets:Investments:*) to track holdings.")
                .block(Block::default().borders(Borders::ALL).title("Holdings"));
            frame.render_widget(msg, area);
            return;
        }

        let header = Row::new(vec![
            Cell::from("Symbol").style(Style::default().bold()),
            Cell::from("Shares").style(Style::default().bold()),
            Cell::from("Cost Basis").style(Style::default().bold()),
            Cell::from("Mkt Price").style(Style::default().bold()),
            Cell::from("Mkt Value").style(Style::default().bold()),
            Cell::from("Gain/Loss").style(Style::default().bold()),
            Cell::from("G/L %").style(Style::default().bold()),
            Cell::from("Alloc %").style(Style::default().bold()),
        ]);

        let rows: Vec<Row> = self
            .holdings
            .iter()
            .map(|h| {
                let gl_color = match h.gain_loss {
                    Some(gl) if gl >= Decimal::ZERO => Color::Green,
                    Some(_) => Color::Red,
                    None => Color::default(),
                };

                Row::new(vec![
                    Cell::from(h.symbol.clone()).style(Style::default().bold()),
                    Cell::from(format_qty(h.shares)),
                    Cell::from(format_money(h.cost_basis)),
                    Cell::from(h.market_price.map_or_else(|| "—".to_string(), format_money)),
                    Cell::from(h.market_value.map_or_else(|| "—".to_string(), format_money)),
                    Cell::from(h.gain_loss.map_or_else(|| "—".to_string(), format_change))
                        .style(Style::default().fg(gl_color)),
                    Cell::from(h.gain_loss_pct.map_or_else(
                        || "—".to_string(),
                        |p| format!("{:.1}%", p.to_f64().unwrap_or(0.0)),
                    ))
                    .style(Style::default().fg(gl_color)),
                    Cell::from(h.allocation_pct.map_or_else(
                        || "—".to_string(),
                        |p| format!("{:.1}%", p.to_f64().unwrap_or(0.0)),
                    )),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(8),  // Symbol
                Constraint::Length(10), // Shares
                Constraint::Length(12), // Cost Basis
                Constraint::Length(12), // Mkt Price
                Constraint::Length(12), // Mkt Value
                Constraint::Length(12), // Gain/Loss
                Constraint::Length(8),  // G/L %
                Constraint::Length(8),  // Alloc %
            ],
        )
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Holdings"))
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        frame.render_stateful_widget(table, area, &mut self.table_state);
    }

    #[allow(clippy::unused_self)]
    fn render_status_bar(&self, frame: &mut Frame, area: Rect) {
        let help = "q:Quit  ↑↓:Navigate  Enter:Chart  r:Refresh";
        let bar = Paragraph::new(Line::from(Span::styled(
            format!(" {help}"),
            Style::default().fg(Color::DarkGray),
        )));
        frame.render_widget(bar, area);
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn format_money(d: Decimal) -> String {
    let v = d.to_f64().unwrap_or(0.0);
    if v.abs() >= 1_000_000.0 {
        format!("${:.2}M", v / 1_000_000.0)
    } else if v.abs() >= 1_000.0 {
        // Format with comma separator
        let s = format!("{v:.2}");
        let parts: Vec<&str> = s.split('.').collect();
        let int_part = parts[0];
        let dec_part = parts.get(1).unwrap_or(&"00");

        let negative = int_part.starts_with('-');
        let digits: &str = if negative { &int_part[1..] } else { int_part };
        let mut result = String::new();
        for (i, ch) in digits.chars().rev().enumerate() {
            if i > 0 && i % 3 == 0 {
                result.push(',');
            }
            result.push(ch);
        }
        let formatted: String = result.chars().rev().collect();
        if negative {
            format!("-${formatted}.{dec_part}")
        } else {
            format!("${formatted}.{dec_part}")
        }
    } else {
        format!("${v:.2}")
    }
}

fn format_change(d: Decimal) -> String {
    let v = d.to_f64().unwrap_or(0.0);
    if v >= 0.0 {
        format!("+${v:.2}")
    } else {
        format!("-${:.2}", v.abs())
    }
}

fn format_qty(d: Decimal) -> String {
    let v = d.to_f64().unwrap_or(0.0);
    if v.fract().abs() < 0.0001 {
        format!("{v:.0}")
    } else {
        format!("{v:.4}")
    }
}
