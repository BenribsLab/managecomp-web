//! Accueil : les compétitions du client, création, import d'un fichier.

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api::{self, ContestSummary, Meta, NewContest};
use crate::icons::Icon;
use crate::properties::{new_properties, PropertiesFields, PropertiesForm};
use crate::{alert, confirm, navigate, Route};

#[component]
pub fn Home() -> impl IntoView {
    let version = RwSignal::new(0u32);
    let contests = LocalResource::new(move || {
        version.get();
        api::get::<Vec<ContestSummary>>("/api/contests")
    });
    let meta = LocalResource::new(|| api::get::<Meta>("/api/meta"));
    let reload = move || version.update(|v| *v += 1);
    let creating = RwSignal::new(false);
    let choosing = RwSignal::new(false);

    let search = RwSignal::new(String::new());
    let show_hidden = RwSignal::new(false);
    let show_pages = RwSignal::new(false);

    view! {
        <header class="page-head">
            <div>
                <h1>"Compétitions"</h1>
                <p class="muted">
                    {move || match contests.get() {
                        Some(Ok(list)) => match list.iter().filter(|c| !c.hidden).count() {
                            0 => "Aucune compétition pour l'instant".to_string(),
                            1 => "1 compétition".to_string(),
                            n => format!("{n} compétitions"),
                        },
                        _ => String::new(),
                    }}
                    {move || {
                        let retired = match contests.get() {
                            Some(Ok(list)) => list.iter().filter(|c| c.hidden).count(),
                            _ => 0,
                        };
                        (retired > 0 && !show_hidden.get()).then(|| view! {
                            " · "
                            <a href="#" on:click=move |ev| { ev.prevent_default(); show_hidden.set(true); }>
                                {format!("{retired} retirée{} de l'accueil", if retired > 1 { "s" } else { "" })}
                            </a>
                        })
                    }}
                </p>
            </div>
            <span class="spacer"></span>
            <label class="input-icon search">
                <Icon name="search" />
                <input type="search" placeholder="Rechercher" prop:value=move || search.get() on:input=move |ev| search.set(event_target_value(&ev)) />
            </label>
            <ImportButton on_done=reload />
            <button on:click=move |_| choosing.set(true)><Icon name="plus" />"Nouvelle compétition"</button>
        </header>
        {move || match contests.get() {
            None => view! { <div class="contest-grid">{(0..3).map(|_| view! { <div class="contest-card skeleton"></div> }).collect_view()}</div> }.into_any(),
            Some(Err(e)) => view! { <p class="error">{e.message}</p> }.into_any(),
            Some(Ok(list)) if list.is_empty() => view! {
                <div class="empty">
                    <Icon name="trophy" />
                    <h2>"Aucune compétition"</h2>
                    <p class="muted">"Créez une compétition ou ouvrez un fichier .cotcot du logiciel de bureau."</p>
                    <button on:click=move |_| choosing.set(true)><Icon name="plus" />"Nouvelle compétition"</button>
                </div>
            }.into_any(),
            Some(Ok(list)) => {
                let needle = search.get().to_lowercase();
                let retired = list.iter().filter(|c| c.hidden).count();
                let hidden = show_hidden.get();
                let list: Vec<_> = list
                    .into_iter()
                    .filter(|c| c.hidden == hidden)
                    .filter(|c| needle.is_empty() || format!("{} {} {} {}", c.name, c.label, c.location, c.category).to_lowercase().contains(&needle))
                    .collect();
                view! {
                    {(hidden && list.is_empty()).then(|| view! { <p class="muted">"Aucune compétition retirée."</p> })}
                    <ContestCards list=list on_change=reload />
                    <div class="row hidden-toggle">
                        {(retired > 0 || hidden).then(|| view! {
                            <button class="ghost small" on:click=move |_| show_hidden.update(|h| *h = !*h)>
                                {if hidden { "‹ Retour aux compétitions".to_string() } else { format!("Compétitions retirées de l'accueil ({retired})") }}
                            </button>
                        })}
                        <button class="ghost small" on:click=move |_| show_pages.update(|v| *v = !*v)>"Pages publiques du club"</button>
                    </div>
                    <Show when=move || show_pages.get()>
                        <PublicPages />
                    </Show>
                }.into_any()
            }
        }}
        <Show when=move || choosing.get()>
            <NewContestChooser
                on_empty=move || { choosing.set(false); creating.set(true); }
                on_close=move || choosing.set(false)
                on_done=reload
            />
        </Show>
        <Show when=move || creating.get()>
            {move || match meta.get() {
                Some(Ok(m)) => view! { <NewContestDialog meta=m on_close=move || creating.set(false) /> }.into_any(),
                Some(Err(e)) => view! { <p class="error">{e.message}</p> }.into_any(),
                None => view! { <p class="muted">"Chargement…"</p> }.into_any(),
            }}
        </Show>
    }
}

