//! Écrans des phases. Chaque écran lit la vue de sa phase
//! (`/api/contests/{id}/stages/{index}`) et envoie ses actions au serveur,
//! qui renvoie la vue à jour.

mod allocator;
mod divisions;
mod groups;
mod lists;
mod lobby;
mod pools;
mod sheets;
mod splitting;
mod table;
mod youth;
mod relay_sheets;

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::Value;

use crate::api;
use crate::alert;

/// Champ texte d'une valeur JSON.
pub fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

pub fn u(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

pub fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

pub fn arr(v: &Value, k: &str) -> Vec<Value> {
    v.get(k).and_then(Value::as_array).cloned().unwrap_or_default()
}

/// Donne le focus à l'élément `id` (après le rafraîchissement de l'écran).
pub fn focus_later(id: String) {
    set_timeout(
        move || {
            if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.get_element_by_id(&id)) {
                if let Some(input) = wasm_bindgen::JsCast::dyn_ref::<web_sys::HtmlInputElement>(&el) {
                    let _ = input.focus();
                    input.select();
                }
            }
        },
        std::time::Duration::from_millis(30),
    );
}

/// Accès à une phase : sa vue, l'envoi d'actions et la case à activer une
/// fois la réponse du serveur affichée.
#[derive(Clone, Copy)]
pub struct StageCtx {
    pub data: RwSignal<Option<Value>>,
    pub act: Callback<Value>,
    pub focus: RwSignal<Option<String>>,
    /// Adresse de la phase (`/api/contests/{id}/stages/{index}`).
    pub path: StoredValue<String>,
}

impl StageCtx {
    /// Action, puis focus sur `next` quand l'écran est à jour.
    pub fn act_then_focus(&self, body: Value, next: String) {
        self.focus.set(Some(next));
        self.act.run(body);
    }
}

#[component]
pub fn StageView(id: String, index: usize, kind: String, on_change: Callback<()>) -> impl IntoView {
    let data = RwSignal::new(None::<Value>);
    let focus = RwSignal::new(None::<String>);
    let path = StoredValue::new(format!("/api/contests/{id}/stages/{index}"));
    {
        let path = path.get_value();
        spawn_local(async move {
            match api::get::<Value>(&path).await {
                Ok(v) => data.set(Some(v)),
                Err(e) => alert(&e.message),
            }
        });
    }
    let act = Callback::new(move |body: Value| {
        let path = path.get_value();
        spawn_local(async move {
            match api::send::<_, Value>("POST", &path, &body).await {
                Ok(v) => {
                    data.set(Some(v));
                    if let Some(id) = focus.get_untracked() {
                        focus.set(None);
                        focus_later(id);
                    }
                    on_change.run(());
                }
                Err(e) => {
                    alert(&e.message);
                    // Revenir à l'état du serveur.
                    if let Ok(v) = api::get::<Value>(&path).await {
                        data.set(Some(v));
                    }
                }
            }
        });
    });
    let ctx = StageCtx { data, act, focus, path };
    let contest = StoredValue::new(id);
    view! {
        <BoutTimingSettings ctx=ctx />
        {move || match kind.as_str() {
            "youth" => view! { <youth::Youth ctx=ctx /> }.into_any(),
            "pool_allocator" => view! { <allocator::Allocator ctx=ctx /> }.into_any(),
            "pool_supervisor" => view! { <pools::Pools ctx=ctx /> }.into_any(),
            "table" => view! { <table::Table ctx=ctx /> }.into_any(),
            "barrage" => view! { <lists::Barrage ctx=ctx /> }.into_any(),
            "splitting" => view! { <splitting::Splitting ctx=ctx /> }.into_any(),
            "divisions" => view! { <divisions::Divisions ctx=ctx /> }.into_any(),
            "lobby" => view! { <lobby::Lobby ctx=ctx /> }.into_any(),
            "classification" => view! { <lists::Classification ctx=ctx contest=contest.get_value() /> }.into_any(),
            _ => view! { <p class="muted">"Rien à afficher pour cette phase."</p> }.into_any(),
        }}
    }
}

