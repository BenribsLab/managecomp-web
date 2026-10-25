//! ManageComp Arbitre : la saisie des matchs par les arbitres, sur leur
//! téléphone, dans le navigateur (iPhone comme Android, rien à installer).
//!
//! Le QR code de l'arbitre ouvre `https://arbitre.<domaine>/?c=<club>&r=<arbitre>&k=<clé>` ;
//! l'identité est mémorisée sur le téléphone. Écrans (adresse après `#`) :
//! `#/` mes matchs, `#/feuille/<jeton>` une poule ou un match,
//! `#/match/<jeton>/<n>` la saisie d'un match (ou d'une rencontre par équipes).
//!
//! Tout passe par l'API (`/api/smartpoule/phone/…`) ; les saisies sont gardées
//! sur le téléphone et renvoyées si le réseau coupe.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use wasm_bindgen::JsCast;

use crate::api;

// ------------------------------------------------------------------ outils

/// « Feuille · match », sans répéter le match quand la feuille le nomme déjà
/// (tableau : « Place n°1 — 1-8.1 »).
fn bout_title(sheet: &Value, bout: &Value) -> String {
    let (sheet, bout) = (s(sheet, "name"), s(bout, "name"));
    if bout.is_empty() || sheet.contains(&bout) { sheet } else { format!("{sheet} · {bout}") }
}

fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}
fn u(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}
fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}
fn arr(v: &Value, k: &str) -> Vec<Value> {
    v.get(k).and_then(Value::as_array).cloned().unwrap_or_default()
}
fn enc(text: &str) -> String {
    js_sys::encode_uri_component(text).into()
}
fn dec(text: &str) -> String {
    js_sys::decode_uri_component(text).map(String::from).unwrap_or_else(|_| text.to_string())
}
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}
fn load<T: DeserializeOwned>(key: &str) -> Option<T> {
    serde_json::from_str(&storage()?.get_item(key).ok()??).ok()
}
fn save<T: Serialize>(key: &str, value: &T) {
    if let (Some(st), Ok(text)) = (storage(), serde_json::to_string(value)) {
        let _ = st.set_item(key, &text);
    }
}
fn forget(key: &str) {
    if let Some(st) = storage() {
        let _ = st.remove_item(key);
    }
}
fn now() -> f64 {
    web_sys::window().and_then(|w| w.performance()).map(|p| p.now()).unwrap_or_else(js_sys::Date::now)
}
fn go(hash: &str) {
    if let Some(w) = web_sys::window() {
        let _ = w.location().set_hash(hash);
    }
}
fn ask(message: &str) -> bool {
    crate::confirm(message)
}
fn mmss(ms: f64) -> String {
    let seconds = (ms.max(0.0) / 1000.0).ceil() as u64;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// Garde l'écran allumé pendant un match (API Wake Lock, si le navigateur la connaît).
fn keep_screen_on() {
    let Some(window) = web_sys::window() else { return };
    let navigator = window.navigator();
    let Ok(lock) = js_sys::Reflect::get(&navigator, &"wakeLock".into()) else { return };
    if lock.is_undefined() || lock.is_null() {
        return;
    }
    if let Ok(request) = js_sys::Reflect::get(&lock, &"request".into()) {
        if let Ok(f) = request.dyn_into::<js_sys::Function>() {
            let _ = f.call1(&lock, &"screen".into());
        }
    }
}

/// Bip de fin de temps.
fn beep() {
    let Ok(ctx) = web_sys::AudioContext::new() else { return };
    let Ok(osc) = ctx.create_oscillator() else { return };
    osc.frequency().set_value(880.0);
    let _ = osc.connect_with_audio_node(&ctx.destination());
    let _ = osc.start();
    let _ = osc.stop_with_when(ctx.current_time() + 0.6);
}

/// Paramètre de l'adresse (`?c=…`).
fn param(name: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    search.trim_start_matches('?').split('&').find_map(|kv| kv.strip_prefix(&format!("{name}=")).map(dec)).filter(|v| !v.is_empty())
}

/// La page est-elle ManageComp Arbitre ? Sous-domaine `arbitre.`, ou
/// chemins `/arbitre/` et `/smartpoule/` (anciens QR codes).
pub fn is_arbitre_page() -> bool {
    let Some(location) = web_sys::window().map(|w| w.location()) else { return false };
    location.hostname().is_ok_and(|h| h.starts_with("arbitre."))
        || location.pathname().is_ok_and(|p| p.starts_with("/arbitre") || p.starts_with("/smartpoule"))
}

// ---------------------------------------------------------------- identité

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Who {
    c: String,
    r: u32,
    k: String,
}

const IDENTITY: &str = "managecomp-arbitre";

impl Who {
    fn query(&self) -> String {
        format!("c={}&r={}&k={}", enc(&self.c), self.r, enc(&self.k))
    }
    fn body(&self, mut v: Value) -> Value {
        v["c"] = json!(self.c);
        v["r"] = json!(self.r);
        v["k"] = json!(self.k);
        v
    }
    /// Clé de stockage local d'un match.
    fn key(&self, token: &str, bout: u64) -> String {
        format!("managecomp-match|{}|{}|{token}|{bout}", self.c, self.r)
    }
}

/// Envoi d'un score ou d'un journal ; suit son sort quand BellePoule le traite
/// plus tard (`pending`). Rend le message à afficher, ou une erreur.
async fn post_score(who: &Who, body: Value) -> Result<&'static str, String> {
    let answer: Value = api::send("POST", "/api/smartpoule/phone/score", &who.body(body)).await.map_err(|e| e.message)?;
    match answer.get("status").and_then(Value::as_str) {
        Some("pending") => {
            let id = u(&answer, "id");
            for _ in 0..40 {
                gloo_timers_sleep(1500).await;
                let path = format!("/api/smartpoule/phone/result?{}&id={id}", who.query());
                if let Ok(r) = api::get::<Value>(&path).await {
                    match r.get("status").and_then(Value::as_str) {
                        Some("ok") => return Ok("Enregistré ✔"),
                        Some("error") => return Err(s(&r, "error")),
                        _ => {}
                    }
                }
            }
            Ok("Transmis au serveur : ManageComp l'intégrera dès qu'il sera connecté.")
        }
        _ => Ok("Enregistré ✔"),
    }
}

async fn gloo_timers_sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Score et chrono envoyés à l'écran de la piste (ManageComp Live).
fn push_live(who: &Who, state: Value) {
    let body = who.body(state);
    spawn_local(async move {
        let _ = api::send::<_, Value>("POST", "/api/smartpoule/phone/live", &body).await;
    });
}

// ------------------------------------------------------------------- page

