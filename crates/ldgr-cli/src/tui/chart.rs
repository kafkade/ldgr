//! Interactive chart TUI: line and candlestick charts with timeframe selection.
//!
//! Renders OHLCV data in a dedicated chart view with zoom, volume overlay,
//! and moving average support.

use std::collections::VecDeque;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Axis, Bar, BarChart, BarGroup, Block, Borders, Chart, Dataset, GraphType, Paragraph,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use ldgr_core::market::Ohlcv;

use crate::theme::CliTheme;

/// Chart display mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartType {
    Line,
    Candlestick,
}

/// Timeframe for historical data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timeframe {
    OneDay,
    OneWeek,
    OneMonth,
    ThreeMonths,
    SixMonths,
    OneYear,
    FiveYears,
}

impl Timeframe {
    pub fn label(self) -> &'static str {
        match self {
            Self::OneDay => "1D",
            Self::OneWeek => "1W",
            Self::OneMonth => "1M",
            Self::ThreeMonths => "3M",
            Self::SixMonths => "6M",
            Self::OneYear => "1Y",
            Self::FiveYears => "5Y",
        }
    }

    /// Number of calendar days for this timeframe.
    pub fn days(self) -> i64 {
        match self {
            Self::OneDay => 1,
            Self::OneWeek => 7,
            Self::OneMonth => 30,
            Self::ThreeMonths => 90,
            Self::SixMonths => 180,
            Self::OneYear => 365,
            Self::FiveYears => 1825,
        }
    }

    /// Yahoo Finance range parameter.
    #[allow(dead_code)]
    pub fn yahoo_range(self) -> &'static str {
        match self {
            Self::OneDay => "1d",
            Self::OneWeek => "5d",
            Self::OneMonth => "1mo",
            Self::ThreeMonths => "3mo",
            Self::SixMonths => "6mo",
            Self::OneYear => "1y",
            Self::FiveYears => "5y",
        }
    }

    pub const ALL: &[Self] = &[
        Self::OneDay,
        Self::OneWeek,
        Self::OneMonth,
        Self::ThreeMonths,
        Self::SixMonths,
        Self::OneYear,
        Self::FiveYears,
    ];

    /// Map key number (1-7) to timeframe.
    pub fn from_key(n: u8) -> Option<Self> {
        Self::ALL.get(n.wrapping_sub(1) as usize).copied()
    }
}

/// Chart application state.
#[allow(clippy::struct_excessive_bools)]
pub struct ChartApp {
    pub symbol: String,
    pub bars: Vec<Ohlcv>,
    pub chart_type: ChartType,
    pub timeframe: Timeframe,
    pub show_volume: bool,
    pub show_ma: bool,
    pub ma_period: usize,
    pub zoom_level: usize,
    pub scroll_offset: usize,
    pub should_quit: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub needs_data: bool,
}

impl ChartApp {
    /// Create a new chart app for a symbol.
    pub fn new(symbol: String) -> Self {
        Self {
            symbol,
            bars: Vec::new(),
            chart_type: ChartType::Line,
            timeframe: Timeframe::OneMonth,
            show_volume: false,
            show_ma: false,
            ma_period: 20,
            zoom_level: 0,
            scroll_offset: 0,
            should_quit: false,
            loading: true,
            error: None,
            needs_data: true,
        }
    }

    /// Update with new OHLCV data.
    pub fn update_data(&mut self, bars: Vec<Ohlcv>) {
        self.bars = bars;
        self.loading = false;
        self.error = None;
        self.scroll_offset = 0;
        self.zoom_level = 0;
        self.needs_data = false;
    }

    /// Set an error state.
    pub fn set_error(&mut self, error: String) {
        self.loading = false;
        self.error = Some(error);
        self.needs_data = false;
    }

    /// Visible window of bars based on zoom and scroll.
    fn visible_bars(&self) -> &[Ohlcv] {
        if self.bars.is_empty() {
            return &[];
        }
        let total = self.bars.len();
        let window = (total / (self.zoom_level + 1)).max(5).min(total);
        let max_offset = total.saturating_sub(window);
        let offset = self.scroll_offset.min(max_offset);
        let end = (offset + window).min(total);
        &self.bars[offset..end]
    }