/// Réglages d'une phase, comme les préférences de la formule du logiciel :
/// accessibles avant que la phase commence (formule complète dès le départ).
#[component]
pub fn StageSettings(id: String, index: usize, kind: String, on_change: Callback<()>) -> impl IntoView {
    let data = RwSignal::new(None::<Value>);
    let focus = RwSignal::new(None::<String>);
    let path = StoredValue::new(format!("/api/contests/{id}/stages/{index}"));
    {
        let path = path.get_value();
        spawn_local(async move {
            match api::get::<Value>(&path).await {
                Ok(v) => data.set(Some(v)),
                Err(e) => alert(&e.message),
            }
        });
    }
    let act = Callback::new(move |body: Value| {
        let path = path.get_value();
        spawn_local(async move {
            match api::send::<_, Value>("POST", &path, &body).await {
                Ok(v) => {
                    data.set(Some(v));
                    on_change.run(());
                }
                Err(e) => {
                    alert(&e.message);
                    if let Ok(v) = api::get::<Value>(&path).await {
                        data.set(Some(v));
                    }
                }
            }
        });
    });
    let ctx = StageCtx { data, act, focus, path };
    let has_timing = matches!(kind.as_str(), "pool_supervisor" | "table" | "barrage" | "youth" | "divisions" | "lobby");
    view! {
        {has_timing.then(|| view! { <BoutTimingSettings ctx=ctx /> })}
        {move || ctx.data.get().map(|v| {
            let d = v["data"].clone();
            let locked = b(&d, "locked");
            match kind.as_str() {
                "pool_supervisor" => view! { <pools::PoolSettings ctx=ctx d=d.clone() locked=locked /> }.into_any(),
                "table" => view! { <table::TableSettings ctx=ctx d=d.clone() locked=locked /> }.into_any(),
                "splitting" => view! {
                    <div class="stage-bar">
                        <label class="inline">
                            "Libellé (utilisé par GREG) "
                            <input type="text" disabled=locked prop:value=s(&d, "label")
                                on:change=move |ev| ctx.act.run(serde_json::json!({ "action": "label", "label": event_target_value(&ev) })) />
                        </label>
                    </div>
                    <splitting::DivisionsSettings ctx=ctx config=d["config"].clone() locked=locked />
                }.into_any(),
                "youth" => view! { <youth::Settings ctx=ctx settings=d["settings"].clone() choices=arr(&d, "swapping_choices") locked=locked /> }.into_any(),
                "divisions" => view! { <divisions::Settings ctx=ctx settings=d["settings"].clone() locked=locked /> }.into_any(),
                "lobby" => view! { <lobby::LobbySettings ctx=ctx d=d.clone() locked=locked /> }.into_any(),
                _ => view! { <p class="muted">"Pas de réglages pour cette phase."</p> }.into_any(),
            }
        })}
    }
}

/// Bouton « Imprimer » (impression du navigateur, mise en page `@media print`).
#[component]
pub fn PrintButton() -> impl IntoView {
    view! {
        <button class="light" on:click=move |_| {
            if let Some(w) = web_sys::window() {
                let _ = w.print();
            }
        }>"Imprimer"</button>
    }
}

/// Impression d'une page HTML complète (export des résultats) dans un cadre
/// caché, avec sa propre mise en page.
pub fn print_html(html: &str) {
    use wasm_bindgen::JsCast;
    let Some(document) = web_sys::window().and_then(|w| w.document()) else { return };
    let Some(frame) = document.create_element("iframe").ok().and_then(|e| e.dyn_into::<web_sys::HtmlIFrameElement>().ok()) else { return };
    let _ = frame.set_attribute("style", "position:fixed;right:0;bottom:0;width:0;height:0;border:0");
    // Sans le rechargement automatique de la page exportée.
    frame.set_srcdoc(&html.replace("setInterval (function(){location.reload ();}, 60*1000);", ""));
    let target = frame.clone();
    let onload = wasm_bindgen::closure::Closure::once_into_js(move || {
        if let Some(w) = target.content_window() {
            let _ = w.focus();
            let _ = w.print();
        }
        // Retiré après l'impression (la boîte d'impression est bloquante).
        let cleanup = target.clone();
        let later = wasm_bindgen::closure::Closure::once_into_js(move || cleanup.remove());
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(later.unchecked_ref(), 60_000);
        }
    });
    frame.set_onload(Some(onload.unchecked_ref()));
    if let Some(body) = document.body() {
        let _ = body.append_child(&frame);
    }
}

/// Liste générique renvoyée par l'API (`columns`, `rows`).
#[component]
pub fn ListTable(list: Value, #[prop(optional)] on_toggle: Option<Callback<(u64, bool)>>, #[prop(optional)] locked: bool) -> impl IntoView {
    let columns = arr(&list, "columns");
    let rows = arr(&list, "rows");
    let has_check = rows.iter().any(|r| r.get("checked").map(|c| !c.is_null()).unwrap_or(false));
    view! {
        <table class="list">
            <thead>
                <tr>
                    {has_check.then(|| view! { <th></th> })}
                    {columns.iter().map(|c| view! { <th>{s(c, "title")}</th> }).collect_view()}
                </tr>
            </thead>
            <tbody>
                {rows
                    .into_iter()
                    .map(|r| {
                        let reference = u(&r, "reference");
                        let checked = r.get("checked").and_then(Value::as_bool);
                        view! {
                            <tr class:absent=b(&r, "dimmed")>
                                {checked
                                    .map(|c| {
                                        view! {
                                            <td>
                                                <input
                                                    type="checkbox"
                                                    checked=c
                                                    disabled=locked
                                                    on:change=move |ev| {
                                                        if let Some(cb) = on_toggle {
                                                            cb.run((reference, event_target_checked(&ev)));
                                                        }
                                                    }
                                                />
                                            </td>
                                        }
                                    })}
                                {arr(&r, "cells").iter().map(|c| view! { <td>{c.as_str().unwrap_or("").to_string()}</td> }).collect_view()}
                            </tr>
                        }
                    })
                    .collect_view()}
            </tbody>
        </table>
    }
}


