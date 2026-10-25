//! SmartPoule : relais entre BellePoule et les téléphones des arbitres.
//!
//! BellePoule (bureau) et les téléphones ne sont pas forcément sur le même
//! réseau : ils passent tous deux par ce serveur.
//!
//! - **BellePoule** (clé d'API du club) dépose un instantané
//!   (`PUT /api/smartpoule/state` : arbitres et leurs clés, matchs à
//!   arbitrer, feuilles), relève les scores saisis
//!   (`GET /api/smartpoule/pending`) et rend compte de leur application
//!   (`POST /api/smartpoule/results`).
//! - **Le téléphone** (identifiant du club, référence et clé de l'arbitre,
//!   lus dans son QR code) consulte ses matchs et leurs feuilles et envoie
//!   les scores (`/api/smartpoule/phone/…`).
//! - Les compétitions gérées directement par la version web sont servies
//!   sans relais (lecture et écriture immédiates).

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::{bad_request, ApiError, Shared};
use crate::auth::Client;

/// Score saisi sur un téléphone, en attente de BellePoule.
#[derive(Clone, Serialize, Deserialize)]
struct Submission {
    id: u64,
    token: String,
    bout: u64,
    values: Option<[String; 2]>,
    officiating: Option<bp_core::officiating::Officiating>,
    /// Rencontre par équipes : journal des relais.
    #[serde(default)]
    relay: Option<bp_core::relay::RelayLog>,
    referee: u32,
    /// `None` : en attente ; `Some(Ok)` / `Some(Err(message))` : traité.
    result: Option<Result<(), String>>,
}

/// Téléphone reconnu après le PIN d'un arbitre (QR code d'une feuille).
#[derive(Clone, Serialize, Deserialize)]
struct PhoneSession {
    key: String,
    referee: u32,
    name: String,
    /// Fin de validité (secondes depuis 1970).
    expires: u64,
}

/// Essais de PIN sur une feuille : 20 essais, 5 minutes de pause, encore
/// 20 essais, puis blocage jusqu'à ce que l'organisation débloque.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct SheetGuard {
    fails: u32,
    blocked_until: u64,
    locked: bool,
    /// Feuille (alerte de l'organisation).
    name: String,
    /// Heure du blocage.
    since: String,
}

/// État du relais d'un club.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Relay {
    /// Dernier instantané déposé par BellePoule.
    state: Value,
    submissions: Vec<Submission>,
    next_id: u64,
    sessions: Vec<PhoneSession>,
    guards: HashMap<String, SheetGuard>,
    /// Matériel vérifié par l'arbitre avant la poule : feuille → tireurs.
    equipment: HashMap<String, Vec<u32>>,
    /// Signalements des arbitres (absence d'un tireur).
    reports: Vec<Report>,
    next_report: u64,
}

/// Signalement d'un arbitre : « Absence de X du club de Y sur la piste 12 ».
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Report {
    id: u64,
    /// `call` : appel du Directoire technique ; vide : absence d'un tireur.
    kind: String,
    token: String,
    fencer: u32,
    name: String,
    club: String,
    piste: String,
    /// Poule ou match, et compétition.
    sheet: String,
    competition: String,
    referee: String,
    at: String,
    /// Traité par l'organisation.
    done: bool,
}

impl Report {
    fn message(&self) -> String {
        if self.kind == "call" {
            let piste = if self.piste.is_empty() { String::new() } else { format!(" piste {}", self.piste) };
            return format!("Directoire technique appelé{piste} par : {}", self.referee);
        }
        let mut m = format!("Absence de {}", self.name);
        if !self.club.is_empty() {
            m.push_str(&format!(" du club de {}", self.club));
        }
        if !self.piste.is_empty() {
            m.push_str(&format!(" sur la piste {}", self.piste));
        }
        m
    }

    fn json(&self) -> Value {
        json!({ "id": self.id, "kind": if self.kind.is_empty() { "absence" } else { self.kind.as_str() }, "message": self.message(), "name": self.name, "club": self.club, "piste": self.piste, "sheet": self.sheet,
            "competition": self.competition, "referee": self.referee, "at": self.at, "done": self.done })
    }
}

/// Validité d'un téléphone reconnu par PIN : la journée ou le week-end.
const SESSION_SECONDS: u64 = 3 * 24 * 3600;
/// Essais de PIN avant une pause, puis avant le blocage définitif.
const PIN_TRIES: u32 = 20;
const PIN_PAUSE_SECONDS: u64 = 5 * 60;

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Comparaison en temps constant.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn forbidden() -> ApiError {
    ApiError(StatusCode::FORBIDDEN, "QR code inconnu : demandez un nouveau QR code à l'organisation".into())
}

// --------------------------------------------------------------- BellePoule

