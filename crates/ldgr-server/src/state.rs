use std::sync::Arc;

use crate::auth::srp::SrpHandshakeStore;
use crate::config::Config;
use crate::storage::ServerDb;

pub struct AppState {
    pub db: ServerDb,
    pub srp_handshakes: SrpHandshakeStore,
    pub config: Config,
}

pub type SharedState = Arc<AppState>;
