//! Écrans des phases : une vue par type de phase
//! (`GET /api/contests/{id}/stages/{index}`) et ses actions
//! (`POST /api/contests/{id}/stages/{index}` avec `{ "action": …, … }`).

use axum::extract::{Path, State};
use axum::Json;
use bp_core::contest::Contest;
use bp_core::i18n::tr;
use bp_core::player::{Owner, PlayerRef, PlayerStore};
use bp_core::rounds::barrage::Barrage;
use bp_core::rounds::classification::GeneralClassificationStage;
use bp_core::rounds::pool::round::{extra_swapping_criteria, regular_swapping_criteria, PoolRound};
use bp_core::rounds::pool::{PoolAllocatorStage, PoolSupervisorStage};
use bp_core::rounds::splitting::Splitting;
use bp_core::rounds::table::layout::LayoutMetrics;
use bp_core::rounds::table::supervisor::{FencedPlaces, TableSupervisor};
use bp_core::rounds::youth::{self, GroupMode, PointsMode, PoolMode, YouthSettings, YouthTraining};
use bp_core::stage::{RankEntry, Stage, StageContext};
use serde::Serialize;
use serde_json::{json, Value};

use crate::api::{bad_request, internal, not_found, ApiError, Shared};
use crate::auth::Client;
use crate::schedule::kind_code;
use crate::view::{self, Column};

// ------------------------------------------------------------------ listes

/// Liste générique (classements, barrage, scission…).
#[derive(Serialize)]
pub struct ListView {
    pub(crate) columns: Vec<Column>,
    pub(crate) rows: Vec<ListRow>,
}

#[derive(Serialize)]
pub struct ListRow {
    pub(crate) reference: PlayerRef,
    pub(crate) cells: Vec<String>,
    /// Éliminé, absent… (ligne grisée).
    dimmed: bool,
    /// Case à cocher de la ligne (promu, exporté…), si la liste en a une.
    checked: Option<bool>,
}

fn list(players: &PlayerStore, codes: &[String], refs: &[(PlayerRef, bool)], owner: Owner) -> ListView {
    ListView {
        columns: view::columns(codes),
        rows: refs
            .iter()
            .filter_map(|(r, dimmed)| {
                let p = players.get(*r)?;
                Some(ListRow { reference: *r, cells: codes.iter().map(|c| view::cell(p, c, owner)).collect(), dimmed: *dimmed, checked: None })
            })
            .collect(),
    }
}

/// Classement d'une phase : rang et statut écrits sur une copie des joueurs.
fn ranked_list(players: &PlayerStore, codes: &[String], entries: &[RankEntry], owner: Owner) -> ListView {
    let mut copy = players.clone();
    for e in entries {
        if let Some(p) = copy.get_mut(e.player) {
            p.set_uint(&bp_core::player::AttrId::local("rank", owner), e.rank);
            p.set_str(&bp_core::player::AttrId::local("status", owner), &e.status.to_string());
        }
    }
    let refs: Vec<(PlayerRef, bool)> = entries.iter().map(|e| (e.player, e.status != 'Q')).collect();
    list(&copy, codes, &refs, owner)
}

fn identity(players: &PlayerStore, r: PlayerRef) -> String {
    players
        .get(r)
        .map(|p| {
            let first = p.str_attr("first_name").unwrap_or_default();
            if first.is_empty() {
                p.name()
            } else {
                format!("{} {first}", p.name())
            }
        })
        .unwrap_or_default()
}

fn club(players: &PlayerStore, r: PlayerRef) -> String {
    players.get(r).and_then(|p| p.str_attr("club")).unwrap_or_default().to_string()
}

// ----------------------------------------------------------------- lecture

pub async fn get_stage(State(store): State<Shared>, client: Client, Path((id, index)): Path<(String, usize)>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let c = store.get_mut(&client, &id).ok_or_else(not_found)?;
    let mut v = stage_view(c, index)?;
    // Club : adresse des QR codes des feuilles (ManageComp Arbitre).
    v["club"] = json!(client.id);
    Ok(Json(v))
}

fn stage_view(c: &mut Contest, index: usize) -> Result<Value, ApiError> {
    let kind = c.schedule.stage(index).map(|s| s.kind()).ok_or_else(not_found)?;
    let code = kind_code(kind);
    let mut body = match code {
        "youth" => youth_view(c, index),
        "divisions" => divisions_view(c, index),
        "lobby" => lobby_view(c, index),
        "pool_allocator" => allocator_view(c, index),
        "pool_supervisor" => pools_view(c, index),
        "table" => table_view(c, index),
        "barrage" => barrage_view(c, index),
        "splitting" => splitting_view(c, index),
        "classification" => classification_view(c, index),
        _ => Some(json!({})),
    }
    .ok_or_else(not_found)?;
    if let Some(stage) = c.schedule.stage(index) {
        let mut timing = stage.bout_timing();
        timing.relay = c.relay_format(index);
        body["timing"] = json!(timing);
        // Rencontres par équipes : ordre des relais et score de la rencontre.
        if c.settings.team_event {
            body["relay"] = json!({
                "bouts": timing.relay.bouts(),
                "numbers": timing.relay.numbers(),
                "total": timing.relay.total(),
            });
        }
    }
    body["team"] = json!(c.settings.team_event);
    body["stuffing"] = json!(c.settings.score_stuffing_allowed);
    let locked = c.schedule.stage(index).map(|s| s.base().locked).unwrap_or(false);
    let name = c.schedule.stage(index).map(|s| s.name()).unwrap_or_default();
    let max_score = c.schedule.stage(index).and_then(|s| s.base().max_score);
    Ok(json!({ "kind": code, "index": index, "name": name, "locked": locked, "max_score": max_score, "data": body }))
}

// ---------------------------------------------------------- entraînement

fn mode_code(m: GroupMode) -> &'static str {
    match m {
        GroupMode::Division => "division",
        GroupMode::Ranking => "ranking",
        GroupMode::Random => "random",
    }
}

fn settings_json(s: &YouthSettings) -> Value {
    let (points_mode, points_value, points_list) = match &s.points {
        PointsMode::Same(n) => ("same", *n, Vec::new()),
        PointsMode::PerVictory(n) => ("victory", *n, Vec::new()),
        PointsMode::ByPlace(l) => ("place", 0, l.clone()),
    };
    json!({
        "group_size": s.group_size,
        "pools_per_group": s.pools_per_group,
        "group_mode": mode_code(s.group_mode),
        "pool_mode": if s.pool_mode == PoolMode::Level { "level" } else { "snake" },
        "swapping": s.swapping,
        "fenced_places": match s.fenced_places { FencedPlaces::AllPlaces => "all", FencedPlaces::ThirdPlaces => "third", _ => "none" },
        "points_mode": points_mode,
        "points_value": points_value,
        "points_list": points_list,
    })
}

