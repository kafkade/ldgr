//! `ldgr portfolio` — portfolio view TUI with interactive charts.
//!
//! Reads investment holdings from the vault, fetches current market
//! prices, and displays a portfolio overview with gain/loss tracking.
//! Press Enter on a holding to view its interactive chart.

use std::collections::BTreeMap;
use std::io;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::DisableMouseCapture;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use rust_decimal::Decimal;
use tokio::sync::mpsc;

use ldgr_core::market::cache::{HISTORICAL_TTL, QUOTE_TTL};
use ldgr_core::market::{MarketCache, PersistentCache, QuoteProvider, YahooFinance};

use crate::market_fetch::{self, ProxyConfig};
use crate::tui::chart::ChartApp;
use crate::tui::event::{AppEvent, EventHandler};
use crate::tui::portfolio::{Holding, PortfolioApp, PortfolioMode};
use crate::{config, theme};

/// Terminal RAII guard.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = crossterm::execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
    }
}

enum FetchResult {
    /// Parsed quotes. `cache_key` is `Some` for network results to write through.
    Quotes {
        quotes: Vec<ldgr_core::market::Quote>,
        cache_key: Option<String>,
    },
    /// Parsed historical bars. `cache_key` is `Some` for network results to persist.
    Historical {
        bars: Vec<ldgr_core::market::Ohlcv>,
        cache_key: Option<String>,
    },
    HistoricalError(String),
    Error(String),
}

/// Build the per-symbol quote cache key (matches the in-memory cache convention).
fn quote_cache_key(symbol: &str) -> String {
    let mut syms = [symbol];
    MarketCache::quote_key("yahoo", &mut syms)
}

/// Run the portfolio TUI.
pub fn run(no_proxy: bool, vault_path: &std::path::Path) -> Result<()> {
    let holdings = load_holdings(vault_path)?;

    if holdings.is_empty() {
        anyhow::bail!(
            "No investment holdings found.\n\
             Track investments by using accounts like:\n\
             - Assets:Investments:Brokerage\n\
             - Assets:Retirement:401k\n\n\
             Holdings are detected from accounts containing \
             'invest', 'brokerage', or 'retirement'."
        );
    }

    let rt = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
    let cache = crate::commands::cache::open_cache(vault_path).ok();
    let proxy = ProxyConfig::resolve(no_proxy);
    rt.block_on(run_portfolio_async(holdings, cache, proxy))
}

/// Load investment holdings from the vault database.
fn load_holdings(vault_path: &std::path::Path) -> Result<Vec<Holding>> {
    let db = crate::db::require_unlocked_db(vault_path)?;

    // Load all transactions and convert to accounting types
    let store_txns = ldgr_core::storage::transactions::list_transactions(
        &db,
        &ldgr_core::storage::accounts::ListOptions::default(),
    )
    .context("failed to load transactions")?;
    let acct_txns = crate::convert::to_accounting_txns(&store_txns);

    // Compute balance sheet to find investment accounts
    let query = ldgr_core::accounting::Query::default();
    let sheet = ldgr_core::accounting::compute_balance_sheet(&acct_txns, &query);

    let mut holdings = Vec::new();

    for ab in &sheet.assets {
        let is_investment = ab.account.to_lowercase().contains("invest")
            || ab.account.to_lowercase().contains("brokerage")
            || ab.account.to_lowercase().contains("retirement");

        if !is_investment {
            continue;
        }

        for (commodity, qty) in &ab.balances {
            // Skip cash-like commodities (USD, EUR, etc.)
            if is_cash_commodity(commodity) {
                continue;
            }

            if qty.is_zero() {
                continue;
            }

            // Try to compute cost basis from lots or fallback to balance
            let cost_basis = compute_cost_basis(&db, &ab.account, commodity).unwrap_or(*qty); // Fallback: cost = current balance in commodity units

            holdings.push(Holding {
                symbol: commodity.clone(),
                shares: *qty,
                cost_basis,
                cost_commodity: "USD".to_string(), // Default; could be improved
                market_price: None,
                market_value: None,
                gain_loss: None,
                gain_loss_pct: None,
                allocation_pct: None,
            });
        }
    }

    // Merge duplicate symbols (same commodity across accounts)
    let mut merged: BTreeMap<String, Holding> = BTreeMap::new();
    for h in holdings {
        merged
            .entry(h.symbol.clone())
            .and_modify(|existing| {
                existing.shares += h.shares;
                existing.cost_basis += h.cost_basis;
            })
            .or_insert(h);
    }

    Ok(merged.into_values().collect())
}

