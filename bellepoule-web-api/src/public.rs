//! Publication des compétitions (compete.managecomp.org) : copie publique
//! d'une épreuve, gardée en archive même si le club supprime ou retire sa
//! compétition.
//!
//! ```text
//! <data>/public/index.json        épreuves publiées (club, adresse, options)
//! <data>/public/events.json       événements (épreuves publiées ensemble)
//! <data>/public/<id>.cotcot        copie de l'épreuve, mise à jour à chaque
//!                                 enregistrement (version web) ou envoi
//!                                 du logiciel de bureau
//! ```
//!
//! Adresses : `/<club>/<épreuve>` (lisible) et `/e/<id>` (QR code, court).
//! Un **événement** regroupe les épreuves publiées ensemble (compétitions
//! ouvertes dans la même session) : `/<club>/<événement>`, puis ses épreuves.
//! La compétition garde l'identifiant de sa publication
//! (`Contest::public_id`), dans la version web comme dans le logiciel.

use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use bp_core::contest::Contest;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::{bad_request, internal, not_found, ApiError, Shared};
use crate::auth::Client;
use crate::store::Store;

/// Une épreuve publiée.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Publication {
    /// Identifiant court (lien du QR code).
    pub id: String,
    /// Club propriétaire (seul à pouvoir la mettre à jour ou la retirer).
    pub club: String,
    pub club_name: String,
    pub club_slug: String,
    pub slug: String,
    pub title: String,
    /// « Fleuret · Dames · M15 ».
    pub subtitle: String,
    /// `aaaa-mm-jj`.
    pub date: String,
    pub location: String,
    /// `full` : noms complets ; `short` : prénom et initiale du nom.
    pub names: String,
    /// Page cachée aux moteurs de recherche.
    pub noindex: bool,
    pub created: u64,
    pub updated: u64,
    /// Événement de l'épreuve (vide : épreuve seule).
    pub event: String,
    /// Compétition de club (points activés) : nom du club dans le titre,
    /// étiquette « Compétition club », comptée dans le classement club.
    pub club_contest: bool,
}

/// Événement publié : plusieurs épreuves d'une même journée (ou d'un
/// week-end), ouvertes ensemble dans le logiciel ou la version web.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PublicEvent {
    pub id: String,
    pub club: String,
    pub club_name: String,
    pub club_slug: String,
    pub slug: String,
    pub name: String,
    /// Catégories des épreuves (« M13, M15, M17 »).
    pub categories: String,
    /// `aaaa-mm-jj` (première épreuve).
    pub date: String,
    pub location: String,
    pub noindex: bool,
    pub created: u64,
    pub updated: u64,
}

/// Ce que la publication reprend de la compétition.
pub struct PublicMeta {
    pub public_id: Option<String>,
    pub public_event: Option<String>,
    title: String,
    subtitle: String,
    date: String,
    location: String,
    /// Compétition de club (points activés pour le classement club).
    club_contest: bool,
}

impl PublicMeta {
    pub fn of(c: &Contest) -> PublicMeta {
        let subtitle = [c.weapon.label(), c.gender_label(), c.category.display_image(), c.label.clone()]
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        PublicMeta {
            public_id: c.public_id.clone(),
            public_event: c.public_event.clone(),
            title: c.name.trim().to_string(),
            subtitle,
            date: format!("{:04}-{:02}-{:02}", c.year, c.month, c.day),
            location: c.location.trim().to_string(),
            club_contest: c.points.enabled,
        }
    }
}

/// Options choisies à la publication.
#[derive(Clone, Default, Deserialize)]
pub struct PublishOptions {
    #[serde(default)]
    pub names: Option<String>,
    #[serde(default)]
    pub noindex: Option<bool>,
    /// Version web : publier toutes les compétitions ouvertes dans un
    /// événement (`{ name, categories }`).
    #[serde(default)]
    pub event: Option<EventOptions>,
}

#[derive(Clone, Default, Deserialize)]
pub struct EventOptions {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub categories: String,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Adresse lisible : minuscules sans accents, mots séparés par des tirets.
pub fn slugify(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let mapped: &str = match c {
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' | 'À' | 'Á' | 'Â' | 'Ä' | 'Ã' | 'Å' => "a",
            'ç' | 'Ç' => "c",
            'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => "e",
            'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => "i",
            'ñ' | 'Ñ' => "n",
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' | 'Ò' | 'Ó' | 'Ô' | 'Ö' | 'Õ' => "o",
            'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => "u",
            'ý' | 'ÿ' | 'Ý' => "y",
            'œ' | 'Œ' => "oe",
            'æ' | 'Æ' => "ae",
            _ => "",
        };
        if !mapped.is_empty() {
            out.push_str(mapped);
        } else if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "competition".into() } else { out.chars().take(80).collect::<String>().trim_matches('-').to_string() }
}

