//! Barrage (promus), scission (tireurs extraits) et classement général
//! (avec le classement de chaque groupe d'un entraînement des jeunes),
//! impression et exports.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::{json, Value};

use super::{arr, b, s, u, ListTable, PrintButton, StageCtx};
use crate::{alert, api};

#[component]
pub fn Barrage(ctx: StageCtx) -> impl IntoView {
    let toggle = Callback::new(move |(player, value): (u64, bool)| ctx.act.run(json!({ "player": player, "value": value })));
    view! {
        {move || {
            ctx.data
                .get()
                .map(|v| {
                    let d = v["data"].clone();
                    let locked = b(&d, "locked");
                    let bouts = arr(&d, "bouts");
                    let has_bouts = !bouts.is_empty();
                    view! {
                        <div class="stage-bar">
                            <strong>{format!("{} qualifiés · {} éliminés", u(&d, "qualified"), u(&d, "dropped"))}</strong>
                            <span class="spacer"></span>
                            {has_bouts.then(|| view! { <super::sheets::MatchSheetsButton ctx=ctx /> })}
                            <PrintButton />
                        </div>
                        {if has_bouts {
                            view! {
                                <p class="muted">
                                    {format!("Matchs en {} touches — {} place(s) à prendre. Le vainqueur est promu automatiquement.", u(&d, "max_score"), u(&d, "places"))}
                                </p>
                                <div class="bouts">
                                    {bouts.into_iter().map(|bout| view! { <BarrageBout ctx=ctx bout=bout locked=locked /> }).collect_view()}
                                </div>
                                <ListTable list=d["list"].clone() locked=true />
                            }
                                .into_any()
                        } else {
                            view! {
                                <p class="muted">"Cochez les tireurs promus à l'issue du barrage."</p>
                                <ListTable list=d["list"].clone() on_toggle=toggle locked=locked />
                            }
                                .into_any()
                        }}
                        <h4>"Classement"</h4>
                        <ListTable list=d["classification"].clone() />
                    }
                })
        }}
    }
}

/// Un match de barrage : deux tireurs et leurs scores (`V`, `3`, `W4`…).
#[component]
fn BarrageBout(ctx: StageCtx, bout: Value, locked: bool) -> impl IntoView {
    let index = u(&bout, "index");
    let winner = bout.get("winner").and_then(Value::as_str).map(str::to_string);
    view! {
        <div class="bout">
            <strong>{s(&bout, "name")}</strong>
            {arr(&bout, "fencers")
                .into_iter()
                .map(|f| {
                    let fencer = u(&f, "reference");
                    let id = format!("barrage-{index}-{fencer}");
                    view! {
                        <span class="opponent">
                            {s(&f, "name")}
                            <span class="muted">{format!(" {}", s(&f, "club"))}</span>
                            <input
                                id=id.clone()
                                class="score"
                                maxlength="4"
                                disabled=locked
                                value=s(&f, "score")
                                on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                                    if ev.key() == "Enter" {
                                        ev.prevent_default();
                                        ctx.act.run(json!({ "action": "score", "bout": index, "fencer": fencer, "value": event_target_value(&ev) }));
                                    }
                                }
                                on:change=move |ev| ctx.act.run(json!({ "action": "score", "bout": index, "fencer": fencer, "value": event_target_value(&ev) }))
                            />
                        </span>
                    }
                })
                .collect_view()}
            {winner.map(|w| view! { <span class="winner">{format!("→ {w}")}</span> })}
        </div>
    }
}

/// Classement de chaque groupe, puis de tous les groupes mélangés.
pub(super) fn youth_sections(results: Vec<Value>) -> impl IntoView {
    let mut groups: Vec<u64> = results.iter().map(|r| u(r, "group")).collect();
    groups.dedup();
    let row = |r: &Value, place: String| {
        view! {
            <tr>
                <td>{u(r, "group")}</td>
                <td>{place}</td>
                <td>{s(r, "name")}</td>
                <td>{s(r, "club")}</td>
                <td class="number">{u(r, "victories")}</td>
                <td class="number">{r.get("points").and_then(Value::as_u64).map(|p| p.to_string()).unwrap_or_default()}</td>
            </tr>
        }
    };
    let head = || {
        view! {
            <thead>
                <tr>
                    <th>"Groupe"</th>
                    <th>"Place"</th>
                    <th>"Nom"</th>
                    <th>"Club"</th>
                    <th class="number">"Victoires"</th>
                    <th class="number">"Points"</th>
                </tr>
            </thead>
        }
    };
    let mut all: Vec<Value> = results.clone();
    all.sort_by_key(|r| (std::cmp::Reverse(r.get("points").and_then(Value::as_u64).unwrap_or(0)), std::cmp::Reverse(u(r, "victories"))));
    let mut general = Vec::new();
    let (mut place, mut previous) = (0usize, None);
    for (i, r) in all.iter().enumerate() {
        let key = (r.get("points").and_then(Value::as_u64).unwrap_or(0), u(r, "victories"));
        if previous != Some(key) {
            place = i + 1;
            previous = Some(key);
        }
        general.push(row(r, place.to_string()));
    }
    view! {
        {groups
            .into_iter()
            .map(|g| {
                view! {
                    <h4>{format!("Groupe {g}")}</h4>
                    <table class="list">
                        {head()}
                        <tbody>
                            {results
                                .iter()
                                .filter(|r| u(r, "group") == g)
                                .map(|r| row(r, r.get("place").and_then(Value::as_u64).map(|p| p.to_string()).unwrap_or_default()))
                                .collect_view()}
                        </tbody>
                    </table>
                }
            })
            .collect_view()}
        <h4>"Classement général (tous les groupes)"</h4>
        <table class="list">
            {head()}
            <tbody>{general}</tbody>
        </table>
    }
}

