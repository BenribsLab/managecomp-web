//! Formule de la compétition et navigation entre les phases
//! (« Tour suivant », « Tour précédent »).

use axum::extract::{Path, State};
use axum::Json;
use bp_core::i18n::tr;
use bp_core::stage::{ErrorLevel, Stage, StageKind};
use serde::{Deserialize, Serialize};

use crate::api::{bad_request, internal, not_found, ApiError, Shared};
use crate::auth::Client;

#[derive(Serialize)]
pub struct StageInfo {
    index: usize,
    /// Type de phase : `checkin`, `pool_allocator`, `pool_supervisor`,
    /// `table`, `barrage`, `splitting`, `classification`, `youth`.
    kind: &'static str,
    class_name: String,
    name: String,
    locked: bool,
    /// La phase peut être retirée de la formule.
    removable: bool,
    /// La phase peut monter / descendre d'un cran dans la formule.
    can_up: bool,
    can_down: bool,
}

#[derive(Serialize)]
pub struct ScheduleView {
    stages: Vec<StageInfo>,
    current: usize,
    can_previous: bool,
    can_next: bool,
    /// Ce qui empêche de passer à la suite (« Tireurs en trop », match sans
    /// vainqueur…).
    error: Option<String>,
}

pub fn kind_code(kind: StageKind) -> &'static str {
    match kind {
        StageKind::Checkin => "checkin",
        StageKind::PoolAllocator => "pool_allocator",
        StageKind::PoolSupervisor => "pool_supervisor",
        StageKind::TableSupervisor => "table",
        StageKind::Barrage => "barrage",
        StageKind::Splitting => "splitting",
        StageKind::GeneralClassification => "classification",
        StageKind::YouthTraining => "youth",
        StageKind::DivisionRound => "divisions",
        StageKind::Lobby => "lobby",
    }
}

pub fn view(c: &mut bp_core::contest::Contest) -> ScheduleView {
    let stages: Vec<StageInfo> = c
        .schedule
        .stages()
        .enumerate()
        .map(|(index, s)| StageInfo {
            index,
            kind: kind_code(s.kind()),
            class_name: tr(s.kind().class_name()),
            name: s.name(),
            locked: s.base().locked,
            removable: false,
            can_up: false,
            can_down: false,
        })
        .collect();
    let mut stages: Vec<StageInfo> = stages;
    for st in stages.iter_mut() {
        st.removable = c.schedule.can_remove(st.index);
        st.can_up = c.schedule.can_move(st.index, true);
        st.can_down = c.schedule.can_move(st.index, false);
    }
    let current = c.schedule.current_index();
    let count = c.schedule.len();
    let (sensitivity, error) = c.with_schedule(|s, ctx| (s.sensitivity(ctx), s.current_error(ctx)));
    ScheduleView {
        stages,
        current,
        can_previous: sensitivity.previous,
        can_next: sensitivity.next && current + 1 < count,
        error: error.map(|e| {
            let prefix = if e.level == ErrorLevel::Warning { "" } else { "" };
            match e.guilty {
                Some(g) => format!("{prefix}{g} : {}", e.message),
                None => format!("{prefix}{}", e.message),
            }
        }),
    }
}

pub async fn get_schedule(State(store): State<Shared>, client: Client, Path(id): Path<String>) -> Result<Json<ScheduleView>, ApiError> {
    let mut store = store.lock().await;
    let v = store.update(&client, &id, view).ok_or_else(not_found)?.map_err(internal)?;
    Ok(Json(v))
}

#[derive(Deserialize, Default)]
pub struct NextOptions {
    /// Ajouter un barrage quand des ex æquo dépassent le quota.
    #[serde(default)]
    add_barrage: bool,
}

