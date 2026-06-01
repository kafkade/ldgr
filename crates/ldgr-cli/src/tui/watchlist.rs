//! Watchlist TUI: real-time price tracking with sparklines.
//!
//! `ldgr watch [symbols...]` — standalone mode, no vault required.
//! Auto-refreshes at a configurable interval (default 15s).

use std::collections::{BTreeMap, VecDeque};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Sparkline, Table, TableState};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use ldgr_core::market::Quote;

use crate::theme::CliTheme;

/// Maximum number of sparkline data points per symbol.
const SPARKLINE_CAPACITY: usize = 60;

/// UI mode for the watchlist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchlistMode {
    Normal,
    AddSymbol { input: String },
    Search { input: String },
    SortMenu,
}

/// Sort field for the watchlist table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    Symbol,
    Price,
    Change,
    ChangePercent,
    Volume,
}

impl SortField {
    fn label(self) -> &'static str {
        match self {
            Self::Symbol => "Symbol",
            Self::Price => "Price",
            Self::Change => "Change",
            Self::ChangePercent => "Change%",
            Self::Volume => "Volume",
        }
    }

    const ALL: &[Self] = &[
        Self::Symbol,
        Self::Price,
        Self::Change,
        Self::ChangePercent,
        Self::Volume,
    ];
}

/// Per-symbol state including sparkline history.
#[derive(Debug, Clone)]
pub struct SymbolState {
    pub quote: Option<Quote>,
    pub sparkline: VecDeque<u64>,
    pub loading: bool,
    pub error: Option<String>,
}

impl SymbolState {
    fn new() -> Self {
        Self {
            quote: None,
            sparkline: VecDeque::with_capacity(SPARKLINE_CAPACITY),
            loading: false,
            error: None,
        }
    }

    fn push_price(&mut self, price: Decimal) {
        // Scale price to integer for sparkline (multiply by 100 for cents)
        let scaled = (price * Decimal::new(100, 0)).to_u64().unwrap_or(0);
        if self.sparkline.len() >= SPARKLINE_CAPACITY {
            self.sparkline.pop_front();
        }
        self.sparkline.push_back(scaled);
    }
}

/// Watchlist application state.
pub struct WatchlistApp {
    pub symbols: Vec<String>,
    pub states: BTreeMap<String, SymbolState>,
    pub table_state: TableState,
    pub mode: WatchlistMode,
    pub sort_field: SortField,
    pub sort_ascending: bool,
    pub search_filter: Option<String>,
    pub should_quit: bool,
    pub last_refresh: Option<String>,
    pub refresh_in_flight: bool,
    pub status_message: Option<String>,
    sort_selection: usize,
}

impl WatchlistApp {
    /// Create a new watchlist app with the given initial symbols.
    pub fn new(symbols: Vec<String>) -> Self {
        let mut states = BTreeMap::new();
        for s in &symbols {
            states.insert(s.to_uppercase(), SymbolState::new());
        }
        let symbols: Vec<String> = symbols.into_iter().map(|s| s.to_uppercase()).collect();

        let mut app = Self {
            symbols,
            states,
            table_state: TableState::default(),
            mode: WatchlistMode::Normal,
            sort_field: SortField::Symbol,
            sort_ascending: true,
            search_filter: None,
            should_quit: false,
            last_refresh: None,
            refresh_in_flight: false,
            status_message: None,
            sort_selection: 0,
        };
        if !app.symbols.is_empty() {
            app.table_state.select(Some(0));
        }
        app
    }

    /// Get the currently selected symbol.
    pub fn selected_symbol(&self) -> Option<&str> {
        let filtered = self.filtered_symbols();
        let idx = self.table_state.selected()?;
        filtered.get(idx).map(|s| {
            // Look up from the canonical list since filtered is temporary
            self.symbols
                .iter()
                .find(|sym| *sym == s)
                .map_or("", String::as_str)
        })
    }