/// `PUT /api/smartpoule/state`.
pub async fn put_state(State(store): State<Shared>, client: Client, Json(state): Json<Value>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    store.ensure_relay(&client.id);
    store.relays.entry(client.id.clone()).or_default().state = state;
    store.save_relay(&client.id)?;
    Ok(Json(json!({ "club": client.id })))
}

/// `GET /api/smartpoule/pending` : scores à appliquer.
pub async fn pending(State(store): State<Shared>, client: Client) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    store.ensure_relay(&client.id);
    let relay = store.relays.entry(client.id.clone()).or_default();
    let list: Vec<Value> = relay
        .submissions
        .iter()
        .filter(|s| s.result.is_none())
        .map(|s| json!({ "id": s.id, "token": s.token, "bout": s.bout, "values": s.values, "officiating": s.officiating, "relay": s.relay, "referee": s.referee }))
        .collect();
    Ok(Json(json!(list)))
}

#[derive(Deserialize)]
pub struct Results {
    results: Vec<ResultEntry>,
}

#[derive(Deserialize)]
pub struct ResultEntry {
    id: u64,
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

/// `POST /api/smartpoule/results` : scores appliqués (ou refusés).
pub async fn results(State(store): State<Shared>, client: Client, Json(r): Json<Results>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    store.ensure_relay(&client.id);
    let relay = store.relays.entry(client.id.clone()).or_default();
    for e in r.results {
        if let Some(s) = relay.submissions.iter_mut().find(|s| s.id == e.id) {
            s.result = Some(if e.ok { Ok(()) } else { Err(e.error.unwrap_or_else(|| "score refusé".into())) });
        }
    }
    // Ne jamais supprimer une saisie encore en attente.
    let mut complete = relay.submissions.iter().filter(|s| s.result.is_some()).count();
    relay.submissions.retain(|s| {
        if s.result.is_some() && complete > 500 { complete -= 1; false } else { true }
    });
    store.save_relay(&client.id)?;
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/smartpoule/admin` : clé d'administration SmartPoule des
/// compétitions de la version web (QR code de l'administrateur).
pub async fn admin_code(State(store): State<Shared>, client: Client) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let key = store.smartpoule_admin_key(&client);
    Ok(Json(json!({ "club": client.id, "name": client.name, "key": key })))
}

// --------------------------------------------------------------- téléphone

#[derive(Deserialize)]
pub struct PhoneQuery {
    /// Club.
    c: String,
    /// Référence de l'arbitre (0 : administrateur).
    r: u32,
    /// Clé de l'arbitre.
    k: String,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    id: Option<u64>,
}

/// Arbitre authentifié : `None` pour l'administrateur.
enum Who {
    Referee(u32, String),
    Admin,
}

impl crate::store::Store {
    fn ensure_relay(&mut self, club: &str) {
        if self.relays.contains_key(club) { return; }
        let path = self.dir.join("clients").join(club).join("smartpoule-relay.json");
        let relay = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        self.relays.insert(club.to_string(), relay);
    }
    fn save_relay(&self, club: &str) -> Result<(), ApiError> {
        let Some(relay) = self.relays.get(club) else { return Ok(()) };
        let path = self.dir.join("clients").join(club).join("smartpoule-relay.json");
        let data = serde_json::to_vec(relay).map_err(crate::api::internal)?;
        if std::fs::read(&path).ok().as_deref() == Some(data.as_slice()) { return Ok(()); }
        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(crate::api::internal)?; }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, data).map_err(crate::api::internal)?;
        std::fs::rename(&temporary, path).map_err(crate::api::internal)?;
        Ok(())
    }

    fn club(&self, id: &str) -> Option<Client> {
        self.clients().iter().find(|c| c.id == id).map(|c| Client { id: c.id.clone(), name: c.name.clone() })
    }

    /// Vérifie la clé présentée par le téléphone : clé d'un arbitre ou clé
    /// d'administration, dans l'instantané de BellePoule ou dans une
    /// compétition de la version web.
    fn authenticate(&mut self, q: &PhoneQuery) -> Result<(Client, Who), ApiError> {
        let club = self.club(&q.c).ok_or_else(forbidden)?;
        self.ensure_relay(&club.id);
        if q.k.len() < 16 {
            return Err(forbidden());
        }
        let state = self.relays.get(&club.id).map(|r| r.state.clone()).unwrap_or(Value::Null);
        if q.r == 0 {
            let admin = state.get("admin_key").and_then(Value::as_str).map(str::to_string);
            let web_admin = self.smartpoule_admin_key(&club);
            if admin.as_deref() == Some(q.k.as_str()) || web_admin == q.k {
                return Ok((club, Who::Admin));
            }
            return Err(forbidden());
        }
        // Téléphone reconnu par le PIN d'une feuille.
        let now = now_secs();
        if let Some(s) = self.relays.get(&club.id).and_then(|r| r.sessions.iter().find(|s| s.referee == q.r && s.expires > now && same(&s.key, &q.k))) {
            let name = s.name.clone();
            return Ok((club, Who::Referee(q.r, name)));
        }
        if let Some(list) = state.get("referees").and_then(Value::as_array) {
            for r in list {
                if r.get("reference").and_then(Value::as_u64) == Some(q.r as u64) && r.get("key").and_then(Value::as_str) == Some(q.k.as_str()) {
                    let name = r.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                    return Ok((club, Who::Referee(q.r, name)));
                }
            }
        }
        let found = self
            .list(&club)
            .map(|(_, c)| c)
            .find(|c| c.referee_password(q.r).as_deref() == Some(q.k.as_str()))
            .map(|c| c.referees.get(q.r).map(|p| format!("{} {}", p.name(), p.first_name())).unwrap_or_default());
        // QR code personnel remis par l'organisation de la version web.
        let found = found.or_else(|| {
            let hall = &self.organisation(&club).hall;
            (hall.referee_key(q.r).as_deref() == Some(q.k.as_str())).then(|| hall.referees.get(q.r).map(|r| format!("{} {}", r.player.name(), r.player.first_name())).unwrap_or_default())
        });
        match found {
            Some(name) => Ok((club, Who::Referee(q.r, name))),
            None => Err(forbidden()),
        }
    }

    /// Clé d'administration SmartPoule des compétitions de la version web.
    pub fn smartpoule_admin_key(&mut self, club: &Client) -> String {
        let path = self.dir.join("clients").join(&club.id).join("smartpoule.json");
        if let Some(k) = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v.get("admin_key").and_then(Value::as_str).map(str::to_string)) {
            return k;
        }
        let key = crate::auth::new_key();
        let _ = std::fs::write(&path, json!({ "admin_key": key }).to_string());
        key
    }
}

impl crate::store::Store {
    /// Clés personnelles connues d'un arbitre : instantané de BellePoule,
    /// compétitions de la version web, arbitres de l'organisation.
    fn referee_keys(&mut self, club: &Client, reference: u32) -> Vec<(String, String)> {
        let mut keys = Vec::new();
        if let Some(list) = self.relays.get(&club.id).and_then(|r| r.state.get("referees")).and_then(Value::as_array) {
            for r in list.iter().filter(|r| r.get("reference").and_then(Value::as_u64) == Some(reference as u64)) {
                if let Some(k) = r.get("key").and_then(Value::as_str) {
                    keys.push((k.to_string(), r.get("name").and_then(Value::as_str).unwrap_or("").to_string()));
                }
            }
        }
        for (_, c) in self.list(club) {
            if let Some(k) = c.referee_password(reference) {
                keys.push((k, c.referees.get(reference).map(|p| format!("{} {}", p.name(), p.first_name())).unwrap_or_default()));
            }
        }
        let hall = &self.organisation(club).hall;
        if let Some(k) = hall.referee_key(reference) {
            keys.push((k, hall.referees.get(reference).map(|r| format!("{} {}", r.player.name(), r.player.first_name())).unwrap_or_default()));
        }
        keys
    }

    /// Matchs à arbitrer du club (version web et instantané de BellePoule).
    fn all_jobs(&self, club: &Client) -> Vec<Value> {
        let mut jobs: Vec<Value> = self.list(club).flat_map(|(_, c)| c.smartpoule_jobs(None)).collect();
        if let Some(list) = self.relays.get(&club.id).and_then(|r| r.state.get("jobs")).and_then(Value::as_array) {
            jobs.extend(list.iter().cloned());
        }
        jobs
    }

    /// Les téléphones d'un arbitre ne sont plus reconnus (nouveau PIN).
    pub fn forget_referee_phones(&mut self, club: &str, reference: u32) {
        self.ensure_relay(club);
        if let Some(r) = self.relays.get_mut(club) {
            r.sessions.retain(|s| s.referee != reference);
        }
        let _ = self.save_relay(club);
    }
}

#[derive(Deserialize)]
pub struct PinLogin {
    c: String,
    token: String,
    pin: String,
}

/// `POST /api/smartpoule/phone/pin` : QR code d'une feuille + PIN d'un
/// arbitre affecté à cette feuille → le téléphone est reconnu (3 jours).
pub async fn phone_pin(State(store): State<Shared>, Json(p): Json<PinLogin>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let club = store.club(&p.c).ok_or_else(forbidden)?;
    store.ensure_relay(&club.id);
    let now = now_secs();
    let job = store
        .all_jobs(&club)
        .into_iter()
        .find(|j| j.get("token").and_then(Value::as_str) == Some(p.token.as_str()))
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "Cette feuille n'est pas (ou plus) à arbitrer : adressez-vous à l'organisation.".into()))?;
    let sheet_name = [job.get("name"), job.get("competition")].into_iter().flatten().filter_map(Value::as_str).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" — ");
    {
        let guard = store.relays.entry(club.id.clone()).or_default().guards.entry(p.token.clone()).or_default();
        if guard.locked {
            return Err(ApiError(StatusCode::LOCKED, "Trop d'essais : cette feuille est bloquée. Adressez-vous à l'organisation.".into()));
        }
        if guard.blocked_until > now {
            let minutes = (guard.blocked_until - now).div_ceil(60);
            return Err(ApiError(StatusCode::TOO_MANY_REQUESTS, format!("Trop d'essais : réessayez dans {minutes} min.")));
        }
    }
    let mut referees: Vec<u32> = job.get("referees").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_u64).map(|r| r as u32).collect()).unwrap_or_default();
    referees.sort_unstable();
    referees.dedup();
    if referees.is_empty() {
        return Err(bad_request("Aucun arbitre n'est encore affecté à cette feuille : adressez-vous à l'organisation."));
    }
    let pin = p.pin.trim().to_string();
    let mut found = None;
    for r in referees {
        if let Some((_, name)) = store.referee_keys(&club, r).into_iter().find(|(k, _)| same(&bp_core::smartpoule::referee_pin(k), &pin)) {
            found = Some((r, name));
            break;
        }
    }
    let relay = store.relays.entry(club.id.clone()).or_default();
    let answer = match found {
        Some((r, name)) => {
            relay.guards.remove(&p.token);
            let key = crate::auth::new_key();
            relay.sessions.retain(|s| s.expires > now);
            relay.sessions.push(PhoneSession { key: key.clone(), referee: r, name: name.clone(), expires: now + SESSION_SECONDS });
            Ok(json!({ "c": club.id, "r": r, "k": key, "name": name, "expires": now + SESSION_SECONDS }))
        }
        None => {
            let guard = relay.guards.entry(p.token.clone()).or_default();
            guard.fails += 1;
            guard.name = sheet_name;
            let message = if guard.fails >= 2 * PIN_TRIES {
                guard.locked = true;
                guard.since = crate::store::chrono_now();
                "Code PIN incorrect. Trop d'essais : cette feuille est bloquée, adressez-vous à l'organisation.".to_string()
            } else if guard.fails % PIN_TRIES == 0 {
                guard.blocked_until = now + PIN_PAUSE_SECONDS;
                format!("Code PIN incorrect. Trop d'essais : réessayez dans {} min.", PIN_PAUSE_SECONDS / 60)
            } else {
                "Code PIN incorrect.".to_string()
            };
            Err(ApiError(StatusCode::FORBIDDEN, message))
        }
    };
    store.save_relay(&club.id)?;
    answer.map(Json)
}

/// `GET /api/smartpoule/security` : feuilles bloquées (alertes) et
/// téléphones reconnus par PIN.
pub async fn security(State(store): State<Shared>, client: Client) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    store.ensure_relay(&client.id);
    let now = now_secs();
    // Feuille d'une compétition de la version web hors de l'organisation :
    // son signalement ne concerne pas la journée en cours.
    let (web, organised) = store.organised_net_ids(&client);
    let concerned = |token: &str| match token.split('~').nth(1).and_then(|n| n.parse::<u32>().ok()) {
        Some(net) => !web.contains(&net) || organised.contains(&net),
        None => true,
    };
    let relay = store.relays.entry(client.id.clone()).or_default();
    let alerts: Vec<Value> = relay.guards.iter().filter(|(t, g)| g.locked && concerned(t)).map(|(t, g)| json!({ "token": t, "name": g.name, "since": g.since })).collect();
    let paused: Vec<Value> = relay
        .guards
        .iter()
        .filter(|(t, g)| !g.locked && g.blocked_until > now && concerned(t))
        .map(|(t, g)| json!({ "token": t, "name": g.name, "minutes": (g.blocked_until - now).div_ceil(60) }))
        .collect();
    let mut phones: HashMap<u32, (String, usize)> = HashMap::new();
    for s in relay.sessions.iter().filter(|s| s.expires > now) {
        phones.entry(s.referee).or_insert_with(|| (s.name.clone(), 0)).1 += 1;
    }
    let phones: Vec<Value> = phones.into_iter().map(|(r, (name, n))| json!({ "referee": r, "name": name, "count": n })).collect();
    // Signalements : en cours d'abord, puis les derniers traités.
    let mut reports: Vec<&Report> = relay.reports.iter().filter(|r| concerned(&r.token)).collect();
    reports.sort_by_key(|r| (r.done, std::cmp::Reverse(r.id)));
    let open = reports.iter().filter(|r| !r.done).count();
    let reports: Vec<Value> = reports.into_iter().take(open + 10).map(Report::json).collect();
    Ok(Json(json!({ "alerts": alerts, "paused": paused, "phones": phones, "reports": reports, "open_reports": open })))
}

/// `POST /api/smartpoule/security` : `{ "action": "unlock", "token" }`
/// débloque une feuille ; `{ "action": "logout", "referee" }` oublie les
/// téléphones d'un arbitre reconnus par PIN.
pub async fn security_action(State(store): State<Shared>, client: Client, Json(a): Json<Value>) -> Result<Json<Value>, ApiError> {
    {
        let mut store = store.lock().await;
        store.ensure_relay(&client.id);
        match a.get("action").and_then(Value::as_str).unwrap_or("") {
            "unlock" => {
                let token = a.get("token").and_then(Value::as_str).unwrap_or("");
                store.relays.entry(client.id.clone()).or_default().guards.remove(token);
                store.save_relay(&client.id)?;
            }
            "logout" => {
                let r = a.get("referee").and_then(Value::as_u64).unwrap_or(0) as u32;
                store.forget_referee_phones(&client.id, r);
            }
            // Signalement traité (ou rouvert).
            "report_done" => {
                let id = a.get("id").and_then(Value::as_u64).unwrap_or(0);
                let done = a.get("done").and_then(Value::as_bool).unwrap_or(true);
                if let Some(r) = store.relays.entry(client.id.clone()).or_default().reports.iter_mut().find(|r| r.id == id) {
                    r.done = done;
                }
                store.save_relay(&client.id)?;
            }
            other => return Err(bad_request(format!("action inconnue : {other}"))),
        }
    }
    security(State(store), client).await
}

fn mine(job: &Value, who: &Who) -> bool {
    match who {
        Who::Admin => true,
        Who::Referee(r, _) => job.get("referees").and_then(Value::as_array).is_some_and(|l| l.iter().any(|x| x.as_u64() == Some(*r as u64))),
    }
}

/// `GET /api/smartpoule/phone/jobs` : matchs de l'arbitre.
pub async fn phone_jobs(State(store): State<Shared>, Query(q): Query<PhoneQuery>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let (club, who) = store.authenticate(&q)?;
    let mut jobs: Vec<Value> = Vec::new();
    if let Some(list) = store.relays.get(&club.id).and_then(|r| r.state.get("jobs")).and_then(Value::as_array) {
        jobs.extend(list.iter().filter(|j| mine(j, &who)).cloned());
    }
    for (_, contest) in store.list(&club) {
        jobs.extend(contest.smartpoule_jobs(None).into_iter().filter(|j| mine(j, &who)));
    }
    let name = match &who {
        Who::Admin => "Administrateur".to_string(),
        Who::Referee(_, n) => n.clone(),
    };
    Ok(Json(json!({ "club": club.name, "name": name, "jobs": jobs })))
}

/// `GET /api/smartpoule/phone/sheet?token=…` : feuille d'un match.
pub async fn phone_sheet(State(store): State<Shared>, Query(q): Query<PhoneQuery>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let (club, who) = store.authenticate(&q)?;
    let token = q.token.clone().ok_or_else(|| bad_request("jeton attendu"))?;
    let referee = match who {
        Who::Admin => None,
        Who::Referee(r, _) => Some(r),
    };
    let mut sheet = store.sheet_of(&club, &token, referee).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "match introuvable".into()))?;
    let relay = store.relays.get(&club.id);
    sheet["equipment"] = json!(relay.and_then(|r| r.equipment.get(&token)).cloned().unwrap_or_default());
    sheet["absences"] = json!(relay.map(|r| r.absences(&token)).unwrap_or_default());
    Ok(Json(sheet))
}

impl Relay {
    /// Tireurs signalés absents (signalements non traités) d'une feuille.
    fn absences(&self, token: &str) -> Vec<u32> {
        self.reports.iter().filter(|r| !r.done && r.kind.is_empty() && r.token == token).map(|r| r.fencer).collect()
    }
}

impl crate::store::Store {
    /// Feuille d'un match (instantané de BellePoule ou version web).
    fn sheet_of(&self, club: &Client, token: &str, referee: Option<u32>) -> Option<Value> {
        let relayed = self.relays.get(&club.id).and_then(|r| r.state.get("sheets")).and_then(|s| s.get(token)).map(|sheet| filter_bouts(sheet.clone(), referee));
        relayed.or_else(|| self.list(club).find_map(|(_, contest)| contest.smartpoule_sheet(token, referee)))
    }

    /// Un arbitre n'agit que sur les feuilles qui lui sont affectées.
    fn check_assigned(&self, club: &Client, who: &Who, token: &str) -> Result<(), ApiError> {
        if let Who::Referee(r, _) = who {
            let assigned = self.all_jobs(club).iter().any(|job| job.get("token").and_then(Value::as_str) == Some(token) && mine(job, &Who::Referee(*r, String::new())));
            if !assigned {
                return Err(forbidden());
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
pub struct PhoneCall {
    c: String,
    r: u32,
    k: String,
    /// Feuille en cours (piste) ; absente depuis la liste des matchs.
    #[serde(default)]
    token: Option<String>,
}

/// `POST /api/smartpoule/phone/call` : l'arbitre appelle le Directoire
/// technique (notification de l'organisation, puce rouge).
pub async fn phone_call(State(store): State<Shared>, Json(a): Json<PhoneCall>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let q = PhoneQuery { c: a.c.clone(), r: a.r, k: a.k.clone(), token: None, id: None };
    let (club, who) = store.authenticate(&q)?;
    let token = a.token.clone().filter(|t| !t.is_empty());
    let sheet = match &token {
        Some(t) => {
            store.check_assigned(&club, &who, t)?;
            store.sheet_of(&club, t, None)
        }
        None => None,
    };
    let text = |k: &str| sheet.as_ref().and_then(|s| s.get(k)).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let referee = match &who {
        Who::Admin => "Administrateur".to_string(),
        Who::Referee(_, n) => n.clone(),
    };
    let relay = store.relays.entry(club.id.clone()).or_default();
    let token = token.unwrap_or_default();
    // Un appel en cours du même arbitre pour la même feuille suffit.
    if relay.reports.iter().any(|r| !r.done && r.kind == "call" && r.token == token && r.referee == referee) {
        return Ok(Json(json!({ "status": "already" })));
    }
    relay.next_report += 1;
    relay.reports.push(Report {
        id: relay.next_report,
        kind: "call".into(),
        token,
        fencer: 0,
        name: String::new(),
        club: String::new(),
        piste: text("piste").split_whitespace().next().unwrap_or("").to_string(),
        sheet: text("name"),
        competition: text("competition"),
        referee,
        at: crate::store::chrono_now(),
        done: false,
    });
    store.save_relay(&club.id)?;
    Ok(Json(json!({ "status": "ok" })))
}

#[derive(Deserialize)]
pub struct PhoneAbsence {
    c: String,
    r: u32,
    k: String,
    token: String,
    fencer: u32,
    /// Faux : le signalement est retiré (le tireur est arrivé).
    absent: bool,
}

/// `POST /api/smartpoule/phone/absence` : l'arbitre signale l'absence d'un
/// tireur de sa poule (ou retire son signalement).
pub async fn phone_absence(State(store): State<Shared>, Json(a): Json<PhoneAbsence>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let q = PhoneQuery { c: a.c.clone(), r: a.r, k: a.k.clone(), token: None, id: None };
    let (club, who) = store.authenticate(&q)?;
    store.check_assigned(&club, &who, &a.token)?;
    let sheet = store.sheet_of(&club, &a.token, None).ok_or_else(|| bad_request("feuille introuvable"))?;
    let fencer = sheet
        .get("fencers")
        .and_then(Value::as_array)
        .and_then(|l| l.iter().find(|f| f.get("reference").and_then(Value::as_u64) == Some(a.fencer as u64)))
        .cloned()
        .ok_or_else(|| bad_request("tireur introuvable"))?;
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let referee = match &who {
        Who::Admin => "Administrateur".to_string(),
        Who::Referee(_, n) => n.clone(),
    };
    let relay = store.relays.entry(club.id.clone()).or_default();
    if a.absent {
        if !relay.reports.iter().any(|r| !r.done && r.kind.is_empty() && r.token == a.token && r.fencer == a.fencer) {
            relay.next_report += 1;
            let piste = text(&sheet, "piste");
            relay.reports.push(Report {
                id: relay.next_report,
                token: a.token.clone(),
                fencer: a.fencer,
                name: text(&fencer, "name"),
                club: text(&fencer, "club"),
                piste: piste.split_whitespace().next().unwrap_or("").to_string(),
                sheet: text(&sheet, "name"),
                competition: text(&sheet, "competition"),
                referee,
                at: crate::store::chrono_now(),
                done: false,
                kind: String::new(),
            });
            // Les plus anciens signalements traités sont oubliés.
            let mut done = relay.reports.iter().filter(|r| r.done).count();
            relay.reports.retain(|r| if r.done && done > 200 { done -= 1; false } else { true });
        }
    } else {
        relay.reports.retain(|r| r.done || !r.kind.is_empty() || r.token != a.token || r.fencer != a.fencer);
    }
    let absences = relay.absences(&a.token);
    store.save_relay(&club.id)?;
    Ok(Json(json!({ "absences": absences })))
}

#[derive(Deserialize)]
pub struct PhoneEquipment {
    c: String,
    r: u32,
    k: String,
    token: String,
    fencer: u32,
    checked: bool,
}

/// `POST /api/smartpoule/phone/equipment` : matériel d'un tireur vérifié
/// (ou non) par l'arbitre de la poule.
pub async fn phone_equipment(State(store): State<Shared>, Json(e): Json<PhoneEquipment>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let q = PhoneQuery { c: e.c.clone(), r: e.r, k: e.k.clone(), token: None, id: None };
    let (club, who) = store.authenticate(&q)?;
    store.check_assigned(&club, &who, &e.token)?;
    let relay = store.relays.entry(club.id.clone()).or_default();
    let list = relay.equipment.entry(e.token.clone()).or_default();
    list.retain(|f| *f != e.fencer);
    if e.checked {
        list.push(e.fencer);
    }
    let list = list.clone();
    store.save_relay(&club.id)?;
    Ok(Json(json!({ "equipment": list })))
}

/// Feuille d'une poule sur plusieurs pistes : l'arbitre ne voit que les
/// matchs de sa piste (champ `referee` de chaque match dans l'instantané).
fn filter_bouts(mut sheet: Value, referee: Option<u32>) -> Value {
    if let (Some(r), Some(bouts)) = (referee, sheet.get_mut("bouts").and_then(Value::as_array_mut)) {
        bouts.retain(|b| b.get("referee").and_then(Value::as_u64).is_none_or(|x| x == r as u64));
    }
    sheet
}

#[derive(Deserialize)]
pub struct PhoneScore {
    c: String,
    r: u32,
    k: String,
    token: String,
    bout: u64,
    values: Option<[String; 2]>,
    officiating: Option<bp_core::officiating::Officiating>,
    /// Rencontre par équipes : journal des relais.
    #[serde(default)]
    relay: Option<bp_core::relay::RelayLog>,
}

/// `POST /api/smartpoule/phone/score` : score d'un match.
pub async fn phone_score(State(store): State<Shared>, Json(s): Json<PhoneScore>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let q = PhoneQuery { c: s.c.clone(), r: s.r, k: s.k.clone(), token: None, id: None };
    let (club, who) = store.authenticate(&q)?;
    let referee = match who {
        Who::Admin => 0,
        Who::Referee(r, _) => r,
    };
    if s.values.is_none() && s.officiating.is_none() && s.relay.is_none() { return Err(bad_request("score ou arbitrage attendu")); }
    // L'authentification seule ne donne pas accès aux matchs d'un autre arbitre.
    let raw = store.list(&club).find_map(|(_, c)| c.smartpoule_sheet(&s.token, None))
        .or_else(|| store.relays.get(&club.id).and_then(|r| r.state.get("sheets")).and_then(|v| v.get(&s.token)).cloned())
        .ok_or_else(|| bad_request("match introuvable"))?;
    let assigned = referee == 0 || store.list(&club).flat_map(|(_, c)| c.smartpoule_jobs(None))
        .chain(store.relays.get(&club.id).and_then(|r| r.state.get("jobs")).and_then(Value::as_array).into_iter().flatten().cloned())
        .any(|job| job.get("token").and_then(Value::as_str) == Some(s.token.as_str()) && mine(&job, &Who::Referee(referee, String::new())));
    if !assigned { return Err(forbidden()); }
    let allowed = filter_bouts(raw, if referee == 0 { None } else { Some(referee) });
    // Lobby (série) : pas de matchs, la manche et le tireur sont dans la saisie.
    let lobby = s.token.starts_with("l~");
    if !lobby && !allowed.get("bouts").and_then(Value::as_array).is_some_and(|bouts| bouts.iter().any(|b| b.get("index").and_then(Value::as_u64) == Some(s.bout))) { return Err(forbidden()); }
    // Compétition de la version web : appliqué tout de suite.
    let web: Option<String> = store.list(&club).find(|(_, c)| c.smartpoule_sheet(&s.token, None).is_some()).map(|(id, _)| id.clone());
    if let Some(id) = web {
        let result = store
            .update(&club, &id, |c| {
                if let Some(state) = s.officiating.clone() { c.smartpoule_apply_officiating(&s.token, s.bout as usize, state)?; }
                if let Some(log) = s.relay.clone() { c.smartpoule_apply_relay(&s.token, s.bout as usize, log)?; }
                if let Some(values) = s.values.clone() { c.smartpoule_apply_score(&s.token, s.bout as usize, values)?; }
                Ok::<(), String>(())
            })
            .ok_or_else(|| bad_request("compétition introuvable"))?
            .map_err(crate::api::internal)?;
        return match result {
            Ok(()) => Ok(Json(json!({ "status": "ok" }))),
            Err(e) => Err(bad_request(e)),
        };
    }
    // Compétition de BellePoule : mis en attente.
    let relay = store.relays.entry(club.id.clone()).or_default();
    if let Some(previous) = relay.submissions.iter().find(|previous| previous.token == s.token && previous.bout == s.bout
        && previous.referee == referee && previous.values == s.values && previous.officiating == s.officiating && previous.relay == s.relay
        && !matches!(&previous.result, Some(Err(_)))) {
        return Ok(Json(match &previous.result { Some(Ok(())) => json!({"status":"ok"}), _ => json!({"status":"pending","id":previous.id}) }));
    }
    relay.next_id += 1;
    let id = relay.next_id;
    relay.submissions.push(Submission { id, token: s.token, bout: s.bout, values: s.values, officiating: s.officiating, relay: s.relay, referee, result: None });
    store.save_relay(&club.id)?;
    Ok(Json(json!({ "status": "pending", "id": id })))
}

/// `GET /api/smartpoule/phone/result?id=…` : sort d'un score en attente.
pub async fn phone_result(State(store): State<Shared>, Query(q): Query<PhoneQuery>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let (club, who) = store.authenticate(&q)?;
    let id = q.id.ok_or_else(|| bad_request("identifiant attendu"))?;
    let relay = store.relays.entry(club.id.clone()).or_default();
    let s = relay.submissions.iter().find(|s| s.id == id).ok_or_else(|| bad_request("saisie inconnue"))?;
    if let Who::Referee(reference, _) = who { if s.referee != reference { return Err(forbidden()); } }
    Ok(Json(match &s.result {
        None => json!({ "status": "pending" }),
        Some(Ok(())) => json!({ "status": "ok" }),
        Some(Err(e)) => json!({ "status": "error", "error": e }),
    }))
}

/// Nombre de saisies en attente (indicateur).
#[allow(dead_code)]
pub fn pending_count(relays: &HashMap<String, Relay>, club: &str) -> usize {
    relays.get(club).map(|r| r.submissions.iter().filter(|s| s.result.is_none()).count()).unwrap_or(0)
}

// ------------------------------------------------------------ en direct

/// Dernier état d'une piste, envoyé par le téléphone de l'arbitre.
pub struct LiveBoard {
    state: Value,
    at: std::time::Instant,
}

/// Un écran sans nouvelles depuis ce délai repasse à vide.
const LIVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15 * 60);

#[derive(Deserialize)]
pub struct PhoneLive {
    c: String,
    r: u32,
    k: String,
    piste: u32,
    #[serde(default)]
    competition: String,
    #[serde(default)]
    color: String,
    #[serde(default)]
    bout: String,
    /// Tireur de gauche (rouge) et de droite (vert) : `{ name, score }`.
    red: Value,
    green: Value,
    #[serde(default)]
    remaining_ms: i64,
    #[serde(default)]
    running: bool,
    #[serde(default)]
    period: u32,
    #[serde(default)]
    periods: u32,
    #[serde(default)]
    over: bool,
}

/// `POST /api/smartpoule/phone/live` : score et chrono du match en cours sur
/// une piste, pour son écran LivePoule.
pub async fn phone_live(State(store): State<Shared>, Json(l): Json<PhoneLive>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let q = PhoneQuery { c: l.c.clone(), r: l.r, k: l.k.clone(), token: None, id: None };
    let (club, _) = store.authenticate(&q)?;
    if l.piste == 0 {
        return Err(bad_request("piste inconnue"));
    }
    let state = json!({
        "piste": l.piste,
        "competition": l.competition,
        "color": l.color,
        "bout": l.bout,
        "red": l.red,
        "green": l.green,
        "remaining_ms": l.remaining_ms.max(0),
        "running": l.running,
        "period": l.period,
        "periods": l.periods,
        "over": l.over,
    });
    store.live.entry(club.id).or_default().insert(l.piste, LiveBoard { state, at: std::time::Instant::now() });
    Ok(Json(json!({ "status": "ok" })))
}

#[derive(Deserialize)]
pub struct LiveQuery {
    /// Club.
    c: String,
    /// Piste.
    p: u32,
}

/// `GET /api/live?c=<club>&p=<piste>` : ce qu'affiche l'écran de la piste
/// (lecture publique : ce sont les scores affichés dans la salle). Le chrono
/// en marche est recalé sur l'âge de l'état.
pub async fn live(State(store): State<Shared>, Query(q): Query<LiveQuery>) -> Result<Json<Value>, ApiError> {
    let store = store.lock().await;
    let club = store.club(&q.c).ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "club inconnu".into()))?;
    let board = store.live.get(&club.id).and_then(|m| m.get(&q.p)).filter(|b| b.at.elapsed() < LIVE_TIMEOUT);
    let Some(board) = board else {
        return Ok(Json(json!({ "active": false, "piste": q.p, "club": club.name })));
    };
    let mut state = board.state.clone();
    let age = board.at.elapsed().as_millis() as i64;
    if state.get("running").and_then(Value::as_bool).unwrap_or(false) {
        let remaining = state.get("remaining_ms").and_then(Value::as_i64).unwrap_or(0);
        state["remaining_ms"] = json!((remaining - age).max(0));
    }
    state["active"] = json!(true);
    state["age_ms"] = json!(age);
    state["club"] = json!(club.name);
    Ok(Json(state))
}