fn youth_view(c: &mut Contest, index: usize) -> Option<Value> {
    let y = c.schedule.stage(index)?.as_any().downcast_ref::<YouthTraining>()?;
    let groups: Vec<Value> = y
        .groups(&c.players)
        .iter()
        .enumerate()
        .map(|(g, group)| {
            let choices: Vec<Value> = youth::pool_choices(group.layout.size)
                .iter()
                .map(|ch| json!({ "nb_pools": ch.nb_pools, "smallest": ch.smallest, "largest": ch.largest }))
                .collect();
            json!({
                "number": g + 1,
                "size": group.layout.size,
                "nb_pools": group.layout.nb_pools,
                "best_pools": youth::best_pools(group.layout.size, &y.settings),
                "pool_choices": choices,
                "bouts": youth::bouts_per_fencer(&group.layout, y.settings.fenced_places),
                "fencers": group.players.iter().map(|r| json!({
                    "reference": r,
                    "name": identity(&c.players, *r),
                    "club": club(&c.players, *r),
                    "ranking": c.players.get(*r).and_then(|p| p.attr("ranking")).map(|v| v.as_uint()),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let swapping: Vec<Value> = regular_swapping_criteria()
        .into_iter()
        .map(|code| json!({ "code": code, "label": tr(bp_core::attribute::desc(code).user_name) }))
        .collect();
    Some(json!({
        "settings": settings_json(&y.settings),
        "customized": y.is_customized(),
        "count": y.base().attendees.len(),
        "groups": groups,
        "swapping_choices": swapping,
        "results": youth_results(c),
    }))
}

fn youth_results(c: &Contest) -> Option<Value> {
    let results = youth::results(&c.schedule, &c.players)?;
    Some(Value::Array(
        results
            .iter()
            .map(|r| json!({
                "reference": r.player,
                "group": r.group,
                "place": r.place,
                "victories": r.victories,
                "points": r.points,
                "name": identity(&c.players, r.player),
                "club": club(&c.players, r.player),
            }))
            .collect(),
    ))
}

fn parse_settings(v: &Value, current: &YouthSettings) -> YouthSettings {
    let u = |k: &str, d: u32| v.get(k).and_then(Value::as_u64).map(|n| n as u32).filter(|n| *n > 0).unwrap_or(d);
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("");
    YouthSettings {
        group_size: u("group_size", current.group_size),
        pools_per_group: u("pools_per_group", current.pools_per_group),
        group_mode: match s("group_mode") {
            "ranking" => GroupMode::Ranking,
            "random" => GroupMode::Random,
            "division" => GroupMode::Division,
            _ => current.group_mode,
        },
        pool_mode: match s("pool_mode") {
            "level" => PoolMode::Level,
            "snake" => PoolMode::Snake,
            _ => current.pool_mode,
        },
        swapping: v
            .get("swapping")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_else(|| current.swapping.clone()),
        fenced_places: match s("fenced_places") {
            "all" => FencedPlaces::AllPlaces,
            "third" => FencedPlaces::ThirdPlaces,
            "none" => FencedPlaces::None,
            _ => current.fenced_places,
        },
        points: match s("points_mode") {
            "same" => PointsMode::Same(v.get("points_value").and_then(Value::as_u64).unwrap_or(10) as u32),
            "victory" => PointsMode::PerVictory(v.get("points_value").and_then(Value::as_u64).unwrap_or(1) as u32),
            "place" => {
                let list: Vec<u32> = v
                    .get("points_list")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|x| x.as_u64().map(|n| n as u32)).collect())
                    .unwrap_or_default();
                PointsMode::ByPlace(if list.is_empty() { vec![10, 8, 6, 5, 4, 3, 2, 1] } else { list })
            }
            _ => current.points.clone(),
        },
    }
}

fn youth_action(c: &mut Contest, index: usize, a: &Value) -> Result<(), ApiError> {
    let players = c.players.clone();
    let y = c
        .schedule
        .stage_mut(index)
        .and_then(|s| s.as_any_mut().downcast_mut::<YouthTraining>())
        .ok_or_else(not_found)?;
    if y.base().locked {
        return Err(bad_request("les groupes sont faits : « Tour précédent » pour les modifier"));
    }
    let n = |k: &str| a.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
    match a.get("action").and_then(Value::as_str).unwrap_or("") {
        "settings" => {
            let settings = parse_settings(a.get("settings").unwrap_or(&Value::Null), &y.settings);
            let regroup = YouthSettings { points: settings.points.clone(), ..y.settings.clone() } != settings;
            y.settings = settings;
            if regroup {
                y.reset_layout();
            }
        }
        "group_count" => y.set_group_count(n("count")),
        "group_size" => y.set_group_size(n("group") as usize, n("size")),
        "group_pools" => y.set_group_pools(n("group") as usize, n("nb_pools")),
        "move" => y.move_player(&players, n("player"), n("to") as usize),
        "reset" => y.reset_layout(),
        other => return Err(bad_request(format!("action inconnue : {other}"))),
    }
    Ok(())
}

// --------------------------------------------------- composition des poules

fn allocator_view(c: &mut Contest, index: usize) -> Option<Value> {
    let a = c.schedule.stage(index)?.as_any().downcast_ref::<PoolAllocatorStage>()?;
    let locked = a.base().locked;
    let round = a.round();
    let owner = round.allocator_owner();
    let codes = round.allocator_filter.clone();
    let pools: Vec<Value> = round
        .pools()
        .iter()
        .enumerate()
        .map(|(i, pool)| {
            json!({
                "index": i,
                "name": pool.name(),
                "balanced": round.pool_is_balanced(i),
                "piste": pool.piste_label(),
                "time": pool.start_time_image(),
                "fencers": pool.fencers().iter().map(|f| {
                    let p = c.players.get(*f);
                    let mark = if round.swapper().is_an_error(*f) { "error" } else if round.is_moved(&c.players, *f) { "moved" } else { "" };
                    json!({
                        "reference": f,
                        "cells": codes.iter().map(|code| p.map(|p| view::cell(p, code, owner)).unwrap_or_default()).collect::<Vec<_>>(),
                        "mark": mark,
                    })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    let configs: Vec<Value> = round
        .configs()
        .iter()
        .enumerate()
        .map(|(i, cfg)| json!({ "index": i, "nb_pools": cfg.nb_pool, "size": cfg.size_label() }))
        .collect();
    let extra = extra_swapping_criteria();
    Some(json!({
        "columns": view::columns(&codes),
        "configs": configs,
        "selected": round.selected_config(),
        "best": round.best_config(),
        "youth": round.is_youth(),
        "divisions": round.group_prefix.is_some(),
        // Retardataires et absents : présents (pour « absent ») et absents (pour « retardataire »).
        "pillows": !locked && round.pillows_allowed(),
        "presence": c.schedule.checkin().map(|ch| ch.players().iter().filter_map(|r| {
            let p = c.players.get(*r)?;
            (p.class == bp_core::player::PlayerClass::Fencer).then(|| json!({
                "reference": r,
                "name": format!("{} {}", p.str_attr("name").unwrap_or_default(), p.str_attr("first_name").unwrap_or_default()).trim().to_string(),
                "attending": p.attending(),
            }))
        }).collect::<Vec<_>>()).unwrap_or_default(),
        "pools": pools,
        "swapping": round.swapping_criteria(),
        "swapping_errors": round.swapper().moved(),
        "regular_choices": regular_swapping_criteria().into_iter().map(|code| json!({ "code": code, "label": tr(bp_core::attribute::desc(code).user_name) })).collect::<Vec<_>>(),
        "extra_choices": extra.into_iter().map(|code| json!({ "code": code, "label": tr(bp_core::attribute::desc(code).user_name) })).collect::<Vec<_>>(),
        "locked": locked,
    }))
}

fn with_round<R>(c: &mut Contest, index: usize, f: impl FnOnce(&mut PoolRound, &mut PlayerStore) -> R) -> Result<R, ApiError> {
    let (schedule, players) = (&mut c.schedule, &mut c.players);
    let stage = schedule.stage_mut(index).ok_or_else(not_found)?;
    let round = if let Some(a) = stage.as_any().downcast_ref::<PoolAllocatorStage>() {
        a.shared_round()
    } else if let Some(s) = stage.as_any().downcast_ref::<PoolSupervisorStage>() {
        s.shared_round()
    } else {
        return Err(not_found());
    };
    let mut round = round.lock().map_err(|_| internal("tour de poules indisponible"))?;
    Ok(f(&mut round, players))
}

fn allocator_action(c: &mut Contest, index: usize, a: &Value) -> Result<(), ApiError> {
    if c.schedule.stage(index).map(|s| s.base().locked).unwrap_or(true) {
        return Err(bad_request("la composition est terminée : « Tour précédent » pour la modifier"));
    }
    let n = |k: &str| a.get(k).and_then(Value::as_u64).unwrap_or(0);
    match a.get("action").and_then(Value::as_str).unwrap_or("") {
        "config" => {
            with_round(c, index, |r, p| r.select_config(p, n("index") as usize))?;
        }
        "move" => {
            let result = with_round(c, index, |r, p| {
                let res = r.move_fencer(p, n("player") as PlayerRef, n("pool") as usize);
                r.check_distribution(p);
                res
            })?;
            result.map_err(|e| bad_request(format!("{e:?}")))?;
        }
        "swapping" => {
            let regular: Vec<String> = a
                .get("regular")
                .and_then(Value::as_array)
                .map(|v| v.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            // `extras` : critères supplémentaires dans l'ordre (`extra` : un seul, ancien format).
            let mut extras: Vec<String> = a
                .get("extras")
                .and_then(Value::as_array)
                .map(|v| v.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            extras.extend(a.get("extra").and_then(Value::as_str).map(str::to_string));
            with_round(c, index, |r, p| {
                let regular: Vec<&str> = regular.iter().map(String::as_str).collect();
                let extras: Vec<&str> = extras.iter().map(String::as_str).collect();
                r.set_swapping_criteria(p, &regular, &extras);
            })?;
        }
        // Retardataire (`attending` vrai) ou absent après la composition.
        "presence" => {
            use bp_core::rounds::pool::round::PillowKind;
            let player = n("player") as PlayerRef;
            let attending = a.get("attending").and_then(Value::as_bool).unwrap_or(true);
            let kind = if attending { PillowKind::Latecomer } else { PillowKind::Absent };
            let accepted = with_round(c, index, |r, p| {
                if !r.pillows_allowed() {
                    return Err(bad_request("retardataires et absents : seulement au premier tour, avant le verrouillage"));
                }
                let (mut round, mut players) = (r.clone(), p.clone());
                let nb_pools = round.nb_pools() as u32;
                if !round.on_attendee_toggled(&mut players, kind, player, attending, nb_pools) {
                    return Err(bad_request("la formule de poules ne permet pas ce changement"));
                }
                *r = round;
                *p = players;
                Ok(())
            })?;
            accepted?;
            c.with_schedule(|schedule, ctx| {
                if let Some(checkin) = schedule.checkin_mut() {
                    checkin.on_attendees_toggled(ctx, &[player]);
                }
                schedule.apply_renumbering(ctx);
                if let Some(allocator) = schedule.stage(index).and_then(|s| s.as_any().downcast_ref::<PoolAllocatorStage>()) {
                    allocator.round().check_distribution(ctx.players);
                }
                schedule.refresh_general_classification(ctx);
            });
        }
        other => return Err(bad_request(format!("action inconnue : {other}"))),
    }
    Ok(())
}

// ------------------------------------------------------------------ poules

fn pools_view(c: &mut Contest, index: usize) -> Option<Value> {
    let s = c.schedule.stage(index)?.as_any().downcast_ref::<PoolSupervisorStage>()?;
    let locked = s.base().locked;
    let round = s.round();
    let pools: Vec<Value> = round
        .pools()
        .iter()
        .enumerate()
        .map(|(i, pool)| {
            let sheet = pool.sheet(&c.players, c.referees.store(), false);
            json!({
                "index": i,
                "name": sheet.title,
                // QR code de la feuille : l'arbitre le scanne puis donne son PIN.
                "token": bp_core::smartpoule::pool_token(c.net_id, round.net_id(), pool.number()),
                "piste": sheet.piste,
                "referee": sheet.referee.map(|(n, f)| format!("{n} {}", f.unwrap_or_default()).trim().to_string()),
                "over": pool.is_over(),
                "error": pool.has_error(),
                "headers": sheet.headers,
                "rows": sheet.rows.iter().map(|row| json!({
                    "reference": row.player,
                    "position": row.position,
                    "name": identity(&c.players, row.player),
                    "club": row.identity.club,
                    "cells": row.cells.iter().map(|cell| json!({ "text": cell.text, "grey": cell.grey, "match": cell.match_index })).collect::<Vec<_>>(),
                    "dashboard": row.dashboard.as_ref().map(|d| json!({
                        "status": d.status.to_string(),
                        "victories": d.victories,
                        "ratio": d.victories_ratio,
                        "hs": d.hits_scored,
                        "hr": d.hits_received,
                        "indice": d.indice,
                        "rank": d.rank,
                    })),
                })).collect::<Vec<_>>(),
                "matches": sheet.matches.iter().map(|bout| json!({
                    "index": bout.match_index,
                    "officiating": bout.officiating,
                    "name": bout.name,
                    "rest_error": bout.rest_error,
                    "opponents": bout.opponents.iter().map(|o| o.as_ref().map(|f| json!({
                        "reference": f.player,
                        "position": f.position,
                        "name": format!("{} {}", f.identity.name, f.identity.first_name).trim().to_string(),
                        "score": f.score,
                    }))).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let (current, combined) = round.compute_classification(&c.players);
    let codes = round.classification_filter.clone();
    let entries = |list: &[(PlayerRef, u32)]| -> Vec<RankEntry> {
        list.iter()
            .map(|(p, rank)| {
                let status = c
                    .players
                    .get(*p)
                    .and_then(|pl| pl.get(&bp_core::player::AttrId::local("status", round.supervisor_owner())))
                    .map(|v| v.xml_image().chars().next().unwrap_or('Q'))
                    .unwrap_or('Q');
                RankEntry { player: *p, rank: *rank, status }
            })
            .collect()
    };
    Some(json!({
        "pools": pools,
        "max_score": round.max_score,
        "nb_qualified": round.nb_qualified(),
        "balanced": round.seeding_is_balanced(),
        "round_name": round.name,
        "youth": round.is_youth(),
        "divisions": round.group_prefix.is_some(),
        "classification": ranked_list(&c.players, &codes, &entries(&combined), round.supervisor_owner()),
        "current_classification": ranked_list(&c.players, &codes, &entries(&current), round.current_round_owner()),
        "locked": locked,
    }))
}

fn pools_action(c: &mut Contest, index: usize, a: &Value) -> Result<(), ApiError> {
    let locked = c.schedule.stage(index).map(|s| s.base().locked).unwrap_or(true);
    let n = |k: &str| a.get(k).and_then(Value::as_u64).unwrap_or(0);
    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let action = s("action");
    if locked && action != "settings" {
        return Err(bad_request("les poules sont terminées : « Tour précédent » pour les modifier"));
    }
    match action.as_str() {
        "score" => {
            let value = s("value").trim().to_ascii_uppercase();
            let value = match value.strip_prefix('V') {
                Some(rest) if !rest.is_empty() => format!("W{rest}"),
                _ => value,
            };
            let (pool, m, fencer) = (n("pool") as usize, n("match") as usize, n("fencer") as PlayerRef);
            with_round(c, index, |r, p| {
                if let Some(pl) = r.pools_mut().get_mut(pool) {
                    pl.set_score_str(p, m, fencer, &value);
                }
                r.refresh_classification(p);
            })?;
        }
        "status" => {
            let (pool, fencer, code) = (n("pool") as usize, n("fencer") as PlayerRef, s("code"));
            with_round(c, index, |r, p| {
                if let Some(pl) = r.pools_mut().get_mut(pool) {
                    pl.set_status(p, fencer, &code);
                }
                r.refresh_classification(p);
            })?;
        }
        "settings" => {
            let name = s("name");
            let max = n("max_score") as u32;
            let qualified = a.get("nb_qualified").and_then(Value::as_u64).map(|v| v as u32);
            let balanced = a.get("balanced").and_then(Value::as_bool);
            with_round(c, index, |r, p| {
                if !name.trim().is_empty() {
                    r.name = name.trim().to_string();
                }
                if max > 0 {
                    r.set_max_score(max);
                }
                if !locked {
                    r.set_nb_qualified(qualified);
                    if let Some(b) = balanced {
                        if r.seeding_is_balanced() != b {
                            r.set_seeding_balanced(p, b);
                        }
                    }
                }
                for pool in r.pools_mut() {
                    pool.refresh_score_data(p);
                }
                r.refresh_classification(p);
            })?;
        }
        "stuff" => {
            if !c.settings.score_stuffing_allowed {
                return Err(bad_request("les scores aléatoires ne sont pas autorisés pour cette compétition"));
            }
            with_round(c, index, |r, p| r.stuff(p))?;
        }
        other => return Err(bad_request(format!("action inconnue : {other}"))),
    }
    Ok(())
}

// ---------------------------------------------------------------- tableaux

const METRICS: LayoutMetrics = LayoutMetrics { cell_width: 260.0, row_height: 34.0, header_height: 40.0, column_spacing: 12.0 };

fn table_view(c: &mut Contest, index: usize) -> Option<Value> {
    let referees = c.referees.store().clone();
    let stage = c.schedule.stage(index)?.as_any().downcast_ref::<TableSupervisor>()?;
    let locked = stage.base().locked;
    let sets: Vec<Value> = stage
        .table_sets()
        .iter()
        .enumerate()
        .filter(|(_, s)| s.is_visible())
        .map(|(i, s)| json!({ "index": i, "name": s.name(), "parent": s.parent(), "group": s.group() }))
        .collect();
    let displayed = stage.displayed_table_set();
    let drawing = displayed.map(|d| {
        let set = &stage.table_sets()[d];
        let dr = set.drawing_with_referees(&c.players, &referees, &METRICS);
        let b = |x: &bp_core::rounds::table::layout::Bounds| json!([x.x1, x.y1, x.x2 - x.x1, x.y2 - x.y1]);
        json!({
            "set": d,
            "width": dr.width,
            "height": dr.height,
            "can_previous": set.can_display_previous(),
            "can_next": set.can_display_next(),
            "headers": dr.headers.iter().map(|h| json!({
                "text": h.image,
                "box": b(&h.bounds),
                "status": h.status.map(|s| format!("{s:?}").to_lowercase()),
            })).collect::<Vec<_>>(),
            "cells": dr.cells.iter().map(|cell| json!({
                "node": cell.node,
                "box": b(&cell.bounds),
                "winner": cell.winner,
                "name": cell.winner.map(|w| identity(&c.players, w)),
                "club": if cell.compact { None } else { cell.winner.map(|w| club(&c.players, w)) },
                "label": cell.match_label.as_ref().map(|l| json!({
                    "box": b(&l.bounds),
                    "name": l.name,
                    "roadmap": l.roadmap.as_ref().map(|r| r.text.clone()),
                })),
                "score": cell.score.as_ref().filter(|s| s.visible && s.fencer.is_some()).map(|s| json!({
                    "box": b(&s.bounds),
                    "node": s.bout_node,
                    "fencer": s.fencer,
                    "text": s.text,
                    "status": s.drop_reason.map(|c| c.to_string()),
                    "error": set.bout(s.bout_node).map(|m| m.has_error()).unwrap_or(false),
                })),
            })).collect::<Vec<_>>(),
            "connectors": dr.connectors.iter().map(|k| k.points.iter().map(|(x, y)| json!([x, y])).collect::<Vec<_>>()).collect::<Vec<_>>(),
        })
    });
    let entries = stage.classification(&StageContext { players: &mut c.players.clone(), settings: &c.settings });
    let codes = stage.base().classification_filter.clone();
    Some(json!({
        "sets": sets,
        "displayed": displayed,
        "drawing": drawing,
        // Entraînement des jeunes : l'impression donne les pages des groupes.
        "youth": c.youth_pool_stage().is_some(),
        "fenced_places": match stage.fenced_places_mode() { FencedPlaces::AllPlaces => "all", FencedPlaces::ThirdPlaces => "third", _ => "none" },
        "max_score": stage.max_score(),
        "classification": ranked_list(&c.players, &codes, &entries, stage.base().owner()),
        "locked": locked,
    }))
}

fn table_action(c: &mut Contest, index: usize, a: &Value) -> Result<(), ApiError> {
    let n = |k: &str| a.get(k).and_then(Value::as_u64).unwrap_or(0);
    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let action = s("action");
    c.with_schedule(|schedule, ctx| -> Result<(), ApiError> {
        let stage = schedule
            .stage_mut(index)
            .and_then(|st| st.as_any_mut().downcast_mut::<TableSupervisor>())
            .ok_or_else(not_found)?;
        let locked = stage.base().locked;
        let edits = !matches!(action.as_str(), "select" | "previous" | "next");
        if locked && edits && action != "settings" {
            return Err(bad_request("les tableaux sont terminés : « Tour précédent » pour les modifier"));
        }
        match action.as_str() {
            "select" => stage.select_table_set(Some(n("set") as usize)),
            "previous" => stage.display_previous(n("set") as usize),
            "next" => stage.display_next(n("set") as usize),
            "score" => {
                let value = s("value").trim().to_ascii_uppercase();
                let value = match value.strip_prefix('V') {
                    Some(rest) if !rest.is_empty() => format!("W{rest}"),
                    _ => value,
                };
                let (set, node, fencer) = (n("set") as usize, n("node") as usize, n("fencer") as PlayerRef);
                if stage.current_status(set, node, fencer) != 'Q' {
                    stage.set_fencer_status(ctx, set, node, fencer, 'Q');
                }
                stage.set_score(ctx, set, node, fencer, &value);
            }
            "status" => {
                let code = s("code").chars().next().unwrap_or('Q');
                stage.set_fencer_status(ctx, n("set") as usize, n("node") as usize, n("fencer") as PlayerRef, code);
            }
            "settings" => {
                if let Some(max) = a.get("max_score").and_then(Value::as_u64).filter(|m| *m > 0) {
                    stage.set_max_score(max as u32);
                }
                if !locked {
                    match s("fenced_places").as_str() {
                        "all" => stage.set_fenced_places(FencedPlaces::AllPlaces),
                        "third" => stage.set_fenced_places(FencedPlaces::ThirdPlaces),
                        "none" => stage.set_fenced_places(FencedPlaces::None),
                        _ => {}
                    }
                    if let Some(q) = a.get("nb_qualified") {
                        stage.base_mut().nb_qualified = q.as_u64().map(|v| v as u32);
                    }
                }
                let name = s("name");
                if !name.trim().is_empty() {
                    stage.base_mut().name = name.trim().to_string();
                }
            }
            "stuff" => {
                if !ctx.settings.score_stuffing_allowed {
                    return Err(bad_request("les scores aléatoires ne sont pas autorisés pour cette compétition"));
                }
                stage.stuff(ctx, &mut rand::thread_rng());
            }
            other => return Err(bad_request(format!("action inconnue : {other}"))),
        }
        if !stage.base().locked {
            stage.update_classification(ctx);
        }
        Ok(())
    })
}

// --------------------------------------------------------- barrage, scission

fn barrage_view(c: &mut Contest, index: usize) -> Option<Value> {
    let b = c.schedule.stage(index)?.as_any().downcast_ref::<Barrage>()?;
    let owner = b.base().owner();
    let codes = b.base().filter.clone();
    let promoted: Vec<(PlayerRef, bool)> = b.barrage_players().iter().map(|r| (*r, b.is_promoted(&c.players, *r))).collect();
    let mut l = list(&c.players, &codes, &promoted.iter().map(|(r, _)| (*r, false)).collect::<Vec<_>>(), owner);
    for (row, (_, p)) in l.rows.iter_mut().zip(promoted.iter()) {
        row.checked = Some(*p);
    }
    let ranking: Vec<(PlayerRef, u32)> = b.compute_classification(&c.players);
    let entries: Vec<RankEntry> = ranking.iter().map(|(p, r)| RankEntry { player: *p, rank: *r, status: 'Q' }).collect();
    // Matchs de barrage (extension) : un match par paire d'ex aequo.
    let bouts: Vec<Value> = b
        .matches()
        .iter()
        .enumerate()
        .map(|(i, m)| {
            json!({
                "index": i,
                "name": m.name,
                "over": m.is_over(),
                "winner": m.winner().map(|w| identity(&c.players, w)),
                "fencers": (0..2).filter_map(|k| m.opponent(k).map(|f| json!({
                    "reference": f,
                    "name": identity(&c.players, f),
                    "club": club(&c.players, f),
                    "score": m.score_at(k).image(m.max_score),
                }))).collect::<Vec<_>>(),
            })
        })
        .collect();
    Some(json!({
        "bouts": bouts,
        "max_score": b.max_score(),
        "places": b.places(),
        "list": l,
        "qualified": b.qualified_count(&c.players),
        "dropped": b.dropped_count(&c.players),
        "classification": ranked_list(&c.players, &b.base().classification_filter, &entries, owner),
        "locked": b.base().locked,
    }))
}

/// Poules de division : réglages, division et mouvement de chaque tireur.
fn divisions_view(c: &mut Contest, index: usize) -> Option<Value> {
    use bp_core::rounds::divisions::{DivisionRound, Move};
    let d = c.schedule.stage(index)?.as_any().downcast_ref::<DivisionRound>()?;
    let owner = d.base().owner();
    let codes: Vec<String> = d.base().filter.clone();
    let refs: Vec<(PlayerRef, bool)> = d.base().attendees.iter().map(|r| (*r, false)).collect();
    let l = list(&c.players, &codes, &refs, owner);
    let fencers: serde_json::Map<String, Value> = d
        .base()
        .attendees
        .iter()
        .map(|r| {
            let movement = match d.movement(*r) {
                Move::Up => "up",
                Move::Down => "down",
                Move::Stays => "stays",
                Move::New => "",
            };
            (r.to_string(), json!({ "division": d.division_of(*r), "origin": d.origin_of(*r), "movement": movement }))
        })
        .collect();
    let sizes: Vec<usize> = (1..=d.settings.divisions).map(|n| d.members(n).len()).collect();
    Some(json!({ "list": l, "locked": d.base().locked, "settings": d.settings, "fencers": fencers, "sizes": sizes }))
}

/// `settings` (réglages) ou `division` (`players`, `division`).
fn divisions_action(c: &mut Contest, index: usize, a: &Value) -> Result<(), ApiError> {
    use bp_core::rounds::divisions::{DivisionRound, DivisionSettings};
    let stage = c.schedule.stage_mut(index).ok_or_else(not_found)?;
    if stage.base().locked {
        return Err(bad_request("la phase est terminée : « Tour précédent » pour la modifier"));
    }
    let d = stage.as_any_mut().downcast_mut::<DivisionRound>().ok_or_else(not_found)?;
    match a.get("action").and_then(Value::as_str).unwrap_or("") {
        "settings" => {
            let settings: DivisionSettings =
                serde_json::from_value(a.get("settings").cloned().unwrap_or(Value::Null)).map_err(|e| bad_request(format!("réglages invalides : {e}")))?;
            d.set_settings(settings);
        }
        "division" => {
            let division = a.get("division").and_then(Value::as_u64).unwrap_or(1) as u32;
            for r in a.get("players").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64) {
                d.set_division(r as PlayerRef, division);
            }
        }
        other => return Err(bad_request(format!("action inconnue : {other}"))),
    }
    Ok(())
}

fn splitting_view(c: &mut Contest, index: usize) -> Option<Value> {
    let s = c.schedule.stage(index)?.as_any().downcast_ref::<Splitting>()?;
    let owner = s.base().owner();
    let codes: Vec<String> = s.base().filter.iter().filter(|c| *c != "exported").cloned().collect();
    let refs: Vec<(PlayerRef, bool)> = s.base().attendees.iter().map(|r| (*r, false)).collect();
    let mut l = list(&c.players, &codes, &refs, owner);
    for row in l.rows.iter_mut() {
        row.checked = Some(s.division_of(&c.players, row.reference) > 0);
    }
    // Division de chaque tireur (0 = il reste) et noms des divisions.
    let assignments: serde_json::Map<String, Value> =
        s.base().attendees.iter().map(|r| (r.to_string(), json!(s.division_of(&c.players, *r)))).collect();
    let names: Vec<String> = (1..=s.config.divisions.len() as u32).map(|d| s.division_name(d)).collect();
    Some(json!({
        "list": l,
        "locked": s.base().locked,
        "label": s.base().name,
        "config": s.config,
        "divisions": names,
        "assignments": assignments,
    }))
}

pub(crate) fn general_list(c: &mut Contest, index: usize) -> Option<ListView> {
    let stage = c.schedule.stage(index)?.as_any().downcast_ref::<GeneralClassificationStage>()?;
    let codes = stage.filter.selected_codes();
    let refs: Vec<(PlayerRef, bool)> = stage.result().iter().map(|r| (*r, false)).collect();
    let mut view = list(&c.players, &codes, &refs, stage.base().owner());
    // Points de la compétition (préférences « Points »).
    if c.points.enabled {
        let points = c.final_points();
        view.columns.push(Column { code: "points".into(), title: "Points".into() });
        for row in view.rows.iter_mut() {
            let p = points.iter().find(|(r, _, _)| *r == row.reference).map(|(_, _, p)| bp_core::points::image(*p));
            row.cells.push(p.unwrap_or_default());
        }
    }
    Some(view)
}

fn classification_view(c: &mut Contest, index: usize) -> Option<Value> {
    Some(json!({
        "list": general_list(c, index)?,
        "youth": youth_results(c),
    }))
}

/// Score d'un match de barrage : `{ "action": "score", "bout", "fencer", "value" }`.
fn barrage_score(c: &mut Contest, index: usize, a: &Value) -> Result<(), ApiError> {
    let bout = a.get("bout").and_then(Value::as_u64).unwrap_or(0) as usize;
    let fencer = a.get("fencer").and_then(Value::as_u64).unwrap_or(0) as PlayerRef;
    let value = a.get("value").and_then(Value::as_str).unwrap_or("").to_string();
    c.with_schedule(|schedule, ctx| {
        let stage = schedule.stage_mut(index).ok_or_else(not_found)?;
        if stage.base().locked {
            return Err(bad_request("la phase est terminée : « Tour précédent » pour la modifier"));
        }
        let b = stage.as_any_mut().downcast_mut::<Barrage>().ok_or_else(not_found)?;
        b.set_score(ctx, bout, fencer, &value);
        Ok(())
    })
}

/// Point de scission : `config` (réglages des divisions), `division`
/// (`players`, `division` : 0 = ils restent) ou case cochée (première division).
fn splitting_action(c: &mut Contest, index: usize, a: &Value) -> Result<(), ApiError> {
    let action = a.get("action").and_then(Value::as_str).unwrap_or("");
    c.with_schedule(|schedule, ctx| {
        let stage = schedule.stage_mut(index).ok_or_else(not_found)?;
        if stage.base().locked {
            return Err(bad_request("la phase est terminée : « Tour précédent » pour la modifier"));
        }
        let s = stage.as_any_mut().downcast_mut::<Splitting>().ok_or_else(not_found)?;
        match action {
            // Libellé (GREG) ; N1/N2 et Tableau A/B préparent deux divisions.
            "label" => s.set_label(ctx, a.get("label").and_then(Value::as_str).unwrap_or("")),
            "config" => {
                let config: bp_core::rounds::splitting::SplitConfig =
                    serde_json::from_value(a.get("config").cloned().unwrap_or(Value::Null)).map_err(|e| bad_request(format!("réglages invalides : {e}")))?;
                s.set_config(ctx, config);
            }
            "division" => {
                let division = a.get("division").and_then(Value::as_u64).unwrap_or(0) as u32;
                let players: Vec<PlayerRef> = a
                    .get("players")
                    .and_then(Value::as_array)
                    .map(|v| v.iter().filter_map(|x| x.as_u64().map(|n| n as PlayerRef)).collect())
                    .unwrap_or_default();
                for r in players {
                    s.set_division(ctx, r, division);
                }
            }
            _ => {
                let r = a.get("player").and_then(Value::as_u64).unwrap_or(0) as PlayerRef;
                s.set_exported(ctx, r, a.get("value").and_then(Value::as_bool).unwrap_or(false));
            }
        }
        Ok(())
    })
}

fn toggle_action(c: &mut Contest, index: usize, a: &Value) -> Result<(), ApiError> {
    let r = a.get("player").and_then(Value::as_u64).unwrap_or(0) as PlayerRef;
    let value = a.get("value").and_then(Value::as_bool).unwrap_or(false);
    c.with_schedule(|schedule, ctx| {
        let stage = schedule.stage_mut(index).ok_or_else(not_found)?;
        if stage.base().locked {
            return Err(bad_request("la phase est terminée : « Tour précédent » pour la modifier"));
        }
        if let Some(b) = stage.as_any_mut().downcast_mut::<Barrage>() {
            b.set_promoted(ctx, r, value);
        } else if let Some(s) = stage.as_any_mut().downcast_mut::<Splitting>() {
            s.set_exported(ctx, r, value);
        } else {
            return Err(not_found());
        }
        Ok(())
    })
}

// ----------------------------------------------------------------- actions

pub async fn stage_action(
    State(store): State<Shared>,
    client: Client,
    Path((id, index)): Path<(String, usize)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let result = store
        .update(&client, &id, |c| -> Result<Value, ApiError> {
            let kind = c.schedule.stage(index).map(|s| s.kind()).ok_or_else(not_found)?;
            if body.get("action").and_then(Value::as_str) == Some("timing") {
                let timing: bp_core::officiating::BoutTiming = serde_json::from_value(body.get("timing").cloned().ok_or_else(|| bad_request("durée attendue"))?).map_err(|_| bad_request("durée invalide"))?;
                c.set_stage_timing(index, timing).map_err(bad_request)?;
                return stage_view(c, index);
            }

            match kind_code(kind) {
                "youth" => youth_action(c, index, &body)?,
                "divisions" => divisions_action(c, index, &body)?,
                "lobby" => lobby_action(c, index, &body)?,
                "pool_allocator" => allocator_action(c, index, &body)?,
                "pool_supervisor" => pools_action(c, index, &body)?,
                "table" => table_action(c, index, &body)?,
                "barrage" if body.get("action").and_then(Value::as_str) == Some("score") => barrage_score(c, index, &body)?,
                "splitting" => splitting_action(c, index, &body)?,
                "barrage" => toggle_action(c, index, &body)?,
                other => return Err(bad_request(format!("pas d'action pour la phase « {other} »"))),
            }
            stage_view(c, index)
        })
        .ok_or_else(not_found)?
        .map_err(internal)?;
    let mut v = result?;
    v["club"] = json!(client.id);
    Ok(Json(v))
}

/// Classement de chaque groupe puis de tous les groupes, en HTML (export).
pub fn youth_html(c: &Contest) -> Option<String> {
    let results = youth::results(&c.schedule, &c.players)?;
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let row = |r: &youth::YouthResult, place: Option<u32>| {
        format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            r.group,
            place.map(|p| p.to_string()).unwrap_or_default(),
            esc(&identity(&c.players, r.player)),
            esc(&club(&c.players, r.player)),
            r.victories,
            r.points.map(|p| p.to_string()).unwrap_or_default()
        )
    };
    let head = format!(
        "<tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr>\n",
        tr("Group"),
        tr("Place in the group"),
        tr("last name"),
        tr("club"),
        tr("Victories"),
        tr("Points")
    );
    let mut html = String::new();
    let mut groups: Vec<u32> = results.iter().map(|r| r.group).collect();
    groups.dedup();
    for g in groups {
        html.push_str(&format!("<h3>{} {g}</h3>\n<table class=\"list\">\n{head}", tr("Group")));
        for r in results.iter().filter(|r| r.group == g) {
            html.push_str(&row(r, r.place));
        }
        html.push_str("</table>\n");
    }
    let mut all: Vec<&youth::YouthResult> = results.iter().collect();
    all.sort_by_key(|r| (std::cmp::Reverse(r.points.unwrap_or(0)), std::cmp::Reverse(r.victories)));
    html.push_str(&format!("<h3>{}</h3>\n<table class=\"list\">\n{head}", tr("General classification (all groups)")));
    let (mut place, mut previous) = (0u32, None);
    for (i, r) in all.iter().enumerate() {
        let k = (r.points.unwrap_or(0), r.victories);
        if previous != Some(k) {
            place = i as u32 + 1;
            previous = Some(k);
        }
        html.push_str(&row(r, Some(place)));
    }
    html.push_str("</table>\n");
    Some(html)
}

// ------------------------------------------------- feuilles des groupes

/// Une feuille par groupe (entraînement des jeunes) : feuilles d'arbitre de
/// ses poules et tableau vierge, pour que l'arbitre ait tout le groupe en main.
pub async fn group_sheets(State(store): State<Shared>, client: Client, Path((id, index)): Path<(String, usize)>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let c = store.get_mut(&client, &id).ok_or_else(not_found)?;
    c.youth_group_sheets(index).map(Json).map_err(bad_request)
}

// ------------------------------------------------------- feuilles de match

/// Feuille de match imprimable : comme celle des tableaux du logiciel
/// (cases de touches à cocher, score, signature).
fn sheet_json(name: String, referee: Option<String>, piste: Option<String>, fencers: Vec<(String, String)>, max_score: u32) -> Value {
    json!({
        "name": name,
        "referee": referee,
        "piste": piste,
        "boxes": if max_score <= 60 { max_score } else { 0 },
        "fencers": fencers.into_iter().map(|(n, d)| json!({ "name": n, "details": d })).collect::<Vec<_>>(),
    })
}

#[derive(serde::Deserialize)]
pub struct RelaySheetsQuery {
    /// Nombre de feuilles vierges (0 : les rencontres de la phase).
    #[serde(default)]
    blank: u32,
    /// Par 3 : une feuille par équipe et une pour l'arbitre.
    #[serde(default)]
    three: bool,
}

/// `GET …/stages/{index}/relay_sheets?blank=&three=` : feuilles de match des
/// rencontres par équipes, déjà mises en page (repère 918 × 1300).
pub async fn relay_sheets(
    State(store): State<Shared>,
    client: Client,
    Path((id, index)): Path<(String, usize)>,
    axum::extract::Query(q): axum::extract::Query<RelaySheetsQuery>,
) -> Result<Json<Value>, ApiError> {
    use bp_core::relay_sheet;
    let mut store = store.lock().await;
    let c = store.get_mut(&client, &id).ok_or_else(not_found)?;
    let (format, bouts) = c.relay_sheets(index, None).ok_or_else(not_found)?;
    let base = if q.blank > 0 { vec![c.blank_relay_sheet(); q.blank.min(200) as usize] } else { bouts };
    let pages: Vec<Value> = base
        .iter()
        .flat_map(|s| relay_sheet::copies(s, q.three))
        .map(|s| json!(relay_sheet::layout(&format, &s)))
        .collect();
    Ok(Json(json!({ "width": relay_sheet::WIDTH, "height": relay_sheet::HEIGHT, "label": format.label(), "pages": pages })))
}

/// `GET …/stages/{index}/match_sheets` : feuilles des matchs du barrage, ou
/// des matchs prêts des tableaux.
pub async fn match_sheets(State(store): State<Shared>, client: Client, Path((id, index)): Path<(String, usize)>) -> Result<Json<Value>, ApiError> {
    let mut store = store.lock().await;
    let c = store.get_mut(&client, &id).ok_or_else(not_found)?;
    let stage = c.schedule.stage(index).ok_or_else(not_found)?;
    let details = |players: &PlayerStore, r: PlayerRef| {
        players
            .get(r)
            .map(|p| ["club", "league", "country"].iter().filter_map(|k| p.str_attr(k)).filter(|v| !v.is_empty()).collect::<Vec<_>>().join(" · "))
            .unwrap_or_default()
    };
    let mut sheets = Vec::new();
    if let Some(b) = stage.as_any().downcast_ref::<Barrage>() {
        for m in b.matches() {
            let fencers = (0..2).filter_map(|k| m.opponent(k)).map(|f| (identity(&c.players, f), details(&c.players, f))).collect();
            sheets.push(sheet_json(m.name.clone(), None, None, fencers, b.max_score()));
        }
    } else if let Some(sup) = stage.as_any().downcast_ref::<TableSupervisor>() {
        let stage_net = sup.base().net_id;
        for set in sup.table_sets().iter().filter(|s| s.is_visible()) {
            for node in set.matches_to_print(None, true) {
                let Some(sheet) = set.score_sheet_with_referees(node, &c.players, c.referees.store(), c.net_id, stage_net) else { continue };
                let max = set.bout(node).map(|m| m.max_score).unwrap_or(0);
                let fencers = sheet.fencers.iter().flatten().map(|f| (identity(&c.players, *f), details(&c.players, *f))).collect();
                let referee = sheet.referee.map(|(n, f)| format!("{n} {f}").trim().to_string());
                let mut result = sheet_json(format!("{} — {}", set.name(), sheet.name), referee, sheet.piste.clone(), fencers, max);
                result["officiating"] = json!(set.bout(node).and_then(|m| m.officiating.as_ref()).map(|s| s.summary()).unwrap_or_default());
                result["token"] = json!(set.match_path(node).map(|p| bp_core::smartpoule::match_token(c.net_id, stage_net, set.id(), &p)));
                sheets.push(result);
            }
        }
    } else {
        return Err(bad_request("feuilles de match : barrage ou tableau attendu"));
    }
    Ok(Json(json!({
        "contest": c.name,
        "club": client.id,
        "header": format!("{} - {} - {}", c.weapon.label(), c.gender_label(), c.category.display_image()),
        "date": c.date_image(),
        "sheets": sheets,
    })))
}

// ------------------------------------------------------------ lobby (série)

fn standing_json(s: &bp_core::rounds::lobby::Standing) -> Value {
    json!({ "points": s.points, "wins": s.wins, "best": s.best, "races": s.races, "abandons": s.abandons })
}

/// Lobby (série) : réglages, lobbys (arrivées de chaque manche), classements.
fn lobby_view(c: &mut Contest, index: usize) -> Option<Value> {
    use bp_core::rounds::lobby::{Lobby, ABANDON};
    let l = c.schedule.stage(index)?.as_any().downcast_ref::<Lobby>()?;
    let players = &c.players;
    let races = l.settings.races.max(1) as usize;
    let lobbies: Vec<Value> = (0..l.lobbies().len())
        .map(|i| {
            let ranking = l.lobby_ranking(i);
            let road = l.roadmap(i).cloned().unwrap_or_default();
            let referees: Vec<String> = road.referees.iter().filter_map(|r| c.referees.store().get(*r)).map(|p| format!("{} {}", p.name(), p.first_name()).trim().to_string()).collect();
            json!({
                "index": i,
                "roadmap": ([Some(road.label()).filter(|s| !s.is_empty()), (!referees.is_empty()).then(|| format!("Arbitre : {}", referees.join(", ")))].into_iter().flatten().collect::<Vec<_>>().join(" · ")),
                "done": l.races_done(i),
                "over": l.lobby_over(i),
                "rows": l.lobbies()[i].iter().map(|r| json!({
                    "reference": r,
                    "name": identity(players, *r),
                    "club": club(players, *r),
                    "finishes": (0..races).map(|k| match l.finish(i, k, *r) {
                        Some(ABANDON) => json!("A"),
                        Some(p) => json!(p),
                        None => Value::Null,
                    }).collect::<Vec<_>>(),
                    "standing": standing_json(&l.standing(*r)),
                    "place": ranking.iter().find(|(x, _, _)| x == r).map(|(_, p, _)| *p),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let members = l.members_race(players);
    let ranking: Vec<Value> = l
        .compute_classification(players)
        .into_iter()
        .map(|(r, place)| {
            let s = if members { l.team_standing(players, r) } else { l.standing(r) };
            json!({ "place": place, "name": identity(players, r), "club": club(players, r), "standing": standing_json(&s), "lobby": l.lobby_of(r).map(|x| x + 1) })
        })
        .collect();
    let individual: Vec<Value> = if members {
        l.individual_ranking(players)
            .into_iter()
            .map(|(r, place, s)| {
                let team = l.team_of(players, r).map(|t| identity(players, t)).unwrap_or_default();
                json!({ "place": place, "name": identity(players, r), "club": team, "standing": standing_json(&s), "lobby": l.lobby_of(r).map(|x| x + 1) })
            })
            .collect()
    } else {
        Vec::new()
    };
    // Formules possibles (nombre de lobbys), comme la composition des poules.
    let n = l.competitors(players).len();
    let choices: Vec<Value> = bp_core::rounds::lobby::lobby_choices(n)
        .into_iter()
        .map(|k| json!({ "lobbies": k, "label": bp_core::rounds::lobby::choice_label(k, n) }))
        .collect();
    Some(json!({
        "settings": l.settings,
        "competitors": n,
        "choices": choices,
        "nb_qualified": l.base().nb_qualified,
        "locked": l.base().locked,
        "team_event": c.settings.team_event,
        "members_race": members,
        "has_results": l.has_results(),
        "lobbies": lobbies,
        "ranking": ranking,
        "individual": individual,
    }))
}

/// Actions : `settings` (`settings`, `nb_qualified`), `composition`
/// (`lobbies`, `distribution` : `balanced` ou `level`), `finish` (`lobby`,
/// `race`, `player`, `value` : place, « A » ou vide), `compose`.
fn lobby_action(c: &mut Contest, index: usize, a: &Value) -> Result<(), ApiError> {
    use bp_core::rounds::lobby::{Lobby, LobbySettings, ABANDON};
    c.with_schedule(|s, ctx| {
        let stage = s.stage_mut(index).ok_or_else(not_found)?;
        if stage.base().locked {
            return Err(bad_request("la phase est terminée : « Tour précédent » pour la modifier"));
        }
        let l = stage.as_any_mut().downcast_mut::<Lobby>().ok_or_else(not_found)?;
        let n = |k: &str| a.get(k).and_then(Value::as_u64).unwrap_or(0);
        match a.get("action").and_then(Value::as_str).unwrap_or("") {
            "settings" => {
                if let Some(v) = a.get("settings") {
                    let settings: LobbySettings = serde_json::from_value(v.clone()).map_err(|e| bad_request(format!("réglages invalides : {e}")))?;
                    l.set_settings(ctx, settings);
                }
                if let Some(q) = a.get("nb_qualified") {
                    l.base_mut().nb_qualified = q.as_u64().filter(|v| *v > 0).map(|v| v as u32);
                }
            }
            "finish" => {
                let raw = a.get("value").map(|v| match v {
                    Value::Number(n) => n.to_string(),
                    Value::String(s) => s.clone(),
                    _ => String::new(),
                }).unwrap_or_default();
                let t = raw.trim().to_uppercase();
                let finish = if t.is_empty() {
                    None
                } else if t.starts_with('A') {
                    Some(ABANDON)
                } else {
                    Some(t.parse::<i32>().ok().filter(|p| *p > 0).ok_or_else(|| bad_request("place : un nombre, ou A pour un abandon"))?)
                };
                l.set_finish(n("lobby") as usize, n("race") as usize, n("player") as PlayerRef, finish);
            }
            "composition" => {
                if l.has_results() {
                    return Err(bad_request("des arrivées sont déjà saisies"));
                }
                let distribution = match a.get("distribution").and_then(Value::as_str) {
                    Some("level") => bp_core::rounds::lobby::Distribution::Level,
                    _ => bp_core::rounds::lobby::Distribution::Balanced,
                };
                l.set_composition(ctx, (n("lobbies") as u32).max(1), distribution);
            }
            "compose" => {
                if l.has_results() {
                    return Err(bad_request("des arrivées sont déjà saisies"));
                }
                l.compose(ctx.players);
            }
            "move" => l.move_competitor(n("player") as PlayerRef, n("lobby") as usize),
            other => return Err(bad_request(format!("action inconnue : {other}"))),
        }
        Ok(())
    })
}