/// Pages publiques du club (compete.<domaine>), y compris celles dont la
/// compétition a été supprimée : chacune peut être retirée.
#[component]
fn PublicPages() -> impl IntoView {
    let version = RwSignal::new(0u32);
    let data = LocalResource::new(move || {
        version.get();
        api::get::<serde_json::Value>("/api/publications")
    });
    let remove = move |path: String, label: String| {
        if !confirm(&format!("Retirer « {label} » du site public ?")) {
            return;
        }
        spawn_local(async move {
            if let Err(e) = api::delete(&path).await {
                alert(&e.message);
            }
            version.update(|v| *v += 1);
        });
    };
    let text = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    view! {
        <section class="public-pages">
            <h2>"Pages publiques du club"</h2>
            {move || match data.get() {
                None => view! { <p class="muted">"Chargement…"</p> }.into_any(),
                Some(Err(e)) => view! { <p class="error">{e.message}</p> }.into_any(),
                Some(Ok(d)) => {
                    let events: Vec<serde_json::Value> = d.get("events").and_then(|x| x.as_array()).cloned().unwrap_or_default();
                    let pubs: Vec<serde_json::Value> = d.get("publications").and_then(|x| x.as_array()).cloned().unwrap_or_default();
                    if events.is_empty() && pubs.is_empty() {
                        return view! { <p class="muted">"Aucune page publique."</p> }.into_any();
                    }
                    view! {
                        <table class="public-list">
                            <tbody>
                                {events.into_iter().map(|e| {
                                    let (id, name) = (text(&e, "id"), text(&e, "name"));
                                    let count = e.get("count").and_then(|x| x.as_u64()).unwrap_or(0);
                                    let label = name.clone();
                                    view! {
                                        <tr>
                                            <td><a href=text(&e, "url") target="_blank">{name}</a><br /><span class="muted small">{format!("Événement · {} · {count} épreuve(s)", text(&e, "date"))}</span></td>
                                            <td class="right"><button class="ghost small danger" on:click=move |_| remove(format!("/api/publication/event/{id}"), label.clone())>"Retirer"</button></td>
                                        </tr>
                                    }
                                }).collect_view()}
                                {pubs.into_iter().map(|p| {
                                    let (id, title) = (text(&p, "id"), text(&p, "title"));
                                    let orphan = !p.get("linked").and_then(|x| x.as_bool()).unwrap_or(false);
                                    let label = title.clone();
                                    view! {
                                        <tr>
                                            <td>
                                                <a href=text(&p, "url") target="_blank">{title}</a><br />
                                                <span class="muted small">{format!("{} · {}", text(&p, "date"), text(&p, "subtitle"))}</span>
                                                {orphan.then(|| view! { <span class="muted small">" · compétition supprimée ou du logiciel de bureau"</span> })}
                                            </td>
                                            <td class="right"><button class="ghost small danger" on:click=move |_| remove(format!("/api/publication/{id}"), label.clone())>"Retirer"</button></td>
                                        </tr>
                                    }
                                }).collect_view()}
                            </tbody>
                        </table>
                    }.into_any()
                }
            }}
        </section>
    }
}

