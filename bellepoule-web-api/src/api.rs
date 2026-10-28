//! Routes HTTP de l'API (JSON).

use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use tokio::sync::Mutex;

use crate::store::Store;
use crate::{accounts, auth, categories, checkin, compete, public, contests, formulas, organisation, schedule, season, smartpoule, stages};

pub type Shared = Arc<Mutex<Store>>;

pub fn router(store: Shared) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/meta", get(contests::meta))
        // Clé d'API : client courant ; administration des clients
        .route("/api/me", get(auth::me))
        .route("/api/admin/clients", get(auth::list_clients).post(auth::create_client))
        .route("/api/admin/clients/{id}", delete(auth::remove_client))
        .route("/api/admin/clients/{id}/key", post(auth::renew_key))
        // Comptes : inscription, connexion, clés du club ; administration
        .route("/api/auth/register", post(accounts::register))
        .route("/api/auth/login", post(accounts::login))
        .route("/api/auth/logout", post(accounts::logout))
        .route("/api/auth/me", get(accounts::me))
        .route("/api/auth/password", put(accounts::change_password))
        .route("/api/account/keys", get(accounts::list_keys).post(accounts::create_key))
        .route("/api/account/keys/{id}", delete(accounts::remove_key))
        .route("/api/admin/users", get(accounts::list_users))
        .route("/api/admin/users/{id}", put(accounts::update_user).delete(accounts::remove_user))
        .route("/api/admin/users/{id}/approve", post(accounts::approve))
        .route("/api/admin/stats", get(accounts::stats))
        // Compétitions
        .route("/api/contests", get(contests::list).post(contests::create))
        .route("/api/contests/import", post(contests::import))
        .route("/api/contests/{id}", get(contests::get_one).delete(contests::remove))
        .route("/api/contests/{id}/hidden", put(contests::set_hidden))
        // Pages publiques des épreuves (compete.<domaine>, /compete/ en local)
        .route("/compete/", get(compete::home))
        .route("/compete/style.css", get(compete::style))
        .route("/compete/app.js", get(compete::script))
        .route("/compete/e/{id}", get(compete::short))
        .route("/compete/{club}", get(compete::club))
        .route("/compete/{club}/{slug}", get(compete::event))
        // Compétitions ouvertes (onglets) : ce sont elles qui sont organisées.
        .route("/api/open-contests", get(organisation::open_tabs))
        .route("/api/contests/{id}/open", post(organisation::open_contest))
        .route("/api/contests/{id}/close", post(organisation::close_contest))
        .route("/api/contests/{id}/publication", get(public::get_contest).post(public::publish_contest).delete(public::unpublish_contest))
        // Publication par le logiciel de bureau (clé d'API du club)
        .route("/api/publication", put(public::publish_desktop))
        .route("/api/publication/event", put(public::event_desktop))
        .route("/api/publication/{id}", get(public::get_desktop).delete(public::unpublish_desktop))
        .route("/api/contests/{id}/properties", get(contests::get_properties).put(contests::set_properties))
        .route("/api/contests/{id}/cotcot", get(contests::download))
        .route("/api/contests/{id}/export/{format}", get(contests::export))
        // Formule et navigation
        .route("/api/contests/{id}/schedule", get(schedule::get_schedule))
        .route("/api/contests/{id}/schedule/stages", post(schedule::add_stage))
        .route("/api/contests/{id}/schedule/stages/{index}", delete(schedule::remove_stage))
        .route("/api/contests/{id}/schedule/stages/{index}/move", post(schedule::move_stage))
        .route("/api/contests/{id}/schedule/stages/{index}/name", post(schedule::rename_stage))
        .route("/api/contests/{id}/next", post(schedule::next))
        .route("/api/contests/{id}/previous", post(schedule::previous))
        // Phases
        .route("/api/contests/{id}/stages/{index}", get(stages::get_stage).post(stages::stage_action))
        .route("/api/contests/{id}/stages/{index}/group_sheets", get(stages::group_sheets))
        .route("/api/contests/{id}/stages/{index}/match_sheets", get(stages::match_sheets))
        .route("/api/contests/{id}/stages/{index}/relay_sheets", get(stages::relay_sheets))
        // Catégories générales du club
        .route("/api/categories", get(categories::get).put(categories::save).delete(categories::remove))
        .route("/api/categories/active", post(categories::activate))
        // Formules de points du club
        .route("/api/points/formulas", get(formulas::list).post(formulas::save).delete(formulas::remove))
        // Classement de saison du club
        .route("/api/season", get(season::get))
        .route("/api/season/corrections", post(season::correct))
        // Organisation (pistes, arbitres, planning)
        .route("/api/organisation", get(organisation::get).post(organisation::action))
        .route("/api/organisation/import", post(organisation::import))
        .route("/api/organisation/referee-codes", get(organisation::referee_codes).post(organisation::renew_referee_code))
        // SmartPoule : relais BellePoule ↔ téléphones des arbitres
        .route("/api/smartpoule/state", put(smartpoule::put_state))
        .route("/api/smartpoule/pending", get(smartpoule::pending))
        .route("/api/smartpoule/results", post(smartpoule::results))
        .route("/api/smartpoule/admin", get(smartpoule::admin_code))
        .route("/api/smartpoule/phone/jobs", get(smartpoule::phone_jobs))
        .route("/api/smartpoule/phone/sheet", get(smartpoule::phone_sheet))
        .route("/api/smartpoule/phone/score", post(smartpoule::phone_score))
        .route("/api/smartpoule/phone/result", get(smartpoule::phone_result))
        .route("/api/smartpoule/phone/live", post(smartpoule::phone_live))
        .route("/api/smartpoule/phone/pin", post(smartpoule::phone_pin))
        .route("/api/smartpoule/phone/equipment", post(smartpoule::phone_equipment))
        .route("/api/smartpoule/phone/absence", post(smartpoule::phone_absence))
        .route("/api/smartpoule/phone/call", post(smartpoule::phone_call))
        .route("/api/smartpoule/security", get(smartpoule::security).post(smartpoule::security_action))
        .route("/api/live", get(smartpoule::live))
        // Appel
        .route("/api/contests/{id}/checkin", get(checkin::get_checkin))
        .route("/api/contests/{id}/checkin/form", get(checkin::form))
        .route("/api/contests/{id}/checkin/choices/{code}", get(checkin::choices))
        .route("/api/contests/{id}/checkin/fencers", post(checkin::add_fencer))
        .route("/api/contests/{id}/checkin/fencers/{reference}", get(checkin::get_fencer).put(checkin::update_fencer))
        .route("/api/contests/{id}/checkin/fencers/{reference}", delete(checkin::remove_fencer))
        .route("/api/contests/{id}/checkin/attending", put(checkin::set_attending))
        .route("/api/contests/{id}/checkin/import", post(checkin::import))
        .route("/api/contests/{id}/checkin/ranking", post(checkin::import_ranking))
        .route("/api/csv-template", get(contests::csv_template))
        .with_state(store)
}

/// Erreur renvoyée en JSON : `{ "error": "…" }`.
#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

pub fn not_found() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "compétition introuvable".into())
}

pub fn bad_request(message: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, message.into())
}

pub fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}
