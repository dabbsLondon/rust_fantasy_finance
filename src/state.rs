use std::sync::Arc;

use crate::holdings::HoldingStore;
use crate::market::MarketData;
use crate::portfolio::HoldingsService;
use crate::activity::ActivityStore;

#[derive(Clone)]
pub struct AppState {
    pub store: HoldingStore,
    pub market: Arc<MarketData>,
    pub holdings: HoldingsService,
    pub activities: ActivityStore,
}