#[component]
fn ContestCards(list: Vec<ContestSummary>, on_change: impl Fn() + Copy + 'static) -> impl IntoView {
    view! {
        <div class="contest-grid">
            {list
                .into_iter()
                .enumerate()
                .map(|(i, c)| {
                    let open_id = c.id.clone();
                    let del_id = c.id.clone();
                    let hide_id = c.id.clone();
                    let name = c.name.clone();
                    let hide_name = c.name.clone();
                    let hidden = c.hidden;
                    let delete = move |ev: leptos::ev::MouseEvent| {
                        ev.stop_propagation();
                        if confirm(&format!("Supprimer définitivement « {name} » ? Ses données seront effacées.")) {
                            let id = del_id.clone();
                            spawn_local(async move {
                                if let Err(e) = api::delete(&format!("/api/contests/{id}")).await {
                                    alert(&e.message);
                                }
                                crate::refresh_tabs();
                                on_change();
                            });
                        }
                    };
                    // Retrait de l'accueil (la compétition est conservée) ou remise.
                    let toggle_hidden = move |ev: leptos::ev::MouseEvent| {
                        ev.stop_propagation();
                        if !hidden && !confirm(&format!("Retirer « {hide_name} » de l'accueil ? La compétition est conservée et reste consultable dans « Compétitions retirées ».")) {
                            return;
                        }
                        let id = hide_id.clone();
                        spawn_local(async move {
                            if let Err(e) = api::send_empty("PUT", &format!("/api/contests/{id}/hidden"), &serde_json::json!({ "hidden": !hidden })).await {
                                alert(&e.message);
                            }
                            on_change();
                        });
                    };
                    let style = format!("--c:{};--i:{}", c.color, i.min(12));
                    let detail = [c.weapon_label.clone(), c.gender_label.clone(), c.category.clone()]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · ");
                    view! {
                        <article class="contest-card" style=style tabindex="0" on:click=move |_| navigate(&Route::Contest(open_id.clone()))>
                            <div class="cc-band"></div>
                            <div class="cc-body">
                                <div class="cc-top">
                                    <span class="cc-weapon">{detail}</span>
                                    <span class="cc-tools">
                                        {if hidden {
                                            view! {
                                                <button class="ghost small" title="Remettre sur l'accueil" on:click=toggle_hidden>"Remettre"</button>
                                                <button class="ghost icon-only danger" title="Supprimer définitivement" on:click=delete><Icon name="trash" /></button>
                                            }.into_any()
                                        } else {
                                            view! { <button class="ghost icon-only" title="Retirer de l'accueil (la compétition est conservée)" on:click=toggle_hidden><Icon name="x" /></button> }.into_any()
                                        }}
                                    </span>
                                </div>
                                <h3>{c.name.clone()}</h3>
                                {(!c.label.is_empty()).then(|| view! { <p class="muted cc-label">{c.label.clone()}</p> })}
                                <div class="cc-meta">
                                    {(!c.date.is_empty()).then(|| view! { <span><Icon name="calendar" />{c.date.clone()}</span> })}
                                    {(!c.location.is_empty()).then(|| view! { <span><Icon name="pin" />{c.location.clone()}</span> })}
                                </div>
                                <div class="cc-foot">
                                    <span class="chip"><Icon name="users" />{c.fencers}</span>
                                    {(!c.stage.is_empty()).then(|| view! { <span class="chip accent">{c.stage.clone()}</span> })}
                                    <span class="spacer"></span>
                                    <span class="cc-open"><Icon name="chevron-right" /></span>
                                </div>
                            </div>
                        </article>
                    }
                })
                .collect_view()}
        </div>
    }
}

#[component]
fn NewContestDialog(meta: Meta, on_close: impl Fn() + Copy + 'static) -> impl IntoView {
    let form = PropertiesForm::new(&new_properties());
    let youth = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let input = NewContest { properties: form.read(), youth_training: youth.get() };
        spawn_local(async move {
            match api::send::<_, ContestSummary>("POST", "/api/contests", &input).await {
                Ok(c) => {
                    on_close();
                    navigate(&Route::Contest(c.id));
                }
                Err(e) => error.set(Some(e.message)),
            }
        });
    };
    view! {
        <div class="overlay">
            <form class="dialog" on:submit=submit>
                <h2>"Nouvelle compétition"</h2>
                <PropertiesFields form=form meta=meta />
                <label class="check">
                    <input type="checkbox" prop:checked=move || youth.get() on:change=move |ev| youth.set(event_target_checked(&ev)) />
                    "Entraînement des jeunes (groupes, poules et tableau par groupe)"
                </label>
                {move || error.get().map(|e| view! { <p class="error">{e}</p> })}
                <div class="buttons">
                    <button type="button" class="light" on:click=move |_| on_close()>"Annuler"</button>
                    <button type="submit">"Créer"</button>
                </div>
            </form>
        </div>
    }
}

