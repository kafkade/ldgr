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

use ldgr_core::market::cache::{HISTORICAL_TTL, QUOTE_TTL};
use ldgr_core::market::{MarketCache, PersistentCache, QuoteProvider, YahooFinance};

use crate::market_fetch::{self, ProxyConfig};
use crate::tui::chart::ChartApp;
use crate::tui::event::{AppEvent, EventHandler};
use crate::tui::watchlist::WatchlistApp;
use crate::{config, theme};

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
    /// Parsed quotes. `cache_key` is `Some` when the result came from the network
    /// and should be written through to the persistent cache.
    Quotes {
        quotes: Vec<ldgr_core::market::Quote>,
        cache_key: Option<String>,
    },
    QuoteError(String, String),
    /// Parsed historical bars. `cache_key` is `Some` for network results to persist.
    Historical {
        bars: Vec<ldgr_core::market::Ohlcv>,
        cache_key: Option<String>,
    },
    HistoricalError(String),
}

/// Build the per-symbol quote cache key (matches the in-memory cache convention).
fn quote_cache_key(symbol: &str) -> String {
    let mut syms = [symbol];
    MarketCache::quote_key("yahoo", &mut syms)
}

/// Run the watchlist TUI.
pub fn run(
    symbols: Vec<String>,
    interval_secs: u64,
    no_proxy: bool,
    vault_path: &std::path::Path,
) -> Result<()> {
    if symbols.is_empty() {
        anyhow::bail!(
            "No symbols provided.\nUsage: ldgr watch AAPL MSFT GOOG\n\nAdd symbols to track their prices in real-time."
        );
    }

    // The price cache is an optimization — if it can't be opened, carry on without it.
    let cache = crate::commands::cache::open_cache(vault_path).ok();
    let proxy = ProxyConfig::resolve(no_proxy);

    let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
    rt.block_on(run_watchlist_async(symbols, interval_secs, cache, proxy))
}