/// Classement final de l'entraînement des jeunes pour l'impression.
fn youth_ranking_print(results: Vec<Value>) -> Value {
    serde_json::json!(results)
}

#[component]
pub fn Classification(ctx: StageCtx, contest: String) -> impl IntoView {
    let contest = StoredValue::new(contest);
    let first_place = RwSignal::new(String::from("1"));
    let export = move |format: &'static str| {
        let id = contest.get_value();
        let place = first_place.get_untracked().trim().parse::<u32>().unwrap_or(1).max(1);
        spawn_local(async move {
            let extension = format.split('-').next().unwrap_or(format);
            if let Err(e) = api::download(&format!("/api/contests/{id}/export/{format}?first_place={place}"), &format!("classement.{extension}")).await {
                alert(&e.message);
            }
        });
    };
    // Impression : le classement final seul, ou toute la compétition.
    let choosing = RwSignal::new(false);
    let print_all = move |final_only: bool| {
        choosing.set(false);
        let id = contest.get_value();
        let format = if final_only { "html-final" } else { "html" };
        spawn_local(async move {
            match api::text(&format!("/api/contests/{id}/export/{format}")).await {
                Ok(html) => super::print_html(&html),
                Err(e) => alert(&e.message),
            }
        });
    };
    view! {
        <div class="stage-bar">
            <span class="spacer"></span>
            <button class="light" title="Le classement final seul" on:click=move |_| export("html-final")>"HTML : classement final"</button>
            <button class="light" title="Poules, tableaux et classement final" on:click=move |_| export("html")>"HTML : résultats complets"</button>
            <label class="inline" title="Décalage des places de l'export FFF : place du premier de cette compétition (ex. 17 pour un tableau B)">
                "FFF à partir de la place "
                <input type="number" min="1" class="place-input" prop:value=move || first_place.get() on:input=move |ev| first_place.set(event_target_value(&ev)) />
            </label>
            <button class="light" on:click=move |_| export("fff")>"Export FFF"</button>
            <button class="light" on:click=move |_| export("frd")>"Export AskFred"</button>
            {move || {
                let d = ctx.data.get().map(|v| v["data"].clone()).unwrap_or(Value::Null);
                match d.get("youth").and_then(Value::as_array).cloned().filter(|r| !r.is_empty()) {
                    // Entraînement des jeunes : classement final puis chaque groupe rempli.
                    Some(results) => view! { <super::groups::GroupSheetsButton ctx=ctx ranking=youth_ranking_print(results) /> }.into_any(),
                    None => view! { <button class="light" on:click=move |_| choosing.set(true)>"Imprimer"</button> }.into_any(),
                }
            }}
        </div>
        {move || choosing.get().then(|| view! {
            <leptos::portal::Portal>
                <div class="modal-backdrop" on:click=move |_| choosing.set(false)>
                    <div class="modal" on:click=|ev| ev.stop_propagation()>
                        <h3>"Imprimer"</h3>
                        <div class="actions">
                            <button class="light" on:click=move |_| choosing.set(false)>"Annuler"</button>
                            <button title="Poules, lobbys, tableaux et classement final" on:click=move |_| print_all(false)>"Intégralité de la compétition"</button>
                            <button on:click=move |_| print_all(true)>"Classement final"</button>
                        </div>
                    </div>
                </div>
            </leptos::portal::Portal>
        })}
        {move || {
            ctx.data
                .get()
                .map(|v| {
                    let d = v["data"].clone();
                    match d.get("youth").and_then(Value::as_array).cloned() {
                        Some(results) => youth_sections(results).into_any(),
                        None => view! { <ListTable list=d["list"].clone() /> }.into_any(),
                    }
                })
        }}
    }
}