    /// Get symbols filtered by search.
    pub fn filtered_symbols(&self) -> Vec<String> {
        let mut syms: Vec<String> = match &self.search_filter {
            Some(filter) => {
                let f = filter.to_uppercase();
                self.symbols
                    .iter()
                    .filter(|s| s.contains(&f))
                    .cloned()
                    .collect()
            }
            None => self.symbols.clone(),
        };

        // Sort
        syms.sort_by(|a, b| {
            let ord = match self.sort_field {
                SortField::Symbol => a.cmp(b),
                SortField::Price => {
                    let pa = self
                        .states
                        .get(a)
                        .and_then(|s| s.quote.as_ref())
                        .map(|q| q.price);
                    let pb = self
                        .states
                        .get(b)
                        .and_then(|s| s.quote.as_ref())
                        .map(|q| q.price);
                    pa.cmp(&pb)
                }
                SortField::Change => {
                    let ca = self
                        .states
                        .get(a)
                        .and_then(|s| s.quote.as_ref())
                        .map(|q| q.change);
                    let cb = self
                        .states
                        .get(b)
                        .and_then(|s| s.quote.as_ref())
                        .map(|q| q.change);
                    ca.cmp(&cb)
                }
                SortField::ChangePercent => {
                    let ca = self
                        .states
                        .get(a)
                        .and_then(|s| s.quote.as_ref())
                        .map(|q| q.change_percent);
                    let cb = self
                        .states
                        .get(b)
                        .and_then(|s| s.quote.as_ref())
                        .map(|q| q.change_percent);
                    ca.cmp(&cb)
                }
                SortField::Volume => {
                    let va = self
                        .states
                        .get(a)
                        .and_then(|s| s.quote.as_ref())
                        .and_then(|q| q.volume);
                    let vb = self
                        .states
                        .get(b)
                        .and_then(|s| s.quote.as_ref())
                        .and_then(|q| q.volume);
                    va.cmp(&vb)
                }
            };
            if self.sort_ascending {
                ord
            } else {
                ord.reverse()
            }
        });

        syms
    }

    /// Update quotes from a fetch result.
    pub fn update_quotes(&mut self, quotes: Vec<Quote>) {
        for quote in quotes {
            let sym = quote.symbol.to_uppercase();
            let state = self.states.entry(sym).or_insert_with(SymbolState::new);
            state.push_price(quote.price);
            state.loading = false;
            state.error = None;
            state.quote = Some(quote);
        }
        self.last_refresh = Some(chrono::Local::now().format("%H:%M:%S").to_string());
        self.refresh_in_flight = false;
    }

    /// Record a fetch error for a symbol.
    pub fn set_error(&mut self, symbol: &str, error: String) {
        let sym = symbol.to_uppercase();
        if let Some(state) = self.states.get_mut(&sym) {
            state.loading = false;
            state.error = Some(error);
        }
        self.refresh_in_flight = false;
    }

    /// Mark all symbols as loading.
    pub fn mark_loading(&mut self) {
        self.refresh_in_flight = true;
        for state in self.states.values_mut() {
            state.loading = true;
        }
    }

    /// Add a new symbol to the watchlist.
    pub fn add_symbol(&mut self, symbol: &str) {
        let sym = symbol.to_uppercase();
        if !self.symbols.contains(&sym) {
            self.symbols.push(sym.clone());
            self.states.insert(sym, SymbolState::new());
            self.status_message = Some(format!("Added {}", symbol.to_uppercase()));
        }
    }

    /// Remove the currently selected symbol.
    pub fn remove_selected(&mut self) {
        let filtered = self.filtered_symbols();
        if let Some(idx) = self.table_state.selected()
            && let Some(sym) = filtered.get(idx).cloned()
        {
            self.symbols.retain(|s| s != &sym);
            self.states.remove(&sym);
            self.status_message = Some(format!("Removed {sym}"));
            // Adjust selection
            let new_len = self.filtered_symbols().len();
            if new_len == 0 {
                self.table_state.select(None);
            } else if idx >= new_len {
                self.table_state.select(Some(new_len - 1));
            }
        }
    }

