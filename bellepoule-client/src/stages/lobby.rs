//! Lobby (série) : course ou jeu, tout le monde joue en même temps. Saisie
//! des arrivées lobby par lobby et manche par manche (place, même place pour
//! des ex aequo, « A » pour un abandon), points et classements ; réglages.

use leptos::prelude::*;
use serde_json::{json, Value};

use super::{arr, b, s, u, PrintButton, StageCtx};

fn standing(v: &Value, k: &str) -> u64 {
    v.get("standing").and_then(|s| s.get(k)).and_then(Value::as_u64).unwrap_or(0)
}

#[component]
pub fn Lobby(ctx: StageCtx) -> impl IntoView {
    let selected = RwSignal::new(0usize);
    let show_ranking = RwSignal::new(false);
    view! {
        {move || ctx.data.get().map(|v| {
            let d = v["data"].clone();
            let locked = b(&d, "locked");
            let lobbies = arr(&d, "lobbies");
            let races = d["settings"].get("races").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
            let sel = selected.get().min(lobbies.len().saturating_sub(1));
            let lobby = lobbies.get(sel).cloned();
            let per_lobby = d["settings"].get("per_lobby").and_then(Value::as_u64);
            let quota = if per_lobby.is_some() { 0 } else { u(&d, "nb_qualified") };
            view! {
                <LobbySettings ctx=ctx d=d.clone() locked=locked />
                <Composition ctx=ctx d=d.clone() locked=locked />
                <div class="stage-bar">
                    {lobbies.iter().map(|l| {
                        let i = u(l, "index") as usize;
                        let label = format!("Lobby {}{}", i + 1, if b(l, "over") { " ✔" } else { "" });
                        view! { <button class="light" class:active=move || !show_ranking.get() && selected.get() == i on:click=move |_| { selected.set(i); show_ranking.set(false); }>{label}</button> }
                    }).collect_view()}
                    <span class="spacer"></span>
                    <button class="light" class:active=move || show_ranking.get() on:click=move |_| show_ranking.update(|x| *x = !*x)>"Classement"</button>
                    <PrintButton />
                </div>
                {move || if show_ranking.get() {
                    view! {
                        <RankingTable title="Classement de la phase".into() list=arr(&d, "ranking") quota=quota />
                        {(!arr(&d, "individual").is_empty()).then(|| view! { <RankingTable title="Classement individuel".into() list=arr(&d, "individual") quota=0 /> })}
                    }.into_any()
                } else {
                    match lobby.clone() {
                        Some(l) => view! { <LobbyGrid ctx=ctx lobby=l races=races locked=locked /> }.into_any(),
                        None => view! { <p class="muted">"Les lobbys sont composés au début de la phase."</p> }.into_any(),
                    }
                }}
            }
        })}
    }
}

/// Formule (nombre de lobbys, d'après le nombre de participants) et
/// répartition, comme la composition des poules.
#[component]
fn Composition(ctx: StageCtx, d: Value, locked: bool) -> impl IntoView {
    let current = d["settings"].get("lobbies").and_then(Value::as_u64).unwrap_or(1);
    let distribution = d["settings"].get("distribution").and_then(Value::as_str).unwrap_or("balanced").to_string();
    let fixed = locked || b(&d, "has_results");
    let dist = StoredValue::new(distribution.clone());
    let lobbies = RwSignal::new(current);
    view! {
        <div class="stage-bar">
            <label class="inline">
                "Formule "
                <select disabled=fixed on:change=move |ev| {
                    let k = event_target_value(&ev).parse::<u64>().unwrap_or(1);
                    lobbies.set(k);
                    ctx.act.run(json!({ "action": "composition", "lobbies": k, "distribution": dist.get_value() }));
                }>
                    {arr(&d, "choices").into_iter().map(|c| {
                        let k = u(&c, "lobbies");
                        view! { <option value=k.to_string() selected=k == current>{s(&c, "label")}</option> }
                    }).collect_view()}
                </select>
            </label>
            <label class="inline" title="Équilibrée : serpentin d'après le classement d'entrée, comme les poules. Par force équivalente : les meilleurs ensemble dans le lobby 1, les suivants dans le lobby 2…">
                "Répartition "
                <select disabled=fixed on:change=move |ev| ctx.act.run(json!({ "action": "composition", "lobbies": lobbies.get_untracked(), "distribution": event_target_value(&ev) }))>
                    <option value="balanced" selected=distribution == "balanced">"équilibrée (serpentin)"</option>
                    <option value="level" selected=distribution == "level">"par force équivalente"</option>
                </select>
            </label>
            {fixed.then(|| view! { <span class="muted small">"Arrivées saisies : la composition est figée."</span> })}
        </div>
    }
}