#[component]
pub fn ArbitrePage() -> impl IntoView {
    let who = RwSignal::new(None::<Who>);
    // QR code d'une feuille (`?c=<club>&s=<jeton>`) : PIN de l'arbitre.
    let pin_for = RwSignal::new(None::<(String, String)>);
    if let (Some(c), Some(t), None) = (param("c"), param("s"), param("k")) {
        pin_for.set(Some((c, t)));
        if let Some(h) = web_sys::window().and_then(|w| w.history().ok()) {
            let _ = h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/"));
        }
    }
    match (param("c"), param("r").and_then(|r| r.parse().ok()), param("k")) {
        (Some(c), Some(r), Some(k)) => {
            let w = Who { c, r, k };
            save(IDENTITY, &w);
            who.set(Some(w));
            // L'adresse n'affiche plus la clé (capture d'écran, partage).
            if let Some(h) = web_sys::window().and_then(|w| w.history().ok()) {
                let hash = web_sys::window().and_then(|w| w.location().hash().ok()).unwrap_or_default();
                let _ = h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&format!("/{hash}")));
            }
        }
        _ => who.set(load(IDENTITY)),
    }
    let route = RwSignal::new(web_sys::window().and_then(|w| w.location().hash().ok()).unwrap_or_default());
    {
        let closure = wasm_bindgen::closure::Closure::<dyn Fn()>::new(move || {
            route.set(web_sys::window().and_then(|w| w.location().hash().ok()).unwrap_or_default());
        });
        if let Some(w) = web_sys::window() {
            w.set_onhashchange(Some(closure.as_ref().unchecked_ref()));
        }
        closure.forget();
    }
    view! {
        <div class="ref-app">
            {move || pin_for.get().map(|(club, token)| {
                let done = Callback::new(move |w: Option<Who>| {
                    if let Some(w) = w {
                        save(IDENTITY, &w);
                        who.set(Some(w));
                    }
                    pin_for.set(None);
                });
                view! { <PinScreen club=club token=token current=who.get_untracked() done=done /> }
            })}
            {move || pin_for.with(Option::is_none).then(|| match who.get() {
                None => view! {
                    <div class="ref-screen ref-center">
                        <h1>"ManageComp Arbitre"</h1>
                        <p>"Scannez avec l'appareil photo de votre téléphone le QR code d'arbitre que vous a remis l'organisation."</p>
                    </div>
                }.into_any(),
                Some(w) => {
                    let r = route.get();
                    if let Some(rest) = r.strip_prefix("#/match/") {
                        let (token, bout) = rest.rsplit_once('/').unwrap_or((rest, "0"));
                        view! { <MatchLoader who=w token=dec(token) bout=bout.parse().unwrap_or(0) /> }.into_any()
                    } else if let Some(token) = r.strip_prefix("#/feuille/") {
                        view! { <SheetScreen who=w token=dec(token) /> }.into_any()
                    } else {
                        view! { <JobsScreen who=w forget=Callback::new(move |_| { forget(IDENTITY); who.set(None); }) /> }.into_any()
                    }
                }
            })}
        </div>
    }
}

// ---------------------------------------------------------------- code PIN

/// QR code d'une feuille : le téléphone déjà reconnu pour cette feuille y
/// va directement ; sinon l'arbitre entre son PIN (une fois par compétition).
#[component]
fn PinScreen(club: String, token: String, current: Option<Who>, done: Callback<Option<Who>>) -> impl IntoView {
    let checking = RwSignal::new(current.as_ref().is_some_and(|w| w.c == club));
    let pin = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let token = StoredValue::new(token);
    let club = StoredValue::new(club);
    let open_sheet = move || go(&format!("#/feuille/{}", enc(&token.get_value())));
    if let Some(w) = current.filter(|w| w.c == club.get_value()) {
        spawn_local(async move {
            let path = format!("/api/smartpoule/phone/jobs?{}", w.query());
            let mine = api::get::<Value>(&path).await.ok().is_some_and(|d| arr(&d, "jobs").iter().any(|j| s(j, "token") == token.get_value()));
            if mine {
                open_sheet();
                done.run(None);
            } else {
                checking.set(false);
            }
        });
    }
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        error.set(None);
        let body = json!({ "c": club.get_value(), "token": token.get_value(), "pin": pin.get_untracked() });
        spawn_local(async move {
            match api::send::<_, Value>("POST", "/api/smartpoule/phone/pin", &body).await {
                Ok(v) => {
                    let w = Who { c: s(&v, "c"), r: u(&v, "r") as u32, k: s(&v, "k") };
                    open_sheet();
                    done.run(Some(w));
                }
                Err(e) => {
                    error.set(Some(e.message));
                    pin.set(String::new());
                }
            }
            busy.set(false);
        });
    };
    view! {
        <div class="ref-screen ref-pin-screen">
            <div class="ref-brand">"ManageComp Arbitre"</div>
            {move || if checking.get() {
                view! { <p class="ref-muted">"Vérification…"</p> }.into_any()
            } else {
                view! {
                    <form class="ref-pin" on:submit=submit>
                        <p>"Entrez votre code PIN d'arbitre pour arbitrer cette feuille. Il ne vous sera demandé qu'une fois pour la compétition."</p>
                        <input
                            class="ref-pin-input"
                            type="password"
                            inputmode="numeric"
                            autocomplete="one-time-code"
                            maxlength="4"
                            placeholder="••••"
                            prop:value=move || pin.get()
                            on:input=move |ev| pin.set(event_target_value(&ev).chars().filter(char::is_ascii_digit).take(4).collect())
                        />
                        {move || error.get().map(|e| view! { <p class="ref-error">{e}</p> })}
                        <button type="submit" class="ref-pin-ok" disabled=move || busy.get() || pin.get().len() != 4>"Valider"</button>
                        <p class="ref-small ref-muted">"Pas de code PIN ? Il est sur votre feuille personnelle d'arbitre, ou demandez-le à l'organisation."</p>
                    </form>
                }.into_any()
            }}
        </div>
    }
}

// -------------------------------------------------------------- mes matchs

#[component]
fn JobsScreen(who: Who, forget: Callback<()>) -> impl IntoView {
    let data = RwSignal::new(None::<Value>);
    let error = RwSignal::new(None::<String>);
    let who = StoredValue::new(who);
    let fetch = move || {
        let path = format!("/api/smartpoule/phone/jobs?{}", who.get_value().query());
        spawn_local(async move {
            match api::get::<Value>(&path).await {
                Ok(v) => {
                    error.set(None);
                    data.set(Some(v));
                }
                Err(e) => error.set(Some(e.message)),
            }
        });
    };
    fetch();
    let handle = set_interval_with_handle(fetch, std::time::Duration::from_secs(15)).ok();
    on_cleanup(move || {
        if let Some(h) = handle {
            h.clear();
        }
    });
    view! {
        <div class="ref-screen">
            <CallButton who=who.get_value() />
            <header class="ref-head">
                <div>
                    <div class="ref-brand">"ManageComp Arbitre"</div>
                    <div class="ref-muted">{move || data.get().map(|d| format!("{} · {}", s(&d, "name"), s(&d, "club"))).unwrap_or_default()}</div>
                </div>
                <button class="ref-light" on:click=move |_| fetch()>"Actualiser"</button>
            </header>
            {move || error.get().map(|e| view! { <p class="ref-error">{e}</p> })}
            {move || {
                let d = data.get()?;
                let mut jobs = arr(&d, "jobs");
                // À arbitrer d'abord, par heure.
                jobs.sort_by_key(|j| (b(j, "over"), { let t = s(j, "time"); if t.is_empty() { "99:99".to_string() } else { t } }));
                if jobs.is_empty() {
                    return Some(view! { <p class="ref-muted ref-center">"Aucun match pour l'instant. Cette liste se met à jour toute seule quand l'organisation vous affecte une poule ou un match."</p> }.into_any());
                }
                Some(jobs.into_iter().map(|j| {
                    let href = format!("#/feuille/{}", enc(&s(&j, "token")));
                    let place = [s(&j, "piste").split(' ').next().filter(|p| !p.is_empty()).map(|p| format!("Piste {p}")), Some(s(&j, "time")).filter(|t| !t.is_empty()).map(|t| format!("à {t}"))]
                        .into_iter().flatten().collect::<Vec<_>>().join(" · ");
                    view! {
                        <a class="ref-card" class:done=b(&j, "over") href=href>
                            <div class="ref-row">
                                <strong class="ref-grow">{s(&j, "name")}</strong>
                                {b(&j, "over").then(|| view! { <span class="ref-ok">"✔ terminé"</span> })}
                            </div>
                            {(!place.is_empty()).then(|| view! { <div class="ref-place">{place}</div> })}
                            <div class="ref-muted">{s(&j, "summary")}</div>
                            <div class="ref-muted ref-small">{s(&j, "competition")}</div>
                        </a>
                    }
                }).collect_view().into_any())
            }}
            <p class="ref-center ref-small"><button class="ref-link" on:click=move |_| {
                if ask("Oublier ce QR code sur ce téléphone ?") { forget.run(()); }
            }>"Changer de QR code"</button></p>
        </div>
    }
}

// ------------------------------------------------- Directoire technique