/// Lit le fichier choisi dans un champ `<input type="file">`.
pub async fn read_file(input: &web_sys::HtmlInputElement) -> Option<(String, Vec<u8>)> {
    let file = input.files()?.get(0)?;
    let name = file.name();
    let buffer = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await.ok()?;
    Some((name, js_sys::Uint8Array::new(&buffer).to_vec()))
}

/// Ouvre le fichier choisi (`.cotcot`, `.xml`, `.zip` de la FFE, `.frd`
/// d'AskFred) : une seule compétition créée est ouverte, plusieurs (AskFred)
/// restent dans la liste.
async fn open_file(el: web_sys::HtmlInputElement, on_done: impl Fn() + 'static) {
    if let Some((name, bytes)) = read_file(&el).await {
        let path = format!("/api/contests/import?name={}", js_sys::encode_uri_component(&name));
        match api::upload::<Vec<ContestSummary>>(&path, bytes).await {
            Ok(list) if list.len() == 1 => {
                if name.to_lowercase().ends_with(".frd") {
                    alert("Import AskFred terminé : vérifiez les données importées.");
                }
                navigate(&Route::Contest(list[0].id.clone()));
            }
            Ok(list) => alert(&format!("Import AskFred terminé : {} compétitions créées. Vérifiez les données importées.", list.len())),
            Err(e) => alert(&e.message),
        }
        on_done();
    }
    el.set_value("");
}

/// Import d'un fichier de compétition.
#[component]
fn ImportButton(on_done: impl Fn() + Copy + 'static) -> impl IntoView {
    let input = NodeRef::<leptos::html::Input>::new();
    let changed = move |_| {
        let Some(el) = input.get() else { return };
        spawn_local(open_file(el, on_done));
    };
    view! {
        <label class="button light">
            <Icon name="upload" />
            "Ouvrir un fichier"
            <input type="file" accept=".cotcot,.xml,.XML,.zip,.frd" class="hidden" node_ref=input on:change=changed />
        </label>
    }
}

/// Nouvelle compétition : méthode de création, comme sur le bureau.
#[component]
fn NewContestChooser(
    on_empty: impl Fn() + Copy + 'static,
    on_close: impl Fn() + Copy + 'static,
    on_done: impl Fn() + Copy + 'static,
) -> impl IntoView {
    let choice = move |accept: &'static str, image: AnyView, label: &'static str| {
        let input = NodeRef::<leptos::html::Input>::new();
        let changed = move |_| {
            let Some(el) = input.get() else { return };
            on_close();
            spawn_local(open_file(el, on_done));
        };
        view! {
            <label class="new-choice">
                <span class="new-choice-icon">{image}</span>
                <span>{label}</span>
                <input type="file" accept=accept class="hidden" node_ref=input on:change=changed />
            </label>
        }
    };
    view! {
        <div class="overlay" on:click=move |_| on_close()>
            <div class="dialog new-chooser" on:click=|ev| ev.stop_propagation()>
                <h2>"Nouvelle compétition"</h2>
                <p class="muted">"Choisissez une méthode de création :"</p>
                <div class="new-choices">
                    {choice(".zip", view! { <img src="/zip.png" alt="" /> }.into_any(), "À partir d'un fichier zip de la FFE (extranet)")}
                    {choice(".xml,.XML", view! { <Icon name="folder" /> }.into_any(), "À partir d'un fichier XML")}
                    {choice(".frd", view! { <img src="/askfred.png" alt="" /> }.into_any(), "À partir d'AskFred (.frd)")}
                    <button type="button" class="new-choice" on:click=move |_| on_empty()>
                        <span class="new-choice-icon"><Icon name="file-plus" /></span>
                        <span>"Compétition vide"</span>
                    </button>
                </div>
                <div class="buttons">
                    <button type="button" class="light" on:click=move |_| on_close()>"Annuler"</button>
                </div>
            </div>
        </div>
    }
}
