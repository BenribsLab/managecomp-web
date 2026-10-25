//! Pages publiques des épreuves (compete.managecomp.org) : accueil (toutes
//! les épreuves publiées), page d'un club, page d'un événement (ses
//! épreuves), page d'une épreuve (formule,
//! tireurs, poules, tableaux, classement), avec filtres par nom et par club
//! et l'encadré « Mon tireur » (`compete/app.js`).
//!
//! Pages HTML fabriquées ici, légères (téléphones, réseau d'un gymnase) ;
//! sur le serveur local elles sont sous `/compete/`.

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use bp_core::contest::Contest;
use bp_core::i18n::tr;
use bp_core::player::{Player, PlayerRef, PlayerStore};
use bp_core::rounds::classification::GeneralClassificationStage;
use bp_core::rounds::pool::PoolSupervisorStage;
use bp_core::rounds::table::supervisor::TableSupervisor;

use crate::api::Shared;
use crate::public::{public_base, PublicEvent, Publication};

const CSS: &str = include_str!("compete/style.css");
const JS: &str = include_str!("compete/app.js");
/// Version des fichiers statiques (adresse changée à chaque modification).
const ASSETS: &str = "3";

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// `2026-10-18` → `18/10/2026`.
fn date_fr(iso: &str) -> String {
    let p: Vec<&str> = iso.split('-').collect();
    match p[..] {
        [y, m, d] => format!("{d}/{m}/{y}"),
        _ => iso.to_string(),
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, Html(layout("", "Page introuvable", true, "<main class=\"wrap\"><h1>Page introuvable</h1><p>Cette épreuve n'existe pas ou n'est plus publiée.</p></main>".into()))).into_response()
}

/// Gabarit commun.
fn layout(base: &str, title: &str, noindex: bool, body: String) -> String {
    let robots = if noindex { "<meta name=\"robots\" content=\"noindex\">" } else { "" };
    format!(
        "<!doctype html><html lang=\"fr\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">{robots}\
         <title>{t}</title><link rel=\"stylesheet\" href=\"{base}style.css?v={ASSETS}\"><script src=\"{base}app.js?v={ASSETS}\" defer></script></head>\
         <body><header class=\"top\"><a class=\"brand\" href=\"{base}\"><b>Manage</b>Comp <span>résultats</span></a></header>{body}\
         <footer class=\"foot\"><a href=\"https://managecomp.org\">ManageComp</a> · logiciel libre de gestion de compétitions d'escrime</footer></body></html>",
        t = esc(title),
    )
}

fn html(body: String) -> Response {
    ([(header::CACHE_CONTROL, "no-cache")], Html(body)).into_response()
}

// ------------------------------------------------------------ noms

/// Noms affichés : complets, ou prénom et initiale du nom (« Lina M. »).
struct Names {
    short: bool,
}

impl Names {
    fn of(&self, p: &Player) -> String {
        let (name, first) = (p.name().trim().to_string(), p.first_name().trim().to_string());
        if first.is_empty() {
            return name;
        }
        if self.short {
            let initial = name.chars().next().map(|c| format!(" {c}.")).unwrap_or_default();
            format!("{first}{initial}")
        } else {
            format!("{name} {first}")
        }
    }
    fn get(&self, players: &PlayerStore, r: PlayerRef) -> (String, String) {
        players.get(r).map(|p| (self.of(p), p.str_attr("club").unwrap_or_default().trim().to_string())).unwrap_or_default()
    }
}

/// Attributs de filtrage d'un tireur (nom affiché, club).
fn data(name: &str, club: &str) -> String {
    format!(" data-f=\"{}\" data-c=\"{}\"", esc(name), esc(club))
}

// ------------------------------------------------------------ fichiers

pub async fn style() -> Response {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8"), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], CSS).into_response()
}

pub async fn script() -> Response {
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8"), (header::CACHE_CONTROL, "public, max-age=31536000, immutable")], JS).into_response()
}

// ------------------------------------------------------------ listes

