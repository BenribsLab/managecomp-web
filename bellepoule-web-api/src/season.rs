//! Classement de saison du club : cumul des points de ses compétitions
//! (celles dont les points sont activés), par circuit (catégories, armes,
//! genres, compétitions choisies), classement général de plusieurs
//! circuits, avec les corrections à la main (`clients/<club>/saison.json`).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use bp_core::season::{self, Circuit, SeasonConfig, SeasonContest};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::{bad_request, internal, not_found, ApiError, Shared};
use crate::auth::Client;
use crate::store::Store;

fn config_path(store: &Store, client: &Client) -> std::path::PathBuf {
    store.dir.join("clients").join(&client.id).join("saison.json")
}

fn load_config(store: &Store, client: &Client) -> SeasonConfig {
    std::fs::read_to_string(config_path(store, client)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_config(store: &Store, client: &Client, o: &SeasonConfig) -> Result<(), ApiError> {
    let path = config_path(store, client);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(internal)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(o).unwrap_or_default()).map_err(internal)
}

#[derive(Deserialize)]
pub struct SeasonQuery {
    /// Circuit affiché (identifiant).
    #[serde(default)]
    circuit: Option<String>,
    /// Sans circuit : seulement les compétitions de cette catégorie (code).
    #[serde(default)]
    category: Option<String>,
}

fn contest_json(c: &SeasonContest) -> Value {
    json!({ "id": c.id, "name": c.name, "date": c.date, "category": c.category, "weapon": c.weapon, "gender": c.gender, "participants": c.participants })
}

/// `{ circuits, categories, weapons, genders, all, contests, rows, corrections, best }`.
pub async fn get(State(store): State<Shared>, client: Client, Query(q): Query<SeasonQuery>) -> Json<Value> {
    let store = store.lock().await;
    let mut all: Vec<SeasonContest> = store.list(&client).filter_map(|(id, c)| SeasonContest::from_contest(id, c)).collect();
    all.sort_by(|a, b| date_key(&a.date).cmp(&date_key(&b.date)).then(a.name.cmp(&b.name)));
    let config = load_config(&store, &client);
    let circuit = q.circuit.filter(|c| config.circuit(c).is_some());
    let (contests, rows, best) = match &circuit {
        Some(id) => {
            let (contests, rows) = config.ranking(&all, id);
            (contests, rows, config.circuit(id).and_then(|c| c.best))
        }
        None => {
            let filter = q.category.filter(|c| !c.is_empty());
            let contests: Vec<SeasonContest> = all.iter().filter(|c| filter.as_ref().is_none_or(|f| c.category == *f)).cloned().collect();
            let rows = season::ranking(&contests, &config.overrides);
            (contests, rows, None)
        }
    };
    Json(json!({
        "circuits": config.circuits,
        "categories": season::distinct(&all, |c| &c.category),
        "weapons": season::distinct(&all, |c| &c.weapon),
        "genders": season::distinct(&all, |c| &c.gender),
        "all": all.iter().map(contest_json).collect::<Vec<_>>(),
        "contests": contests.iter().map(contest_json).collect::<Vec<_>>(),
        "rows": rows,
        "best": best,
        "corrections": config.overrides.merges.len() + config.overrides.splits.len(),
    }))
}

#[derive(Deserialize)]
pub struct Correction {
    /// `merge` (`keys` : la première garde les autres), `split` (`first_name`), `reset`.
    action: String,
    #[serde(default)]
    keys: Vec<String>,
    #[serde(default)]
    first_name: String,
}

pub async fn correct(State(store): State<Shared>, client: Client, Json(c): Json<Correction>) -> Result<StatusCode, ApiError> {
    let store = store.lock().await;
    let mut config = load_config(&store, &client);
    let o = &mut config.overrides;
    match c.action.as_str() {
        "merge" if c.keys.len() >= 2 => o.merge(&c.keys[0], &c.keys[1..]),
        "split" if !c.first_name.trim().is_empty() => o.split(&c.first_name),
        "reset" => *o = Default::default(),
        _ => return Err(bad_request("correction invalide")),
    }
    save_config(&store, &client, &config)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Crée ou modifie un circuit : `{ id }`.
pub async fn save_circuit(State(store): State<Shared>, client: Client, Json(mut c): Json<Circuit>) -> Result<Json<Value>, ApiError> {
    c.name = c.name.trim().chars().take(80).collect();
    if c.name.is_empty() {
        return Err(bad_request("nom du circuit manquant"));
    }
    let store = store.lock().await;
    let mut config = load_config(&store, &client);
    let id = config.upsert(c);
    save_config(&store, &client, &config)?;
    Ok(Json(json!({ "id": id })))
}

pub async fn delete_circuit(State(store): State<Shared>, client: Client, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let store = store.lock().await;
    let mut config = load_config(&store, &client);
    if config.circuit(&id).is_none() {
        return Err(not_found());
    }
    config.remove(&id);
    save_config(&store, &client, &config)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Date « jj/mm/aaaa » comparable.
fn date_key(date: &str) -> String {
    let parts: Vec<&str> = date.split(['/', '.', '-']).collect();
    match parts[..] {
        [d, m, y] if y.len() == 4 => format!("{y}{m:0>2}{d:0>2}"),
        _ => date.to_string(),
    }
}
