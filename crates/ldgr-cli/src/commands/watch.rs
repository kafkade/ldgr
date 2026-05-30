//! `ldgr watch [symbols...]` — real-time watchlist TUI.
//!
//! Standalone mode: no vault required. Fetches quotes via Yahoo Finance
//! and displays them in a ratatui TUI with auto-refresh.

use std::io;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::DisableMouseCapture;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use ldgr_core::market::{QuoteProvider, YahooFinance};

use crate::tui::chart::ChartApp;
use crate::tui::event::{AppEvent, EventHandler};
use crate::tui::watchlist::WatchlistApp;

/// Terminal RAII guard — restores terminal state on drop.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = crossterm::execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
    }
}

/// Messages from background fetch tasks to the TUI.
enum FetchResult {
    Quotes(Vec<ldgr_core::market::Quote>),
    QuoteError(String, String),
    Historical(Vec<ldgr_core::market::Ohlcv>),
    HistoricalError(String),
}

/// Run the watchlist TUI.
pub fn run(symbols: Vec<String>, interval_secs: u64) -> Result<()> {
    if symbols.is_empty() {
        anyhow::bail!(
            "No symbols provided.\nUsage: ldgr watch AAPL MSFT GOOG\n\nAdd symbols to track their prices in real-time."
        );
    }

    let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
    rt.block_on(run_watchlist_async(symbols, interval_secs))
}

#[allow(clippy::unused_async, clippy::too_many_lines)]
async fn run_watchlist_async(symbols: Vec<String>, interval_secs: u64) -> Result<()> {
    // Set up terminal
    enable_raw_mode().context("failed to enable raw mode")?;
    let _guard = TerminalGuard;
    crossterm::execute!(io::stdout(), EnterAlternateScreen)
        .context("failed to enter alternate screen")?;

    // Install panic hook that restores terminal
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = crossterm::execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original_hook(info);
    }));

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).context("failed to create terminal")?;

    let tick_rate = Duration::from_secs(interval_secs);
    let event_handler = EventHandler::new(tick_rate);

    let mut app = WatchlistApp::new(symbols);
    let provider = YahooFinance;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("failed to create HTTP client")?;

    let (fetch_tx, mut fetch_rx) = mpsc::unbounded_channel::<FetchResult>();

    // Initial fetch
    spawn_quote_fetch(&client, &provider, &app.symbols, &fetch_tx);
    app.mark_loading();

    let mut chart_app: Option<ChartApp> = None;

    loop {
        // Draw
        if let Some(ref chart) = chart_app {
            terminal.draw(|f| chart.render(f))?;
        } else {
            terminal.draw(|f| app.render(f))?;
        }

        // Process fetch results (non-blocking)
        while let Ok(result) = fetch_rx.try_recv() {
            match result {
                FetchResult::Quotes(quotes) => app.update_quotes(quotes),
                FetchResult::QuoteError(sym, err) => app.set_error(&sym, err),
                FetchResult::Historical(bars) => {
                    if let Some(ref mut ca) = chart_app {
                        ca.update_data(bars);
                    }
                }
                FetchResult::HistoricalError(err) => {
                    if let Some(ref mut ca) = chart_app {
                        ca.set_error(err);
                    }
                }
            }
        }

        // Process events
        let event = event_handler.next()?;
        match event {
            AppEvent::Key(key) => {
                if let Some(ref mut ca) = chart_app {
                    ca.handle_key(key);
                    if ca.should_quit {
                        chart_app = None;
                        continue;
                    }
                    if ca.needs_data {
                        spawn_historical_fetch(
                            &client,
                            &provider,
                            &ca.symbol,
                            ca.timeframe,
                            &fetch_tx,
                        );
                    }
                } else {
                    // Check for Enter (open chart) before handling
                    if key.code == crossterm::event::KeyCode::Enter {
                        if let Some(sym) = app.selected_symbol().map(String::from) {
                            let ca = ChartApp::new(sym.clone());
                            spawn_historical_fetch(
                                &client,
                                &provider,
                                &sym,
                                ca.timeframe,
                                &fetch_tx,
                            );
                            chart_app = Some(ca);
                            continue;
                        }
                    }

                    let symbols_before: Vec<String> = app.symbols.clone();
                    app.handle_key(key);

                    if app.should_quit {
                        break;
                    }

                    // If new symbols were added, fetch them
                    let new_symbols: Vec<String> = app
                        .symbols
                        .iter()
                        .filter(|s| !symbols_before.contains(s))
                        .cloned()
                        .collect();
                    if !new_symbols.is_empty() {
                        spawn_quote_fetch(&client, &provider, &new_symbols, &fetch_tx);
                    }

                    // Manual refresh
                    if key.code == crossterm::event::KeyCode::Char('r') {
                        spawn_quote_fetch(&client, &provider, &app.symbols, &fetch_tx);
                        app.mark_loading();
                    }
                }
            }
            AppEvent::Tick => {
                if chart_app.is_none() {
                    // Auto-refresh quotes
                    spawn_quote_fetch(&client, &provider, &app.symbols, &fetch_tx);
                    app.mark_loading();
                    app.status_message = None;
                }
            }
            AppEvent::Resize(_, _) => {
                // Terminal will redraw on next iteration
            }
        }
    }

    Ok(())
}

