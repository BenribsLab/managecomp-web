//! Page d'une compétition : en-tête, propriétés, phases de la formule et
//! navigation (« Tour précédent », « Tour suivant »).

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api::{self, ContestSummary, Meta, Properties, ScheduleView};
use crate::checkin::Checkin;
use crate::icons::Icon;
use crate::properties::{PropertiesFields, PropertiesForm};
use crate::{alert, confirm};

#[component]
pub fn ContestPage(id: String) -> impl IntoView {
    crate::open_tab(&id);
    let id = StoredValue::new(id);
    let summary = RwSignal::new(None::<ContestSummary>);
    let schedule = RwSignal::new(None::<ScheduleView>);
    let selected = RwSignal::new(None::<usize>);
    let error = RwSignal::new(None::<String>);
    let editing = RwSignal::new(None::<(Properties, Meta)>);
    let formula = RwSignal::new(false);
    // Incrémenté quand la compétition change (rafraîchit l'écran de la phase).
    let version = RwSignal::new(0u32);
    // Écran affiché : onglet, type de phase, génération (changée au passage de phase).
    let panel = Memo::new(move |_| {
        let index = selected.get()?;
        let kind = schedule.with(|s| s.as_ref().and_then(|s| s.stages.get(index).map(|st| st.kind.clone())))?;
        Some((index, kind, version.get()))
    });

    let reload = move || {
        let id = id.get_value();
        spawn_local(async move {
            match api::get::<ContestSummary>(&format!("/api/contests/{id}")).await {
                Ok(s) => summary.set(Some(s)),
                Err(e) => error.set(Some(e.message)),
            }
            match api::get::<ScheduleView>(&format!("/api/contests/{id}/schedule")).await {
                Ok(s) => {
                    if selected.get_untracked().is_none_or(|i| i >= s.stages.len()) {
                        selected.set(Some(s.current));
                    }
                    schedule.set(Some(s));
                }
                Err(e) => error.set(Some(e.message)),
            }
        });
    };
    reload();

    let step = move |forward: bool| {
        let id = id.get_value();
        if !forward && !confirm("Revenir à la phase précédente ? Ce qui a été fait dans la phase en cours sera défait.") {
            return;
        }
        spawn_local(async move {
            let path = format!("/api/contests/{id}/{}", if forward { "next" } else { "previous" });
            match api::send::<_, ScheduleView>("POST", &path, &serde_json::json!({})).await {
                Ok(s) => {
                    selected.set(Some(s.current));
                    schedule.set(Some(s));
                    // Nouvelle phase : l'écran est reconstruit.
                    version.update(|v| *v += 1);
                    reload();
                    // Point de scission : les divisions s'ouvrent en onglets.
                    crate::refresh_tabs();
                }
                Err(e) => alert(&e.message),
            }
        });
    };

    let open_properties = move |_| {
        let id = id.get_value();
        spawn_local(async move {
            let p = api::get::<Properties>(&format!("/api/contests/{id}/properties")).await;
            let m = api::get::<Meta>("/api/meta").await;
            match (p, m) {
                (Ok(p), Ok(m)) => editing.set(Some((p, m))),
                (Err(e), _) | (_, Err(e)) => alert(&e.message),
            }
        });
    };

    let download = move |_| {
        let id = id.get_value();
        let name = summary.get_untracked().map(|s| format!("{}.cotcot", s.name)).unwrap_or_else(|| "competition.cotcot".into());
        spawn_local(async move {
            if let Err(e) = api::download(&format!("/api/contests/{id}/cotcot"), &name).await {
                alert(&e.message);
            }
        });
    };

    view! {
        {move || error.get().map(|e| view! { <p class="error">{e}</p> })}
        {move || {
            summary
                .get()
                .map(|s| {
                    let style = format!("--c:{}", s.color);
                    let detail = [s.weapon_label.clone(), s.gender_label.clone(), s.category.clone()]
                        .into_iter()
                        .filter(|x| !x.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · ");
                    view! {
                        <section class="contest-hero" style=style>
                            <div class="hero-main">
                                <a href="#/" class="back"><Icon name="chevron-left" />"Compétitions"</a>
                                <h1>{s.name.clone()}</h1>
                                <div class="hero-chips">
                                    <span class="chip glass"><Icon name="blade" />{detail}</span>
                                    {(!s.date.is_empty()).then(|| view! { <span class="chip glass"><Icon name="calendar" />{s.date.clone()}</span> })}
                                    {(!s.location.is_empty()).then(|| view! { <span class="chip glass"><Icon name="pin" />{s.location.clone()}</span> })}
                                    <span class="chip glass"><Icon name="users" />{format!("{} {}", s.fencers, if s.team_event { "équipes" } else { "tireurs" })}</span>
                                </div>
                            </div>
                            <div class="hero-actions">
                                <button class="glass" on:click=open_properties><Icon name="settings" />"Propriétés"</button>
                                <button class="glass" on:click=move |_| formula.set(true)><Icon name="flow" />"Formule"</button>
                                <button class="glass" on:click=download><Icon name="download" />"Enregistrer"</button>
                                <crate::publish::PublishButton contest=id.get_value() />
                            </div>
                        </section>
                    }
                })
        }}
        <section class="stepper-bar">
            <button
                class="light round"
                title="Tour précédent"
                disabled=move || !schedule.get().map(|s| s.can_previous).unwrap_or(false)
                on:click=move |_| step(false)
            >
                <Icon name="chevron-left" />
            </button>
            <nav class="stepper">
                {move || {
                    schedule
                        .get()
                        .map(|s| {
                            let current = s.current;
                            s.stages
                                .iter()
                                .map(|st| {
                                    let index = st.index;
                                    let label = if st.name.is_empty() { st.class_name.clone() } else { format!("{} — {}", st.class_name, st.name) };
                                    let state = if index < current { "done" } else if index == current { "current" } else { "future" };
                                    view! {
                                        <button
                                            class=format!("step {state}")
                                            class:active=move || selected.get() == Some(index)
                                            on:click=move |_| selected.set(Some(index))
                                        >
                                            <span class="dot">
                                                {if index < current { view! { <Icon name="check" /> }.into_any() } else { view! { <span>{index + 1}</span> }.into_any() }}
                                            </span>
                                            <span class="step-label">{label}</span>
                                            {st.locked.then(|| view! { <Icon name="lock" /> })}
                                        </button>
                                    }
                                })
                                .collect_view()
                        })
                }}
            </nav>
            <button
                class="round"
                title="Tour suivant"
                disabled=move || !schedule.get().map(|s| s.can_next).unwrap_or(false)
                on:click=move |_| step(true)
            >
                <Icon name="chevron-right" />
            </button>
        </section>
        {move || schedule.get().and_then(|s| s.error.clone()).map(|e| view! { <div class="banner warn"><Icon name="alert" />{e}</div> })}
        <section class="stage">
            {move || {
                // Seuls le changement d'onglet ou de phase reconstruisent l'écran :
                // une saisie (score, présence…) ne doit pas faire perdre le focus.
                let (index, kind, _) = panel.get()?;
                let contest = id.get_value();
                let screen = match kind.as_str() {
                    "checkin" => view! { <Checkin id=contest on_change=Callback::new(move |_| reload()) /> }.into_any(),
                    kind => view! { <crate::stages::StageView id=contest index=index kind=kind.to_string() on_change=Callback::new(move |_| reload()) /> }.into_any(),
                };
                Some(view! { <div class="stage-in">{screen}</div> })
            }}
        </section>
        {move || {
            editing
                .get()
                .map(|(p, m)| {
                    let form = PropertiesForm::new(&p);
                    let team_editable = summary.get_untracked().map(|s| s.fencers == 0).unwrap_or(true);
                    let err = RwSignal::new(None::<String>);
                    let save = move |ev: leptos::ev::SubmitEvent| {
                        ev.prevent_default();
                        let id = id.get_value();
                        let props = form.read();
                        spawn_local(async move {
                            match api::send::<_, ContestSummary>("PUT", &format!("/api/contests/{id}/properties"), &props).await {
                                Ok(s) => {
                                    summary.set(Some(s));
                                    editing.set(None);
                                }
                                Err(e) => err.set(Some(e.message)),
                            }
                        });
                    };
                    view! {
                        <div class="overlay">
                            <form class="dialog" on:submit=save>
                                <h2>"Propriétés de la compétition"</h2>
                                <PropertiesFields form=form meta=m team_editable=team_editable />
                                {move || err.get().map(|e| view! { <p class="error">{e}</p> })}
                                <div class="buttons">
                                    <button type="button" class="light" on:click=move |_| editing.set(None)>"Annuler"</button>
                                    <button type="submit">"Enregistrer"</button>
                                </div>
                            </form>
                        </div>
                    }
                })
        }}
        <Show when=move || formula.get()>
            <FormulaDialog id=id.get_value() schedule=schedule on_close=Callback::new(move |_| { formula.set(false); version.update(|v| *v += 1); reload(); }) />
        </Show>
    }
}

/// Phases montrées dans la formule, comme dans le logiciel de bureau : la
/// composition des poules suit toujours son tour de poules et le classement
/// général termine la compétition, ni l'une ni l'autre ne se règlent ici.
fn in_formula(st: &api::StageInfo) -> bool {
    st.kind != "pool_allocator" && st.kind != "classification"
}

/// Formule : phases de la compétition, ajout, retrait et ordre (« Préférences » de l'application).
#[component]
fn FormulaDialog(id: String, schedule: RwSignal<Option<ScheduleView>>, on_close: Callback<()>) -> impl IntoView {
    let id = StoredValue::new(id);
    // Phase dont les réglages sont affichés (clic sur la ligne, comme le logiciel).
    let selected = RwSignal::new(None::<usize>);
    let reload = Callback::new(move |_| {
        let id = id.get_value();
        spawn_local(async move {
            if let Ok(s) = api::get::<ScheduleView>(&format!("/api/contests/{id}/schedule")).await {
                schedule.set(Some(s));
            }
        });
    });
    let kind = RwSignal::new("table".to_string());
    let after = RwSignal::new(String::new());
    let add = move |_| {
        let id = id.get_value();
        let body = serde_json::json!({ "kind": kind.get(), "after": after.get().parse::<usize>().ok() });
        spawn_local(async move {
            match api::send::<_, ScheduleView>("POST", &format!("/api/contests/{id}/schedule/stages"), &body).await {
                Ok(s) => schedule.set(Some(s)),
                Err(e) => alert(&e.message),
            }
        });
    };
    let remove = move |index: usize| {
        if !confirm("Retirer cette phase de la formule ?") {
            return;
        }
        let id = id.get_value();
        spawn_local(async move {
            match api::send::<_, ScheduleView>("DELETE", &format!("/api/contests/{id}/schedule/stages/{index}"), &serde_json::json!({})).await {
                Ok(s) => schedule.set(Some(s)),
                Err(e) => alert(&e.message),
            }
        });
    };
    let shift = move |index: usize, up: bool| {
        let id = id.get_value();
        spawn_local(async move {
            match api::send::<_, ScheduleView>("POST", &format!("/api/contests/{id}/schedule/stages/{index}/move"), &serde_json::json!({ "up": up })).await {
                Ok(s) => schedule.set(Some(s)),
                Err(e) => alert(&e.message),
            }
        });
    };
    view! {
        <div class="overlay">
            <div class="dialog">
                <h2>"Formule de la compétition"</h2>
                <table class="list">
                    <tbody>
                        {move || {
                            schedule
                                .get()
                                .map(|s| {
                                    s.stages
                                        .iter()
                                        .filter(|st| in_formula(st))
                                        .enumerate()
                                        .map(|(n, st)| {
                                            let index = st.index;
                                            let label = if st.name.is_empty() { st.class_name.clone() } else { format!("{} — {}", st.class_name, st.name) };
                                            view! {
                                                <tr class="clickable" class:selected=move || selected.get() == Some(index) title="Réglages de la phase"
                                                    on:click=move |_| selected.set(if selected.get_untracked() == Some(index) { None } else { Some(index) })>
                                                    <td class="number">{n + 1}</td>
                                                    <td>{label}{if st.locked { " 🔒" } else { "" }}{if index == s.current { " (en cours)" } else { "" }}</td>
                                                    <td class="actions">
                                                        {st.can_up.then(|| view! { <button class="small" title="Monter" on:click=move |_| shift(index, true)>"▲"</button> })}
                                                        {st.can_down.then(|| view! { <button class="small" title="Descendre" on:click=move |_| shift(index, false)>"▼"</button> })}
                                                        {st.removable.then(|| view! { <button class="danger small" on:click=move |_| remove(index)>"Retirer"</button> })}
                                                    </td>
                                                </tr>
                                            }
                                        })
                                        .collect_view()
                                })
                        }}
                    </tbody>
                </table>
                // Réglages de la phase choisie (avant même qu'elle commence).
                {move || selected.get().and_then(|i| schedule.get().and_then(|s| s.stages.iter().find(|st| st.index == i).cloned())).map(|st| {
                    let label = if st.name.is_empty() { st.class_name.clone() } else { format!("{} — {}", st.class_name, st.name) };
                    view! {
                        <div class="stage-settings">
                            <h3>{format!("Réglages : {label}")}</h3>
                            <crate::stages::StageSettings id=id.get_value() index=st.index kind=st.kind.clone() on_change=reload />
                        </div>
                    }
                })}
                <p class="muted small">"Cliquez sur une phase pour la régler, même si elle n'a pas encore commencé."</p>
                <div class="settings-row">
                    "Ajouter "
                    <select on:change=move |ev| kind.set(event_target_value(&ev))>
                        <option value="table" selected=true>"Tableau"</option>
                        <option value="pool_supervisor">"Tour de poules"</option>
                        <option value="splitting">"Point de scission"</option>
                        <option value="youth">"Entraînement des jeunes"</option>
                        <option value="divisions">"Poules de division"</option>
                        <option value="lobby">"Lobby (série)"</option>
                    </select>
                    " après "
                    <select on:change=move |ev| after.set(event_target_value(&ev))>
                        <option value="">"(à la fin)"</option>
                        {move || {
                            schedule
                                .get()
                                .map(|s| {
                                    s.stages
                                        .iter()
                                        .filter(|st| in_formula(st))
                                        .enumerate()
                                        .map(|(n, st)| view! { <option value=st.index.to_string()>{format!("{}. {}", n + 1, st.class_name)}</option> })
                                        .collect_view()
                                })
                        }}
                    </select>
                    <button on:click=add>"Ajouter"</button>
                </div>
                <div class="buttons">
                    <button on:click=move |_| on_close.run(())>"Fermer"</button>
                </div>
            </div>
        </div>
    }
}
