use std::sync::Arc;

use crate::holdings::HoldingStore;
use crate::market::MarketData;

#[derive(Clone)]
pub struct AppState {
    pub store: HoldingStore,
    pub market: Arc<MarketData>,
}