/// « Appeler le Directoire technique », tout en haut des écrans de
/// l'arbitre : l'organisation reçoit une notification (puce rouge).
#[component]
fn CallButton(who: Who, #[prop(optional)] token: Option<String>) -> impl IntoView {
    let busy = RwSignal::new(false);
    let sent = RwSignal::new(false);
    let call = move |_| {
        if busy.get_untracked() || !ask("Appeler le Directoire technique ?") {
            return;
        }
        busy.set(true);
        let body = who.body(json!({ "token": token.clone() }));
        spawn_local(async move {
            match api::send::<_, Value>("POST", "/api/smartpoule/phone/call", &body).await {
                Ok(_) => sent.set(true),
                Err(e) => crate::alert(&e.message),
            }
            busy.set(false);
        });
    };
    view! {
        <button class="ref-call" class:sent=move || sent.get() disabled=move || busy.get() on:click=call>
            {move || if sent.get() { "📣 Directoire technique appelé ✔" } else { "📣 Appeler le Directoire technique" }}
        </button>
    }
}

// ------------------------------------------------------------------ feuille

async fn fetch_sheet(who: &Who, token: &str) -> Result<Value, String> {
    let path = format!("/api/smartpoule/phone/sheet?{}&token={}", who.query(), enc(token));
    api::get::<Value>(&path).await.map_err(|e| e.message)
}

#[component]
fn SheetScreen(who: Who, token: String) -> impl IntoView {
    let data = RwSignal::new(None::<Result<Value, String>>);
    {
        let (who, token) = (who.clone(), token.clone());
        spawn_local(async move { data.set(Some(fetch_sheet(&who, &token).await)) });
    }
    let token = StoredValue::new(token);
    let who = StoredValue::new(who);
    view! {
        <div class="ref-screen">
            <CallButton who=who.get_value() token=token.get_value() />
            <header class="ref-head">
                <a class="ref-back" href="#/">"‹ Mes matchs"</a>
            </header>
            {move || match data.get() {
                None => view! { <p class="ref-muted">"Chargement…"</p> }.into_any(),
                Some(Err(e)) => view! { <p class="ref-error">{e}</p> }.into_any(),
                Some(Ok(sheet)) if s(&sheet, "kind") == "lobby" => {
                    // Lobby (série) : saisie des places, manche par manche.
                    let reload = Callback::new(move |_| {
                        let (w, t) = (who.get_value(), token.get_value());
                        spawn_local(async move { data.set(Some(fetch_sheet(&w, &t).await)) });
                    });
                    view! { <LobbySheet sheet=sheet.clone() who=who.get_value() token=token.get_value() reload=reload /> }.into_any()
                }
                Some(Ok(sheet)) => {
                    let pool = s(&sheet, "kind") == "pool";
                    let bouts = arr(&sheet, "bouts");
                    view! {
                        <h1 class="ref-title">{s(&sheet, "name")}</h1>
                        <div class="ref-muted ref-small">{s(&sheet, "competition")}</div>
                        {(!s(&sheet, "piste").is_empty()).then(|| view! { <div class="ref-place">{format!("Piste {}", s(&sheet, "piste"))}</div> })}
                        {pool.then(|| view! { <PoolGrid sheet=sheet.clone() who=who.get_value() token=token.get_value() /> })}
                        {pool.then(|| view! { <h2>"Matchs"</h2> })}
                        {if bouts.is_empty() {
                            view! { <p class="ref-muted">"Aucun match à saisir pour l'instant : la composition des poules n'est pas encore validée."</p> }.into_any()
                        } else {
                            bouts.into_iter().map(|bout| {
                                let href = format!("#/match/{}/{}", enc(&token.get_value()), u(&bout, "index"));
                                let fencers = arr(&bout, "fencers");
                                view! {
                                    <a class="ref-card" class:done=b(&bout, "over") href=href>
                                        <div class="ref-row">
                                            <strong class="ref-grow ref-muted">{s(&bout, "name")}</strong>
                                            <span class="ref-muted ref-small ref-pre">{s(&bout, "resource")}</span>
                                            {b(&bout, "over").then(|| view! { <span class="ref-ok">" ✔"</span> })}
                                        </div>
                                        {fencers.into_iter().map(|f| view! {
                                            <div class="ref-row ref-fencer">
                                                <span class="ref-pos">{let p = u(&f, "position"); if p > 0 { p.to_string() } else { String::new() }}</span>
                                                <span class="ref-grow"><strong>{s(&f, "name")}</strong><br /><span class="ref-muted ref-small">{s(&f, "club")}</span></span>
                                                <span class="ref-score">{s(&f, "score")}</span>
                                            </div>
                                        }).collect_view()}
                                    </a>
                                }
                            }).collect_view().into_any()
                        }}
                    }.into_any()
                }
            }}
        </div>
    }
}

/// Nom court pour la grille : « DUPONT J. » (nom en capitales, initiale du
/// prénom), pour que la poule tienne dans la largeur du téléphone.
fn short_name(full: &str) -> String {
    let words: Vec<&str> = full.split_whitespace().collect();
    let is_last = |w: &&str| w.chars().any(char::is_alphabetic) && !w.chars().any(char::is_lowercase);
    let last: Vec<&str> = words.iter().copied().take_while(is_last).collect();
    let (last, rest) = if last.is_empty() { (words.iter().take(1).copied().collect::<Vec<_>>(), &words[words.len().min(1)..]) } else { (last.clone(), &words[last.len()..]) };
    let initial = rest.first().and_then(|w| w.chars().next()).map(|c| format!(" {c}.")).unwrap_or_default();
    format!("{}{initial}", last.join(" "))
}

/// Feuille de poule du téléphone : tient dans la largeur jusqu'à 9 tireurs
/// (texte plus petit au-delà de 7) ; au-delà, défilement horizontal avec les
/// noms fixés à gauche.
/// Tireurs dont le matériel a été vérifié (références).
fn checked_list(v: &Value) -> Vec<u64> {
    arr(v, "equipment").iter().filter_map(Value::as_u64).collect()
}

/// Tireurs signalés absents (références).
fn absent_list(v: &Value) -> Vec<u64> {
    arr(v, "absences").iter().filter_map(Value::as_u64).collect()
}