#[allow(clippy::unused_async, clippy::too_many_lines)]
async fn run_watchlist_async(
    symbols: Vec<String>,
    interval_secs: u64,
    mut cache: Option<PersistentCache>,
    proxy: ProxyConfig,
) -> Result<()> {
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

    // Load theme from config (supports live reload via mtime check)
    let cfg = config::load_config();
    let mut current_theme = theme::resolve_theme(&cfg);
    let mut last_config_mtime = config::config_mtime();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("failed to create HTTP client")?;

    let (fetch_tx, mut fetch_rx) = mpsc::unbounded_channel::<FetchResult>();

    // Initial fetch
    dispatch_quote_fetch(
        cache.as_mut(),
        &client,
        &provider,
        &proxy,
        &app.symbols,
        &fetch_tx,
    );
    app.mark_loading();

    let mut chart_app: Option<ChartApp> = None;

    loop {
        // Draw
        if let Some(ref chart) = chart_app {
            terminal.draw(|f| chart.render(f, &current_theme))?;
        } else {
            terminal.draw(|f| app.render(f, &current_theme))?;
        }

        // Process fetch results (non-blocking)
        while let Ok(result) = fetch_rx.try_recv() {
            match result {
                FetchResult::Quotes { quotes, cache_key } => {
                    if let (Some(c), Some(key)) = (cache.as_mut(), cache_key)
                        && let Ok(json) = serde_json::to_string(&quotes)
                    {
                        let now = chrono::Utc::now().timestamp();
                        let _ = c.set(key, json, QUOTE_TTL, now);
                    }
                    app.update_quotes(quotes);
                }
                FetchResult::QuoteError(sym, err) => app.set_error(&sym, err),
                FetchResult::Historical { bars, cache_key } => {
                    if let (Some(c), Some(key)) = (cache.as_mut(), cache_key)
                        && let Ok(json) = serde_json::to_string(&bars)
                    {
                        let now = chrono::Utc::now().timestamp();
                        let _ = c.set(key, json, HISTORICAL_TTL, now);
                    }
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
                        dispatch_historical_fetch(
                            cache.as_mut(),
                            &client,
                            &provider,
                            &proxy,
                            &ca.symbol,
                            ca.timeframe,
                            &fetch_tx,
                        );
                    }
                } else {
                    // Check for Enter (open chart) before handling
                    if key.code == crossterm::event::KeyCode::Enter
                        && let Some(sym) = app.selected_symbol().map(String::from)
                    {
                        let ca = ChartApp::new(sym.clone());
                        dispatch_historical_fetch(
                            cache.as_mut(),
                            &client,
                            &provider,
                            &proxy,
                            &sym,
                            ca.timeframe,
                            &fetch_tx,
                        );
                        chart_app = Some(ca);
                        continue;
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
                        dispatch_quote_fetch(
                            cache.as_mut(),
                            &client,
                            &provider,
                            &proxy,
                            &new_symbols,
                            &fetch_tx,
                        );
                    }

                    // Manual refresh
                    if key.code == crossterm::event::KeyCode::Char('r') {
                        dispatch_quote_fetch(
                            cache.as_mut(),
                            &client,
                            &provider,
                            &proxy,
                            &app.symbols,
                            &fetch_tx,
                        );
                        app.mark_loading();
                    }
                }
            }
            AppEvent::Tick => {
                // Reload theme if config file changed
                let mtime = config::config_mtime();
                if mtime != last_config_mtime {
                    let cfg = config::load_config();
                    current_theme = theme::resolve_theme(&cfg);
                    last_config_mtime = mtime;
                }

                if chart_app.is_none() {
                    // Auto-refresh quotes
                    dispatch_quote_fetch(
                        cache.as_mut(),
                        &client,
                        &provider,
                        &proxy,
                        &app.symbols,
                        &fetch_tx,
                    );
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

/// Serve fresh cached quotes where possible, fetching the rest from the network.
///
/// Cache hits are delivered immediately with no network request; misses spawn an
/// async fetch that writes its result back through the cache on arrival.
fn dispatch_quote_fetch(
    mut cache: Option<&mut PersistentCache>,
    client: &reqwest::Client,
    provider: &YahooFinance,
    proxy: &ProxyConfig,
    symbols: &[String],
    tx: &mpsc::UnboundedSender<FetchResult>,
) {
    let now = chrono::Utc::now().timestamp();

    for symbol in symbols {
        let key = quote_cache_key(symbol);

        // Cache hit within TTL → serve locally, no network request.
        if let Some(c) = cache.as_deref_mut()
            && let Ok(Some(data)) = c.get(&key, now)
            && let Ok(quotes) = serde_json::from_str::<Vec<ldgr_core::market::Quote>>(&data)
        {
            let _ = tx.send(FetchResult::Quotes {
                quotes,
                cache_key: None,
            });
            continue;
        }

        let direct_url = provider.quote_url(&[symbol.as_str()]);
        if direct_url.is_empty() {
            continue;
        }
        let proxy_url = proxy.quote_url(symbol);
        let client = client.clone();
        let tx = tx.clone();
        let sym = symbol.clone();
        let provider = YahooFinance;

        // Fetch via proxy first, falling back to the direct provider URL.
        tokio::spawn(async move {
            match market_fetch::fetch_text(&client, proxy_url, &direct_url).await {
                Ok(text) => match provider.parse_quotes(&text) {
                    Ok(quotes) => {
                        let _ = tx.send(FetchResult::Quotes {
                            quotes,
                            cache_key: Some(key),
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(FetchResult::QuoteError(sym, e.to_string()));
                    }
                },
                Err(e) => {
                    let _ = tx.send(FetchResult::QuoteError(sym, e));
                }
            }
        });
    }
}

/// Serve fresh cached historical bars if available, otherwise fetch from network.
fn dispatch_historical_fetch(
    cache: Option<&mut PersistentCache>,
    client: &reqwest::Client,
    provider: &YahooFinance,
    proxy: &ProxyConfig,
    symbol: &str,
    timeframe: crate::tui::chart::Timeframe,
    tx: &mpsc::UnboundedSender<FetchResult>,
) {
    let now = chrono::Utc::now().timestamp();
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let start_date = (chrono::Utc::now() - chrono::Duration::days(timeframe.days()))
        .format("%Y-%m-%d")
        .to_string();

    let key = MarketCache::historical_key("yahoo", symbol, &start_date, &today);

    if let Some(c) = cache
        && let Ok(Some(data)) = c.get(&key, now)
        && let Ok(bars) = serde_json::from_str::<Vec<ldgr_core::market::Ohlcv>>(&data)
    {
        let _ = tx.send(FetchResult::Historical {
            bars,
            cache_key: None,
        });
        return;
    }

    let range = ldgr_core::market::DateRange {
        start: start_date,
        end: today,
    };
    let direct_url = provider.historical_url(symbol, &range);
    let proxy_url = proxy.historical_url(symbol, &range);
    let client = client.clone();
    let tx = tx.clone();
    let provider = YahooFinance;

    // Fetch via proxy first, falling back to the direct provider URL.
    tokio::spawn(async move {
        match market_fetch::fetch_text(&client, proxy_url, &direct_url).await {
            Ok(text) => match provider.parse_historical(&text) {
                Ok(bars) => {
                    let _ = tx.send(FetchResult::Historical {
                        bars,
                        cache_key: Some(key),
                    });
                }
                Err(e) => {
                    let _ = tx.send(FetchResult::HistoricalError(e.to_string()));
                }
            },
            Err(e) => {
                let _ = tx.send(FetchResult::HistoricalError(e));
            }
        }
    });
}
