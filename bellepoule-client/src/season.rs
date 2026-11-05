//! Classement de saison du club : cumul des points des compétitions dont
//! les points sont activés, par catégorie ou par circuit (catégories,
//! armes, genres, compétitions choisies), classement général de plusieurs
//! circuits, avec les corrections à la main.

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::{json, Value};

use crate::alert;
use crate::api;
use crate::icons::Icon;

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn number(v: &Value) -> String {
    match v.as_f64() {
        Some(n) if (n - n.round()).abs() < 1e-9 => format!("{}", n.round() as i64),
        Some(n) => format!("{n:.2}").trim_end_matches('0').replace('.', ","),
        None => String::new(),
    }
}

/// Téléchargement d'un texte (CSV) produit dans le navigateur.
fn save_text(name: &str, text: &str) {
    let array = js_sys::Array::new();
    array.push(&wasm_bindgen::JsValue::from_str(&format!("\u{feff}{text}")));
    let Ok(blob) = web_sys::Blob::new_with_str_sequence(&array) else { return };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else { return };
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if let Ok(a) = doc.create_element("a") {
            let _ = a.set_attribute("href", &url);
            let _ = a.set_attribute("download", name);
            if let Some(a) = wasm_bindgen::JsCast::dyn_ref::<web_sys::HtmlElement>(&a) {
                a.click();
            }
        }
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}