/// Spawn async quote fetches for the given symbols.
fn spawn_quote_fetch(
    client: &reqwest::Client,
    provider: &YahooFinance,
    symbols: &[String],
    tx: &mpsc::UnboundedSender<FetchResult>,
) {
    for symbol in symbols {
        let url = provider.quote_url(&[symbol.as_str()]);
        if url.is_empty() {
            continue;
        }
        let client = client.clone();
        let tx = tx.clone();
        let sym = symbol.clone();
        let provider = YahooFinance;

        tokio::spawn(async move {
            match client.get(&url).send().await {
                Ok(resp) => match resp.text().await {
                    Ok(text) => match provider.parse_quotes(&text) {
                        Ok(quotes) => {
                            let _ = tx.send(FetchResult::Quotes(quotes));
                        }
                        Err(e) => {
                            let _ = tx.send(FetchResult::QuoteError(sym, e.to_string()));
                        }
                    },
                    Err(e) => {
                        let _ = tx.send(FetchResult::QuoteError(sym, e.to_string()));
                    }
                },
                Err(e) => {
                    let _ = tx.send(FetchResult::QuoteError(sym, e.to_string()));
                }
            }
        });
    }
}

/// Spawn an async historical data fetch.
fn spawn_historical_fetch(
    client: &reqwest::Client,
    provider: &YahooFinance,
    symbol: &str,
    timeframe: crate::tui::chart::Timeframe,
    tx: &mpsc::UnboundedSender<FetchResult>,
) {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let start_date = (chrono::Utc::now() - chrono::Duration::days(timeframe.days()))
        .format("%Y-%m-%d")
        .to_string();

    let range = ldgr_core::market::DateRange {
        start: start_date,
        end: today,
    };
    let url = provider.historical_url(symbol, &range);
    let client = client.clone();
    let tx = tx.clone();
    let provider = YahooFinance;

    tokio::spawn(async move {
        match client.get(&url).send().await {
            Ok(resp) => match resp.text().await {
                Ok(text) => match provider.parse_historical(&text) {
                    Ok(bars) => {
                        let _ = tx.send(FetchResult::Historical(bars));
                    }
                    Err(e) => {
                        let _ = tx.send(FetchResult::HistoricalError(e.to_string()));
                    }
                },
                Err(e) => {
                    let _ = tx.send(FetchResult::HistoricalError(e.to_string()));
                }
            },
            Err(e) => {
                let _ = tx.send(FetchResult::HistoricalError(e.to_string()));
            }
        }
    });
}
