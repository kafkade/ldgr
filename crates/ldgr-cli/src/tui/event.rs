//! Terminal event handling with tick-based auto-refresh.
//!
//! Spawns a background thread to read crossterm events and emit periodic
//! ticks for data refresh. The TUI main loop never blocks on I/O.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as CrosstermEvent, KeyEvent};

/// Application events: user input or periodic tick.
#[derive(Debug)]
pub enum AppEvent {
    /// A keyboard event from the user.
    Key(KeyEvent),
    /// A periodic tick for data refresh.
    Tick,
    /// Terminal resize event.
    #[allow(dead_code)]
    Resize(u16, u16),
}

/// Event handler that polls crossterm events with periodic ticks.
pub struct EventHandler {
    rx: mpsc::Receiver<AppEvent>,
    _tx: mpsc::Sender<AppEvent>,
}

impl EventHandler {
    /// Create a new event handler with the given tick rate.
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::channel();
        let event_tx = tx.clone();

        std::thread::spawn(move || {
            let mut last_tick = Instant::now();
            loop {
                let timeout = tick_rate
                    .checked_sub(last_tick.elapsed())
                    .unwrap_or(Duration::ZERO);

                if event::poll(timeout).unwrap_or(false) {
                    match event::read() {
                        Ok(CrosstermEvent::Key(key)) => {
                            if event_tx.send(AppEvent::Key(key)).is_err() {
                                return;
                            }
                        }
                        Ok(CrosstermEvent::Resize(w, h))
                            if event_tx.send(AppEvent::Resize(w, h)).is_err() =>
                        {
                            return;
                        }
                        _ => {}
                    }
                }

                if last_tick.elapsed() >= tick_rate {
                    if event_tx.send(AppEvent::Tick).is_err() {
                        return;
                    }
                    last_tick = Instant::now();
                }
            }
        });

        Self { rx, _tx: tx }
    }

    /// Receive the next event (blocking).
    pub fn next(&self) -> Result<AppEvent, mpsc::RecvError> {
        self.rx.recv()
    }
}
