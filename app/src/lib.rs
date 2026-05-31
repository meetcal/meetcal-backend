pub mod error;
pub mod query_convex;
pub mod routes;
pub mod sort;

use convex::ConvexClient;
pub use error::AppError;

#[derive(Clone)]
pub struct AppState {
    pub convex: ConvexClient,
}