#[component]
fn PoolGrid(sheet: Value, who: Who, token: String) -> impl IntoView {
    let rows = arr(&sheet, "fencers");
    let n = rows.len();
    // Matériel vérifié avant la poule (case du nom soulignée en vert).
    let equipment = RwSignal::new(checked_list(&sheet));
    // Absences signalées à l'organisation.
    let absences = RwSignal::new(absent_list(&sheet));
    let open = RwSignal::new(None::<Value>);
    let (who, token) = (StoredValue::new(who), StoredValue::new(token));
    let toggle = move |fencer: u64, checked: bool| {
        let set = move |l: &mut Vec<u64>, on: bool| {
            l.retain(|f| *f != fencer);
            if on {
                l.push(fencer);
            }
        };
        equipment.update(|l| set(l, checked));
        let body = who.get_value().body(json!({ "token": token.get_value(), "fencer": fencer, "checked": checked }));
        spawn_local(async move {
            match api::send::<_, Value>("POST", "/api/smartpoule/phone/equipment", &body).await {
                Ok(v) => equipment.set(checked_list(&v)),
                Err(e) => {
                    equipment.update(|l| set(l, !checked));
                    crate::alert(&e.message);
                }
            }
        });
    };
    let report = move |fencer: u64, absent: bool, name: String| {
        let question = if absent {
            format!("Signaler à l'organisation l'absence de {name} ?")
        } else {
            format!("{name} est arrivé : retirer le signalement d'absence ?")
        };
        if !ask(&question) {
            return;
        }
        let body = who.get_value().body(json!({ "token": token.get_value(), "fencer": fencer, "absent": absent }));
        spawn_local(async move {
            match api::send::<_, Value>("POST", "/api/smartpoule/phone/absence", &body).await {
                Ok(v) => absences.set(absent_list(&v)),
                Err(e) => crate::alert(&e.message),
            }
        });
    };
    let size = match n {
        0..=6 => "l",
        7 => "m",
        8..=9 => "s",
        _ => "xs",
    };
    view! {
        <div class="ref-grid-scroll">
            <table class=format!("ref-grid ref-grid-{size}")>
                <colgroup>
                    <col class="c-name" />
                    <col class="c-pos" />
                    {(0..n).map(|_| view! { <col class="c-cell" /> }).collect_view()}
                    <col class="c-res" />
                    <col class="c-ind" />
                    <col class="c-res" />
                </colgroup>
                <thead>
                    <tr>
                        <th class="ref-gname"></th><th></th>
                        {(1..=n).map(|j| view! { <th>{j}</th> }).collect_view()}
                        <th>"V"</th><th>"Ind"</th><th>"Pl"</th>
                    </tr>
                </thead>
                <tbody>
                    {rows.into_iter().map(|r| {
                        let cells = arr(&r, "cells");
                        let full = s(&r, "name");
                        let reference = r.get("reference").and_then(Value::as_u64);
                        let row = r.clone();
                        view! {
                            <tr>
                                <td
                                    class="ref-gname"
                                    class:checked=move || reference.is_some_and(|f| equipment.with(|l| l.contains(&f)))
                                    class:absent=move || reference.is_some_and(|f| absences.with(|l| l.contains(&f)))
                                    on:click=move |_| open.set(Some(row.clone()))
                                >{short_name(&full)}</td>
                                <td class="ref-gpos">{s(&r, "position")}</td>
                                {(0..n).map(|j| {
                                    let v = cells.get(j).cloned().unwrap_or(Value::Null);
                                    let grey = v.is_null();
                                    let text = v.as_str().unwrap_or("").to_string();
                                    let win = text.starts_with('V');
                                    view! { <td class:grey=grey class:win=win>{text}</td> }
                                }).collect_view()}
                                <td class="ref-gres">{s(&r, "victories")}</td>
                                <td class="ref-gres">{s(&r, "indice")}</td>
                                <td class="ref-gres"><strong>{s(&r, "rank")}</strong></td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
        </div>
        {move || open.get().map(|r| {
            let reference = r.get("reference").and_then(Value::as_u64);
            view! {
                <div class="ref-modal" on:click=move |_| open.set(None)>
                    <div class="ref-modal-card" on:click=|ev| ev.stop_propagation()>
                        <div class="ref-muted ref-small">{format!("Tireur n° {}", s(&r, "position"))}</div>
                        <div class="ref-modal-name">{s(&r, "name")}</div>
                        <div class="ref-muted">{s(&r, "club")}</div>
                        {match reference {
                            Some(f) => view! {
                                <label class="ref-switch">
                                    <input
                                        type="checkbox"
                                        prop:checked=move || equipment.with(|l| l.contains(&f))
                                        on:change=move |ev| toggle(f, event_target_checked(&ev))
                                    />
                                    <span class="ref-switch-track"><span class="ref-switch-thumb"></span></span>
                                    <span>"Matériel vérifié"</span>
                                </label>
                                {
                                    let name = s(&r, "name");
                                    move || if absences.with(|l| l.contains(&f)) {
                                        let name = name.clone();
                                        view! {
                                            <div class="ref-absent-note">"🔴 Absence signalée à l'organisation"</div>
                                            <button class="ref-light" on:click=move |_| report(f, false, name.clone())>"Le tireur est arrivé"</button>
                                        }.into_any()
                                    } else {
                                        let name = name.clone();
                                        view! { <button class="ref-absent" on:click=move |_| report(f, true, name.clone())>"Signaler l'absence du tireur"</button> }.into_any()
                                    }
                                }
                            }.into_any(),
                            None => view! { <p class="ref-muted ref-small">"Vérification du matériel : mettez à jour ManageComp sur l'ordinateur de l'organisation."</p> }.into_any(),
                        }}
                        <button class="ref-light" on:click=move |_| open.set(None)>"Fermer"</button>
                    </div>
                </div>
            }
        })}
    }
}

// ------------------------------------------------------------ lobby (série)

/// Feuille d'un lobby : l'arbitre choisit la manche et tape la place de
/// chaque tireur (même place pour des ex aequo, « A » pour un abandon).
#[component]
fn LobbySheet(sheet: Value, who: Who, token: String, reload: Callback<()>) -> impl IntoView {
    let races = u(&sheet, "races").max(1) as usize;
    let fencers = arr(&sheet, "fencers");
    // Première manche pas encore complète.
    let first_open = (0..races).find(|k| fencers.iter().any(|f| arr(f, "finishes").get(*k).is_none_or(Value::is_null))).unwrap_or(races - 1);
    let race = RwSignal::new(first_open);
    let message = RwSignal::new(None::<Result<&'static str, String>>);
    let (who, token) = (StoredValue::new(who), StoredValue::new(token));
    let locked = b(&sheet, "locked");
    let send = move |player: u64, value: String| {
        let body = json!({ "token": token.get_value(), "bout": race.get_untracked(), "values": [player.to_string(), value] });
        spawn_local(async move {
            let r = post_score(&who.get_value(), body).await;
            message.set(Some(r));
            reload.run(());
        });
    };
    view! {
        <h1 class="ref-title">{s(&sheet, "name")}</h1>
        <div class="ref-muted ref-small">{s(&sheet, "competition")}</div>
        {(!s(&sheet, "piste").is_empty()).then(|| view! { <div class="ref-place">{format!("Piste {}{}", s(&sheet, "piste"), if s(&sheet, "time").is_empty() { String::new() } else { format!(" · {}", s(&sheet, "time")) })}</div> })}
        <div class="ref-races">
            {(0..races).map(|k| view! {
                <button class="ref-light" class:chosen=move || race.get() == k on:click=move |_| race.set(k)>{format!("Manche {}", k + 1)}</button>
            }).collect_view()}
        </div>
        <p class="ref-muted ref-small">"Tapez la place de chaque tireur (la même pour des ex aequo, A pour un abandon)."</p>
        {move || message.get().map(|m| match m {
            Ok(text) => view! { <p class="ref-ok">{text}</p> }.into_any(),
            Err(e) => view! { <p class="ref-error">{e}</p> }.into_any(),
        })}
        <div class="ref-grid-scroll">
            <table class="ref-grid ref-lobby">
                <thead><tr><th class="ref-gname">"Tireur"</th><th>"Place"</th><th>"Pts"</th><th>"Cl."</th></tr></thead>
                <tbody>
                    {fencers.into_iter().map(|f| {
                        let player = u(&f, "reference");
                        let finishes = arr(&f, "finishes");
                        let raced = b(&f, "raced");
                        let value = move || match finishes.get(race.get()) {
                            Some(Value::Number(n)) => n.to_string(),
                            Some(Value::String(x)) => x.clone(),
                            _ => String::new(),
                        };
                        view! {
                            <tr>
                                <td class="ref-gname"><strong>{s(&f, "name")}</strong><br /><span class="ref-muted ref-small">{s(&f, "club")}</span></td>
                                <td><input class="ref-place-input" type="text" inputmode="numeric" disabled=locked prop:value=value
                                    on:change=move |ev| send(player, event_target_value(&ev)) /></td>
                                <td>{u(&f, "points")}</td>
                                <td><strong>{if raced { f.get("place").and_then(Value::as_u64).map(|p| p.to_string()).unwrap_or_default() } else { String::new() }}</strong></td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
        </div>
    }
}

// ------------------------------------------------------------------ chrono

/// Décompte d'après l'horloge du navigateur, indépendant du rafraîchissement.
#[derive(Clone, Copy)]
struct Clock {
    running: RwSignal<bool>,
    deadline: StoredValue<f64>,
    stored: RwSignal<f64>,
    period: StoredValue<f64>,
    tick: RwSignal<f64>,
}

impl Clock {
    fn new(period_ms: f64, remaining_ms: f64) -> Clock {
        Clock {
            running: RwSignal::new(false),
            deadline: StoredValue::new(0.0),
            stored: RwSignal::new(remaining_ms.clamp(0.0, period_ms)),
            period: StoredValue::new(period_ms),
            tick: RwSignal::new(now()),
        }
    }
    fn remaining(&self) -> f64 {
        if self.running.get() {
            self.tick.track();
            (self.deadline.get_value() - now()).max(0.0)
        } else {
            self.stored.get()
        }
    }
    fn remaining_now(&self) -> f64 {
        if self.running.get_untracked() { (self.deadline.get_value() - now()).max(0.0) } else { self.stored.get_untracked() }
    }
    fn start(&self) {
        if !self.running.get_untracked() && self.stored.get_untracked() > 0.0 {
            self.deadline.set_value(now() + self.stored.get_untracked());
            self.running.set(true);
        }
    }
    fn pause(&self) {
        if self.running.get_untracked() {
            self.stored.set(self.remaining_now());
            self.running.set(false);
        }
    }
    fn adjust(&self, delta: f64) {
        if !self.running.get_untracked() {
            self.stored.update(|v| *v = (*v + delta).clamp(0.0, self.period.get_value()));
        }
    }
    fn reset(&self) {
        self.pause();
        self.stored.set(self.period.get_value());
    }
}

/// Mise à jour du chrono ; `each` est appelé à chaque pas (fin de temps, direct).
fn run_clock(clock: Clock, each: impl Fn() + 'static) {
    let handle = set_interval_with_handle(
        move || {
            clock.tick.set(now());
            each();
        },
        std::time::Duration::from_millis(200),
    )
    .ok();
    keep_screen_on();
    on_cleanup(move || {
        if let Some(h) = handle {
            h.clear();
        }
    });
}

// ---------------------------------------------------------- match (chargement)

#[component]
fn MatchLoader(who: Who, token: String, bout: u64) -> impl IntoView {
    let data = RwSignal::new(None::<Result<Value, String>>);
    {
        let (who, token) = (who.clone(), token.clone());
        spawn_local(async move { data.set(Some(fetch_sheet(&who, &token).await)) });
    }
    let back = format!("#/feuille/{}", enc(&token));
    let (who, token) = (StoredValue::new(who), StoredValue::new(token));
    view! {
        <div class="ref-screen">
            <CallButton who=who.get_value() token=token.get_value() />
            <header class="ref-head"><a class="ref-back" href=back>"‹ Feuille"</a></header>
            {move || match data.get() {
                None => view! { <p class="ref-muted">"Chargement…"</p> }.into_any(),
                Some(Err(e)) => view! { <p class="ref-error">{e}</p> }.into_any(),
                Some(Ok(sheet)) => {
                    let Some(found) = arr(&sheet, "bouts").into_iter().find(|x| u(x, "index") == bout) else {
                        return view! { <p class="ref-error">"Ce match ne vous est pas affecté."</p> }.into_any();
                    };
                    if sheet.get("relay").is_some_and(|r| !r.is_null()) {
                        view! { <TeamMatch who=who.get_value() token=token.get_value() sheet=sheet bout=found /> }.into_any()
                    } else {
                        view! { <SingleMatch who=who.get_value() token=token.get_value() sheet=sheet bout=found /> }.into_any()
                    }
                }
            }}
        </div>
    }
}

// --------------------------------------------------------- match individuel

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct SingleState {
    revision: u64,
    period: u32,
    remaining_ms: f64,
    scores: [u32; 2],
    green_side: u8,
    cards: Vec<Value>,
    winner: Option<u8>,
}

fn parse_score(text: &str, max: u32) -> u32 {
    let t = text.trim().to_uppercase();
    if t == "V" {
        return max;
    }
    t.trim_start_matches(['V', 'W']).parse().unwrap_or(0)
}

#[component]
fn SingleMatch(who: Who, token: String, sheet: Value, bout: Value) -> impl IntoView {
    // Match de poule : la double défaite est possible (temps de la poule écoulé).
    let pool = token.starts_with("p~");
    let index = u(&bout, "index");
    let max = u(&sheet, "max_score").max(1) as u32;
    let timing = sheet.get("timing").cloned().unwrap_or(Value::Null);
    let period_ms = (u(&timing, "period_seconds").clamp(1, 3600) as f64) * 1000.0;
    let periods = u(&timing, "periods").clamp(1, 20) as u32;
    let fencers = arr(&bout, "fencers");
    let names: [String; 2] = [0, 1].map(|k| fencers.get(k).map(|f| s(f, "name")).unwrap_or_default());
    let key = who.key(&token, index);
    // État : téléphone d'abord, sinon le serveur (chrono et cartons déjà envoyés).
    let initial: SingleState = load(&key).unwrap_or_else(|| {
        let o = bout.get("officiating").cloned().unwrap_or(Value::Null);
        SingleState {
            revision: u(&o, "revision"),
            period: (u(&o, "period") as u32).max(1),
            remaining_ms: if o.is_null() { period_ms } else { u(&o, "remaining_ms") as f64 },
            scores: [0, 1].map(|k| fencers.get(k).map(|f| parse_score(&s(f, "score"), max)).unwrap_or(0)),
            green_side: u(&o, "green_side") as u8,
            cards: arr(&o, "cards"),
            winner: None,
        }
    });
    let clock = Clock::new(period_ms, initial.remaining_ms);
    let scores = RwSignal::new(initial.scores);
    let period = RwSignal::new(initial.period.clamp(1, periods));
    let green = RwSignal::new(initial.green_side.min(1));
    let cards = RwSignal::new(initial.cards.clone());
    let winner = RwSignal::new(initial.winner);
    let revision = StoredValue::new(initial.revision);
    let status = RwSignal::new(String::new());
    let sending = RwSignal::new(false);
    let ctx = StoredValue::new((who, token, key, names.clone()));
    let piste = u(&bout, "piste");
    let title = bout_title(&sheet, &bout);
    let (competition, color) = (s(&sheet, "competition"), s(&sheet, "color"));

    let officiating = move || {
        json!({
            "revision": revision.get_value(),
            "period": period.get_untracked(),
            "remaining_ms": clock.remaining_now().round() as u64,
            "scores": scores.get_untracked(),
            "green_side": green.get_untracked(),
            "cards": cards.get_untracked(),
        })
    };
    // Chaque changement : gardé sur le téléphone, puis envoyé au serveur.
    let persist = move || {
        revision.update_value(|r| *r += 1);
        let (who, token, key, _) = ctx.get_value();
        save(&key, &SingleState {
            revision: revision.get_value(),
            period: period.get_untracked(),
            remaining_ms: clock.remaining_now(),
            scores: scores.get_untracked(),
            green_side: green.get_untracked(),
            cards: cards.get_untracked(),
            winner: winner.get_untracked(),
        });
        let body = json!({ "token": token, "bout": index, "officiating": officiating() });
        spawn_local(async move {
            if let Err(e) = post_score(&who, body).await {
                status.set(format!("Gardé sur ce téléphone : {e}"));
            }
        });
    };

    // Direct vers l'écran de piste, et fin de temps.
    let live_key = StoredValue::new((String::new(), 0.0_f64));
    let live_state = move |over: bool| {
        let (_, _, _, names) = ctx.get_value();
        let g = green.get_untracked() as usize;
        let sc = scores.get_untracked();
        json!({
            "piste": piste, "competition": competition, "color": color, "bout": title,
            "red": { "name": names[1 - g], "score": sc[1 - g].to_string() },
            "green": { "name": names[g], "score": sc[g].to_string() },
            "remaining_ms": clock.remaining_now().round() as u64, "running": clock.running.get_untracked(),
            "period": period.get_untracked(), "periods": periods, "over": over,
        })
    };
    let live_state = StoredValue::new(live_state);
    run_clock(clock, move || {
        if clock.running.get_untracked() && clock.remaining_now() <= 0.0 {
            clock.pause();
            beep();
            persist();
        }
        if piste == 0 {
            return;
        }
        let sc = scores.get_untracked();
        let k = format!("{:?}|{}|{}|{}|{}", sc, green.get_untracked(), clock.running.get_untracked(), period.get_untracked(),
            if clock.running.get_untracked() { String::new() } else { (clock.remaining_now() as u64).to_string() });
        let (last, at) = live_key.get_value();
        let wait = if clock.running.get_untracked() { 3000.0 } else { 20000.0 };
        if k != last || now() - at > wait {
            live_key.set_value((k, now()));
            push_live(&ctx.get_value().0, (live_state.get_value())(false));
        }
    });

    let change = move |side: usize, delta: i32| {
        clock.pause();
        scores.update(|sc| sc[side] = (sc[side] as i32 + delta).clamp(0, max as i32) as u32);
        winner.set(None);
        persist();
    };
    let card = move |side: usize, color: &'static str, label: &'static str| {
        let (_, _, _, names) = ctx.get_value();
        clock.pause();
        if !ask(&format!("Carton {label} pour {} ?", names[side])) {
            return;
        }
        let id = format!("{}-{}", js_sys::Date::now() as u64, (js_sys::Math::random() * 1e6) as u64);
        cards.update(|l| l.push(json!({
            "id": id, "side": side, "color": color, "period": period.get_untracked(),
            "remaining_ms": clock.remaining_now().round() as u64, "issued_at_ms": js_sys::Date::now() as u64, "cancelled": false,
        })));
        persist();
    };
    let undo_card = move || {
        let last = cards.get_untracked().iter().rposition(|c| !b(c, "cancelled"));
        if let Some(i) = last {
            if ask("Annuler le dernier carton ?") {
                cards.update(|l| l[i]["cancelled"] = json!(true));
                persist();
            }
        }
    };
    let current_winner = move || {
        let sc = scores.get();
        match sc[0].cmp(&sc[1]) {
            std::cmp::Ordering::Greater => Some(0u8),
            std::cmp::Ordering::Less => Some(1u8),
            std::cmp::Ordering::Equal => winner.get(),
        }
    };
    let submit = move |_| {
        if sending.get_untracked() {
            return;
        }
        let (who, token, key, names) = ctx.get_value();
        let sc = scores.get_untracked();
        let Some(w) = current_winner() else {
            status.set(if pool {
                "Égalité : touchez « Vainqueur » sous le tireur gagnant, ou « Double défaite ».".into()
            } else {
                "Égalité : touchez « Vainqueur » sous le tireur gagnant.".into()
            });
            return;
        };
        // w = 2 : double défaite (poule), sans vainqueur.
        let result = if w == 2 { "Double défaite".to_string() } else { format!("Vainqueur : {}", names[w as usize]) };
        if !ask(&format!("Envoyer ce score ?\n\n{} : {}\n{} : {}\n\n{result}", names[0], sc[0], names[1], sc[1])) {
            return;
        }
        clock.pause();
        revision.update_value(|r| *r += 1);
        let values: Vec<String> = (0..2).map(|k| if k == w as usize { format!("W{}", sc[k]) } else { sc[k].to_string() }).collect();
        let body = json!({ "token": token, "bout": index, "values": values, "officiating": officiating() });
        sending.set(true);
        status.set("Envoi…".into());
        let back = format!("#/feuille/{}", enc(&token));
        spawn_local(async move {
            match post_score(&who, body).await {
                Ok(message) => {
                    forget(&key);
                    push_live(&who, (live_state.get_value())(true));
                    status.set(message.into());
                    gloo_timers_sleep(900).await;
                    go(&back);
                }
                Err(e) => {
                    sending.set(false);
                    status.set(e);
                }
            }
        });
    };

    let panel = move |pos: usize| {
        // Rouge à gauche, vert à droite ; le premier tireur de la feuille est vert.
        let side = move || if pos == 0 { 1 - green.get() as usize } else { green.get() as usize };
        let names = ctx.get_value().3;
        view! {
            <div class="ref-side" class:red=pos == 0 class:green=pos == 1>
                <div class="ref-name">{move || names[side()].clone()}</div>
                <div class="ref-big">{move || scores.get()[side()]}</div>
                <div class="ref-row ref-center">
                    <button class="ref-step" on:click=move |_| change(side(), -1)>"−"</button>
                    <button class="ref-step plus" on:click=move |_| change(side(), 1)>"+"</button>
                </div>
                <div class="ref-cards">
                    <button class="card-y" on:click=move |_| card(side(), "yellow", "jaune")>{move || cards.get().iter().filter(|c| !b(c, "cancelled") && u(c, "side") as usize == side() && s(c, "color") == "yellow").count()}</button>
                    <button class="card-r" on:click=move |_| card(side(), "red", "rouge")>{move || cards.get().iter().filter(|c| !b(c, "cancelled") && u(c, "side") as usize == side() && s(c, "color") == "red").count()}</button>
                    <button class="card-b" on:click=move |_| card(side(), "black", "noir")>{move || cards.get().iter().filter(|c| !b(c, "cancelled") && u(c, "side") as usize == side() && s(c, "color") == "black").count()}</button>
                </div>
                {move || {
                    let sc = scores.get();
                    (sc[0] == sc[1]).then(|| view! {
                        <button class="ref-light" class:chosen=move || winner.get() == Some(side() as u8) on:click=move |_| { winner.set(Some(side() as u8)); }>"Vainqueur"</button>
                    })
                }}
            </div>
        }
    };

    view! {
        <h1 class="ref-title">{bout_title(&sheet, &bout)}</h1>
        <div class="ref-muted ref-small">{format!("Match en {max} touches · manche ")}{move || period.get()}{format!(" / {periods}")}</div>
        <div class="ref-clock" class:zero=move || clock.remaining() <= 0.0 on:click=move |_| { if clock.running.get_untracked() { clock.pause(); persist(); } else { clock.start(); } }>
            {move || mmss(clock.remaining())}
        </div>
        <div class="ref-row ref-center">
            <button class="ref-main" class:pause=move || clock.running.get() disabled=move || clock.remaining() <= 0.0 on:click=move |_| {
                if clock.running.get_untracked() { clock.pause(); persist(); } else { clock.start(); }
            }>{move || if clock.running.get() { "Pause" } else { "Démarrer" }}</button>
        </div>
        <Show when=move || !clock.running.get()>
            <div class="ref-row ref-center ref-adjust">
                {[-10.0, -1.0, 1.0, 10.0].into_iter().map(|d| view! {
                    <button class="ref-light" on:click=move |_| { clock.adjust(d * 1000.0); persist(); }>{if d > 0.0 { format!("+{d}s") } else { format!("{d}s") }}</button>
                }).collect_view()}
                <button class="ref-light" on:click=move |_| { if ask("Remettre le chrono à zéro ?") { clock.reset(); persist(); } }>"↺"</button>
                {(periods > 1).then(|| view! {
                    <button class="ref-light" disabled=move || period.get() >= periods on:click=move |_| { period.update(|p| *p += 1); clock.reset(); persist(); }>"Manche suivante"</button>
                })}
            </div>
        </Show>
        <div class="ref-board">{panel(0)}{panel(1)}</div>
        {pool.then(|| view! {
            <Show when=move || { let sc = scores.get(); sc[0] == sc[1] }>
                <div class="ref-row ref-center ref-double">
                    <button class="ref-light" class:chosen=move || winner.get() == Some(2) on:click=move |_| winner.set(Some(2))>
                        "Double défaite"
                    </button>
                    <span class="ref-muted ref-small">"Temps de la poule écoulé : aucun vainqueur."</span>
                </div>
            </Show>
        })}
        <div class="ref-row ref-center">
            <button class="ref-light" on:click=move |_| { green.update(|g| *g = 1 - *g); persist(); }>"⇄ Inverser rouge / vert"</button>
            <button class="ref-light" on:click=move |_| undo_card()>"Annuler le dernier carton"</button>
        </div>
        <button class="ref-send" disabled=move || sending.get() on:click=submit>"Envoyer le score"</button>
        <p class="ref-status">{move || status.get()}</p>
    }
}

// ------------------------------------------------------ rencontre par équipes

/// Format des relais reçu avec la feuille.
#[derive(Clone, Debug)]
struct Relay {
    json: Value,
    single: bool,
    size: u32,
    points: u32,
}

impl Relay {
    fn new(json: Value) -> Relay {
        Relay {
            single: s(&json, "kind") == "single",
            size: (u(&json, "team_size") as u32).clamp(1, 20),
            points: (u(&json, "points") as u32).clamp(1, 15),
            json,
        }
    }
    fn bouts(&self) -> Vec<(u32, u32)> {
        let n = self.size;
        if self.single {
            return (0..n).map(|k| (2 * k + 1, 2 * k + 2)).collect();
        }
        match n {
            2 => vec![(1, 4), (2, 3), (1, 3), (2, 4)],
            3 => vec![(3, 6), (1, 5), (2, 4), (1, 6), (3, 4), (2, 5), (1, 4), (2, 6), (3, 5)],
            _ => (0..n).flat_map(|r| (0..n).map(move |i| (i + 1, n + 1 + (i + r) % n))).collect(),
        }
    }
    fn target(&self, i: usize) -> u32 {
        self.points * (i as u32 + 1)
    }
    fn total(&self) -> u32 {
        self.target(self.bouts().len().saturating_sub(1))
    }
    fn numbers(&self, side: usize) -> Vec<u32> {
        let n = self.size;
        if self.single { (0..n).map(|k| 2 * k + 1 + side as u32).collect() } else { (1..=n).map(|k| k + side as u32 * n).collect() }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct TeamState {
    revision: u64,
    top: Option<u8>,
    lineup: Vec<String>,
    relays: Vec<[u32; 2]>,
    current: [u32; 2],
    winner: Option<u8>,
    remaining_ms: f64,
}

#[component]
fn TeamMatch(who: Who, token: String, sheet: Value, bout: Value) -> impl IntoView {
    let index = u(&bout, "index");
    let relay = Relay::new(sheet["relay"].clone());
    let timing = sheet.get("timing").cloned().unwrap_or(Value::Null);
    let period_ms = (u(&timing, "period_seconds").clamp(1, 3600) as f64) * 1000.0;
    let fencers = arr(&bout, "fencers");
    let teams: [String; 2] = [0, 1].map(|k| fencers.get(k).map(|f| s(f, "name")).unwrap_or_default());
    let members: [Vec<String>; 2] = [0, 1].map(|k| fencers.get(k).map(|f| arr(f, "members").iter().filter_map(|m| m.as_str().map(str::to_string)).collect()).unwrap_or_default());
    let key = who.key(&token, index);
    let initial: TeamState = load(&key).unwrap_or_else(|| {
        let l = bout.get("relay_log").cloned().unwrap_or(Value::Null);
        let relays: Vec<[u32; 2]> = arr(&l, "relays").iter().filter_map(|r| Some([r.get(0)?.as_u64()? as u32, r.get(1)?.as_u64()? as u32])).collect();
        TeamState {
            revision: u(&l, "revision"),
            top: (!l.is_null()).then(|| u(&l, "top") as u8),
            lineup: arr(&l, "lineup").iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
            current: relays.last().copied().unwrap_or([0, 0]),
            relays,
            winner: l.get("winner").and_then(Value::as_u64).map(|w| w as u8),
            remaining_ms: period_ms,
        }
    });
    let clock = Clock::new(period_ms, initial.remaining_ms);
    let top = RwSignal::new(initial.top);
    let started = RwSignal::new(initial.top.is_some());
    let lineup = RwSignal::new(if initial.lineup.is_empty() { vec![String::new(); 2 * relay.size as usize] } else { initial.lineup.clone() });
    let relays = RwSignal::new(initial.relays.clone());
    let current = RwSignal::new(initial.current);
    let winner = RwSignal::new(initial.winner);
    let revision = StoredValue::new(initial.revision);
    let status = RwSignal::new(String::new());
    let sending = RwSignal::new(false);
    let relay = StoredValue::new(relay);
    let ctx = StoredValue::new((who, token, key, teams.clone(), members));
    let piste = u(&bout, "piste");
    let (competition, color) = (s(&sheet, "competition"), s(&sheet, "color"));

    let log = move || {
        json!({
            "revision": revision.get_value(), "format": relay.get_value().json, "top": top.get_untracked().unwrap_or(0),
            "lineup": lineup.get_untracked(), "relays": relays.get_untracked(), "winner": winner.get_untracked(),
        })
    };
    let store = move || {
        let (_, _, key, _, _) = ctx.get_value();
        save(&key, &TeamState {
            revision: revision.get_value(), top: top.get_untracked(), lineup: lineup.get_untracked(), relays: relays.get_untracked(),
            current: current.get_untracked(), winner: winner.get_untracked(), remaining_ms: clock.remaining_now(),
        });
    };
    // Journal envoyé après la composition et chaque relais (sans valider le score).
    let sync = move || {
        revision.update_value(|r| *r += 1);
        store();
        let (who, token, _, _, _) = ctx.get_value();
        let body = json!({ "token": token, "bout": index, "relay": log() });
        spawn_local(async move {
            if let Err(e) = post_score(&who, body).await {
                status.set(format!("Gardé sur ce téléphone : {e}"));
            }
        });
    };
    let over = move || {
        let r = relay.get_value();
        let list = relays.get();
        list.len() >= r.bouts().len() || list.last().is_some_and(|x| x[0] >= r.total() || x[1] >= r.total())
    };

    let live_key = StoredValue::new((String::new(), 0.0_f64));
    let live_state = move |end: bool| {
        let r = relay.get_value();
        let t = top.get_untracked().unwrap_or(0) as usize;
        let (_, _, _, teams, _) = ctx.get_value();
        let i = relays.get_untracked().len().min(r.bouts().len() - 1);
        let (a, b2) = r.bouts()[i];
        let cur = current.get_untracked();
        json!({
            "piste": piste, "competition": competition, "color": color,
            "bout": format!("Relais {}/{} : {a}-{b2}", i + 1, r.bouts().len()),
            "red": { "name": teams[t], "score": cur[0].to_string() },
            "green": { "name": teams[1 - t], "score": cur[1].to_string() },
            "remaining_ms": clock.remaining_now().round() as u64, "running": clock.running.get_untracked(),
            "period": i + 1, "periods": r.bouts().len(), "over": end,
        })
    };
    let live_state = StoredValue::new(live_state);
    run_clock(clock, move || {
        if clock.running.get_untracked() && clock.remaining_now() <= 0.0 {
            clock.pause();
            beep();
            store();
        }
        if piste == 0 || !started.get_untracked() {
            return;
        }
        let k = format!("{}|{:?}|{}|{}", relays.get_untracked().len(), current.get_untracked(), clock.running.get_untracked(),
            if clock.running.get_untracked() { String::new() } else { (clock.remaining_now() as u64).to_string() });
        let (last, at) = live_key.get_value();
        if k != last || now() - at > if clock.running.get_untracked() { 3000.0 } else { 20000.0 } {
            live_key.set_value((k, now()));
            push_live(&ctx.get_value().0, (live_state.get_value())(false));
        }
    });

    let name = move |n: u32| lineup.get().get(n as usize - 1).cloned().unwrap_or_default();

    // Composition : équipe du haut, puis le tireur de chaque numéro.
    let composition = move || {
        let (_, _, _, teams, members) = ctx.get_value();
        let r = relay.get_value();
        view! {
            <h1 class="ref-title">"Composition de la rencontre"</h1>
            <p class="ref-muted">{format!("{} · {} touches", if r.single { "Relais simple" } else { "Chacun contre chacun" }, r.total())}</p>
            <h2>{format!("Quelle équipe est en haut ({}) ?", r.numbers(0).iter().map(u32::to_string).collect::<Vec<_>>().join("-"))}</h2>
            <div class="ref-row">
                {[0u8, 1u8].into_iter().map(|k| { let label = teams[k as usize].clone(); view! {
                    <button class="ref-team" class:chosen=move || top.get() == Some(k) on:click=move |_| top.set(Some(k))>{label}</button>
                }}).collect_view()}
            </div>
            {move || top.get().map(|t| {
                let r = relay.get_value();
                [0usize, 1usize].into_iter().map(|side| {
                    let team = if side == 0 { t as usize } else { 1 - t as usize };
                    let options = members[team].clone();
                    view! {
                        <h2>{format!("{} : {}", if side == 0 { "En haut" } else { "En bas" }, teams[team])}</h2>
                        {r.numbers(side).into_iter().map(|n| {
                            let options = options.clone();
                            view! {
                                <div class="ref-row ref-lineup">
                                    <span class="ref-num">{format!("{n}.")}</span>
                                    <select on:change=move |ev| { let v = event_target_value(&ev); lineup.update(|l| { if let Some(x) = l.get_mut(n as usize - 1) { *x = v; } }); }>
                                        <option value="">"—"</option>
                                        {options.into_iter().map(|m| { let sel = lineup.get_untracked().get(n as usize - 1) == Some(&m); view! { <option value=m.clone() selected=sel>{m.clone()}</option> } }).collect_view()}
                                    </select>
                                </div>
                            }
                        }).collect_view()}
                    }
                }).collect_view()
            })}
            <p class="ref-muted ref-small">"Les noms sont facultatifs : seuls les numéros comptent pour l'ordre des relais."</p>
            <button class="ref-send" disabled=move || top.get().is_none() on:click=move |_| { started.set(true); sync(); }>"Commencer la rencontre"</button>
        }
    };

    // Relais en cours.
    let relay_screen = move || {
        let r = relay.get_value();
        let (_, _, _, teams, _) = ctx.get_value();
        let i = relays.get().len();
        let (a, b2) = r.bouts()[i.min(r.bouts().len() - 1)];
        let target = r.target(i);
        let previous = relays.get().last().copied().unwrap_or([0, 0]);
        let t = top.get().unwrap_or(0) as usize;
        let team_names = [teams[t].clone(), teams[1 - t].clone()];
        view! {
            <h1 class="ref-title">{format!("Relais {} / {}", i + 1, r.bouts().len())}</h1>
            <div class="ref-place">{move || format!("{a}. {}  contre  {b2}. {}", name(a), name(b2))}</div>
            <div class="ref-muted">{format!("Objectif : {target} touches (ou fin du temps)")}</div>
            <div class="ref-clock" class:zero=move || clock.remaining() <= 0.0 on:click=move |_| { if clock.running.get_untracked() { clock.pause(); store(); } else { clock.start(); } }>{move || mmss(clock.remaining())}</div>
            <div class="ref-row ref-center">
                <button class="ref-main" class:pause=move || clock.running.get() on:click=move |_| { if clock.running.get_untracked() { clock.pause(); store(); } else { clock.start(); } }>
                    {move || if clock.running.get() { "Pause" } else { "Démarrer le chrono" }}
                </button>
            </div>
            <div class="ref-board">
                {[0usize, 1usize].into_iter().map(|side| { let label = team_names[side].clone(); view! {
                    <div class="ref-side" class:red=side == 0 class:green=side == 1>
                        <div class="ref-name">{label}</div>
                        <div class="ref-big">{move || current.get()[side]}</div>
                        <div class="ref-row ref-center">
                            <button class="ref-step" on:click=move |_| { current.update(|c| c[side] = c[side].saturating_sub(1).max(previous[side])); store(); }>"−"</button>
                            <button class="ref-step plus" on:click=move |_| {
                                current.update(|c| c[side] = (c[side] + 1).min(target));
                                if current.get_untracked()[side] >= target { clock.pause(); }
                                store();
                            }>"+"</button>
                        </div>
                    </div>
                }}).collect_view()}
            </div>
            <button class="ref-send" on:click=move |_| {
                let c = current.get_untracked();
                let reached = c[0] >= target || c[1] >= target;
                if !reached && clock.remaining_now() > 0.0 && !ask(&format!("L'objectif de {target} n'est pas atteint. Terminer le relais au temps sur {} – {} ?", c[0], c[1])) {
                    return;
                }
                clock.pause();
                relays.update(|l| l.push(c));
                clock.reset();
                sync();
            }>{format!("Fin du relais {}", i + 1)}</button>
            {(i > 0).then(|| view! {
                <button class="ref-light ref-wide" on:click=move |_| {
                    if ask("Reprendre le relais précédent ?") {
                        relays.update(|l| { l.pop(); });
                        sync();
                    }
                }>"Corriger le relais précédent"</button>
            })}
        }
    };

    // Fin de la rencontre.
    let end_screen = move || {
        let (_, _, _, teams, _) = ctx.get_value();
        let last = relays.get().last().copied().unwrap_or([0, 0]);
        let t = top.get().unwrap_or(0) as usize;
        let (top_name, bottom_name) = (teams[t].clone(), teams[1 - t].clone());
        view! {
            <h1 class="ref-title">"Fin de la rencontre"</h1>
            <div class="ref-final">{format!("{top_name}  {} – {}  {bottom_name}", last[0], last[1])}</div>
            {(last[0] == last[1]).then(|| view! {
                <p class="ref-error">"Égalité : désignez l'équipe gagnante."</p>
                <div class="ref-row">
                    {[0u8, 1u8].into_iter().map(|k| { let label = teams[k as usize].clone(); view! {
                        <button class="ref-team" class:chosen=move || winner.get() == Some(k) on:click=move |_| { winner.set(Some(k)); store(); }>{label}</button>
                    }}).collect_view()}
                </div>
            })}
            <button class="ref-send" disabled=move || sending.get() on:click=move |_| {
                let last = relays.get_untracked().last().copied().unwrap_or([0, 0]);
                let t = top.get_untracked().unwrap_or(0);
                let by_sheet = if t == 0 { last } else { [last[1], last[0]] };
                let w = match by_sheet[0].cmp(&by_sheet[1]) {
                    std::cmp::Ordering::Greater => Some(0),
                    std::cmp::Ordering::Less => Some(1),
                    std::cmp::Ordering::Equal => winner.get_untracked().map(|w| w as usize),
                };
                let Some(w) = w else { status.set("Désignez l'équipe gagnante.".into()); return };
                let values: Vec<String> = (0..2).map(|k| if k == w { format!("W{}", by_sheet[k]) } else { by_sheet[k].to_string() }).collect();
                revision.update_value(|r| *r += 1);
                store();
                let (who, token, key, _, _) = ctx.get_value();
                let body = json!({ "token": token, "bout": index, "values": values, "relay": log() });
                sending.set(true);
                status.set("Envoi…".into());
                let back = format!("#/feuille/{}", enc(&token));
                spawn_local(async move {
                    match post_score(&who, body).await {
                        Ok(message) => {
                            forget(&key);
                            push_live(&who, (live_state.get_value())(true));
                            status.set(message.into());
                            gloo_timers_sleep(900).await;
                            go(&back);
                        }
                        Err(e) => {
                            sending.set(false);
                            status.set(e);
                        }
                    }
                });
            }>"Envoyer le résultat"</button>
            <button class="ref-light ref-wide" on:click=move |_| { relays.update(|l| { l.pop(); }); winner.set(None); sync(); }>"Corriger le dernier relais"</button>
        }
    };

    view! {
        <div class="ref-muted ref-small">{bout_title(&sheet, &bout)}</div>
        {move || if !started.get() {
            composition().into_any()
        } else if over() {
            end_screen().into_any()
        } else {
            relay_screen().into_any()
        }}
        {move || (started.get() && !relays.get().is_empty()).then(|| {
            let r = relay.get_value();
            view! {
                <h2>"Relais terminés"</h2>
                <ul class="ref-history">
                    {relays.get().into_iter().enumerate().map(|(i, x)| { let (a, b2) = r.bouts()[i]; view! { <li>{format!("{}. {a}-{b2} · {} – {}", i + 1, x[0], x[1])}</li> } }).collect_view()}
                </ul>
            }
        })}
        <p class="ref-status">{move || status.get()}</p>
    }
}
