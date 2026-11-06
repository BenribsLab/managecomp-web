//! Classement de saison du club : cumul des points de ses compétitions
//! (celles dont les points sont activés), par circuit (catégories, armes,
//! genres, compétitions choisies), classement général de plusieurs
//! circuits, avec les corrections à la main (`clients/<club>/saison.json`).

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use bp_core::season::{self, Circuit, SeasonConfig, SeasonContest};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::{bad_request, internal, not_found, ApiError, Shared};
use crate::auth::Client;
use crate::store::Store;

fn config_path(store: &Store, client: &Client) -> std::path::PathBuf {
    store.dir.join("clients").join(&client.id).join("saison.json")
}

pub(crate) fn load_config(store: &Store, client: &Client) -> SeasonConfig {
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

// ------------------------------------------------- publication du classement

/// Publication du classement club sur compete.<domaine>, indépendante de
/// celle des compétitions : le classement compte toutes les compétitions du
/// club dont les points sont activés, publiées ou non
/// (`clients/<club>/classement-public.json`).
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RankingPublication {
    pub published: bool,
    /// Adresse du club sur le site public.
    pub slug: String,
    /// `full` : noms complets ; `short` : prénom et initiale du nom.
    pub names: String,
    /// Circuits publiés (vide : tous, plus le classement général).
    pub circuits: Vec<String>,
    /// Liste des compétitions retenues affichée sous chaque classement.
    pub show_contests: bool,
    pub updated: u64,
}

fn publication_path(store: &Store, club: &str) -> std::path::PathBuf {
    store.dir.join("clients").join(club).join("classement-public.json")
}

pub(crate) fn load_publication(store: &Store, club: &str) -> RankingPublication {
    std::fs::read_to_string(publication_path(store, club)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Compétitions du club comptées dans son classement (points activés).
pub(crate) fn season_contests(store: &Store, client: &Client) -> Vec<SeasonContest> {
    let mut all: Vec<SeasonContest> = store.list(client).filter_map(|(id, c)| SeasonContest::from_contest(id, c)).collect();
    all.sort_by(|a, b| date_key(&a.date).cmp(&date_key(&b.date)).then(a.name.cmp(&b.name)));
    all
}

fn publication_json(headers: &HeaderMap, p: &RankingPublication) -> Value {
    let url = (!p.slug.is_empty()).then(|| format!("{}classement-club/{}", crate::public::public_base(headers), p.slug));
    json!({
        "published": p.published,
        "url": if p.published { url } else { None },
        "names": if p.names.is_empty() { "full" } else { p.names.as_str() },
        "circuits": p.circuits,
        "show_contests": p.show_contests,
        "updated": p.updated,
    })
}

/// `GET /api/season/publication` : état de la publication du classement.
pub async fn get_publication(State(store): State<Shared>, client: Client, headers: HeaderMap) -> Json<Value> {
    let store = store.lock().await;
    Json(publication_json(&headers, &load_publication(&store, &client.id)))
}

#[derive(Deserialize)]
pub struct PublicationInput {
    published: bool,
    #[serde(default)]
    names: Option<String>,
    #[serde(default)]
    circuits: Option<Vec<String>>,
    #[serde(default)]
    show_contests: Option<bool>,
}

/// `PUT /api/season/publication` `{ published, names?, circuits?, show_contests? }` :
/// publie (ou retire) le classement club du club.
pub async fn put_publication(State(store): State<Shared>, client: Client, headers: HeaderMap, Json(i): Json<PublicationInput>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let mut p = load_publication(&store, &client.id);
    p.published = i.published;
    if let Some(names) = i.names {
        if names != "full" && names != "short" {
            return Err(bad_request("names : full ou short"));
        }
        p.names = names;
    }
    if p.names.is_empty() {
        p.names = "full".into();
    }
    if let Some(circuits) = i.circuits {
        let config = load_config(&store, &client);
        if let Some(bad) = circuits.iter().find(|c| config.circuit(c).is_none()) {
            return Err(bad_request(format!("circuit inconnu : {bad}")));
        }
        p.circuits = circuits;
    }
    if let Some(show) = i.show_contests {
        p.show_contests = show;
    }
    if p.slug.is_empty() {
        p.slug = store.club_slug(&client);
    }
    p.updated = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let path = publication_path(&store, &client.id);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(internal)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(&p).unwrap_or_default()).map_err(internal)?;
    Ok(Json(publication_json(&headers, &p)))
}