/// Identifiant court (6 caractères, lettres et chiffres).
fn short_id() -> String {
    const ALPHABET: &[u8] = b"23456789abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ";
    uuid::Uuid::new_v4().as_bytes().iter().take(6).map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect()
}

/// Racine des pages publiques vue par le navigateur : `compete.<domaine>`
/// derrière `app.`, `api.` ou `arbitre.<domaine>`, sinon `/compete/` sur le
/// même site (serveur local).
pub fn public_base(headers: &HeaderMap) -> String {
    let host = headers.get("host").and_then(|h| h.to_str().ok()).unwrap_or("localhost");
    let proto = headers.get("x-forwarded-proto").and_then(|h| h.to_str().ok()).filter(|p| !p.is_empty()).unwrap_or("http");
    if host.starts_with("compete.") {
        return format!("{proto}://{host}/");
    }
    for prefix in ["app.", "api.", "arbitre.", "live."] {
        if let Some(domain) = host.strip_prefix(prefix) {
            return format!("{proto}://compete.{domain}/");
        }
    }
    format!("{proto}://{host}/compete/")
}

impl Store {
    fn public_dir(&self) -> PathBuf {
        self.dir.join("public")
    }

    /// Épreuves publiées (lues au premier accès).
    pub fn publications(&mut self) -> &mut Vec<Publication> {
        if self.publications.is_none() {
            let list = std::fs::read(self.public_dir().join("index.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            self.publications = Some(list);
        }
        self.publications.as_mut().unwrap()
    }

    fn save_publications(&mut self) -> std::io::Result<()> {
        let dir = self.public_dir();
        std::fs::create_dir_all(&dir)?;
        let data = serde_json::to_vec_pretty(self.publications())?;
        let tmp = dir.join("index.json.tmp");
        std::fs::write(&tmp, data)?;
        std::fs::rename(tmp, dir.join("index.json"))
    }

    /// Événements publiés (lus au premier accès).
    pub fn public_events(&mut self) -> &mut Vec<PublicEvent> {
        if self.public_events.is_none() {
            let list = std::fs::read(self.public_dir().join("events.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            self.public_events = Some(list);
        }
        self.public_events.as_mut().unwrap()
    }

    pub(crate) fn save_events(&mut self) -> std::io::Result<()> {
        let dir = self.public_dir();
        std::fs::create_dir_all(&dir)?;
        let data = serde_json::to_vec_pretty(self.public_events())?;
        let tmp = dir.join("events.json.tmp");
        std::fs::write(&tmp, data)?;
        std::fs::rename(tmp, dir.join("events.json"))
    }

    /// Identifiant court libre (épreuves et événements).
    fn free_id(&mut self) -> String {
        loop {
            let id = short_id();
            if !self.publications().iter().any(|p| p.id == id) && !self.public_events().iter().any(|e| e.id == id) {
                return id;
            }
        }
    }

    /// Adresse du club, unique entre les clubs.
    pub(crate) fn club_slug(&mut self, client: &Client) -> String {
        if let Some(s) = self.publications().iter().find(|p| p.club == client.id).map(|p| p.club_slug.clone()) {
            return s;
        }
        if let Some(s) = self.public_events().iter().find(|e| e.club == client.id).map(|e| e.club_slug.clone()) {
            return s;
        }
        let base = slugify(&client.name);
        let taken = self.publications().iter().any(|p| p.club != client.id && p.club_slug == base) || self.public_events().iter().any(|e| e.club != client.id && e.club_slug == base);
        if taken { format!("{base}-{}", client.id.chars().take(4).collect::<String>()) } else { base }
    }

    /// Adresse libre dans le club (épreuves et événements partagent l'espace).
    fn free_slug(&mut self, club_slug: &str, id: &str, base: String) -> String {
        let taken = self.publications().iter().any(|p| p.id != id && p.club_slug == club_slug && p.slug == base)
            || self.public_events().iter().any(|e| e.id != id && e.club_slug == club_slug && e.slug == base);
        if taken { format!("{base}-{}", id.to_lowercase()) } else { base }
    }

    /// Crée (ou met à jour) l'événement `id` du club.
    pub fn upsert_event(&mut self, client: &Client, id: Option<&str>, name: &str, categories: &str, date: &str, location: &str, noindex: Option<bool>) -> std::io::Result<PublicEvent> {
        let index = match id.and_then(|id| self.public_events().iter().position(|e| e.id == id && e.club == client.id)) {
            Some(i) => i,
            None => {
                let id = self.free_id();
                let club_slug = self.club_slug(client);
                let created = now();
                self.public_events().push(PublicEvent { id, club: client.id.clone(), club_slug, created, ..Default::default() });
                self.public_events().len() - 1
            }
        };
        let mut e = self.public_events()[index].clone();
        e.club_name = client.name.clone();
        if !name.trim().is_empty() {
            e.name = name.trim().to_string();
        }
        e.categories = categories.trim().to_string();
        if !date.is_empty() {
            e.date = date.to_string();
        }
        if !location.trim().is_empty() {
            e.location = location.trim().to_string();
        }
        if let Some(n) = noindex {
            e.noindex = n;
        }
        if e.slug.is_empty() {
            let base = slugify(&format!("{} {}", e.date, e.name));
            e.slug = self.free_slug(&e.club_slug.clone(), &e.id.clone(), base);
        }
        e.updated = now();
        self.public_events()[index] = e.clone();
        self.save_events()?;
        Ok(e)
    }

    /// Copie publique d'une épreuve.
    pub fn publication_file(&self, id: &str) -> PathBuf {
        self.public_dir().join(format!("{id}.cotcot"))
    }

    /// Publie (ou met à jour) l'épreuve : copie, adresse, options. Une
    /// publication d'un autre club n'est jamais touchée.
    pub fn publish(&mut self, client: &Client, meta: &PublicMeta, xml: &str, options: Option<&PublishOptions>) -> std::io::Result<Publication> {
        let existing = meta.public_id.as_deref().and_then(|id| self.publications().iter().position(|p| p.id == id && p.club == client.id));
        let index = match existing {
            Some(i) => i,
            None => {
                let id = self.free_id();
                let club_slug = self.club_slug(client);
                let created = now();
                self.publications().push(Publication { id, club: client.id.clone(), club_slug, names: "full".into(), created, ..Default::default() });
                self.publications().len() - 1
            }
        };
        let mut p = self.publications()[index].clone();
        p.club_name = client.name.clone();
        p.club_contest = meta.club_contest;
        // Compétition de club : le nom du club dans le titre.
        p.title = if meta.club_contest && !meta.title.to_lowercase().contains(&client.name.trim().to_lowercase()) && !client.name.trim().is_empty() {
            format!("{} — {}", client.name.trim(), meta.title)
        } else {
            meta.title.clone()
        };
        p.subtitle = meta.subtitle.clone();
        p.date = meta.date.clone();
        p.location = meta.location.clone();
        if p.slug.is_empty() {
            let base = slugify(&format!("{} {} {}", p.date, p.title, p.subtitle));
            p.slug = self.free_slug(&p.club_slug.clone(), &p.id.clone(), base);
        }
        // Événement de l'épreuve (s'il existe et appartient au club).
        p.event = meta
            .public_event
            .clone()
            .filter(|e| self.public_events().iter().any(|x| x.id == *e && x.club == client.id))
            .unwrap_or_default();
        if let Some(o) = options {
            if let Some(names) = o.names.as_deref().filter(|n| *n == "full" || *n == "short") {
                p.names = names.to_string();
            }
            if let Some(noindex) = o.noindex {
                p.noindex = noindex;
            }
        }
        p.updated = now();
        std::fs::create_dir_all(self.public_dir())?;
        let file = self.publication_file(&p.id);
        let tmp = file.with_extension("cotcot.tmp");
        std::fs::write(&tmp, xml)?;
        std::fs::rename(tmp, file)?;
        self.publications()[index] = p.clone();
        self.save_publications()?;
        Ok(p)
    }

    /// Point de scission franchi : si la compétition est publiée, ses
    /// divisions le sont aussi, dans son événement (créé s'il n'existe pas).
    pub fn publish_split(&mut self, client: &Client, source: &str, divisions: &[String]) -> std::io::Result<()> {
        let Some((public_id, event, name, category, date, location)) = self.get(client, source).map(|c| {
            (c.public_id.clone(), c.public_event.clone(), c.name.clone(), c.category.display_image(), format!("{:04}-{:02}-{:02}", c.year, c.month, c.day), c.location.clone())
        }) else {
            return Ok(());
        };
        let Some(published) = public_id.and_then(|id| self.publications().iter().find(|p| p.id == id && p.club == client.id).cloned()) else { return Ok(()) };
        let options = PublishOptions { names: Some(published.names.clone()), noindex: Some(published.noindex), event: None };
        let existing = event.filter(|e| self.public_events().iter().any(|x| x.id == *e && x.club == client.id));
        let event = match existing {
            Some(e) => e,
            None => {
                let e = self.upsert_event(client, None, &name, &category, &date, &location, Some(published.noindex))?;
                let eid = e.id.clone();
                // La compétition rejoint l'événement (sa copie publique suit).
                if let Some(r) = self.update(client, source, |c| c.public_event = Some(eid)) {
                    r?;
                }
                e.id
            }
        };
        for id in divisions {
            let eid = event.clone();
            if let Some(r) = self.update(client, id, |c| c.public_event = Some(eid)) {
                r?;
            }
            let Some((meta, xml)) = self.get_mut(client, id).map(|c| (PublicMeta::of(c), c.to_xml())) else { continue };
            let p = self.publish(client, &meta, &xml, Some(&options))?;
            let pid = p.id.clone();
            if let Some(r) = self.update(client, id, |c| c.public_id = Some(pid)) {
                r?;
            }
        }
        Ok(())
    }

    /// Après chaque enregistrement d'une compétition publiée : la copie
    /// publique suit.
    pub fn refresh_publication(&mut self, client: &Client, meta: &PublicMeta, xml: &str) {
        let Some(id) = meta.public_id.as_deref() else { return };
        if self.publications().iter().any(|p| p.id == id && p.club == client.id) {
            let _ = self.publish(client, meta, xml, None);
        }
    }

    /// Retire une épreuve publiée (copie effacée).
    pub fn unpublish(&mut self, client: &Client, id: &str) -> std::io::Result<bool> {
        let before = self.publications().len();
        self.publications().retain(|p| !(p.id == id && p.club == client.id));
        if self.publications().len() == before {
            return Ok(false);
        }
        let _ = std::fs::remove_file(self.publication_file(id));
        self.save_publications()?;
        // Événement vide : retiré aussi.
        let used: Vec<String> = self.publications().iter().map(|p| p.event.clone()).collect();
        let before = self.public_events().len();
        self.public_events().retain(|e| e.club != client.id || used.contains(&e.id));
        if self.public_events().len() != before {
            self.save_events()?;
        }
        Ok(true)
    }
}

fn event_json(base: &str, e: &PublicEvent) -> Value {
    json!({
        "id": e.id,
        "name": e.name,
        "categories": e.categories,
        "url": format!("{base}{}/{}", e.club_slug, e.slug),
        "short_url": format!("{base}e/{}", e.id),
    })
}

/// État d'une publication pour les applications.
fn status(store: &mut Store, headers: &HeaderMap, p: Option<&Publication>) -> Value {
    let base = public_base(headers);
    let event = p.filter(|p| !p.event.is_empty()).and_then(|p| store.public_events().iter().find(|e| e.id == p.event).cloned()).map(|e| event_json(&base, &e));
    match p {
        None => json!({ "published": false, "base": base }),
        Some(p) => json!({
            "published": true,
            "id": p.id,
            "url": format!("{base}{}/{}", p.club_slug, p.slug),
            "short_url": format!("{base}e/{}", p.id),
            "club_url": format!("{base}{}", p.club_slug),
            "names": p.names,
            "noindex": p.noindex,
            "updated": p.updated,
            "event": event,
        }),
    }
}

// ------------------------------------------------- version web

/// `GET /api/contests/{id}/publication`.
pub async fn get_contest(State(store): State<Shared>, client: Client, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let public_id = store.get(&client, &id).ok_or_else(not_found)?.public_id.clone();
    let p = public_id.and_then(|pid| store.publications().iter().find(|p| p.id == pid && p.club == client.id).cloned());
    let mut v = status(&mut store, &headers, p.as_ref());
    // Compétitions ouvertes (onglets) : proposées dans un événement.
    let open = store.organised_contests(&client);
    let mut list = Vec::new();
    let mut categories: Vec<String> = Vec::new();
    for cid in &open {
        if let Some(c) = store.get(&client, cid) {
            let cat = c.category.display_image();
            if !cat.is_empty() && !categories.contains(&cat) {
                categories.push(cat.clone());
            }
            list.push(json!({ "id": cid, "name": c.name, "category": cat }));
        }
    }
    let first = open.first().and_then(|cid| store.get(&client, cid)).map(|c| c.name.clone()).unwrap_or_default();
    v["open"] = json!(list);
    v["event_defaults"] = json!({ "name": first, "categories": categories.join(", ") });
    Ok(Json(v))
}

/// `POST /api/contests/{id}/publication` `{ names, noindex }` : publie
/// l'épreuve, ou change ses options.
pub async fn publish_contest(State(store): State<Shared>, client: Client, headers: HeaderMap, Path(id): Path<String>, Json(o): Json<PublishOptions>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    store.get(&client, &id).ok_or_else(not_found)?;
    // Événement : toutes les compétitions ouvertes, publiées ensemble.
    let ids: Vec<String> = match &o.event {
        Some(_) => {
            let mut open = store.organised_contests(&client);
            if !open.contains(&id) {
                open.insert(0, id.clone());
            }
            open
        }
        None => vec![id.clone()],
    };
    if let Some(ev) = &o.event {
        let existing = ids
            .iter()
            .filter_map(|cid| store.get(&client, cid).and_then(|c| c.public_event.clone()))
            .find(|e| !e.is_empty());
        let (date, location) = ids
            .iter()
            .filter_map(|cid| store.get(&client, cid).map(|c| (format!("{:04}-{:02}-{:02}", c.year, c.month, c.day), c.location.clone())))
            .min()
            .unwrap_or_default();
        let e = store.upsert_event(&client, existing.as_deref(), &ev.name, &ev.categories, &date, &location, o.noindex).map_err(internal)?;
        for cid in &ids {
            let eid = e.id.clone();
            store.update(&client, cid, |c| c.public_event = Some(eid)).ok_or_else(not_found)?.map_err(internal)?;
        }
    }
    let mut mine = None;
    for cid in &ids {
        let (meta, xml) = {
            let c = store.get_mut(&client, cid).ok_or_else(not_found)?;
            (PublicMeta::of(c), c.to_xml())
        };
        let p = store.publish(&client, &meta, &xml, Some(&o)).map_err(internal)?;
        let pid = p.id.clone();
        store.update(&client, cid, |c| c.public_id = Some(pid)).ok_or_else(not_found)?.map_err(internal)?;
        if *cid == id {
            mine = Some(p);
        }
    }
    Ok(Json(status(&mut store, &headers, mine.as_ref())))
}

/// `DELETE /api/contests/{id}/publication` : l'épreuve n'est plus publiée.
pub async fn unpublish_contest(State(store): State<Shared>, client: Client, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let public_id = store.get(&client, &id).ok_or_else(not_found)?.public_id.clone();
    if let Some(pid) = public_id {
        store.unpublish(&client, &pid).map_err(internal)?;
    }
    store.update(&client, &id, |c| {
        c.public_id = None;
        c.public_event = None;
    })
    .ok_or_else(not_found)?
    .map_err(internal)?;
    Ok(Json(status(&mut store, &headers, None)))
}

// ------------------------------------------------- logiciel de bureau

#[derive(Deserialize)]
pub struct DesktopPublish {
    /// Fichier de la compétition (`.cotcot`).
    xml: String,
    /// Publication demandée (bouton « Publier ») : une publication retirée
    /// entre-temps est recréée. Sinon (mise à jour automatique) : refusée.
    #[serde(default)]
    renew: bool,
    #[serde(flatten)]
    options: PublishOptions,
}

/// `PUT /api/publication` (clé d'API du club) : le logiciel publie ou met à
/// jour une épreuve ; l'identifiant renvoyé est gardé dans la compétition.
pub async fn publish_desktop(State(store): State<Shared>, client: Client, headers: HeaderMap, Json(d): Json<DesktopPublish>) -> Result<Json<Value>, ApiError> {
    let mut contest = Contest::new(None, false);
    contest.load_xml_str(&d.xml).map_err(|e| bad_request(e.to_string()))?;
    let mut meta = PublicMeta::of(&contest);
    let mut store = store.lock().await;
    if let Some(id) = meta.public_id.clone() {
        if !store.publications().iter().any(|p| p.id == id && p.club == client.id) {
            if !d.renew {
                return Err(ApiError(StatusCode::GONE, "publication retirée".into()));
            }
            meta.public_id = None;
        }
    }
    let p = store.publish(&client, &meta, &d.xml, Some(&d.options)).map_err(internal)?;
    Ok(Json(status(&mut store, &headers, Some(&p))))
}

#[derive(Deserialize)]
pub struct DesktopEvent {
    #[serde(default)]
    id: Option<String>,
    name: String,
    #[serde(default)]
    categories: String,
    #[serde(default)]
    date: String,
    #[serde(default)]
    location: String,
    #[serde(default)]
    noindex: Option<bool>,
}

/// `PUT /api/publication/event` (clé d'API du club) : le logiciel crée (ou
/// met à jour) l'événement de ses compétitions ouvertes ; il en garde
/// l'identifiant dans chaque compétition avant de les publier.
pub async fn event_desktop(State(store): State<Shared>, client: Client, headers: HeaderMap, Json(d): Json<DesktopEvent>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let e = store.upsert_event(&client, d.id.as_deref(), &d.name, &d.categories, &d.date, &d.location, d.noindex).map_err(internal)?;
    Ok(Json(event_json(&public_base(&headers), &e)))
}

/// `GET /api/publication/{id}` (clé d'API du club) : état d'une publication.
pub async fn get_desktop(State(store): State<Shared>, client: Client, headers: HeaderMap, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let p = store.publications().iter().find(|p| p.id == id && p.club == client.id).cloned();
    Ok(Json(status(&mut store, &headers, p.as_ref())))
}

/// `DELETE /api/publication/{id}` (clé d'API du club).
pub async fn unpublish_desktop(State(store): State<Shared>, client: Client, headers: HeaderMap, Path(id): Path<String>) -> Result<(StatusCode, Json<Value>), ApiError> {
    let mut store = store.lock().await;
    if !store.unpublish(&client, &id).map_err(internal)? {
        return Err(not_found());
    }
    Ok((StatusCode::OK, Json(status(&mut store, &headers, None))))
}

// ------------------------------------------------- publications du club

/// `GET /api/publications` : pages publiques du club (épreuves et
/// événements), y compris celles dont la compétition n'existe plus.
pub async fn list_publications(State(store): State<Shared>, client: Client, headers: HeaderMap) -> Json<Value> {
    let mut store = store.lock().await;
    let base = public_base(&headers);
    // Épreuves encore liées à une compétition de la version web.
    let linked: Vec<String> = store.list(&client).filter_map(|(_, c)| c.public_id.clone()).collect();
    let mut publications: Vec<Publication> = store.publications().iter().filter(|p| p.club == client.id).cloned().collect();
    publications.sort_by(|a, b| b.date.cmp(&a.date).then(a.title.cmp(&b.title)));
    let mut events: Vec<PublicEvent> = store.public_events().iter().filter(|e| e.club == client.id).cloned().collect();
    events.sort_by(|a, b| b.date.cmp(&a.date));
    Json(json!({
        "publications": publications.iter().map(|p| json!({
            "id": p.id,
            "title": p.title,
            "subtitle": p.subtitle,
            "date": p.date,
            "url": format!("{base}{}/{}", p.club_slug, p.slug),
            "event": p.event,
            "club_contest": p.club_contest,
            "linked": linked.contains(&p.id),
        })).collect::<Vec<_>>(),
        "events": events.iter().map(|e| {
            let mut v = event_json(&base, e);
            v["date"] = json!(e.date);
            v["count"] = json!(publications.iter().filter(|p| p.event == e.id).count());
            v
        }).collect::<Vec<_>>(),
    }))
}

/// `DELETE /api/publication/event/{id}` : retire un événement et toutes ses épreuves.
pub async fn unpublish_event(State(store): State<Shared>, client: Client, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let mut store = store.lock().await;
    if !store.public_events().iter().any(|e| e.id == id && e.club == client.id) {
        return Err(not_found());
    }
    let ids: Vec<String> = store.publications().iter().filter(|p| p.event == id && p.club == client.id).map(|p| p.id.clone()).collect();
    for pid in &ids {
        store.unpublish(&client, pid).map_err(internal)?;
    }
    // Compétitions encore ouvertes : plus liées à ces pages.
    let contests: Vec<String> = store.list(&client).filter(|(_, c)| c.public_id.as_ref().is_some_and(|p| ids.contains(p)) || c.public_event.as_deref() == Some(id.as_str())).map(|(cid, _)| cid.clone()).collect();
    for cid in contests {
        store.update(&client, &cid, |c| {
            c.public_id = None;
            c.public_event = None;
        });
    }
    store.public_events().retain(|e| !(e.id == id && e.club == client.id));
    store.save_events().map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}