    /// Handle a key event.
    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('l') => self.chart_type = ChartType::Line,
            KeyCode::Char('c') => self.chart_type = ChartType::Candlestick,
            KeyCode::Char('v') => self.show_volume = !self.show_volume,
            KeyCode::Char('m') => self.show_ma = !self.show_ma,
            KeyCode::Char('+' | '=') => {
                if self.bars.len() / (self.zoom_level + 2) >= 5 {
                    self.zoom_level += 1;
                }
            }
            KeyCode::Char('-') => {
                self.zoom_level = self.zoom_level.saturating_sub(1);
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            KeyCode::Right => {
                let total = self.bars.len();
                let window = total / (self.zoom_level + 1);
                let max_offset = total.saturating_sub(window);
                if self.scroll_offset < max_offset {
                    self.scroll_offset += 1;
                }
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let n = c as u8 - b'0';
                if let Some(tf) = Timeframe::from_key(n)
                    && tf != self.timeframe
                {
                    self.timeframe = tf;
                    self.needs_data = true;
                    self.loading = true;
                }
            }
            _ => {}
        }
    }

    /// Render the chart view.
    pub fn render(&self, frame: &mut Frame, theme: &CliTheme) {
        let area = frame.area();

        if self.loading {
            let msg = Paragraph::new(format!(
                "Loading {} data for {}…",
                self.timeframe.label(),
                self.symbol
            ))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} ", self.symbol)),
            );
            frame.render_widget(msg, area);
            return;
        }

        if let Some(ref err) = self.error {
            let msg = Paragraph::new(format!("Error: {err}"))
                .style(Style::default().fg(theme.negative))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {} ", self.symbol)),
                );
            frame.render_widget(msg, area);
            return;
        }

        if self.bars.is_empty() {
            let msg = Paragraph::new("No data available").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!(" {} ", self.symbol)),
            );
            frame.render_widget(msg, area);
            return;
        }

        // Layout: header + chart + (optional volume) + footer
        let mut constraints = vec![
            Constraint::Length(1), // header
        ];
        if self.show_volume {
            constraints.push(Constraint::Percentage(65)); // price chart
            constraints.push(Constraint::Percentage(25)); // volume
        } else {
            constraints.push(Constraint::Min(10)); // price chart
        }
        constraints.push(Constraint::Length(1)); // status bar

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        self.render_chart_header(frame, chunks[0], theme);

        let chart_idx = 1;
        match self.chart_type {
            ChartType::Line => self.render_line_chart(frame, chunks[chart_idx], theme),
            ChartType::Candlestick => self.render_candlestick(frame, chunks[chart_idx], theme),
        }

        if self.show_volume {
            self.render_volume_bars(frame, chunks[chart_idx + 1], theme);
        }

        self.render_chart_footer(frame, *chunks.last().unwrap(), theme);
    }

    fn render_chart_header(&self, frame: &mut Frame, area: Rect, theme: &CliTheme) {
        let visible = self.visible_bars();
        let (latest_price, change_str) = if let Some(last) = visible.last() {
            let first_close = visible.first().map_or(last.close, |b| b.close);
            let change = last.close - first_close;
            let pct = if first_close.is_zero() {
                Decimal::ZERO
            } else {
                (change / first_close) * Decimal::new(100, 0)
            };
            let color = if change >= Decimal::ZERO { "+" } else { "" };
            (
                format_price(last.close),
                format!(
                    "{color}{:.2} ({color}{:.2}%)",
                    change.to_f64().unwrap_or(0.0),
                    pct.to_f64().unwrap_or(0.0)
                ),
            )
        } else {
            ("—".to_string(), String::new())
        };

        let tf_indicators: Vec<Span> = Timeframe::ALL
            .iter()
            .map(|tf| {
                if *tf == self.timeframe {
                    Span::styled(
                        format!(" {} ", tf.label()),
                        Style::default().bold().fg(theme.accent),
                    )
                } else {
                    Span::raw(format!(" {} ", tf.label()))
                }
            })
            .collect();

        let chart_label = match self.chart_type {
            ChartType::Line => "Line",
            ChartType::Candlestick => "Candle",
        };

        let mut spans = vec![
            Span::styled(format!(" {} ", self.symbol), Style::default().bold()),
            Span::raw(latest_price),
            Span::raw(format!(" {change_str} ")),
            Span::raw(" │ "),
        ];
        spans.extend(tf_indicators);
        spans.push(Span::raw(format!(" │ {chart_label}")));

        if self.show_ma {
            spans.push(Span::styled(" MA", Style::default().fg(theme.chart_ma)));
        }

        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    #[allow(clippy::cast_precision_loss)]
    fn render_line_chart(&self, frame: &mut Frame, area: Rect, theme: &CliTheme) {
        let visible = self.visible_bars();
        if visible.is_empty() {
            return;
        }

        let close_data: Vec<(f64, f64)> = visible
            .iter()
            .enumerate()
            .map(|(i, bar)| (i as f64, bar.close.to_f64().unwrap_or(0.0)))
            .collect();

        let min_y = close_data
            .iter()
            .map(|(_, y)| *y)
            .fold(f64::INFINITY, f64::min);
        let max_y = close_data
            .iter()
            .map(|(_, y)| *y)
            .fold(f64::NEG_INFINITY, f64::max);
        let y_margin = (max_y - min_y) * 0.05;

        let mut datasets = vec![
            Dataset::default()
                .name("Close")
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(theme.chart_line))
                .data(&close_data),
        ];

        // Moving average
        let ma_data: Vec<(f64, f64)>;
        if self.show_ma && visible.len() > self.ma_period {
            ma_data = compute_ma(visible, self.ma_period);
            datasets.push(
                Dataset::default()
                    .name(format!("MA{}", self.ma_period))
                    .marker(symbols::Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(Style::default().fg(theme.chart_ma))
                    .data(&ma_data),
            );
        }

        let first_date = visible.first().map_or("", |b| b.date.as_str());
        let last_date = visible.last().map_or("", |b| b.date.as_str());

        let chart = Chart::new(datasets)
            .block(Block::default().borders(Borders::ALL))
            .x_axis(
                Axis::default()
                    .bounds([0.0, (visible.len().max(1) - 1) as f64])
                    .labels(vec![Line::from(first_date), Line::from(last_date)]),
            )
            .y_axis(
                Axis::default()
                    .bounds([min_y - y_margin, max_y + y_margin])
                    .labels(vec![
                        Line::from(format!("{min_y:.2}")),
                        Line::from(format!("{:.2}", f64::midpoint(min_y, max_y))),
                        Line::from(format!("{max_y:.2}")),
                    ]),
            );

        frame.render_widget(chart, area);
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    fn render_candlestick(&self, frame: &mut Frame, area: Rect, theme: &CliTheme) {
        let visible = self.visible_bars();
        if visible.is_empty() {
            return;
        }

        // Custom candlestick rendering using block characters
        // Each candle is rendered in a column
        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width < 2 || inner.height < 3 {
            return;
        }

        let min_price = visible
            .iter()
            .map(|b| b.low.to_f64().unwrap_or(0.0))
            .fold(f64::INFINITY, f64::min);
        let max_price = visible
            .iter()
            .map(|b| b.high.to_f64().unwrap_or(0.0))
            .fold(f64::NEG_INFINITY, f64::max);
        let price_range = if (max_price - min_price).abs() < f64::EPSILON {
            1.0
        } else {
            max_price - min_price
        };

        // How many candles can we fit
        let candle_width = 3u16; // body + gap
        let max_candles = (inner.width / candle_width) as usize;
        let display_bars = if visible.len() > max_candles {
            &visible[visible.len() - max_candles..]
        } else {
            visible
        };

        let height = f64::from(inner.height);

        for (i, bar) in display_bars.iter().enumerate() {
            let x = inner.x + (i as u16) * candle_width;
            if x >= inner.x + inner.width {
                break;
            }

            let open = bar.open.to_f64().unwrap_or(0.0);
            let close = bar.close.to_f64().unwrap_or(0.0);
            let high = bar.high.to_f64().unwrap_or(0.0);
            let low = bar.low.to_f64().unwrap_or(0.0);

            let is_up = close >= open;
            let color = if is_up {
                theme.positive
            } else {
                theme.negative
            };

            // Map price to y coordinate (inverted: top = max price)
            let to_y = |price: f64| -> u16 {
                let ratio = (max_price - price) / price_range;
                let y = (ratio * (height - 1.0)).round() as u16;
                inner.y + y.min(inner.height - 1)
            };

            let high_y = to_y(high);
            let low_y = to_y(low);
            let body_top = to_y(open.max(close));
            let body_bottom = to_y(open.min(close));

            // Draw wick (high to body top)
            for y in high_y..body_top {
                let buf = frame.buffer_mut();
                if x + 1 < inner.x + inner.width {
                    buf[(x + 1, y)].set_char('│').set_fg(color);
                }
            }

            // Draw body
            for y in body_top..=body_bottom {
                let buf = frame.buffer_mut();
                let ch = if is_up { '▓' } else { '▒' };
                buf[(x, y)].set_char(ch).set_fg(color);
                if x + 1 < inner.x + inner.width {
                    buf[(x + 1, y)].set_char(ch).set_fg(color);
                }
            }

            // Draw wick (body bottom to low)
            for y in (body_bottom + 1)..=low_y {
                let buf = frame.buffer_mut();
                if x + 1 < inner.x + inner.width {
                    buf[(x + 1, y)].set_char('│').set_fg(color);
                }
            }
        }

        // Y-axis labels
        let buf = frame.buffer_mut();
        let top_label = format!("{max_price:.1}");
        let bot_label = format!("{min_price:.1}");
        for (i, ch) in top_label.chars().enumerate() {
            let lx = inner.x + inner.width.saturating_sub(top_label.len() as u16) + i as u16;
            if lx < inner.x + inner.width {
                buf[(lx, inner.y)].set_char(ch).set_fg(theme.muted);
            }
        }
        for (i, ch) in bot_label.chars().enumerate() {
            let lx = inner.x + inner.width.saturating_sub(bot_label.len() as u16) + i as u16;
            let ly = inner.y + inner.height - 1;
            if lx < inner.x + inner.width {
                buf[(lx, ly)].set_char(ch).set_fg(theme.muted);
            }
        }
    }

    fn render_volume_bars(&self, frame: &mut Frame, area: Rect, theme: &CliTheme) {
        let visible = self.visible_bars();
        if visible.is_empty() {
            return;
        }

        let bars: Vec<Bar> = visible
            .iter()
            .map(|bar| {
                let is_up = bar.close >= bar.open;
                let color = if is_up {
                    theme.positive
                } else {
                    theme.negative
                };
                Bar::default()
                    .value(bar.volume)
                    .style(Style::default().fg(color))
            })
            .collect();

        let bar_chart = BarChart::default()
            .block(Block::default().borders(Borders::ALL).title("Volume"))
            .data(BarGroup::default().bars(&bars))
            .bar_width(1)
            .bar_gap(0);

        frame.render_widget(bar_chart, area);
    }

    #[allow(clippy::unused_self)]
    fn render_chart_footer(&self, frame: &mut Frame, area: Rect, theme: &CliTheme) {
        let help = "q:Back  1-7:Timeframe  l:Line  c:Candle  v:Volume  m:MA  +/-:Zoom  ←→:Scroll";
        let bar = Paragraph::new(Line::from(Span::styled(
            format!(" {help}"),
            Style::default().fg(theme.muted),
        )));
        frame.render_widget(bar, area);
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn format_price(d: Decimal) -> String {
    format!("{:.2}", d.to_f64().unwrap_or(0.0))
}

/// Compute simple moving average data points for chart overlay.
#[allow(clippy::cast_precision_loss)]
fn compute_ma(bars: &[Ohlcv], period: usize) -> Vec<(f64, f64)> {
    let mut result = Vec::new();
    let mut window: VecDeque<f64> = VecDeque::with_capacity(period);

    for (i, bar) in bars.iter().enumerate() {
        let close = bar.close.to_f64().unwrap_or(0.0);
        window.push_back(close);
        if window.len() > period {
            window.pop_front();
        }
        if window.len() == period {
            let avg: f64 = window.iter().sum::<f64>() / period as f64;
            result.push((i as f64, avg));
        }
    }

    result
}