    /// Handle a key event.
    pub fn handle_key(&mut self, key: KeyEvent) {
        // Ctrl+C always quits
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        match &self.mode {
            WatchlistMode::Normal => self.handle_normal_key(key),
            WatchlistMode::AddSymbol { .. } => self.handle_add_key(key),
            WatchlistMode::Search { .. } => self.handle_search_key(key),
            WatchlistMode::SortMenu => self.handle_sort_key(key),
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('a') => {
                self.mode = WatchlistMode::AddSymbol {
                    input: String::new(),
                };
            }
            KeyCode::Char('d') => self.remove_selected(),
            KeyCode::Char('s') => {
                self.sort_selection = SortField::ALL
                    .iter()
                    .position(|&f| f == self.sort_field)
                    .unwrap_or(0);
                self.mode = WatchlistMode::SortMenu;
            }
            KeyCode::Char('/') => {
                self.mode = WatchlistMode::Search {
                    input: String::new(),
                };
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.filtered_symbols().len();
                if len > 0 {
                    let i = self.table_state.selected().map_or(0, |i| (i + 1) % len);
                    self.table_state.select(Some(i));
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let len = self.filtered_symbols().len();
                if len > 0 {
                    let i = self
                        .table_state
                        .selected()
                        .map_or(0, |i| if i == 0 { len - 1 } else { i - 1 });
                    self.table_state.select(Some(i));
                }
            }
            _ => {}
        }
    }

    fn handle_add_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                if let WatchlistMode::AddSymbol { input } = &self.mode {
                    let sym = input.trim().to_string();
                    if !sym.is_empty() {
                        self.add_symbol(&sym);
                    }
                }
                self.mode = WatchlistMode::Normal;
            }
            KeyCode::Esc => {
                self.mode = WatchlistMode::Normal;
            }
            KeyCode::Backspace => {
                if let WatchlistMode::AddSymbol { input } = &mut self.mode {
                    input.pop();
                }
            }
            KeyCode::Char(c) => {
                if let WatchlistMode::AddSymbol { input } = &mut self.mode {
                    input.push(c);
                }
            }
            _ => {}
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                if let WatchlistMode::Search { input } = &self.mode {
                    if input.is_empty() {
                        self.search_filter = None;
                    } else {
                        self.search_filter = Some(input.clone());
                    }
                }
                self.mode = WatchlistMode::Normal;
                // Reset selection
                if self.filtered_symbols().is_empty() {
                    self.table_state.select(None);
                } else {
                    self.table_state.select(Some(0));
                }
            }
            KeyCode::Esc => {
                self.search_filter = None;
                self.mode = WatchlistMode::Normal;
                if !self.filtered_symbols().is_empty() {
                    self.table_state.select(Some(0));
                }
            }
            KeyCode::Backspace => {
                if let WatchlistMode::Search { input } = &mut self.mode {
                    input.pop();
                }
            }
            KeyCode::Char(c) => {
                if let WatchlistMode::Search { input } = &mut self.mode {
                    input.push(c);
                }
            }
            _ => {}
        }
    }

    fn handle_sort_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => {
                self.sort_field = SortField::ALL[self.sort_selection];
                self.mode = WatchlistMode::Normal;
            }
            KeyCode::Esc | KeyCode::Char('s') => {
                self.mode = WatchlistMode::Normal;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.sort_selection = (self.sort_selection + 1) % SortField::ALL.len();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.sort_selection == 0 {
                    self.sort_selection = SortField::ALL.len() - 1;
                } else {
                    self.sort_selection -= 1;
                }
            }
            KeyCode::Char('r') => {
                self.sort_ascending = !self.sort_ascending;
            }
            _ => {}
        }
    }

    /// Render the watchlist TUI.
    pub fn render(&mut self, frame: &mut Frame, theme: &CliTheme) {
        let area = frame.area();

        // Main layout: header + table + footer
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // header
                Constraint::Min(5),    // table
                Constraint::Length(1), // status bar
            ])
            .split(area);

        self.render_header(frame, chunks[0]);
        self.render_table(frame, chunks[1], theme);
        self.render_status_bar(frame, chunks[2], theme);

        // Overlay for modal dialogs
        match &self.mode {
            WatchlistMode::AddSymbol { input } => {
                self.render_input_dialog(frame, "Add Symbol", input, area, theme);
            }
            WatchlistMode::Search { input } => {
                self.render_input_dialog(frame, "Search", input, area, theme);
            }
            WatchlistMode::SortMenu => {
                self.render_sort_menu(frame, area, theme);
            }
            WatchlistMode::Normal => {}
        }
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let refresh_text = match &self.last_refresh {
            Some(t) => format!(" | Last: {t}"),
            None => String::new(),
        };
        let loading = if self.refresh_in_flight { " ⟳" } else { "" };
        let filter = match &self.search_filter {
            Some(f) => format!(" | Filter: {f}"),
            None => String::new(),
        };
        let header = Line::from(vec![
            Span::styled(" 📈 ldgr watch", Style::default().bold()),
            Span::raw(format!(
                " — {} symbols{refresh_text}{loading}{filter}",
                self.symbols.len()
            )),
        ]);
        frame.render_widget(Paragraph::new(header), area);
    }

    fn render_table(&mut self, frame: &mut Frame, area: Rect, theme: &CliTheme) {
        let filtered = self.filtered_symbols();

        let header = Row::new(vec![
            Cell::from("Symbol").style(Style::default().bold()),
            Cell::from("Price").style(Style::default().bold()),
            Cell::from("Change").style(Style::default().bold()),
            Cell::from("Change%").style(Style::default().bold()),
            Cell::from("Volume").style(Style::default().bold()),
            Cell::from("Sparkline").style(Style::default().bold()),
        ]);

        let rows: Vec<Row> = filtered
            .iter()
            .map(|sym| {
                let state = self.states.get(sym);
                if let Some(q) = state.and_then(|s| s.quote.as_ref()) {
                    let change_color = if q.change >= Decimal::ZERO {
                        theme.positive
                    } else {
                        theme.negative
                    };
                    let vol_str = q.volume.map_or_else(|| "—".to_string(), format_volume);

                    let sparkline_str = state
                        .map(|s| render_inline_sparkline(&s.sparkline))
                        .unwrap_or_default();

                    Row::new(vec![
                        Cell::from(sym.clone()).style(Style::default().bold()),
                        Cell::from(format_decimal(q.price)),
                        Cell::from(format_change(q.change))
                            .style(Style::default().fg(change_color)),
                        Cell::from(format!("{:.2}%", q.change_percent.to_f64().unwrap_or(0.0)))
                            .style(Style::default().fg(change_color)),
                        Cell::from(vol_str),
                        Cell::from(sparkline_str),
                    ])
                } else {
                    let status = if state.is_some_and(|s| s.loading) {
                        "Loading…"
                    } else if let Some(s) = state {
                        if let Some(ref e) = s.error {
                            return Row::new(vec![
                                Cell::from(sym.clone()),
                                Cell::from(e.as_str()).style(Style::default().fg(theme.negative)),
                                Cell::from(""),
                                Cell::from(""),
                                Cell::from(""),
                                Cell::from(""),
                            ]);
                        }
                        "—"
                    } else {
                        "—"
                    };
                    Row::new(vec![
                        Cell::from(sym.clone()),
                        Cell::from(status),
                        Cell::from(""),
                        Cell::from(""),
                        Cell::from(""),
                        Cell::from(""),
                    ])
                }
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(8),  // Symbol
                Constraint::Length(12), // Price
                Constraint::Length(12), // Change
                Constraint::Length(10), // Change%
                Constraint::Length(10), // Volume
                Constraint::Min(10),    // Sparkline
            ],
        )
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Watchlist"))
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        frame.render_stateful_widget(table, area, &mut self.table_state);
    }

    fn render_status_bar(&self, frame: &mut Frame, area: Rect, theme: &CliTheme) {
        let status = match &self.status_message {
            Some(msg) => msg.clone(),
            None => "q:Quit  a:Add  d:Delete  s:Sort  /:Search  ↑↓:Navigate  r:Refresh".to_string(),
        };
        let bar = Paragraph::new(Line::from(Span::styled(
            format!(" {status}"),
            Style::default().fg(theme.muted),
        )));
        frame.render_widget(bar, area);
    }

    #[allow(clippy::unused_self)]
    fn render_input_dialog(
        &self,
        frame: &mut Frame,
        title: &str,
        input: &str,
        area: Rect,
        theme: &CliTheme,
    ) {
        let dialog_area = centered_rect(40, 3, area);
        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {title} "))
            .style(Style::default().fg(theme.accent));

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        let text = Paragraph::new(format!("{input}▌"));
        frame.render_widget(text, inner);
    }

    #[allow(clippy::cast_possible_truncation)]
    fn render_sort_menu(&self, frame: &mut Frame, area: Rect, theme: &CliTheme) {
        let menu_height = SortField::ALL.len() as u16 + 2;
        let dialog_area = centered_rect(30, menu_height, area);
        frame.render_widget(Clear, dialog_area);

        let dir_label = if self.sort_ascending {
            "↑ Asc"
        } else {
            "↓ Desc"
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" Sort ({dir_label}, r:toggle) "))
            .style(Style::default().fg(theme.info));

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        let items: Vec<Line> = SortField::ALL
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let marker = if i == self.sort_selection {
                    "▸ "
                } else {
                    "  "
                };
                let style = if i == self.sort_selection {
                    Style::default().bold().fg(theme.info)
                } else {
                    Style::default()
                };
                Line::from(Span::styled(format!("{marker}{}", f.label()), style))
            })
            .collect();

        frame.render_widget(Paragraph::new(items), inner);
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn format_decimal(d: Decimal) -> String {
    format!("{:.2}", d.to_f64().unwrap_or(0.0))
}