/// Attempt to compute cost basis from lot tracking data.
fn compute_cost_basis(
    db: &rusqlite::Connection,
    account: &str,
    commodity: &str,
) -> Option<Decimal> {
    // Try to query lots table if it exists
    let mut stmt = db
        .prepare(
            "SELECT SUM(cost_basis) FROM lots \
             WHERE account_id = ?1 AND commodity = ?2 AND disposal_date IS NULL",
        )
        .ok()?;

    let result: Option<String> = stmt
        .query_row(rusqlite::params![account, commodity], |row| row.get(0))
        .ok()?;

    result?.parse::<Decimal>().ok()
}

/// Check if a commodity is a fiat currency (not a stock/crypto symbol).
fn is_cash_commodity(commodity: &str) -> bool {
    matches!(
        commodity.to_uppercase().as_str(),
        "USD" | "EUR" | "GBP" | "JPY" | "CHF" | "CAD" | "AUD" | "NZD" | "CNY" | "HKD" | "SGD"
    )
}

#[allow(clippy::unused_async, clippy::too_many_lines)]
async fn run_portfolio_async(
    holdings: Vec<Holding>,
    mut cache: Option<PersistentCache>,
    proxy: ProxyConfig,
) -> Result<()> {
    enable_raw_mode().context("failed to enable raw mode")?;
    let _guard = TerminalGuard;
    crossterm::execute!(io::stdout(), EnterAlternateScreen)
        .context("failed to enter alternate screen")?;

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = crossterm::execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original_hook(info);
    }));

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).context("failed to create terminal")?;

    let tick_rate = Duration::from_mins(1); // Refresh every 60s for portfolio
    let event_handler = EventHandler::new(tick_rate);

    let symbols: Vec<String> = holdings.iter().map(|h| h.symbol.clone()).collect();
    let mut app = PortfolioApp::new(holdings);

    // Load theme from config (supports live reload via mtime check)
    let cfg = config::load_config();
    let mut current_theme = theme::resolve_theme(&cfg);
    let mut last_config_mtime = config::config_mtime();

    let provider = YahooFinance;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("failed to create HTTP client")?;

    let (fetch_tx, mut fetch_rx) = mpsc::unbounded_channel::<FetchResult>();

    // Initial price fetch
    dispatch_portfolio_quotes(
        cache.as_mut(),
        &client,
        &provider,
        &proxy,
        &symbols,
        &fetch_tx,
    );

    let mut chart_app: Option<ChartApp> = None;

    loop {
        // Draw
        if let Some(ref chart) = chart_app {
            terminal.draw(|f| chart.render(f, &current_theme))?;
        } else {
            terminal.draw(|f| app.render(f, &current_theme))?;
        }

        // Process fetch results
        while let Ok(result) = fetch_rx.try_recv() {
            match result {
                FetchResult::Quotes { quotes, cache_key } => {
                    if let (Some(c), Some(key)) = (cache.as_mut(), cache_key)
                        && let Ok(json) = serde_json::to_string(&quotes)
                    {
                        let now = chrono::Utc::now().timestamp();
                        let _ = c.set(key, json, QUOTE_TTL, now);
                    }
                    app.update_quotes(&quotes);
                }
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
                FetchResult::Error(err) => app.set_error(err),
            }
        }

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
                    app.handle_key(key);

                    if app.should_quit {
                        break;
                    }

                    // Enter chart mode
                    if app.mode == PortfolioMode::Chart
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
                        app.mode = PortfolioMode::Normal;
                    }

                    // Manual refresh
                    if app.loading {
                        dispatch_portfolio_quotes(
                            cache.as_mut(),
                            &client,
                            &provider,
                            &proxy,
                            &symbols,
                            &fetch_tx,
                        );
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
                    dispatch_portfolio_quotes(
                        cache.as_mut(),
                        &client,
                        &provider,
                        &proxy,
                        &symbols,
                        &fetch_tx,
                    );
                }
            }
            AppEvent::Resize(_, _) => {}
        }
    }

    Ok(())
}

/// Serve fresh cached quotes where possible, fetching the rest from the network.
fn dispatch_portfolio_quotes(
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
                        let _ = tx.send(FetchResult::Error(format!("{sym}: {e}")));
                    }
                },
                Err(e) => {
                    let _ = tx.send(FetchResult::Error(format!("{sym}: {e}")));
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