fn strings(v: &Value, k: &str) -> Vec<String> {
    v.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

#[component]
pub fn SeasonPage() -> impl IntoView {
    // Classement affiché : "" (toutes), "cat:M11" (une catégorie), "cir:c1" (un circuit).
    let category = RwSignal::new(String::new());
    let version = RwSignal::new(0u32);
    let data = LocalResource::new(move || {
        version.get();
        let c = category.get();
        let query = match c.split_once(':') {
            Some(("cir", id)) => format!("circuit={}", js_sys::encode_uri_component(id)),
            Some(("cat", cat)) => format!("category={}", js_sys::encode_uri_component(cat)),
            _ => String::new(),
        };
        async move { api::get::<Value>(&format!("/api/season?{query}")).await }
    });
    // Fenêtre du circuit : `Some(Null)` pour un nouveau circuit.
    let editing = RwSignal::new(None::<Value>);
    let saved = Callback::new(move |id: String| {
        editing.set(None);
        category.set(format!("cir:{id}"));
        version.update(|v| *v += 1);
    });
    let current_circuit = move || {
        let sel = category.get();
        let id = sel.strip_prefix("cir:")?.to_string();
        data.get().and_then(Result::ok)?["circuits"].as_array()?.iter().find(|c| s(c, "id") == id).cloned()
    };
    let delete_circuit = move |_| {
        let Some(c) = current_circuit() else { return };
        if !crate::confirm(&format!("Supprimer le circuit « {} » ? Les compétitions ne sont pas touchées.", s(&c, "name"))) {
            return;
        }
        let id = s(&c, "id");
        spawn_local(async move {
            if let Err(e) = api::delete(&format!("/api/season/circuits/{}", js_sys::encode_uri_component(&id))).await {
                alert(&e.message);
            }
            category.set(String::new());
            version.update(|v| *v += 1);
        });
    };
    let selected = RwSignal::new(Vec::<String>::new());
    let correct = move |body: Value| {
        spawn_local(async move {
            if let Err(e) = api::post_empty("/api/season/corrections", &body).await {
                alert(&e.message);
            }
            selected.set(Vec::new());
            version.update(|v| *v += 1);
        });
    };
    view! {
        <header class="page-head">
            <div>
                <h1>"Classement de saison"</h1>
                <p class="muted">"Cumul des points des compétitions du club dont les points sont activés (préférences de la compétition)."</p>
            </div>
            <span class="spacer"></span>
            <label class="inline">
                "Classement "
                <select on:change=move |ev| { category.set(event_target_value(&ev)); selected.set(Vec::new()); }>
                    <option value="">"Toutes les compétitions"</option>
                    {move || data.get().and_then(Result::ok).map(|d| {
                        let current = category.get_untracked();
                        let circuits = d["circuits"].as_array().cloned().unwrap_or_default();
                        let categories = strings(&d, "categories");
                        view! {
                            {(!circuits.is_empty()).then(|| view! {
                                <optgroup label="Circuits">
                                    {circuits.into_iter().map(|c| {
                                        let value = format!("cir:{}", s(&c, "id"));
                                        let general = c["circuits"].as_array().is_some_and(|a| !a.is_empty());
                                        let label = if general { format!("{} (général)", s(&c, "name")) } else { s(&c, "name") };
                                        let on = current == value;
                                        view! { <option value=value selected=on>{label}</option> }
                                    }).collect_view()}
                                </optgroup>
                            })}
                            <optgroup label="Par catégorie">
                                {categories.into_iter().map(|c| {
                                    let value = format!("cat:{c}");
                                    let on = current == value;
                                    view! { <option value=value selected=on>{format!("Catégorie {c}")}</option> }
                                }).collect_view()}
                            </optgroup>
                        }
                    })}
                </select>
            </label>
            <button class="light small" on:click=move |_| editing.set(Some(Value::Null))>"Nouveau circuit"</button>
            {move || current_circuit().map(|c| view! {
                <button class="light small" on:click=move |_| editing.set(Some(c.clone()))>"Modifier le circuit"</button>
                <button class="ghost small" on:click=delete_circuit>"Supprimer"</button>
            })}
        </header>
        {move || editing.get().map(|c| {
            let d = data.get().and_then(Result::ok).unwrap_or(Value::Null);
            view! { <CircuitDialog data=d circuit=c on_saved=saved on_close=Callback::new(move |_| editing.set(None)) /> }
        })}
        {move || match data.get() {
            None => view! { <div class="card skeleton-block"></div> }.into_any(),
            Some(Err(e)) => view! { <p class="error">{e.message}</p> }.into_any(),
            Some(Ok(d)) => {
                let contests = d["contests"].as_array().cloned().unwrap_or_default();
                let rows = d["rows"].as_array().cloned().unwrap_or_default();
                if contests.is_empty() && !category.get_untracked().is_empty() {
                    return view! {
                        <div class="empty">
                            <Icon name="trophy" />
                            <h2>"Aucune compétition dans ce classement"</h2>
                            <p class="muted">"Modifiez le circuit pour choisir ses catégories, armes, genres ou compétitions."</p>
                        </div>
                    }.into_any();
                }
                if contests.is_empty() {
                    return view! {
                        <div class="empty">
                            <Icon name="trophy" />
                            <h2>"Aucune compétition avec des points"</h2>
                            <p class="muted">"Activez les points dans les propriétés d'une compétition (partie « Points ») ; elle compte dès qu'elle est classée."</p>
                        </div>
                    }.into_any();
                }
                let titles: Vec<String> = contests.iter().map(|c| format!("{} {}", s(c, "name"), s(c, "date")).trim().to_string()).collect();
                let csv = {
                    let mut out = format!("Rang;Prénom;Nom;Catégorie;Compétitions;Total;{}\n", titles.join(";"));
                    for r in &rows {
                        let pts: Vec<String> = r["points"].as_array().cloned().unwrap_or_default().iter().map(number).collect();
                        out.push_str(&format!("{};{};{};{};{};{};{}\n", r["rank"], s(r, "first_name"), s(r, "last_name"), s(r, "category"), r["competitions"], number(&r["total"]), pts.join(";")));
                    }
                    out
                };
                let corrections = d["corrections"].as_u64().unwrap_or(0);
                let best = d["best"].as_u64();
                view! {
                    <section class="card">
                        <div class="card-head">
                            <span class="chip">{format!("{} compétition(s)", contests.len())}</span>
                            <span class="chip">{format!("{} tireur(s)", rows.len())}</span>
                            {best.map(|n| view! { <span class="chip accent">{format!("{n} meilleur(s) résultat(s)")}</span> })}
                            {(corrections > 0).then(|| view! { <span class="chip accent">{format!("{corrections} correction(s)")}</span> })}
                            <span class="spacer"></span>
                            <button class="light small" disabled=move || selected.get().len() < 2 title="Les lignes cochées sont un seul et même tireur"
                                on:click=move |_| correct(json!({ "action": "merge", "keys": selected.get_untracked() }))>"Regrouper"</button>
                            <button class="light small" disabled=move || selected.get().len() != 1 title="Distinguer les tireurs de ce prénom par leur nom et leur catégorie"
                                on:click={
                                    let rows = rows.clone();
                                    move |_| {
                                        let key = selected.get_untracked().first().cloned().unwrap_or_default();
                                        let first = rows.iter().find(|r| s(r, "key") == key).map(|r| s(r, "first_name")).unwrap_or_default();
                                        correct(json!({ "action": "split", "first_name": first }));
                                    }
                                }>"Séparer ce prénom"</button>
                            {(corrections > 0).then(|| view! {
                                <button class="ghost small" on:click=move |_| { if crate::confirm("Annuler toutes les corrections faites à la main ?") { correct(json!({ "action": "reset" })); } }>"Annuler les corrections"</button>
                            })}
                            <button class="light small" on:click=move |_| save_text("classement-saison.csv", &csv)><Icon name="download" />"CSV"</button>
                            <button class="light small" on:click=move |_| { if let Some(w) = web_sys::window() { let _ = w.print(); } }>"Imprimer"</button>
                        </div>
                        <div class="table-scroll">
                            <table class="list season">
                                <thead>
                                    <tr>
                                        <th></th>
                                        <th class="number">"Rang"</th>
                                        <th>"Prénom"</th>
                                        <th>"Nom"</th>
                                        <th>"Catégorie"</th>
                                        <th class="number">"Compét."</th>
                                        <th class="number">"Total"</th>
                                        {titles.iter().map(|t| view! { <th class="number">{t.clone()}</th> }).collect_view()}
                                    </tr>
                                </thead>
                                <tbody>
                                    {rows.into_iter().map(|r| {
                                        let key = s(&r, "key");
                                        let k2 = key.clone();
                                        view! {
                                            <tr class:selected=move || selected.get().contains(&k2)>
                                                <td>
                                                    <input type="checkbox" prop:checked={ let k = key.clone(); move || selected.get().contains(&k) }
                                                        on:change={ let k = key.clone(); move |ev| { let on = event_target_checked(&ev); selected.update(|l| { l.retain(|x| *x != k); if on { l.push(k.clone()); } }); } } />
                                                </td>
                                                <td class="number">{r["rank"].as_u64().unwrap_or(0)}</td>
                                                <td class="name">{s(&r, "first_name")}</td>
                                                <td>{s(&r, "last_name")}</td>
                                                <td>{s(&r, "category")}</td>
                                                <td class="number">{r["competitions"].as_u64().unwrap_or(0)}</td>
                                                <td class="number"><strong>{number(&r["total"])}</strong></td>
                                                {r["points"].as_array().cloned().unwrap_or_default().iter().map(|p| view! { <td class="number">{number(p)}</td> }).collect_view()}
                                            </tr>
                                        }
                                    }).collect_view()}
                                </tbody>
                            </table>
                        </div>
                        <p class="muted small">"Reconnaissance des tireurs : prénom seul s'il est unique, sinon prénom et catégorie, sinon prénom, nom et catégorie. Cochez des lignes pour les regrouper ou séparer un prénom."</p>
                    </section>
                }.into_any()
            }
        }}
    }
}

/// Coche d'une valeur dans une liste (catégories, armes, genres, circuits).
fn toggle(list: RwSignal<Vec<String>>, value: String, on: bool) {
    list.update(|l| {
        l.retain(|x| *x != value);
        if on {
            l.push(value);
        }
    });
}

/// Cases à cocher d'un filtre (vide : tout).
fn checks(title: &'static str, values: Vec<String>, list: RwSignal<Vec<String>>) -> impl IntoView {
    view! {
        <fieldset>
            <legend>{title}<span class="muted small">" (aucune cochée : toutes)"</span></legend>
            <div class="check-row">
                {values.into_iter().map(|v| {
                    let v2 = v.clone();
                    let v3 = v.clone();
                    view! {
                        <label class="inline">
                            <input type="checkbox" prop:checked=move || list.get().contains(&v2) on:change=move |ev| toggle(list, v3.clone(), event_target_checked(&ev)) />
                            {v}
                        </label>
                    }
                }).collect_view()}
            </div>
        </fieldset>
    }
}

/// Création / modification d'un circuit : filtres (catégories, armes,
/// genres), compétitions cochées à la main, ou classement général de
/// plusieurs circuits ; N meilleurs résultats.
#[component]
fn CircuitDialog(data: Value, circuit: Value, on_saved: Callback<String>, on_close: Callback<()>) -> impl IntoView {
    let id = s(&circuit, "id");
    let name = RwSignal::new(s(&circuit, "name"));
    let categories = RwSignal::new(strings(&circuit, "categories"));
    let weapons = RwSignal::new(strings(&circuit, "weapons"));
    let genders = RwSignal::new(strings(&circuit, "genders"));
    let include = RwSignal::new(strings(&circuit, "include"));
    let exclude = RwSignal::new(strings(&circuit, "exclude"));
    let circuits = RwSignal::new(strings(&circuit, "circuits"));
    let general = RwSignal::new(!circuits.get_untracked().is_empty());
    let best = RwSignal::new(circuit.get("best").and_then(Value::as_u64).map(|n| n.to_string()).unwrap_or_default());
    let all = data["all"].as_array().cloned().unwrap_or_default();
    let others: Vec<Value> = data["circuits"].as_array().cloned().unwrap_or_default().into_iter().filter(|c| s(c, "id") != id).collect();
    let id = StoredValue::new(id);
    // La compétition répond aux filtres (sans les ajouts / retraits).
    let matches = move |c: &Value| {
        let ok = |list: RwSignal<Vec<String>>, v: String| list.with(|l| l.is_empty() || l.contains(&v));
        ok(categories, s(c, "category")) && ok(weapons, s(c, "weapon")) && ok(genders, s(c, "gender"))
    };
    let save = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let general = general.get_untracked();
        let body = json!({
            "id": id.get_value(),
            "name": name.get_untracked(),
            "categories": if general { Vec::new() } else { categories.get_untracked() },
            "weapons": if general { Vec::new() } else { weapons.get_untracked() },
            "genders": if general { Vec::new() } else { genders.get_untracked() },
            "include": if general { Vec::new() } else { include.get_untracked() },
            "exclude": if general { Vec::new() } else { exclude.get_untracked() },
            "circuits": if general { circuits.get_untracked() } else { Vec::new() },
            "best": best.get_untracked().trim().parse::<u32>().ok().filter(|n| *n > 0),
        });
        if general && circuits.get_untracked().is_empty() {
            alert("Cochez les circuits à réunir.");
            return;
        }
        spawn_local(async move {
            match api::send::<_, Value>("POST", "/api/season/circuits", &body).await {
                Ok(v) => on_saved.run(s(&v, "id")),
                Err(e) => alert(&e.message),
            }
        });
    };
    view! {
        <div class="overlay">
            <form class="dialog" on:submit=save>
                <h2>{if id.get_value().is_empty() { "Nouveau circuit" } else { "Modifier le circuit" }}</h2>
                <label>"Nom" <input type="text" required maxlength="80" placeholder="ex. Circuit M11, Challenge des jeunes, Général"
                    prop:value=move || name.get() on:input=move |ev| name.set(event_target_value(&ev)) /></label>
                <div class="check-row">
                    <label class="inline"><input type="radio" name="circuit-kind" prop:checked=move || !general.get() on:change=move |_| general.set(false) />"Circuit (des compétitions)"</label>
                    <label class="inline"><input type="radio" name="circuit-kind" prop:checked=move || general.get() on:change=move |_| general.set(true) />"Classement général (plusieurs circuits)"</label>
                </div>
                {move || if general.get() {
                    view! {
                        <fieldset>
                            <legend>"Circuits réunis"</legend>
                            {if others.is_empty() {
                                view! { <p class="muted small">"Créez d'abord les circuits à réunir."</p> }.into_any()
                            } else {
                                view! {
                                    <div class="check-row">
                                        {others.iter().map(|c| {
                                            let cid = s(c, "id");
                                            let c2 = cid.clone();
                                            view! {
                                                <label class="inline">
                                                    <input type="checkbox" prop:checked=move || circuits.get().contains(&cid) on:change=move |ev| toggle(circuits, c2.clone(), event_target_checked(&ev)) />
                                                    {s(c, "name")}
                                                </label>
                                            }
                                        }).collect_view()}
                                    </div>
                                }.into_any()
                            }}
                        </fieldset>
                    }.into_any()
                } else {
                    let all = all.clone();
                    view! {
                        {checks("Catégories", strings(&data, "categories"), categories)}
                        {checks("Armes", strings(&data, "weapons"), weapons)}
                        {checks("Genres", strings(&data, "genders"), genders)}
                        <fieldset>
                            <legend>"Compétitions"<span class="muted small">" (cochées d'après les filtres ; cochez ou décochez pour ajouter ou retirer)"</span></legend>
                            <div class="circuit-contests">
                                {all.into_iter().map(|c| {
                                    let cid = s(&c, "id");
                                    let (c2, cid2, cid3) = (c.clone(), cid.clone(), cid.clone());
                                    let checked = move || !exclude.get().contains(&cid2) && (include.get().contains(&cid2) || matches(&c2));
                                    let c3 = c.clone();
                                    view! {
                                        <label class="inline">
                                            <input type="checkbox" prop:checked=checked on:change=move |ev| {
                                                let on = event_target_checked(&ev);
                                                let by_filter = matches(&c3);
                                                toggle(include, cid3.clone(), on && !by_filter);
                                                toggle(exclude, cid3.clone(), !on && by_filter);
                                            } />
                                            {format!("{} {} · {} · {} {}", s(&c, "name"), s(&c, "date"), s(&c, "category"), s(&c, "weapon"), s(&c, "gender"))}
                                        </label>
                                    }
                                }).collect_view()}
                            </div>
                        </fieldset>
                    }.into_any()
                }}
                <label class="inline">
                    "Ne compter que les "
                    <input type="number" min="0" class="place-input" placeholder="tous" prop:value=move || best.get() on:input=move |ev| best.set(event_target_value(&ev)) />
                    " meilleurs résultats de chaque tireur (vide : tous)"
                </label>
                <div class="actions">
                    <button type="button" class="light" on:click=move |_| on_close.run(())>"Annuler"</button>
                    <button type="submit">"Enregistrer"</button>
                </div>
            </form>
        </div>
    }
}