#[component]
fn BoutTimingSettings(ctx: StageCtx) -> impl IntoView {
    view! { {move || ctx.data.get().and_then(|view| {
        let kind = s(&view, "kind");
        if !["pool_supervisor", "pool_allocator", "table", "barrage"].contains(&kind.as_str()) { return None; }
        let timing = &view["data"]["timing"];
        let seconds = RwSignal::new(timing.get("period_seconds").and_then(Value::as_u64).unwrap_or(180).to_string());
        let periods = RwSignal::new(timing.get("periods").and_then(Value::as_u64).unwrap_or(1).to_string());
        let locked = b(&view, "locked");
        let team = b(&view["data"], "team");
        let relay = &timing["relay"];
        let relay_kind = RwSignal::new(relay.get("kind").and_then(Value::as_str).unwrap_or("each_vs_each").to_string());
        let team_size = relay.get("team_size").and_then(Value::as_u64).unwrap_or(3);
        let substitutes = relay.get("substitutes").and_then(Value::as_u64).unwrap_or(1);
        let points = RwSignal::new(relay.get("points").and_then(Value::as_u64).unwrap_or(5).to_string());
        let order = view["data"]["relay"]["bouts"].as_array().map(|l| l.iter().filter_map(|p| Some(format!("{}-{}", p.get(0)?, p.get(1)?))).collect::<Vec<_>>().join(" / ")).unwrap_or_default();
        let total = view["data"]["relay"]["total"].as_u64().unwrap_or(0);
        let numbers = |k: usize| view["data"]["relay"]["numbers"][k].as_array().map(|l| l.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("-")).unwrap_or_default();
        let (top, bottom) = (numbers(0), numbers(1));
        let apply = move |_| {
            ctx.act.run(serde_json::json!({"action":"timing", "timing":{
                "period_seconds": seconds.get().parse::<u32>().unwrap_or(0),
                "periods": periods.get().parse::<u32>().unwrap_or(1),
                "relay": {
                    "kind": relay_kind.get(),
                    "points": points.get().parse::<u32>().unwrap_or(5),
                },
            }}))
        };
        if team {
            return Some(view! {
                <details class="settings" open=true>
                    <summary>"Rencontres par équipes — relais"</summary>
                    <div class="settings-row">
                        <label class="inline">"Type de rencontre "
                            <select disabled=locked on:change=move |ev| relay_kind.set(event_target_value(&ev))>
                                <option value="each_vs_each" selected=move || relay_kind.get() == "each_vs_each">"Chacun contre chacun"</option>
                                <option value="single" selected=move || relay_kind.get() == "single">"Relais simple (un match par tireur)"</option>
                            </select>
                        </label>
                        <label class="inline">"Points par relais "<input type="number" min="1" max="15" disabled=locked prop:value=move || points.get() on:input=move |ev| points.set(event_target_value(&ev)) /></label>
                        <label class="inline">"Temps maximum d'un relais (secondes) "<input type="number" min="1" max="3600" disabled=locked prop:value=move || seconds.get() on:input=move |ev| seconds.set(event_target_value(&ev)) /></label>
                        <button disabled=locked on:click=apply>"Appliquer"</button>
                        <relay_sheets::RelaySheetsButton ctx=ctx blank_first=true />
                    </div>
                    <p class="muted small">{format!("Équipes de {team_size} tireurs et {substitutes} remplaçant(s) (propriétés de la compétition) · Haut : {top} · Bas : {bottom} · Ordre des relais : {order} · Score de la rencontre : {total} touches")}</p>
                </details>
            }.into_any());
        }
        Some(view! {
            <details class="settings">
                <summary>"Chronomètre — formule des matchs"</summary>
                <div class="settings-row">
                    <label class="inline">"Durée d'une manche (secondes) "<input type="number" min="1" max="3600" disabled=locked prop:value=move || seconds.get() on:input=move |ev| seconds.set(event_target_value(&ev)) /></label>
                    <label class="inline">"Nombre de manches "<input type="number" min="1" max="20" disabled=locked prop:value=move || periods.get() on:input=move |ev| periods.set(event_target_value(&ev)) /></label>
                    <button disabled=locked on:click=apply>"Appliquer"</button>
                </div>
            </details>
        }.into_any())
    })} }
}