pub async fn next(State(store): State<Shared>, client: Client, Path(id): Path<String>, options: Option<Json<NextOptions>>) -> Result<Json<ScheduleView>, ApiError> {
    let add_barrage = options.map(|o| o.add_barrage).unwrap_or(false);
    let mut store = store.lock().await;
    let mut created: Vec<bp_core::contest::Contest> = Vec::new();
    let result = store
        .update(&client, &id, |c| {
            let before = view(c);
            if !before.can_next {
                return Err(bad_request(before.error.unwrap_or_else(|| "la phase en cours n'est pas terminée".into())));
            }
            // Point de scission : une nouvelle compétition par division.
            let index = c.schedule.current_index();
            if c.schedule.current().is_some_and(|s| s.kind() == StageKind::Splitting) {
                for division in c.split_divisions(index) {
                    created.push(c.duplicate_split(index, division, None).map_err(|e| bad_request(e.to_string()))?);
                }
            }
            c.with_schedule(|s, ctx| {
                let exceedance = s.lock_current(ctx);
                if exceedance > 0 && add_barrage {
                    s.add_barrage(ctx);
                }
                s.enter_next(ctx);
            });
            Ok(view(c))
        })
        .ok_or_else(not_found)?
        .map_err(internal)?;
    let result = result?;
    // Divisions : ouvertes (onglets, comme le logiciel) et publiées si la
    // compétition l'est.
    let mut ids = Vec::new();
    for contest in created {
        let new_id = store.insert(&client, contest).map_err(internal)?;
        store.open_contest(&client, &new_id);
        ids.push(new_id);
    }
    if !ids.is_empty() {
        store.publish_split(&client, &id, &ids).map_err(internal)?;
    }
    Ok(Json(result))
}

pub async fn previous(State(store): State<Shared>, client: Client, Path(id): Path<String>) -> Result<Json<ScheduleView>, ApiError> {
    let mut store = store.lock().await;
    let result = store
        .update(&client, &id, |c| {
            if !view(c).can_previous {
                return Err(bad_request("impossible de revenir en arrière"));
            }
            c.with_schedule(|s, ctx| s.go_back(ctx));
            Ok(view(c))
        })
        .ok_or_else(not_found)?
        .map_err(internal)?;
    Ok(Json(result?))
}

#[allow(dead_code)]
fn _unused(_: &dyn Stage) {}

#[derive(Deserialize)]
pub struct NewStage {
    /// `pool_supervisor`, `table`, `splitting`, `youth`.
    kind: String,
    /// Ajoutée après cette phase (à la fin, avant le classement, sinon).
    after: Option<usize>,
}

/// Ajoute une phase à la formule.
pub async fn add_stage(State(store): State<Shared>, client: Client, Path(id): Path<String>, Json(input): Json<NewStage>) -> Result<Json<ScheduleView>, ApiError> {
    let kind = match input.kind.as_str() {
        "pool_supervisor" => StageKind::PoolSupervisor,
        "table" => StageKind::TableSupervisor,
        "splitting" => StageKind::Splitting,
        "youth" => StageKind::YouthTraining,
        "divisions" => StageKind::DivisionRound,
        "lobby" => StageKind::Lobby,
        other => return Err(bad_request(format!("phase inconnue : {other}"))),
    };
    let mut store = store.lock().await;
    let result = store
        .update(&client, &id, |c| {
            let after = input.after.unwrap_or_else(|| c.schedule.len().saturating_sub(2));
            if !c.schedule.can_add_after(after) {
                return Err(bad_request("impossible d'ajouter une phase entre deux phases terminées"));
            }
            c.with_schedule(|s, ctx| {
                s.add_kind(ctx, kind, Some(after));
            });
            Ok(view(c))
        })
        .ok_or_else(not_found)?
        .map_err(internal)?;
    Ok(Json(result?))
}

/// Retire une phase de la formule.
pub async fn remove_stage(State(store): State<Shared>, client: Client, Path((id, index)): Path<(String, usize)>) -> Result<Json<ScheduleView>, ApiError> {
    let mut store = store.lock().await;
    let result = store
        .update(&client, &id, |c| {
            if !c.schedule.can_remove(index) {
                return Err(bad_request("cette phase ne peut pas être retirée"));
            }
            c.with_schedule(|s, ctx| s.remove_user_stage(ctx, index));
            Ok(view(c))
        })
        .ok_or_else(not_found)?
        .map_err(internal)?;
    Ok(Json(result?))
}

#[derive(Deserialize)]
pub struct MoveStage {
    /// Vers le haut (sinon vers le bas).
    up: bool,
}

/// Monte ou descend une phase (avec sa composition de poules) dans la formule.
pub async fn move_stage(State(store): State<Shared>, client: Client, Path((id, index)): Path<(String, usize)>, Json(input): Json<MoveStage>) -> Result<Json<ScheduleView>, ApiError> {
    let mut store = store.lock().await;
    let result = store
        .update(&client, &id, |c| {
            if c.with_schedule(|s, _| s.move_user_stage(index, input.up)).is_none() {
                return Err(bad_request("cette phase ne peut pas être déplacée"));
            }
            Ok(view(c))
        })
        .ok_or_else(not_found)?
        .map_err(internal)?;
    Ok(Json(result?))
}