fn format_change(d: Decimal) -> String {
    let v = d.to_f64().unwrap_or(0.0);
    if v >= 0.0 {
        format!("+{v:.2}")
    } else {
        format!("{v:.2}")
    }
}

#[allow(clippy::cast_precision_loss)]
fn format_volume(v: u64) -> String {
    if v >= 1_000_000_000 {
        format!("{:.1}B", v as f64 / 1_000_000_000.0)
    } else if v >= 1_000_000 {
        format!("{:.1}M", v as f64 / 1_000_000.0)
    } else if v >= 1_000 {
        format!("{:.1}K", v as f64 / 1_000.0)
    } else {
        v.to_string()
    }
}

/// Render sparkline as Unicode block characters inline.
fn render_inline_sparkline(data: &VecDeque<u64>) -> String {
    if data.is_empty() {
        return String::new();
    }
    let blocks = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let min = *data.iter().min().unwrap_or(&0);
    let max = *data.iter().max().unwrap_or(&0);
    let range = if max == min { 1 } else { max - min };

    data.iter()
        .map(|&v| {
            #[allow(clippy::cast_possible_truncation)]
            let idx = ((v - min) * 7 / range) as usize;
            blocks[idx.min(7)]
        })
        .collect()
}

/// Create a centered rectangle within the given area.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

/// Sparkline widget rendering for a dedicated area (used in chart view).
#[allow(dead_code)]
pub fn render_sparkline_widget(
    frame: &mut Frame,
    area: Rect,
    data: &[u64],
    title: &str,
    theme: &CliTheme,
) {
    let sparkline = Sparkline::default()
        .block(Block::default().borders(Borders::ALL).title(title))
        .data(data)
        .style(Style::default().fg(theme.chart_line));
    frame.render_widget(sparkline, area);
}
