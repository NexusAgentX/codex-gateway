mod auth;
mod contracts;
mod error;
mod keys;
mod limits;
mod models;
mod observability;
mod settings;
mod upstreams;
mod users;

use axum::Router;

use crate::AppState;

pub use auth::{authenticate, authenticate_api_key, require_admin};
pub use error::ApiError;

pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(auth::router())
        .merge(users::router())
        .merge(keys::router())
        .merge(upstreams::router())
        .merge(models::router())
        .merge(observability::router())
        .merge(limits::router())
        .merge(settings::router())
        .with_state(state)
}