/// Saisie d'un lobby : une colonne par manche.
#[component]
fn LobbyGrid(ctx: StageCtx, lobby: Value, races: usize, locked: bool) -> impl IntoView {
    let index = u(&lobby, "index");
    view! {
        {(!s(&lobby, "roadmap").is_empty()).then(|| view! { <p class="roadmap">{s(&lobby, "roadmap")}</p> })}
        <p class="muted small">"Tapez la place de chaque tireur dans la manche (la même place pour des ex aequo, A pour un abandon)."</p>
        <div class="table-scroll">
            <table class="list lobby-grid">
                <thead>
                    <tr>
                        <th>"Tireur"</th><th>"Club"</th>
                        {(1..=races).map(|k| view! { <th class="num">{format!("M{k}")}</th> }).collect_view()}
                        <th class="num">"Points"</th><th class="num">"Victoires"</th><th class="num">"Place"</th>
                    </tr>
                </thead>
                <tbody>
                    {arr(&lobby, "rows").into_iter().map(|r| {
                        let player = u(&r, "reference");
                        let finishes = arr(&r, "finishes");
                        let raced = standing(&r, "races") > 0;
                        view! {
                            <tr>
                                <td><strong>{s(&r, "name")}</strong></td>
                                <td class="muted">{s(&r, "club")}</td>
                                {(0..races).map(|k| {
                                    let value = match finishes.get(k) {
                                        Some(Value::Number(n)) => n.to_string(),
                                        Some(Value::String(x)) => x.clone(),
                                        _ => String::new(),
                                    };
                                    view! {
                                        <td class="num"><input class="place-input" type="text" inputmode="numeric" disabled=locked prop:value=value
                                            on:change=move |ev| ctx.act.run(json!({ "action": "finish", "lobby": index, "race": k, "player": player, "value": event_target_value(&ev) })) /></td>
                                    }
                                }).collect_view()}
                                <td class="num"><strong>{standing(&r, "points")}</strong></td>
                                <td class="num">{standing(&r, "wins")}</td>
                                <td class="num"><strong>{if raced { r.get("place").and_then(Value::as_u64).map(|p| p.to_string()).unwrap_or_default() } else { String::new() }}</strong></td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
        </div>
    }
}

#[component]
fn RankingTable(title: String, list: Vec<Value>, quota: u64) -> impl IntoView {
    view! {
        <h3>{title}</h3>
        <table class="list">
            <thead><tr><th class="num">"Pl."</th><th>"Nom"</th><th>"Club"</th><th class="num">"Lobby"</th><th class="num">"Points"</th><th class="num">"Victoires"</th><th class="num">"Meilleure place"</th></tr></thead>
            <tbody>
                {list.into_iter().map(|r| {
                    let place = u(&r, "place");
                    let best = standing(&r, "best");
                    view! {
                        <tr class:qualified={quota > 0 && place <= quota}>
                            <td class="num"><strong>{place}</strong></td>
                            <td><strong>{s(&r, "name")}</strong></td>
                            <td class="muted">{s(&r, "club")}</td>
                            <td class="num">{r.get("lobby").and_then(Value::as_u64).map(|l| l.to_string()).unwrap_or_default()}</td>
                            <td class="num"><strong>{standing(&r, "points")}</strong></td>
                            <td class="num">{standing(&r, "wins")}</td>
                            <td class="num">{if best > 0 { best.to_string() } else { "—".into() }}</td>
                        </tr>
                    }
                }).collect_view()}
            </tbody>
        </table>
    }
}

/// Réglages de la phase (aussi dans la fenêtre Formule).
#[component]
pub(crate) fn LobbySettings(ctx: StageCtx, d: Value, locked: bool) -> impl IntoView {
    let st = d["settings"].clone();
    let num = |k: &str, default: u64| st.get(k).and_then(Value::as_u64).unwrap_or(default).to_string();
    let lobbies = num("lobbies", 1);
    let distribution = st.get("distribution").and_then(Value::as_str).unwrap_or("balanced").to_string();
    let races = RwSignal::new(num("races", 1));
    let scale = RwSignal::new(st.get("scale").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_u64).map(|v| v.to_string()).collect::<Vec<_>>().join(", ")).unwrap_or_default());
    let abandon = RwSignal::new(num("abandon_points", 0));
    let per_lobby_on = RwSignal::new(st.get("per_lobby").is_some_and(|v| !v.is_null()));
    let per_lobby = RwSignal::new(st.get("per_lobby").and_then(Value::as_u64).unwrap_or(2).to_string());
    let qualified = RwSignal::new(d.get("nb_qualified").and_then(Value::as_u64).map(|q| q.to_string()).unwrap_or_default());
    let swapping = RwSignal::new(st.get("swapping").and_then(Value::as_array).map(|l| l.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>()).unwrap_or_default());
    let team_mode = RwSignal::new(st.get("team_mode").and_then(Value::as_str).unwrap_or("team").to_string());
    let team_rule = RwSignal::new(st.get("team_rule").and_then(Value::as_str).unwrap_or("sum").to_string());
    let team_best = RwSignal::new(num("team_best", 1));
    let team_event = b(&d, "team_event");
    let has_results = b(&d, "has_results");
    let save = move |_| {
        let parse = |v: String, d: u64| v.trim().parse::<u64>().unwrap_or(d);
        let settings = json!({
            "lobbies": parse(lobbies.clone(), 1).max(1),
            "distribution": distribution.clone(),
            "races": parse(races.get(), 1).max(1),
            "scale": scale.get().split([',', ';', ' ']).filter_map(|v| v.trim().parse::<u64>().ok()).collect::<Vec<_>>(),
            "abandon_points": parse(abandon.get(), 0),
            "per_lobby": if per_lobby_on.get() { json!(parse(per_lobby.get(), 1).max(1)) } else { Value::Null },
            "swapping": swapping.get(),
            "team_mode": team_mode.get(),
            "team_rule": team_rule.get(),
            "team_best": parse(team_best.get(), 1).max(1),
        });
        let q = qualified.get().trim().parse::<u64>().unwrap_or(0);
        ctx.act.run(json!({ "action": "settings", "settings": settings, "nb_qualified": q }));
    };
    let check = move |code: &'static str, label: &'static str| view! {
        <label class="check">
            <input type="checkbox" disabled=locked prop:checked=move || swapping.with(|l| l.iter().any(|x| x == code))
                on:change=move |ev| { let on = event_target_checked(&ev); swapping.update(|l| { l.retain(|x| x != code); if on { l.push(code.to_string()); } }); } />
            {label}
        </label>
    };
    view! {
        <details class="settings-panel" open=true>
            <summary>"Réglages du lobby"</summary>
            <div class="settings-grid">
                <label>"Manches"<input type="number" min="1" max="50" disabled=locked prop:value=move || races.get() on:input=move |ev| races.set(event_target_value(&ev)) /></label>
                <label title="Points du 1er, du 2e, du 3e… séparés par des virgules (ex. 25, 18, 15, 12, 10). Vide : N points au 1er, 1 au dernier.">
                    "Points par place"
                    <input type="text" placeholder="vide : N points au 1er, 1 au dernier" disabled=locked prop:value=move || scale.get() on:input=move |ev| scale.set(event_target_value(&ev)) />
                </label>
                <label>"Points d'un abandon"<input type="number" min="0" disabled=locked prop:value=move || abandon.get() on:input=move |ev| abandon.set(event_target_value(&ev)) /></label>
                <label>
                    "Qualifiés"
                    <select disabled=locked on:change=move |ev| per_lobby_on.set(event_target_value(&ev) == "lobby")>
                        <option value="total" selected=move || !per_lobby_on.get()>"au total (classement fusionné)"</option>
                        <option value="lobby" selected=move || per_lobby_on.get()>"par lobby"</option>
                    </select>
                </label>
                <Show when=move || per_lobby_on.get() fallback=move || view! {
                    <label>"Qualifiés au total (vide : tous)"<input type="number" min="0" disabled=locked prop:value=move || qualified.get() on:input=move |ev| qualified.set(event_target_value(&ev)) /></label>
                }>
                    <label>"Qualifiés par lobby"<input type="number" min="1" disabled=locked prop:value=move || per_lobby.get() on:input=move |ev| per_lobby.set(event_target_value(&ev)) /></label>
                </Show>
            </div>
            <div class="row">
                <span>"Séparer dans les lobbys par :"</span>
                {check("club", "club")}
                {check("league", "ligue")}
                {check("country", "pays")}
            </div>
            {team_event.then(|| view! {
                <div class="settings-grid">
                    <label>
                        "Épreuve par équipes"
                        <select disabled=locked on:change=move |ev| team_mode.set(event_target_value(&ev))>
                            <option value="team" selected=move || team_mode.get() == "team">"l'équipe court comme un seul participant"</option>
                            <option value="members" selected=move || team_mode.get() == "members">"chaque tireur de l'équipe court"</option>
                        </select>
                    </label>
                    <Show when=move || team_mode.get() == "members">
                        <label>
                            "Classement des équipes"
                            <select disabled=locked on:change=move |ev| team_rule.set(event_target_value(&ev))>
                                <option value="sum" selected=move || team_rule.get() == "sum">"somme des points de ses tireurs"</option>
                                <option value="best" selected=move || team_rule.get() == "best">"somme de ses meilleurs tireurs"</option>
                            </select>
                        </label>
                        <Show when=move || team_rule.get() == "best">
                            <label>"Tireurs comptés"<input type="number" min="1" disabled=locked prop:value=move || team_best.get() on:input=move |ev| team_best.set(event_target_value(&ev)) /></label>
                        </Show>
                    </Show>
                </div>
            })}
            <p class="muted small">"Dans un lobby, tout le monde court en même temps ; chaque manche rapporte des points selon la place d'arrivée. Ex aequo : même place, mêmes points. Classement : points, puis victoires, puis meilleure place."</p>
            <div class="row">
                <button disabled=locked on:click=save>"Enregistrer les réglages"</button>
                {(!has_results && !locked).then(|| view! { <button class="light" title="Nouvelle répartition en serpentin" on:click=move |_| ctx.act.run(json!({ "action": "compose" }))>"Recomposer les lobbys"</button> })}
            </div>
        </details>
    }
}