fn cards(base: &str, list: &[Publication], with_club: bool) -> String {
    if list.is_empty() {
        return "<p class=\"muted\">Aucune épreuve publiée pour l'instant.</p>".into();
    }
    let mut out = String::from("<div class=\"cards\">");
    for p in list {
        let search = format!("{} {} {} {} {}", p.title, p.subtitle, p.club_name, p.location, date_fr(&p.date)).to_lowercase();
        out.push_str(&format!(
            "<a class=\"card\" href=\"{base}{}/{}\" data-s=\"{}\"><span class=\"date\">{}</span><strong>{}</strong><span>{}</span><span class=\"muted\">{}</span></a>",
            esc(&p.club_slug),
            esc(&p.slug),
            esc(&search),
            date_fr(&p.date),
            esc(&p.title),
            esc(&p.subtitle),
            esc(&[if with_club { p.club_name.clone() } else { String::new() }, p.location.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")),
        ));
    }
    out.push_str("</div>");
    out
}

fn listed(mut list: Vec<Publication>) -> Vec<Publication> {
    list.retain(|p| !p.noindex);
    list.sort_by(|a, b| b.date.cmp(&a.date).then(b.updated.cmp(&a.updated)));
    list
}

/// Cartes des événements, puis des épreuves publiées seules, par date.
fn listing(base: &str, events: &[PublicEvent], publications: &[Publication], with_club: bool) -> String {
    let mut events: Vec<&PublicEvent> = events.iter().filter(|e| !e.noindex).collect();
    events.sort_by(|a, b| b.date.cmp(&a.date).then(b.updated.cmp(&a.updated)));
    let alone = listed(publications.iter().filter(|p| p.event.is_empty() || !events.iter().any(|e| e.id == p.event)).cloned().collect());
    if events.is_empty() {
        return cards(base, &alone, with_club);
    }
    let mut out = String::from("<div class=\"cards\">");
    for e in &events {
        let count = publications.iter().filter(|p| p.event == e.id).count();
        let search = format!("{} {} {} {} {}", e.name, e.categories, e.club_name, e.location, date_fr(&e.date)).to_lowercase();
        out.push_str(&format!(
            "<a class=\"card event-card\" href=\"{base}{}/{}\" data-s=\"{}\"><span class=\"date\">{} · événement</span><strong>{}</strong><span>{}</span><span class=\"muted\">{}</span></a>",
            esc(&e.club_slug),
            esc(&e.slug),
            esc(&search),
            date_fr(&e.date),
            esc(&e.name),
            esc(&format!("{} · {count} épreuve{}", e.categories, if count > 1 { "s" } else { "" })),
            esc(&[if with_club { e.club_name.clone() } else { String::new() }, e.location.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")),
        ));
    }
    out.push_str("</div>");
    if !alone.is_empty() {
        out.push_str("<h2>Épreuves</h2>");
        out.push_str(&cards(base, &alone, with_club));
    }
    out
}

/// Accueil : toutes les épreuves publiées (sauf celles cachées).
pub async fn home(State(store): State<Shared>, headers: HeaderMap) -> Response {
    let base = public_base(&headers);
    let (events, list) = {
        let mut store = store.lock().await;
        (store.public_events().clone(), store.publications().clone())
    };
    let body = format!(
        "<main class=\"wrap\"><h1>Compétitions</h1><input class=\"search\" id=\"cards-q\" type=\"search\" placeholder=\"Rechercher une épreuve, un club, une ville…\">{}</main>",
        listing(&base, &events, &list, true)
    );
    html(layout(&base, "Compétitions — ManageComp", false, body))
}

/// Page d'un club : ses épreuves publiées.
pub async fn club(State(store): State<Shared>, headers: HeaderMap, Path(club): Path<String>) -> Response {
    let base = public_base(&headers);
    let (events, list) = {
        let mut store = store.lock().await;
        let events: Vec<PublicEvent> = store.public_events().iter().filter(|e| e.club_slug == club).cloned().collect();
        let list: Vec<Publication> = store.publications().iter().filter(|p| p.club_slug == club).cloned().collect();
        (events, list)
    };
    let Some(name) = list.first().map(|p| p.club_name.clone()).or_else(|| events.first().map(|e| e.club_name.clone())) else { return not_found() };
    let body = format!(
        "<main class=\"wrap\"><p class=\"crumbs\"><a href=\"{base}\">Compétitions</a></p><h1>{}</h1><input class=\"search\" id=\"cards-q\" type=\"search\" placeholder=\"Rechercher une épreuve…\">{}</main>",
        esc(&name),
        listing(&base, &events, &list, false)
    );
    html(layout(&base, &format!("{name} — ManageComp"), false, body))
}

/// Lien court du QR code.
pub async fn short(State(store): State<Shared>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    let base = public_base(&headers);
    let mut store = store.lock().await;
    if let Some(p) = store.publications().iter().find(|p| p.id == id) {
        return Redirect::to(&format!("{base}{}/{}", p.club_slug, p.slug)).into_response();
    }
    match store.public_events().iter().find(|e| e.id == id) {
        Some(e) => Redirect::to(&format!("{base}{}/{}", e.club_slug, e.slug)).into_response(),
        None => not_found(),
    }
}

// ------------------------------------------------------------ épreuve

/// Page d'un événement : ses épreuves.
fn event_group_page(base: &str, e: &PublicEvent, list: &[Publication]) -> String {
    let mut list: Vec<&Publication> = list.iter().filter(|p| p.event == e.id).collect();
    list.sort_by(|a, b| a.subtitle.cmp(&b.subtitle));
    let cards: String = list
        .iter()
        .map(|p| {
            format!(
                "<a class=\"card\" href=\"{base}{}/{}\"><strong>{}</strong><span>{}</span></a>",
                esc(&p.club_slug),
                esc(&p.slug),
                esc(&p.subtitle),
                esc(if p.title != e.name { &p.title } else { "" })
            )
        })
        .collect();
    let meta = [date_fr(&e.date), e.location.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
    format!(
        "<main class=\"wrap\"><p class=\"crumbs\"><a href=\"{base}\">Compétitions</a> › <a href=\"{base}{cs}\">{club}</a></p>\
         <h1>{name}</h1><p class=\"sub\">{cats}</p><p class=\"muted\">{meta}</p><h2>Épreuves</h2><div class=\"cards\">{cards}</div></main>",
        cs = esc(&e.club_slug),
        club = esc(&e.club_name),
        name = esc(&e.name),
        cats = esc(&e.categories),
        meta = esc(&meta),
    )
}

/// Réponse vide si la page n'a pas changé depuis `etag` (actualisation).
fn unchanged(headers: &HeaderMap, etag: &str) -> bool {
    headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()).is_some_and(|v| v == etag)
}

fn with_etag(etag: String, body: String) -> Response {
    ([(header::CACHE_CONTROL, "no-cache".to_string()), (header::ETAG, etag)], Html(body)).into_response()
}

pub async fn event(State(store): State<Shared>, headers: HeaderMap, Path((club, slug)): Path<(String, String)>) -> Response {
    let base = public_base(&headers);
    let (p, xml, event, links, siblings, etag) = {
        let mut store = store.lock().await;
        // Événement : la liste de ses épreuves.
        if let Some(e) = store.public_events().iter().find(|e| e.club_slug == club && e.slug == slug).cloned() {
            let list = store.publications().clone();
            let etag = format!("\"e{}-{}-{ASSETS}\"", e.updated, list.iter().filter(|p| p.event == e.id).map(|p| p.updated).max().unwrap_or(0));
            if unchanged(&headers, &etag) {
                return StatusCode::NOT_MODIFIED.into_response();
            }
            return with_etag(etag, layout(&base, &e.name, e.noindex, event_group_page(&base, &e, &list)));
        }
        let Some(p) = store.publications().iter().find(|p| p.club_slug == club && p.slug == slug).cloned() else { return not_found() };
        let sisters = store.publications().iter().filter(|o| !p.event.is_empty() && o.event == p.event && o.id != p.id).map(|o| o.updated).max().unwrap_or(0);
        let etag = format!("\"p{}-{sisters}-{}-{ASSETS}\"", p.updated, p.names);
        if unchanged(&headers, &etag) {
            return StatusCode::NOT_MODIFIED.into_response();
        }
        let Ok(xml) = std::fs::read_to_string(store.publication_file(&p.id)) else { return not_found() };
        let event = store.public_events().iter().find(|e| e.id == p.event).cloned();
        // Épreuves du même événement (divisions d'un point de scission).
        let links: Vec<(String, String)> = store
            .publications()
            .iter()
            .filter(|o| !p.event.is_empty() && o.event == p.event && o.club == p.club && o.id != p.id)
            .map(|o| (o.title.clone(), format!("{base}{}/{}", o.club_slug, o.slug)))
            .collect();
        // Copies des épreuves sœurs, lues seulement si la formule a une scission.
        let siblings: Vec<(String, String, String)> = if xml.contains("Scission") || xml.contains("Splitting") {
            store
                .publications()
                .iter()
                .filter(|o| !p.event.is_empty() && o.event == p.event && o.club == p.club && o.id != p.id)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .filter_map(|o| Some((o.title.clone(), format!("{base}{}/{}", o.club_slug, o.slug), std::fs::read_to_string(store.publication_file(&o.id)).ok()?)))
                .collect()
        } else {
            Vec::new()
        };
        (p, xml, event, links, siblings, etag)
    };
    let mut c = Contest::new(None, false);
    if c.load_xml_str(&xml).is_err() {
        return not_found();
    }
    let names = Names { short: p.names == "short" };
    // Épreuves du même événement (divisions d'un point de scission) : leur suite.
    let divisions: Vec<DivisionContest> = siblings
        .into_iter()
        .filter_map(|(title, url, xml)| {
            let mut contest = Contest::new(None, false);
            contest.load_xml_str(&xml).ok()?;
            Some(DivisionContest { title, url, contest })
        })
        .collect();
    let body = event_body(&base, &p, event.as_ref(), &links, &divisions, &mut c, &names);
    with_etag(etag, layout(&base, &format!("{} — {}", p.title, p.subtitle), p.noindex, body))
}

fn event_body(base: &str, p: &Publication, event: Option<&PublicEvent>, links: &[(String, String)], divisions: &[DivisionContest], c: &mut Contest, names: &Names) -> String {
    let over = c.schedule.is_over();
    let sections: Vec<(&str, &str, String)> = vec![
        ("formule", "Formule", formula(c, links, divisions)),
        ("tireurs", "Tireurs", fencers(c, names)),
        ("poules", "Poules", pools(c, names)),
        ("tableaux", "Tableaux", tables(c, names)),
        ("classement", "Classement", ranking(c, names)),
    ];
    let sections: Vec<_> = sections.into_iter().filter(|(_, _, s)| !s.is_empty()).collect();
    // Clubs des tireurs (filtre).
    let mut clubs: Vec<String> = c
        .schedule
        .checkin()
        .map(|ch| ch.fencers(&c.players).into_iter().filter_map(|r| c.players.get(r).and_then(|x| x.str_attr("club").map(|s| s.trim().to_string()))).collect())
        .unwrap_or_default();
    clubs.retain(|s| !s.is_empty());
    clubs.sort();
    clubs.dedup();
    let tabs: String = sections.iter().map(|(id, title, _)| format!("<a href=\"#{id}\" data-tab=\"{id}\">{title}</a>")).collect();
    let content: String = sections.iter().map(|(id, title, s)| format!("<section id=\"{id}\" class=\"tab\"><h2>{title}</h2>{s}</section>")).collect();
    let meta = [date_fr(&p.date), p.location.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
    format!(
        "<main class=\"wrap event\" data-live=\"{live}\" data-updated=\"{updated}\" data-event=\"{id}\">\
         <p class=\"crumbs\"><a href=\"{base}\">Compétitions</a> › <a href=\"{base}{club_slug}\">{club}</a>{event_link}</p>\
         <h1>{title}</h1><p class=\"sub\">{sub}</p><p class=\"muted\">{meta} <span id=\"updated\"></span></p>\
         <div class=\"filters\"><input id=\"q\" type=\"search\" placeholder=\"Rechercher un tireur…\" autocomplete=\"off\">\
         <select id=\"club\"><option value=\"\">Tous les clubs</option>{club_options}</select></div>\
         <div id=\"mine\"></div>\
         <nav class=\"tabs\">{tabs}</nav><div id=\"content\">{content}</div></main>",
        live = if over { 0 } else { 1 },
        updated = p.updated,
        id = esc(&p.id),
        club_slug = esc(&p.club_slug),
        club = esc(&p.club_name),
        title = esc(&p.title),
        sub = esc(&p.subtitle),
        meta = esc(&meta),
        club_options = clubs.iter().map(|c| format!("<option>{}</option>", esc(c))).collect::<String>(),
        event_link = event.map(|e| format!(" › <a href=\"{base}{}/{}\">{}</a>", esc(&e.club_slug), esc(&e.slug), esc(&e.name))).unwrap_or_default(),
    )
}

/// « 39 tireuses », « 1 équipe »…
fn people(c: &Contest, n: usize) -> String {
    let word = if c.settings.team_event {
        "équipe"
    } else if c.gender == 1 {
        "tireuse"
    } else {
        "tireur"
    };
    format!("{n} {word}{}", if n > 1 { "s" } else { "" })
}

/// « 16 qualifiées ».
fn qualified(c: &Contest, n: usize) -> String {
    let feminine = c.settings.team_event || c.gender == 1;
    format!("{n} qualifié{}{}", if feminine { "e" } else { "" }, if n > 1 { "s" } else { "" })
}

/// Compétition d'une division : nom, adresse publique, contenu.
pub struct DivisionContest {
    title: String,
    url: String,
    contest: Contest,
}

/// Formule : tireurs au départ, puis chaque phase (entrants, poules,
/// décalage, qualifiés ; tableau, exemptés) ; après un point de scission,
/// la suite de chaque division.
fn formula(c: &Contest, links: &[(String, String)], divisions: &[DivisionContest]) -> String {
    let start = c
        .schedule
        .checkin()
        .map(|ch| {
            let all = ch.fencers(&c.players);
            let present = all.iter().filter(|r| c.players.get(**r).is_some_and(|p| p.attending())).count();
            if present > 0 { present } else { all.len() }
        })
        .unwrap_or(0);
    let pool_rounds = (0..c.schedule.len()).filter(|i| c.schedule.stage(*i).is_some_and(|s| s.as_any().downcast_ref::<PoolSupervisorStage>().is_some())).count();
    let mut out = format!("<div class=\"formula\"><p class=\"big\">{}</p>", esc(&people(c, start)));
    if pool_rounds > 0 {
        out.push_str(&format!("<p>{pool_rounds} tour{} de poules</p>", if pool_rounds > 1 { "s" } else { "" }));
    }
    out.push_str(&phases(c, 0, (start > 0).then_some(start), links, divisions));
    out.push_str("</div>");
    out
}

fn block(state: &str, title: &str, lines: &[String]) -> String {
    format!(
        "<hr><div class=\"phase {state}\"><h4>{}</h4>{}</div>",
        esc(title),
        lines
            .iter()
            .map(|l| match l.strip_prefix('\u{1}') {
                Some(html) => format!("<p>{html}</p>"),
                None => format!("<p>{}</p>", esc(l)),
            })
            .collect::<String>()
    )
}

/// Phases de `c` à partir de `from`. `carry` : nombre de tireurs attendus
/// (qualifiés de la phase précédente) tant qu'une phase n'a pas commencé.
fn phases(c: &Contest, from: usize, mut carry: Option<usize>, links: &[(String, String)], divisions: &[DivisionContest]) -> String {
    let current = c.schedule.current_index();
    let over = c.schedule.is_over();
    let mut out = String::new();
    for i in from..c.schedule.len() {
        let Some(s) = c.schedule.stage(i) else { continue };
        let state = if over || i < current { "done" } else if i == current { "current" } else { "next" };
        let entrants = Some(s.base().attendees.len()).filter(|n| *n > 0);
        let mut lines: Vec<String> = Vec::new();
        if let Some(sup) = s.as_any().downcast_ref::<PoolSupervisorStage>() {
            let round = sup.round();
            let n = entrants.or(Some(round.attendees().len()).filter(|n| *n > 0)).or(carry);
            if let Some(n) = n {
                lines.push(people(c, n));
            }
            // Poules : nombre et tailles (« 3 poules de 7, 3 poules de 6 »).
            let mut sizes: Vec<u32> = round.pools().iter().map(|p| p.nb_players() as u32).filter(|n| *n > 0).collect();
            if sizes.is_empty() {
                if let Some(cfg) = round.selected() {
                    sizes = (0..cfg.nb_pool).map(|k| if k < cfg.nb_overloaded { cfg.size + 1 } else { cfg.size }).collect();
                }
            }
            if !sizes.is_empty() {
                let mut groups: Vec<(u32, usize)> = Vec::new();
                for size in &sizes {
                    match groups.iter_mut().find(|(s, _)| s == size) {
                        Some(g) => g.1 += 1,
                        None => groups.push((*size, 1)),
                    }
                }
                groups.sort_by(|a, b| b.0.cmp(&a.0));
                let detail = groups.iter().map(|(size, k)| format!("{k} poule{} de {size}", if *k > 1 { "s" } else { "" })).collect::<Vec<_>>().join(", ");
                lines.push(if groups.len() > 1 { format!("{} poules ({detail})", sizes.len()) } else { detail });
            }
            let criteria: Vec<String> = round.swapping_criteria().iter().map(|code| tr(bp_core::attribute::desc(code).user_name).to_lowercase()).collect();
            if !criteria.is_empty() {
                lines.push(format!("Décalage par : {}", criteria.join(", ")));
            }
            let q = round.nb_qualified().map(|q| q as usize).or(n);
            if let Some(q) = q {
                lines.push(qualified(c, q));
            }
            carry = q;
            out.push_str(&block(state, &format!("Poules — {}", round.name), &lines));
        } else if let Some(sup) = s.as_any().downcast_ref::<TableSupervisor>() {
            let n = entrants.or(carry);
            let title = match n {
                Some(n) => format!("Élimination directe : {}", people(c, n)),
                None => "Élimination directe".to_string(),
            };
            // Premier tableau : taille (connue, sinon d'après les entrants) et exemptés.
            let size = sup.table_sets().first().and_then(|set| set.tables().iter().map(|t| t.size()).max()).map(|s| s as usize).filter(|s| *s > 1).or(n.map(|n| n.max(2).next_power_of_two()));
            let feminine = c.gender == 1 && !c.settings.team_event;
            if let Some(size) = size {
                // Exemptés : qualifiés d'office pour le tableau suivant (pas jusqu'à la finale).
                let exempted = n.map(|n| size.saturating_sub(n)).unwrap_or(0);
                let to_final = s.base().nb_qualified.is_none();
                let e = if feminine { "e" } else { "" };
                let pl = if exempted > 1 { "s" } else { "" };
                lines.push(match (exempted > 0, to_final) {
                    (true, true) => format!("Tableau de {size} avec {exempted} qualifié{e}{pl} d'office, puis tableau de {} jusqu'à la finale", size / 2),
                    (true, false) => format!("Tableau de {size} avec {exempted} qualifié{e}{pl} d'office pour le tableau de {}", size / 2),
                    (false, true) => format!("Tableau de {size} jusqu'à la finale"),
                    (false, false) => format!("Tableau de {size}"),
                });
            }
            if let Some(q) = s.base().nb_qualified {
                lines.push(qualified(c, q as usize));
            }
            carry = s.base().nb_qualified.map(|q| q as usize).or(n);
            out.push_str(&block(state, &title, &lines));
        } else if let Some(split) = s.as_any().downcast_ref::<bp_core::rounds::splitting::Splitting>() {
            if let Some(n) = entrants.or(carry) {
                lines.push(people(c, n));
            }
            // Lignes déjà en HTML (liens vers les divisions) : marquées.
            lines.extend(split_lines(c, split, s.name(), links).into_iter().map(|h| format!("\u{1}{h}")));
            out.push_str(&block(state, &tr("Splitting"), &lines));
            let used = split.divisions_in_use(&c.players);
            if !used.is_empty() {
                // La suite de chaque division (sa propre compétition).
                for d in &used {
                    let label = split.division_name(*d);
                    let count = split.exported_players(&c.players, *d).len();
                    let target = if c.name.trim().is_empty() { label.clone() } else { format!("{} - {}", c.name.trim(), label.trim()) };
                    let found = divisions.iter().find(|x| x.title == target);
                    let head = match found {
                        Some(x) => format!("<a href=\"{}\">{}</a>", esc(&x.url), esc(&label)),
                        None => esc(&label),
                    };
                    out.push_str(&format!("<div class=\"division\"><h4 class=\"division-head\">{head}</h4>"));
                    if let Some(x) = found {
                        out.push_str(&phases(&x.contest, 1, Some(count), &[], &[]));
                    }
                    out.push_str("</div>");
                }
                let remaining = split.remaining(&c.players).len();
                if remaining == 0 {
                    // Plus personne ici : la suite est dans les divisions.
                    break;
                }
                out.push_str(&format!("<hr><p class=\"division-head\">Suite de cette épreuve : {}</p>", esc(&people(c, remaining))));
                carry = Some(remaining);
            } else {
                carry = entrants.or(carry);
            }
        } else {
            let kind = s.kind();
            // Appel, composition des poules, classement : déjà dits.
            if matches!(kind, bp_core::stage::StageKind::Checkin | bp_core::stage::StageKind::PoolAllocator | bp_core::stage::StageKind::GeneralClassification) {
                continue;
            }
            if let Some(n) = entrants.or(carry) {
                lines.push(people(c, n));
            }
            out.push_str(&block(state, &format!("{} {}", tr(kind.class_name()), s.name()).trim().to_string(), &lines));
        }
    }
    out
}

/// Point de scission : les divisions (épreuves à part) et ceux qui restent ;
/// la répartition faite, sinon la règle de la formule.
/// Lignes en HTML : le nom d'une division mène à sa page publique (même
/// événement, nom « compétition - division » comme le logiciel le donne).
fn split_lines(c: &Contest, split: &bp_core::rounds::splitting::Splitting, stage_name: String, links: &[(String, String)]) -> Vec<String> {
    use bp_core::rounds::splitting::{Criterion, Size, SplitMode};
    let used = split.divisions_in_use(&c.players);
    if !used.is_empty() {
        let only = split.config.mode == SplitMode::Manual && split.config.divisions.len() <= 1 && split.config.divisions.first().is_none_or(|d| d.name == "Division 2");
        let mut lines: Vec<String> = used
            .iter()
            .map(|d| {
                let label = if only { stage_name.clone() } else { split.division_name(*d) };
                let target = if c.name.trim().is_empty() { label.clone() } else { format!("{} - {}", c.name.trim(), label.trim()) };
                let name = match links.iter().find(|(title, _)| *title == target) {
                    Some((_, url)) => format!("<a href=\"{}\">{}</a>", esc(url), esc(&split.division_name(*d))),
                    None => esc(&split.division_name(*d)),
                };
                format!("{name} : {} → épreuve à part", esc(&people(c, split.exported_players(&c.players, *d).len())))
            })
            .collect();
        let remaining = split.remaining(&c.players).len();
        if remaining > 0 {
            lines.push(esc(&format!("Restent : {}", people(c, remaining))));
        }
        return lines;
    }
    let config = &split.config;
    let size = |s: &Size| match s {
        Size::Count(n) => n.to_string(),
        Size::Percent(p) => format!("{p} %"),
    };
    let mut lines = Vec::new();
    match config.mode {
        SplitMode::Manual => {
            let names: Vec<String> = (1..=config.divisions.len() as u32).map(|d| split.division_name(d)).collect();
            lines.push(format!("Divisions choisies à la main : {}", names.join(", ")));
        }
        SplitMode::Ranking => {
            if let Some(k) = &config.keep {
                lines.push(format!("Les {} premiers restent", size(k)));
            }
            let mut first = config.keep.is_none();
            for (i, d) in config.divisions.iter().enumerate() {
                let name = split.division_name(i as u32 + 1);
                lines.push(match &d.size {
                    Some(s) if first => format!("{name} : les {} premiers", size(s)),
                    Some(s) => format!("{name} : les {} suivants", size(s)),
                    None => format!("{name} : le reste"),
                });
                first = false;
            }
        }
        SplitMode::Criteria => {
            for (i, d) in config.divisions.iter().enumerate() {
                let name = split.division_name(i as u32 + 1);
                let rule = match &d.criterion {
                    Some(Criterion::Category { values }) => values.join(", "),
                    Some(Criterion::BirthYears { years }) => format!("nés en {}", years.iter().map(|y| y.to_string()).collect::<Vec<_>>().join(", ")),
                    Some(Criterion::BornBefore { year }) => format!("nés en {year} ou avant"),
                    Some(Criterion::BornFrom { year }) => format!("nés en {year} ou après"),
                    None => "—".into(),
                };
                lines.push(format!("{name} : {rule}"));
            }
            lines.push("Les autres restent".into());
        }
    }
    lines.into_iter().map(|l| esc(&l)).collect()
}

/// Tireurs présents.
fn fencers(c: &Contest, names: &Names) -> String {
    let Some(ch) = c.schedule.checkin() else { return String::new() };
    let mut list: Vec<(String, String)> = ch
        .fencers(&c.players)
        .into_iter()
        .filter(|r| c.players.get(*r).is_some_and(|p| p.attending()))
        .map(|r| names.get(&c.players, r))
        .collect();
    if list.is_empty() {
        return String::new();
    }
    list.sort_by_key(|(n, _)| n.to_uppercase());
    let rows: String = list.iter().map(|(n, club)| format!("<tr{}><td class=\"name\">{}</td><td class=\"muted\">{}</td></tr>", data(n, club), esc(n), esc(club))).collect();
    format!("<p class=\"muted\">{} présents</p><table class=\"list\"><tbody>{rows}</tbody></table>", list.len())
}

/// Poules de chaque tour : grille, piste, heure, arbitre.
fn pools(c: &Contest, names: &Names) -> String {
    let mut out = String::new();
    let rounds: Vec<usize> = (0..c.schedule.len()).filter(|i| c.schedule.stage(*i).is_some_and(|s| s.as_any().downcast_ref::<PoolSupervisorStage>().is_some())).collect();
    for i in &rounds {
        let Some(stage) = c.schedule.stage(*i).and_then(|s| s.as_any().downcast_ref::<PoolSupervisorStage>()) else { continue };
        let round = stage.round();
        if round.pools().is_empty() {
            continue;
        }
        if rounds.len() > 1 {
            out.push_str(&format!("<h3>{}</h3>", esc(&round.name)));
        }
        for pool in round.pools() {
            let sheet = pool.sheet(&c.players, c.referees.store(), false);
            let n = sheet.rows.len();
            let piste = pool.piste_label().unwrap_or_default();
            let time = pool.start_time_image().unwrap_or_default();
            let referee = sheet.referee.as_ref().map(|(n, f)| format!("{n} {}", f.clone().unwrap_or_default()).trim().to_string()).unwrap_or_default();
            let info = [
                (!piste.is_empty()).then(|| format!("Piste {piste}")),
                (!time.is_empty()).then(|| time.clone()),
                (!referee.is_empty()).then(|| format!("Arbitre : {referee}")),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            let head: String = (1..=n).map(|j| format!("<th>{j}</th>")).collect();
            let mut rows = String::new();
            for row in &sheet.rows {
                let (name, club) = names.get(&c.players, row.player);
                let cells: String = row
                    .cells
                    .iter()
                    .map(|cell| {
                        let class = if cell.grey { " class=\"grey\"" } else if cell.text.starts_with('V') { " class=\"win\"" } else { "" };
                        format!("<td{class}>{}</td>", if cell.grey { String::new() } else { esc(&cell.text) })
                    })
                    .collect();
                let d = row.dashboard.as_ref();
                let dash = |f: fn(&bp_core::rounds::pool::pool::Dashboard) -> &String| d.map(|d| esc(f(d))).unwrap_or_default();
                rows.push_str(&format!(
                    "<tr{}><td class=\"name\">{}<small>{}</small></td><td class=\"pos\">{}</td>{cells}<td class=\"res\">{}</td><td class=\"res\">{}</td><td class=\"res\"><b>{}</b></td></tr>",
                    data(&name, &club),
                    esc(&name),
                    esc(&club),
                    row.position,
                    dash(|d| &d.victories),
                    dash(|d| &d.indice),
                    dash(|d| &d.rank),
                ));
            }
            out.push_str(&format!(
                "<div class=\"pool\" data-title=\"{t}\" data-piste=\"{p}\" data-time=\"{h}\"><h4>{t}{over}</h4><p class=\"muted\">{info}</p>\
                 <div class=\"scroll\"><table class=\"grid\"><thead><tr><th></th><th></th>{head}<th>V</th><th>Ind</th><th>Pl</th></tr></thead><tbody>{rows}</tbody></table></div></div>",
                t = esc(pool.name()),
                p = esc(&piste),
                h = esc(&time),
                over = if pool.is_over() { " <span class=\"ok\">✔</span>" } else { "" },
                info = esc(&info),
            ));
        }
    }
    out
}

/// Matchs des tableaux, tour par tour (piste, heure, score) : affichés
/// quand un filtre est actif, et lus par « Mon tireur ».
fn table_lists(c: &Contest, names: &Names) -> String {
    let mut out = String::new();
    for i in 0..c.schedule.len() {
        let Some(sup) = c.schedule.stage(i).and_then(|s| s.as_any().downcast_ref::<TableSupervisor>()) else { continue };
        for set in sup.table_sets().iter().filter(|s| s.is_visible()) {
            let mut set_html = String::new();
            for table in set.tables() {
                let mut bouts = String::new();
                for node in table.matches() {
                    let Some(m) = set.bout(*node) else { continue };
                    if m.opponent(0).is_none() && m.opponent(1).is_none() {
                        continue;
                    }
                    let winner = m.winner();
                    let mut lines = String::new();
                    let mut keys = Vec::new();
                    let mut clubs = Vec::new();
                    for k in 0..2 {
                        let Some(f) = m.opponent(k) else {
                            lines.push_str("<div class=\"line empty\">—</div>");
                            continue;
                        };
                        let (name, club) = names.get(&c.players, f);
                        let score = if m.is_over() || m.score(f).is_some_and(|s| !s.image(m.max_score).is_empty()) { m.score_at(k).image(m.max_score) } else { String::new() };
                        lines.push_str(&format!(
                            "<div class=\"line{}\"><span class=\"who\">{}<small>{}</small></span><span class=\"score\">{}</span></div>",
                            if winner == Some(f) { " won" } else { "" },
                            esc(&name),
                            esc(&club),
                            esc(&score)
                        ));
                        keys.push(name);
                        clubs.push(club);
                    }
                    let piste = if m.piste != 0 { format!("{:02}", m.piste) } else { String::new() };
                    let time = m.start_time.map(|t| t.0.format("%H:%M").to_string()).unwrap_or_default();
                    let info = [
                        (m.number != 0).then(|| format!("Match {}", m.number)),
                        (!piste.is_empty()).then(|| format!("Piste {piste}")),
                        (!time.is_empty()).then(|| time.clone()),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · ");
                    bouts.push_str(&format!(
                        "<div class=\"bout\" data-f=\"{}\" data-c=\"{}\" data-over=\"{}\" data-piste=\"{}\" data-time=\"{}\" data-table=\"{}\"><div class=\"info muted\">{}</div>{lines}</div>",
                        esc(&keys.join("|")),
                        esc(&clubs.join("|")),
                        if m.is_over() { 1 } else { 0 },
                        esc(&piste),
                        esc(&time),
                        esc(&table.image()),
                        esc(&info)
                    ));
                }
                if !bouts.is_empty() {
                    set_html.push_str(&format!("<h4>{}</h4><div class=\"bouts\">{bouts}</div>", esc(&table.image())));
                }
            }
            if !set_html.is_empty() {
                out.push_str(&format!("<h3>{}</h3>{set_html}", esc(&set.name())));
            }
        }
    }
    out
}

/// Dimensions de l'arbre sur la page publique (téléphone).
const BRACKET: bp_core::rounds::table::layout::LayoutMetrics =
    bp_core::rounds::table::layout::LayoutMetrics { cell_width: 200.0, row_height: 30.0, header_height: 34.0, column_spacing: 10.0 };

/// Arbre d'une suite de tableaux, du plus grand tableau à la finale.
fn bracket(c: &Contest, set: &bp_core::rounds::table::table_set::TableSet, names: &Names) -> String {
    let dr = set.drawing_with_referees(&c.players, c.referees.store(), &BRACKET);
    let pos = |b: &bp_core::rounds::table::layout::Bounds| format!("left:{:.0}px;top:{:.0}px;width:{:.0}px;height:{:.0}px", b.x1, b.y1, b.x2 - b.x1, b.y2 - b.y1);
    let mut out = format!("<div class=\"bracket-scroll\"><div class=\"bracket\" style=\"width:{:.0}px;height:{:.0}px\">", dr.width, dr.height);
    out.push_str(&format!("<svg class=\"lines\" width=\"{:.0}\" height=\"{:.0}\">", dr.width, dr.height));
    for k in &dr.connectors {
        let points = k.points.iter().map(|(x, y)| format!("{x:.1},{y:.1}")).collect::<Vec<_>>().join(" ");
        out.push_str(&format!("<polyline points=\"{points}\"/>"));
    }
    out.push_str("</svg>");
    for h in &dr.headers {
        out.push_str(&format!("<div class=\"bh\" style=\"{}\">{}</div>", pos(&h.bounds), esc(&h.image)));
    }
    for cell in &dr.cells {
        if let Some(label) = &cell.match_label {
            let road = label.roadmap.as_ref().map(|r| r.text.clone()).unwrap_or_default();
            if !label.name.is_empty() || !road.is_empty() {
                out.push_str(&format!("<div class=\"bl\" style=\"{}\">{} <span>{}</span></div>", pos(&label.bounds), esc(&label.name), esc(&road)));
            }
        }
        let (name, club) = cell.winner.map(|w| names.get(&c.players, w)).unwrap_or_default();
        if !name.is_empty() {
            out.push_str(&format!(
                "<div class=\"cell\"{} style=\"{}\"><b>{}</b>{}</div>",
                data(&name, &club),
                pos(&cell.bounds),
                esc(&name),
                if cell.compact || club.is_empty() { String::new() } else { format!("<small>{}</small>", esc(&club)) }
            ));
        }
        if let Some(score) = cell.score.as_ref().filter(|s| s.visible && s.fencer.is_some() && !s.text.is_empty()) {
            out.push_str(&format!("<div class=\"bs\" style=\"{}\">{}</div>", pos(&score.bounds), esc(&score.text)));
        }
    }
    out.push_str("</div></div>");
    out
}

/// Tableaux : l'arbre complet de chaque suite ; avec un filtre, la liste des
/// matchs qui le concernent.
fn tables(c: &mut Contest, names: &Names) -> String {
    // Arbre complet : du plus grand tableau (le logiciel peut n'en montrer qu'une partie).
    for i in 0..c.schedule.len() {
        if let Some(sup) = c.schedule.stage_mut(i).and_then(|s| s.as_any_mut().downcast_mut::<TableSupervisor>()) {
            for k in 0..sup.table_sets().len() {
                let set = sup.table_set_mut(k);
                while set.can_display_previous() {
                    set.display_previous();
                }
            }
        }
    }
    let lists = table_lists(c, names);
    if lists.is_empty() {
        return String::new();
    }
    let mut trees = String::new();
    for i in 0..c.schedule.len() {
        let Some(sup) = c.schedule.stage(i).and_then(|s| s.as_any().downcast_ref::<TableSupervisor>()) else { continue };
        for set in sup.table_sets().iter().filter(|s| s.is_visible()) {
            trees.push_str(&format!("<h3>{}</h3>{}", esc(&set.name()), bracket(c, set, names)));
        }
    }
    format!("<div class=\"trees\">{trees}</div><div class=\"bouts-list\">{lists}</div>")
}

/// Classement final (phase « Classement général »).
fn ranking(c: &mut Contest, names: &Names) -> String {
    let index = (0..c.schedule.len()).rev().find(|i| c.schedule.stage(*i).is_some_and(|s| s.as_any().downcast_ref::<GeneralClassificationStage>().is_some()));
    let Some(index) = index else { return String::new() };
    if index > c.schedule.current_index() {
        return "<p class=\"muted\">Le classement s'affichera à la fin de la compétition.</p>".into();
    }
    let Some(view) = crate::stages::general_list(c, index) else { return String::new() };
    let rank_col = view.columns.iter().position(|col| col.code.contains("rank"));
    let club_col = view.columns.iter().position(|col| col.code == "club");
    let points_col = view.columns.iter().position(|col| col.code == "points");
    let mut rows = String::new();
    for row in &view.rows {
        let (name, club) = names.get(&c.players, row.reference);
        let rank = rank_col.and_then(|i| row.cells.get(i)).cloned().unwrap_or_default();
        let club = club_col.and_then(|i| row.cells.get(i)).cloned().unwrap_or(club);
        let points = points_col.and_then(|i| row.cells.get(i)).map(|p| format!("<td class=\"res\">{}</td>", esc(p))).unwrap_or_default();
        rows.push_str(&format!("<tr{}><td class=\"rank\">{}</td><td class=\"name\">{}</td><td class=\"muted\">{}</td>{points}</tr>", data(&name, &club), esc(&rank), esc(&name), esc(&club)));
    }
    let points_head = if points_col.is_some() { "<th>Points</th>" } else { "" };
    format!("<table class=\"list\"><thead><tr><th>Pl</th><th>Tireur</th><th>Club</th>{points_head}</tr></thead><tbody>{rows}</tbody></table>")
}
