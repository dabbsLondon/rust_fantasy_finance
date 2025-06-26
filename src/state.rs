use crate::holdings::HoldingStore;
use crate::market::MarketData;
use crate::portfolio::HoldingsService;
use crate::strava::StravaFetcher;
use crate::activities::ActivityStore;

use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub store: HoldingStore,
    pub market: Arc<MarketData>,
    pub holdings: HoldingsService,
    pub strava: Arc<dyn StravaFetcher>,
    pub activities: ActivityStore,
}
