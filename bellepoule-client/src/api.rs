//! Appels à l'API (`/api/…`, relayée par le même domaine). Chaque requête
//! porte la clé d'API du client, gardée dans le navigateur.

use std::collections::BTreeMap;

use gloo_net::http::{Request, RequestBuilder, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const KEY: &str = "bellepoule_api_key";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

pub fn api_key() -> Option<String> {
    storage()?.get_item(KEY).ok()?.filter(|k| !k.is_empty())
}

pub fn set_api_key(key: Option<&str>) {
    if let Some(s) = storage() {
        let _ = match key {
            Some(k) => s.set_item(KEY, k),
            None => s.remove_item(KEY),
        };
    }
}

/// Erreur d'un appel : `unauthorized` quand la clé n'est pas (ou plus) valable.
#[derive(Clone, Debug, PartialEq)]
pub struct ApiError {
    pub message: String,
    pub unauthorized: bool,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

fn err(message: impl ToString) -> ApiError {
    ApiError { message: message.to_string(), unauthorized: false }
}

fn with_key(req: RequestBuilder) -> RequestBuilder {
    match api_key() {
        Some(k) => req.header("x-api-key", &k),
        None => req,
    }
}

async fn read<T: DeserializeOwned>(resp: Response) -> Result<T, ApiError> {
    if resp.ok() {
        return resp.json().await.map_err(err);
    }
    #[derive(Deserialize)]
    struct Body {
        error: String,
    }
    let unauthorized = resp.status() == 401;
    let message = match resp.json::<Body>().await {
        Ok(b) => b.error,
        Err(_) => format!("erreur HTTP {}", resp.status()),
    };
    Err(ApiError { message, unauthorized })
}

pub async fn get<T: DeserializeOwned>(path: &str) -> Result<T, ApiError> {
    let resp = with_key(Request::get(path)).send().await.map_err(err)?;
    read(resp).await
}

pub async fn send<B: Serialize, T: DeserializeOwned>(method: &str, path: &str, body: &B) -> Result<T, ApiError> {
    let builder = match method {
        "POST" => Request::post(path),
        "PUT" => Request::put(path),
        "DELETE" => Request::delete(path),
        _ => Request::get(path),
    };
    let resp = with_key(builder).json(body).map_err(err)?.send().await.map_err(err)?;
    read(resp).await
}

/// Envoi sans réponse attendue (204).
pub async fn post_empty<B: Serialize>(path: &str, body: &B) -> Result<(), ApiError> {
    send_empty("POST", path, body).await
}

/// Envoi (`POST`, `PUT`) sans réponse attendue (204).
pub async fn send_empty<B: Serialize>(method: &str, path: &str, body: &B) -> Result<(), ApiError> {
    let builder = if method == "PUT" { Request::put(path) } else { Request::post(path) };
    let resp = with_key(builder).json(body).map_err(err)?.send().await.map_err(err)?;
    if resp.ok() {
        Ok(())
    } else {
        read::<serde_json::Value>(resp).await.map(|_| ())
    }
}

pub async fn delete(path: &str) -> Result<(), ApiError> {
    let resp = with_key(Request::delete(path)).send().await.map_err(err)?;
    if resp.ok() {
        Ok(())
    } else {
        read::<serde_json::Value>(resp).await.map(|_| ())
    }
}

pub async fn upload<T: DeserializeOwned>(path: &str, bytes: Vec<u8>) -> Result<T, ApiError> {
    let body = js_sys::Uint8Array::from(&bytes[..]);
    let resp = with_key(Request::post(path))
        .header("content-type", "application/octet-stream")
        .body(body)
        .map_err(err)?
        .send()
        .await
        .map_err(err)?;
    read(resp).await
}

/// Lecture authentifiée d'un texte (page HTML d'un export…).
pub async fn text(path: &str) -> Result<String, ApiError> {
    let resp = with_key(Request::get(path)).send().await.map_err(err)?;
    if !resp.ok() {
        return read::<serde_json::Value>(resp).await.map(|_| String::new());
    }
    resp.text().await.map_err(err)
}

/// Téléchargement authentifié : le fichier est ouvert depuis un objet Blob.
pub async fn download(path: &str, file_name: &str) -> Result<(), ApiError> {
    let resp = with_key(Request::get(path)).send().await.map_err(err)?;
    if !resp.ok() {
        return read::<serde_json::Value>(resp).await.map(|_| ());
    }
    let bytes = resp.binary().await.map_err(err)?;
    let array = js_sys::Array::new();
    array.push(&js_sys::Uint8Array::from(&bytes[..]));
    let blob = web_sys::Blob::new_with_u8_array_sequence(&array).map_err(|_| err("fichier illisible"))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(|_| err("fichier illisible"))?;
    let document = web_sys::window().and_then(|w| w.document()).ok_or_else(|| err("pas de document"))?;
    let a = document.create_element("a").map_err(|_| err("lien"))?;
    let _ = a.set_attribute("href", &url);
    let _ = a.set_attribute("download", file_name);
    if let Some(a) = wasm_bindgen::JsCast::dyn_ref::<web_sys::HtmlElement>(&a) {
        a.click();
    }
    let _ = web_sys::Url::revoke_object_url(&url);
    Ok(())
}

// ---------------------------------------------------------------- données

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub value: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Meta {
    pub weapons: Vec<Choice>,
    pub genders: Vec<Choice>,
    pub categories: Vec<Choice>,
    pub levels: Vec<Choice>,
    pub colors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Me {
    pub id: String,
    pub name: String,
}

/// Compte connecté (absent quand on utilise une clé d'API).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct UserInfo {
    pub id: String,
    pub email: String,
    pub name: String,
    pub club_name: String,
    pub status: String,
    pub admin: bool,
    pub client: Option<String>,
    pub client_name: Option<String>,
    pub created: String,
    pub last_login: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct LoggedIn {
    pub token: String,
    pub user: UserInfo,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct KeyInfo {
    pub id: String,
    pub name: String,
    pub created: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct CreatedKey {
    pub id: String,
    pub name: String,
    pub key: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ClubInfo {
    pub id: String,
    pub name: String,
    pub created: String,
    pub contests: usize,
    pub users: usize,
}

/// Date lisible d'un horodatage du serveur (secondes depuis 1970).
pub fn date_label(stamp: &str) -> String {
    let Ok(secs) = stamp.parse::<f64>() else { return stamp.to_string() };
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(secs * 1000.0));
    let opts = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&opts, &"dateStyle".into(), &"medium".into());
    let _ = js_sys::Reflect::set(&opts, &"timeStyle".into(), &"short".into());
    d.to_locale_string("fr-FR", &opts).as_string().unwrap_or_default()
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ContestSummary {
    pub id: String,
    pub name: String,
    pub weapon: String,
    pub weapon_label: String,
    pub gender: String,
    pub gender_label: String,
    pub category: String,
    pub level: String,
    pub label: String,
    pub date: String,
    pub location: String,
    pub organizer: String,
    pub color: String,
    pub team_event: bool,
    pub fencers: usize,
    pub stage: String,
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Properties {
    pub name: String,
    pub organizer: String,
    pub location: String,
    pub label: String,
    pub web_site: String,
    pub weapon: String,
    pub gender: String,
    pub category: usize,
    pub level: usize,
    pub date: String,
    pub color: String,
    pub team_event: bool,
    #[serde(default)]
    pub team_size: Option<u32>,
    #[serde(default)]
    pub substitutes: Option<u32>,
    /// Points (préférences « Points ») : `enabled`, `weight`, `place_bonus`,
    /// `top_bonus`, `formula`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<serde_json::Value>,
    /// Remplissage automatique des scores.
    #[serde(default)]
    pub score_stuffing: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct NewContest {
    #[serde(flatten)]
    pub properties: Properties,
    pub youth_training: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct StageInfo {
    pub index: usize,
    pub kind: String,
    pub class_name: String,
    pub name: String,
    #[serde(default)]
    pub named: bool,
    pub locked: bool,
    #[serde(default)]
    pub removable: bool,
    #[serde(default)]
    pub can_up: bool,
    #[serde(default)]
    pub can_down: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ScheduleView {
    pub stages: Vec<StageInfo>,
    pub current: usize,
    pub can_previous: bool,
    pub can_next: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Column {
    pub code: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct FencerRow {
    pub reference: u32,
    pub attending: bool,
    pub cells: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct CheckinView {
    pub columns: Vec<Column>,
    pub fencers: Vec<FencerRow>,
    pub presents: u32,
    pub absents: u32,
    pub locked: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct FormField {
    pub code: String,
    pub title: String,
    pub kind: String,
    pub choices: Vec<Choice>,
    pub singular: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ImportReport {
    pub imported: usize,
    pub encoding_error: bool,
    pub checkin: CheckinView,
}

#[derive(Serialize)]
pub struct FencerInput {
    pub values: BTreeMap<String, String>,
}

#[derive(Serialize)]
pub struct AttendingInput {
    pub references: Vec<u32>,
    pub attending: bool,
}
