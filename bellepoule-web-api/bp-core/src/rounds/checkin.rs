//! Phase de pointage (port de `rounds/checkin/checkin_supervisor.cpp`, de
//! `actors/checkin.cpp`, `players_list.cpp` et `tally_counter.cpp` pour la
//! partie logique).
//!
//! Le pointage tient la liste des tireurs et des équipes de la compétition
//! (`list`, l'ordre de `PlayersList::_player_list`). Les joueurs eux-mêmes
//! vivent dans le `PlayerStore` de la compétition. À son verrouillage :
//! - les joueurs sont renumérotés (`UpdateChecksum`) : présents de 1 à n
//!   dans l'ordre alphabétique, absents à partir de 10000 ; la graine de la
//!   phase est tirée d'une empreinte MD5 des initiales ;
//! - le rang de départ est calculé d'après le classement (`UpdateRanking`) ;
//! - la sortie (qualifiés) est la liste des présents (et des tireurs ayant
//!   un incident) triée par rang.
//!
//! Pendant le chargement d'un fichier (`set_loading(true)`), la
//! renumérotation est différée : `take_renumbering` la fournit à la
//! compétition, qui l'applique à toutes les phases en fin de chargement.

pub mod import;
pub mod rank_importer;
pub mod source;

use std::any::Any;
use std::collections::HashMap;
use std::path::Path;

use md5::{Digest, Md5};

use crate::attribute::{Look, Value};
use crate::contest::players::{
    compare_players, compare_values, guess_player_organization, load_player, multi_compare, new_player,
    write_player_attributes,
};
use crate::contest::teams;
use crate::filter::Filter;
use crate::player::{AttrId, Player, PlayerClass, PlayerRef, PlayerStore};
use crate::stage::{RankEntry, Stage, StageBase, StageContext, StageKind};
use crate::xml::XmlWriter;

pub use import::ImportResult;
pub use rank_importer::RankImporter;
pub use source::Source;

/// Fabrique (signature fixe, appelée par `rounds::create_stage`).
pub fn new_stage(kind: StageKind, id: u32) -> Box<dyn Stage> {
    Box::new(CheckinStage::new(kind, id))
}

/// Compteurs de présence (`TallyCounter`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TallyCounter {
    /// Tireurs inscrits (`GetTotalFencerCount`).
    pub total_fencers: u32,
    /// Tireurs présents.
    pub present_fencers: u32,
    /// Équipes inscrites (hors équipe nulle).
    pub total_teams: u32,
    /// Équipes présentes.
    pub present_teams: u32,
    /// Mode équipes (épreuve par équipes).
    pub team_mode: bool,
}

impl TallyCounter {
    /// `GetPresentsCount`.
    pub fn presents(&self) -> u32 {
        if self.team_mode {
            self.present_teams
        } else {
            self.present_fencers
        }
    }

    /// `GetAbsentsCount`.
    pub fn absents(&self) -> u32 {
        if self.team_mode {
            self.total_teams - self.present_teams
        } else {
            self.total_fencers - self.present_fencers
        }
    }

    /// `GetTotalCount`.
    pub fn total(&self) -> u32 {
        self.presents() + self.absents()
    }
}

/// Résultat d'un changement de présence (`TogglePlayerAttr`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleOutcome {
    Done,
    /// La présence d'une équipe ne se coche pas : il faut cocher ses
    /// membres (message du C++ rappelant la taille minimale d'équipe).
    TeamNotToggleable { minimum_team_size: u32 },
    UnknownPlayer,
}

pub struct CheckinStage {
    base: StageBase,
    /// Tireurs et équipes, dans l'ordre de la liste.
    list: Vec<PlayerRef>,
    /// Joueurs de la « liste anticipée » (présents à la fermeture du pointage).
    checksum_list: Vec<PlayerRef>,
    pub source: Source,
    /// Colonnes de la liste des tireurs.
    pub filter: Filter,
    /// Colonnes du formulaire des équipes.
    pub team_filter: Filter,
    loading: bool,
    result: Vec<PlayerRef>,
    output: Vec<PlayerRef>,
    presents: Vec<PlayerRef>,
    absents: Vec<PlayerRef>,
    pending_renumber: Option<HashMap<PlayerRef, PlayerRef>>,
    last_renumber: Option<HashMap<PlayerRef, PlayerRef>>,
}

const FENCER_EXCLUDED: &[&str] = &[
    "ref",
    "plugin_ID",
    "IP",
    "password",
    "cyphered_password",
    "HS",
    "exported",
    "final_rank",
    "global_status",
    "indice",
    "level",
    "workload_rate",
    "pool_nr",
    "stage_start_rank",
    "promoted",
    "rank",
    "status",
    "victories_count",
    "bouts_count",
    "victories_ratio",
    "strip",
    "time",
];

impl CheckinStage {
    pub fn new(kind: StageKind, id: u32) -> Self {
        let mut filter = Filter::excluding("CheckinSupervisor", FENCER_EXCLUDED);
        for c in ["attending", "name", "first_name", "birth_date", "gender", "club", "league", "region", "country", "ranking", "licence"] {
            filter.show_attribute(c);
        }
        let team_filter = Filter::including("CheckinSupervisor", &["name", "ranking"]);
        let mut base = StageBase::new(kind, id);
        base.filter = filter.selected_codes();
        CheckinStage {
            base,
            list: Vec::new(),
            checksum_list: Vec::new(),
            source: Source::new(),
            filter,
            team_filter,
            loading: false,
            result: Vec::new(),
            output: Vec::new(),
            presents: Vec::new(),
            absents: Vec::new(),
            pending_renumber: None,
            last_renumber: None,
        }
    }

    fn owner(&self) -> crate::player::Owner {
        self.base.owner()
    }

    /// Indique que la compétition est en cours de chargement (`State::LOADING`).
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    /// Renumérotation déjà appliquée par le dernier verrouillage, à
    /// propager aux autres phases.
    pub fn take_renumbering(&mut self) -> Option<HashMap<PlayerRef, PlayerRef>> {
        self.last_renumber.take()
    }

    /// Applique une renumérotation aux listes internes.
    fn remap_lists(&mut self, map: &HashMap<PlayerRef, PlayerRef>) {
        let tr = |v: &mut Vec<PlayerRef>| {
            for r in v.iter_mut() {
                if let Some(n) = map.get(r) {
                    *r = *n;
                }
            }
        };
        tr(&mut self.list);
        tr(&mut self.checksum_list);
        tr(&mut self.result);
        tr(&mut self.output);
        tr(&mut self.presents);
        tr(&mut self.absents);
        tr(&mut self.base.attendees);
    }

    // ----------------------------------------------------------------------
    // Accès à la liste

    /// Tireurs et équipes, dans l'ordre de la liste.
    pub fn players(&self) -> &[PlayerRef] {
        &self.list
    }

    /// Tireurs de la liste.
    pub fn fencers(&self, store: &PlayerStore) -> Vec<PlayerRef> {
        self.of_class(store, PlayerClass::Fencer)
    }

    /// Équipes de la liste.
    pub fn teams(&self, store: &PlayerStore) -> Vec<PlayerRef> {
        self.of_class(store, PlayerClass::Team)
    }

    fn of_class(&self, store: &PlayerStore, class: PlayerClass) -> Vec<PlayerRef> {
        self.list.iter().copied().filter(|r| store.get(*r).map(|p| p.class == class).unwrap_or(false)).collect()
    }

    /// Tireurs sans équipe (rattachés à l'équipe nulle).
    pub fn fencers_without_team(&self, store: &PlayerStore) -> Vec<PlayerRef> {
        self.fencers(store).into_iter().filter(|r| store.get(*r).map(|p| p.team.is_none()).unwrap_or(false)).collect()
    }

    /// Noms d'équipes connus (valeurs discrètes de l'attribut `team`).
    pub fn team_names(&self, store: &PlayerStore) -> Vec<String> {
        self.teams(store).iter().filter_map(|t| store.get(*t)).map(|t| t.name()).collect()
    }

    /// `GetTeam`.
    pub fn team_by_name(&self, store: &PlayerStore, name: &str) -> Option<PlayerRef> {
        teams::find_team(store, &self.list, name)
    }

    /// Le tireur `fencer` (ou un nouveau tireur) peut-il rejoindre l'équipe
    /// `team_name` ? Une équipe compte au plus ses tireurs et ses remplaçants
    /// (réglés à l'appel). Renvoie le message à afficher sinon.
    pub fn team_full_error(&self, store: &PlayerStore, settings: &crate::stage::ContestSettings, fencer: Option<PlayerRef>, team_name: &str) -> Option<String> {
        if !settings.team_event || team_name.trim().is_empty() {
            return None;
        }
        let team = self.team_by_name(store, team_name)?;
        let t = store.get(team)?;
        if fencer.is_some_and(|f| t.members.contains(&f)) {
            return None;
        }
        let capacity = settings.team_size + settings.substitutes;
        (t.members.len() as u32 >= capacity).then(|| {
            format!(
                "L'équipe {} est complète : {} tireur(s) et {} remplaçant(s) au plus.",
                t.name(),
                settings.team_size,
                settings.substitutes
            )
        })
    }

    /// Équipes venues (au moins un tireur présent) qui n'ont pas assez de
    /// tireurs présents pour une rencontre : (nom, présents).
    pub fn incomplete_teams(&self, store: &PlayerStore, settings: &crate::stage::ContestSettings) -> Vec<(String, u32)> {
        if !settings.team_event {
            return Vec::new();
        }
        self.list
            .iter()
            .filter_map(|r| store.get(*r))
            .filter(|p| p.class == PlayerClass::Team)
            .filter_map(|t| {
                let present = t.members.iter().filter_map(|m| store.get(*m)).filter(|m| m.attending()).count() as u32;
                // Équipe venue (un tireur au moins) mais incomplète.
                ((t.attending() || present > 0) && present < settings.team_size).then(|| (t.name(), present))
            })
            .collect()
    }

    /// `GetPlayerFromRef`.
    pub fn contains(&self, r: PlayerRef) -> bool {
        self.list.contains(&r)
    }

    /// Recherche d'un joueur par la valeur d'un attribut global
    /// (`GetPlayerWithAttribute`).
    pub fn find_with_attribute(&self, store: &PlayerStore, code: &str, value: &Value) -> Option<PlayerRef> {
        let id = AttrId::global(code);
        self.list.iter().copied().find(|r| {
            store
                .get(*r)
                .map(|p| compare_values(code, Some(value), p.get(&id)) == std::cmp::Ordering::Equal)
                .unwrap_or(false)
        })
    }

    /// Compteurs (`TallyCounter`), mode équipes selon l'épreuve.
    pub fn tally(&self, store: &PlayerStore, team_event: bool) -> TallyCounter {
        let mut t = TallyCounter { team_mode: team_event, ..Default::default() };
        for p in self.list.iter().filter_map(|r| store.get(*r)) {
            match p.class {
                PlayerClass::Team => {
                    t.total_teams += 1;
                    t.present_teams += p.attending() as u32;
                }
                _ => {
                    t.total_fencers += 1;
                    t.present_fencers += p.attending() as u32;
                }
            }
        }
        t
    }

    /// Présents (au sens de l'épreuve) à la fermeture du pointage.
    pub fn presents(&self) -> &[PlayerRef] {
        &self.presents
    }

    /// Absents à la fermeture du pointage.
    pub fn absents(&self) -> &[PlayerRef] {
        &self.absents
    }

    // ----------------------------------------------------------------------
    // Filtres de liste

    fn class_matches(p: &Player, team_event: bool) -> bool {
        (p.class == PlayerClass::Team) == team_event
    }

    /// `Checkin::PresentPlayerFilter` + classe de l'épreuve.
    pub fn is_present(p: &Player, team_event: bool) -> bool {
        let attending = p.attr("attending").map(Value::as_bool).unwrap_or(true);
        attending && Self::class_matches(p, team_event)
    }

    fn is_absent(p: &Player, team_event: bool) -> bool {
        !Self::is_present(p, team_event) && Self::class_matches(p, team_event)
    }

    fn is_involved(p: &Player, team_event: bool) -> bool {
        if p.attr("incident").is_some() {
            Self::class_matches(p, team_event)
        } else {
            Self::is_present(p, team_event)
        }
    }

    /// `PlayerBelongsToEarlyList`.
    fn belongs_to_early_list(p: &Player) -> bool {
        match p.attr("incident") {
            Some(v) => v.xml_image().starts_with('A'),
            None => p.attending(),
        }
    }

    fn custom_list(&self, store: &PlayerStore, f: impl Fn(&Player) -> bool) -> Vec<PlayerRef> {
        self.list.iter().copied().filter(|r| store.get(*r).map(&f).unwrap_or(false)).collect()
    }

    // ----------------------------------------------------------------------
    // Ajout, suppression, modifications

    /// Ajoute un joueur à la compétition (`CheckinSupervisor::Add`) et
    /// renvoie sa référence. Une équipe est enregistrée ; un tireur portant
    /// un attribut `team` est rattaché à l'équipe de ce nom (créée au besoin).
    pub fn add(&mut self, ctx: &mut StageContext, player: Player) -> PlayerRef {
        self.add_with_owner(ctx, player, None)
    }

    fn add_with_owner(&mut self, ctx: &mut StageContext, player: Player, owner: Option<PlayerRef>) -> PlayerRef {
        let class = player.class;
        let r = ctx.players.add(player);
        if class == PlayerClass::Fencer {
            // `OnPlayerLoaded`
            teams::set_team(ctx.players, r, owner);
        }
        if class == PlayerClass::Team {
            self.register_new_team(ctx, r);
        }
        self.list.push(r);
        // `Monitor` → `NotifyChange ("team")`
        if class == PlayerClass::Fencer {
            if let Some(name) = ctx.players.get(r).and_then(|p| p.str_attr("team")).map(str::to_string) {
                self.on_team_attr_changed(ctx, r, None, &name);
            }
        }
        r
    }

    /// `RegisterNewTeam` : configuration de l'équipe (présence, classement).
    fn register_new_team(&mut self, ctx: &mut StageContext, team: PlayerRef) {
        teams::set_attending_from_members(ctx.players, team, ctx.settings);
    }

    /// `OnAttrTeamChanged` : le tireur rejoint l'équipe `team_name` (créée si
    /// besoin), l'ancienne et la nouvelle équipe recalculent leurs attributs.
    fn on_team_attr_changed(&mut self, ctx: &mut StageContext, fencer: PlayerRef, previous: Option<String>, team_name: &str) {
        let attending = ctx.players.get(fencer).map(|p| p.attending()).unwrap_or(false);
        // Étape BEFORE_CHANGE : présence forcée à 0 (statut global « F »).
        teams::set_attending(ctx.players, fencer, false);
        let team = if team_name.is_empty() {
            None
        } else {
            match self.team_by_name(ctx.players, team_name) {
                Some(t) => Some(t),
                None => {
                    let mut t = new_player(PlayerClass::Team);
                    t.set_str(&AttrId::global("name"), team_name);
                    Some(self.add(ctx, t))
                }
            }
        };
        teams::set_team(ctx.players, fencer, team);
        if let Some(t) = team {
            teams::set_attributes_from_members(ctx.players, t);
        }
        if let Some(prev) = previous.and_then(|n| self.team_by_name(ctx.players, &n)) {
            teams::set_attributes_from_members(ctx.players, prev);
        }
        // Étape AFTER_CHANGE : présence restaurée.
        teams::set_attending(ctx.players, fencer, attending);
    }

    /// Supprime un joueur (`PlayersList::Remove`) ; supprimer une équipe
    /// supprime aussi ses membres.
    pub fn remove(&mut self, ctx: &mut StageContext, r: PlayerRef) {
        let Some(p) = ctx.players.get(r) else { return };
        match p.class {
            PlayerClass::Team => {
                for m in p.members.clone() {
                    self.remove(ctx, m);
                }
            }
            _ => {
                if let Some(team) = p.team {
                    teams::set_team(ctx.players, r, None);
                    teams::set_attributes_from_members(ctx.players, team);
                }
            }
        }
        self.list.retain(|x| *x != r);
        ctx.players.remove(r);
        self.on_list_changed(ctx);
    }

    /// `Wipe` : vide la liste.
    pub fn wipe(&mut self, ctx: &mut StageContext) {
        for r in std::mem::take(&mut self.list) {
            ctx.players.remove(r);
        }
    }

    /// `OnListChanged` : recalcul de la présence des équipes.
    pub fn on_list_changed(&mut self, ctx: &mut StageContext) {
        for t in self.teams(ctx.players) {
            teams::set_attending_from_members(ctx.players, t, ctx.settings);
        }
    }

    /// `ApplyConfig` (changement de taille d'équipe, de classement par
    /// défaut ou de mode de classement).
    pub fn apply_config(&mut self, ctx: &mut StageContext) {
        self.on_list_changed(ctx);
    }

    /// Coche/décoche la présence d'un joueur (`TogglePlayerAttr`).
    pub fn set_attending(&mut self, ctx: &mut StageContext, r: PlayerRef, attending: bool) -> ToggleOutcome {
        match ctx.players.get(r).map(|p| p.class) {
            None => ToggleOutcome::UnknownPlayer,
            Some(PlayerClass::Team) => {
                ToggleOutcome::TeamNotToggleable { minimum_team_size: ctx.settings.minimum_team_size }
            }
            Some(_) => {
                teams::set_attending(ctx.players, r, attending);
                self.on_list_changed(ctx);
                ToggleOutcome::Done
            }
        }
    }

    /// Tous présents / tous absents (`OnToggleAllPlayers`).
    pub fn set_all_attending(&mut self, ctx: &mut StageContext, attending: bool) {
        for r in self.fencers(ctx.players) {
            teams::set_attending(ctx.players, r, attending);
        }
        self.on_list_changed(ctx);
    }

    /// Modifie un attribut global d'un joueur, avec les effets du C++ :
    /// présence → statut global, `team` d'un tireur → changement d'équipe,
    /// `name` d'une équipe → renommage chez ses membres.
    pub fn set_player_attribute(&mut self, ctx: &mut StageContext, r: PlayerRef, code: &str, value: Option<&str>) {
        let Some(p) = ctx.players.get(r) else { return };
        let class = p.class;
        let id = AttrId::global(code);
        let previous = p.get(&id).map(Value::xml_image);
        if code == "attending" {
            let v = value.map(|v| Value::parse(crate::attribute::AttrType::Bool, v).as_bool()).unwrap_or(false);
            self.set_attending(ctx, r, v);
            return;
        }
        {
            let p = ctx.players.get_mut(r).unwrap();
            match value {
                Some(v) => {
                    p.set_str(&id, v);
                }
                None => p.remove(&id),
            }
        }
        if previous.as_deref() == value {
            return;
        }
        if class == PlayerClass::Fencer && code == "team" {
            self.on_team_attr_changed(ctx, r, previous, value.unwrap_or(""));
            self.on_list_changed(ctx);
        } else if class == PlayerClass::Team && code == "name" {
            // `OnAttrTeamRenamed`
            let members = ctx.players.get(r).map(|t| t.members.clone()).unwrap_or_default();
            for m in members {
                if let Some(mp) = ctx.players.get_mut(m) {
                    mp.set_str(&AttrId::global("team"), value.unwrap_or(""));
                }
            }
            self.register_new_team(ctx, r);
        } else if class == PlayerClass::Fencer {
            if let Some(t) = ctx.players.get(r).and_then(|p| p.team) {
                teams::set_attributes_from_members(ctx.players, t);
            }
            if code == "ranking" {
                self.on_list_changed(ctx);
            }
        }
    }

    /// Événement du formulaire (`OnFormEvent`) : `new_player` vrai pour un
    /// ajout (le joueur n'est pas encore dans la compétition), faux pour la
    /// mise à jour d'un joueur existant `r`.
    pub fn form_new_player(&mut self, ctx: &mut StageContext, mut player: Player) -> Option<PlayerRef> {
        if player.class == PlayerClass::Team && self.team_by_name(ctx.players, &player.name()).is_some() {
            return None;
        }
        let status = if player.attending() { "Q" } else { "F" };
        player.set_str(&AttrId::global("global_status"), status);
        let r = self.add(ctx, player);
        self.on_list_changed(ctx);
        Some(r)
    }

    /// Mise à jour par le formulaire d'un joueur existant.
    pub fn form_update_player(&mut self, ctx: &mut StageContext, r: PlayerRef) {
        if let Some(t) = ctx.players.get(r).and_then(|p| p.team) {
            teams::set_attributes_from_members(ctx.players, t);
        }
        self.on_list_changed(ctx);
    }

    /// Glisser-déposer d'un tireur sur une ligne (`OnDragDrop`) : le tireur
    /// rejoint l'équipe de la ligne (ou celle du tireur visé).
    pub fn move_fencer_to(&mut self, ctx: &mut StageContext, fencer: PlayerRef, dest: PlayerRef) {
        let Some(d) = ctx.players.get(dest) else { return };
        let team = if d.class == PlayerClass::Fencer { d.team } else { Some(dest) };
        let name = team.and_then(|t| ctx.players.get(t)).map(|t| t.name()).unwrap_or_default();
        self.set_player_attribute(ctx, fencer, "team", Some(&name));
    }

    /// « Coller une copie » (`OnPasteCloneSelection`) : duplique les joueurs
    /// (et les membres d'une équipe).
    pub fn paste_clones(&mut self, ctx: &mut StageContext, clipboard: &[PlayerRef]) -> Vec<PlayerRef> {
        let mut created = Vec::new();
        for r in clipboard {
            let Some(p) = ctx.players.get(*r) else { continue };
            let mut clones = vec![*r];
            if p.class == PlayerClass::Team {
                clones.extend(p.members.iter().copied());
            }
            for c in clones {
                if let Some(src) = ctx.players.get(c) {
                    let mut dup = new_player(src.class);
                    dup.update_from(src);
                    dup.reference = 0;
                    created.push(self.add(ctx, dup));
                }
            }
        }
        self.on_list_changed(ctx);
        created
    }

    // ----------------------------------------------------------------------
    // Fichiers

    /// Charge une liste depuis un document (`Checkin::LoadList`) : les
    /// équipes (`<Equipes><Equipe><Tireur/>…`) puis les tireurs.
    pub fn load_list(&mut self, ctx: &mut StageContext, root: roxmltree::Node, class: PlayerClass, extra: &[String]) {
        let list_tag = format!("{}s", class.xml_tag());
        let Some(list_node) = crate::xml::child(root, &list_tag) else { return };
        for n in list_node.children().filter(|n| n.is_element()) {
            let tag = n.tag_name().name();
            if tag == class.xml_tag() {
                let r = self.load_player(ctx, n, class, None, extra);
                if class == PlayerClass::Team {
                    for m in n.children().filter(|m| m.is_element()) {
                        if m.tag_name().name() == PlayerClass::Fencer.xml_tag() {
                            self.load_player(ctx, m, PlayerClass::Fencer, Some(r), extra);
                        } else {
                            break;
                        }
                    }
                }
            } else {
                break;
            }
        }
        self.on_list_changed(ctx);
    }

    fn load_player(
        &mut self,
        ctx: &mut StageContext,
        node: roxmltree::Node,
        class: PlayerClass,
        owner: Option<PlayerRef>,
        extra: &[String],
    ) -> PlayerRef {
        let mut p = load_player(class, node, extra);
        guess_player_organization(&mut p, "league");
        guess_player_organization(&mut p, "region");
        self.add_with_owner(ctx, p, owner)
    }

    /// Enregistre la liste d'une classe (`SaveList`) : `<Tireurs>` ou
    /// `<Equipes>`. En épreuve par équipes, les membres d'une équipe sont
    /// écrits dans leur équipe et non dans `<Tireurs>`.
    pub fn save_list(&self, ctx: &StageContext, w: &mut XmlWriter, class: PlayerClass, extra: &[String]) {
        let team_event = ctx.settings.team_event;
        w.start(&format!("{}s", class.xml_tag()));
        for r in &self.list {
            let Some(p) = ctx.players.get(*r) else { continue };
            if p.class != class {
                continue;
            }
            if team_event && class == PlayerClass::Fencer && p.team.is_some() {
                continue;
            }
            w.start(p.class.xml_tag());
            write_player_attributes(p, w, extra, false);
            if p.class == PlayerClass::Team && team_event {
                for m in &p.members {
                    if let Some(mp) = ctx.players.get(*m) {
                        w.start(mp.class.xml_tag());
                        write_player_attributes(mp, w, extra, false);
                        w.end();
                    }
                }
            }
            w.end();
        }
        w.end();
    }

    /// Ajoute les joueurs importés (`Checkin::Import` sans dialogue).
    fn add_imported(&mut self, ctx: &mut StageContext, result: ImportResult) -> ImportResult {
        let count = result.players.len();
        let mut added = Vec::with_capacity(count);
        for p in result.players {
            added.push(p.clone());
            self.add(ctx, p);
        }
        self.on_list_changed(ctx);
        ImportResult { players: added, encoding_error: result.encoding_error }
    }

    /// `Import` : FFF (`.fff`) ou CSV ; mémorise le fichier source.
    pub fn import(&mut self, ctx: &mut StageContext, path: &Path) -> std::io::Result<ImportResult> {
        let name = path.to_string_lossy().to_string();
        let result = if name.ends_with(".fff") || name.ends_with(".FFF") {
            import::import_fff(path, PlayerClass::Fencer)?
        } else {
            import::import_csv(path, PlayerClass::Fencer)?
        };
        let r = self.add_imported(ctx, result);
        self.source.set_url(&name);
        Ok(r)
    }

    /// `ImportFFF` seul.
    pub fn import_fff(&mut self, ctx: &mut StageContext, path: &Path) -> std::io::Result<ImportResult> {
        let r = import::import_fff(path, PlayerClass::Fencer)?;
        Ok(self.add_imported(ctx, r))
    }

    /// `ImportCSV` seul.
    pub fn import_csv(&mut self, ctx: &mut StageContext, path: &Path) -> std::io::Result<ImportResult> {
        let r = import::import_csv(path, PlayerClass::Fencer)?;
        Ok(self.add_imported(ctx, r))
    }

    /// `ReloadFencers` : réimporte le fichier source s'il a changé.
    pub fn reload_fencers(&mut self, ctx: &mut StageContext) -> std::io::Result<bool> {
        if self.base.locked || !self.source.has_updates() {
            return Ok(false);
        }
        let Some(url) = self.source.url().map(str::to_string) else { return Ok(false) };
        self.wipe(ctx);
        self.import(ctx, Path::new(&url))?;
        Ok(true)
    }

    /// `OnImportRanking` : applique un fichier de classement.
    pub fn import_ranking(&mut self, ctx: &mut StageContext, importer: &RankImporter) {
        for r in self.list.clone() {
            if let Some(p) = ctx.players.get_mut(r) {
                importer.modify_rank(p);
            }
        }
        self.on_list_changed(ctx);
    }

    /// `ConvertFromBaseToResult` : dans les fichiers FIE « de base »,
    /// `Classement` est le classement d'entrée et non le rang final.
    pub fn convert_from_base_to_result(&mut self, ctx: &mut StageContext) {
        let place = AttrId::global("final_rank");
        let ranking = AttrId::global("ranking");
        for r in &self.list {
            if let Some(p) = ctx.players.get_mut(*r) {
                if let Some(v) = p.get(&place).map(Value::as_uint) {
                    p.set_uint(&ranking, v);
                    p.remove(&place);
                }
            }
        }
        for t in self.teams(ctx.players) {
            teams::set_attending_from_members(ctx.players, t, ctx.settings);
        }
    }

    // ----------------------------------------------------------------------
    // Verrouillage

    /// `UpdateChecksum` : tri alphabétique, références et graine.
    fn update_checksum(&mut self, ctx: &mut StageContext) {
        let ids = [AttrId::global("name"), AttrId::global("first_name")];
        let store = &*ctx.players;
        self.list.sort_by(|a, b| match (store.get(*a), store.get(*b)) {
            (Some(x), Some(y)) => multi_compare(x, y, &ids, 0),
            _ => std::cmp::Ordering::Equal,
        });
        self.checksum_list.clear();
        let mut md5 = Md5::new();
        let mut map = HashMap::new();
        let mut ref_ = 1;
        let mut absent_ref = 10000;
        for r in &self.list {
            let Some(p) = store.get(*r) else { continue };
            if Self::belongs_to_early_list(p) {
                self.checksum_list.push(*r);
                if let Some(first) = p.name().bytes().next() {
                    md5.update([first.to_ascii_uppercase()]);
                }
                map.insert(*r, ref_);
                ref_ += 1;
            } else {
                map.insert(*r, absent_ref);
                absent_ref += 1;
            }
        }
        let digest = md5.finalize();
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        self.base.rand_seed = u32::from_str_radix(&hex[..8], 16).unwrap_or(0);
        if map.iter().any(|(a, b)| a != b) {
            if self.loading {
                self.pending_renumber = Some(map);
            } else {
                ctx.players.renumber(&map);
                self.remap_lists(&map);
                self.last_renumber = Some(map);
            }
        }
    }

    /// Applique la renumérotation différée (fin de chargement,
    /// `OnLoadingCompleted`) ; renvoie la table à propager aux autres phases.
    pub fn apply_pending_renumbering(&mut self, ctx: &mut StageContext) -> Option<HashMap<PlayerRef, PlayerRef>> {
        let map = self.pending_renumber.take()?;
        ctx.players.renumber(&map);
        self.remap_lists(&map);
        Some(map)
    }

    /// `UpdateRanking` : rang de départ d'après le classement (aléa pour
    /// les ex aequo).
    fn update_ranking(&mut self, ctx: &mut StageContext) {
        let ranking = AttrId::global("ranking");
        let seed = self.base.rand_seed;
        {
            let store = &*ctx.players;
            crate::sort::glib_sort_by(&mut self.list, |a, b| match (store.get(*a), store.get(*b)) {
                (Some(x), Some(y)) => compare_players(x, y, &ranking, seed),
                _ => std::cmp::Ordering::Equal,
            });
        }
        let start_id = AttrId::local("stage_start_rank", self.owner());
        let rank_id = AttrId::local("rank", self.owner());
        let nb_player = self.list.len() as u32;
        let mut previous: Option<PlayerRef> = None;
        let mut stage_start_rank = 0;
        let mut nb_present = 1;
        for r in self.list.clone() {
            let early = ctx.players.get(r).map(Self::belongs_to_early_list).unwrap_or(false);
            if early {
                let tie = previous
                    .and_then(|pr| Some((ctx.players.get(pr)?, ctx.players.get(r)?)))
                    .map(|(a, b)| compare_players(a, b, &ranking, seed) == std::cmp::Ordering::Equal)
                    .unwrap_or(false);
                let value = if tie { stage_start_rank } else { nb_present };
                if !tie {
                    stage_start_rank = nb_present;
                }
                let p = ctx.players.get_mut(r).unwrap();
                p.set_uint(&start_id, value);
                p.set_uint(&rank_id, value);
                previous = Some(r);
                nb_present += 1;
            } else if let Some(p) = ctx.players.get_mut(r) {
                p.set_uint(&start_id, nb_player);
                p.set_uint(&rank_id, nb_player);
            }
        }
    }

    /// `GetCurrentClassification` + `SetOutputShortlist`.
    fn set_result(&mut self, ctx: &mut StageContext) {
        let team_event = ctx.settings.team_event;
        let mut all = self.custom_list(ctx.players, |p| Self::is_involved(p, team_event));
        let rank_id = AttrId::local("rank", self.owner());
        crate::contest::players::sort_refs(ctx.players, &mut all, &rank_id, self.base.rand_seed);
        self.presents = self.custom_list(ctx.players, |p| Self::is_present(p, team_event));
        self.absents = self.custom_list(ctx.players, |p| Self::is_absent(p, team_event));
        self.output = all.clone();
        self.result = all;
    }

    /// Un tireur absent est déclaré présent (ou inversement) depuis une
    /// phase ultérieure (`OnAttendeeToggled`) : le pointage est reverrouillé.
    pub fn on_attendee_toggled(&mut self, ctx: &mut StageContext, attendee: PlayerRef) {
        self.on_attendees_toggled(ctx, &[attendee]);
    }

    /// Valide ensemble les modifications d'un dialogue. Le verrouillage
    /// renumérote les joueurs : il doit suivre l'inscription de tous les ajouts.
    pub fn on_attendees_toggled(&mut self, ctx: &mut StageContext, attendees: &[PlayerRef]) {
        for attendee in attendees {
            if !self.list.contains(attendee) && ctx.players.get(*attendee).is_some() {
                self.list.push(*attendee);
            }
        }
        self.lock(ctx);
    }

    /// Configure l'aspect d'une colonne (sources AskFred : ligue en texte court).
    pub fn set_column_look(&mut self, code: &str, look: Look) {
        if let Some(i) = self.filter.entries().iter().position(|e| e.code_name == code) {
            self.filter.set_look(i, look, None);
        }
    }

    /// Affiche une colonne supplémentaire (`ShowAttribute` + `UpdateAttrList`).
    pub fn show_column(&mut self, code: &str) {
        if !self.filter.selected_codes().iter().any(|c| c == code) {
            self.filter.show_attribute(code);
            self.filter.update_attr_list(None);
            self.base.filter = self.filter.selected_codes();
        }
    }
}

impl Stage for CheckinStage {
    /// Équipes auxquelles il manque des tireurs (avertissement).
    fn error(&self, ctx: &StageContext) -> Option<crate::stage::StageError> {
        let missing = self.incomplete_teams(ctx.players, ctx.settings);
        if missing.is_empty() {
            return None;
        }
        let size = ctx.settings.team_size;
        let lines: Vec<String> = missing
            .iter()
            .map(|(name, present)| {
                let n = size - present;
                format!("{name} : il manque {n} tireur{} ({present} présent{} sur {size})", if n > 1 { "s" } else { "" }, if *present > 1 { "s" } else { "" })
            })
            .collect();
        Some(crate::stage::StageError {
            level: crate::stage::ErrorLevel::Warning,
            message: lines.join("\n"),
            guilty: Some(if missing.len() > 1 { format!("{} équipes incomplètes", missing.len()) } else { "Équipe incomplète".into() }),
        })
    }

    fn base(&self) -> &StageBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut StageBase {
        &mut self.base
    }

    /// `CheckinSupervisor::LoadConfiguration` : seul `Source` est lu.
    fn load(&mut self, _ctx: &mut StageContext, node: roxmltree::Node) -> Result<(), crate::Error> {
        self.source.load(node.attribute("Source"));
        Ok(())
    }

    /// `<Pointage Source="…"/>` (pas de participants).
    fn save(&self, _ctx: &StageContext, w: &mut XmlWriter) {
        w.start("Pointage");
        self.save_configuration(w);
        w.end();
    }

    fn save_configuration(&self, w: &mut XmlWriter) {
        if let Some(s) = self.source.serialize() {
            w.attr("Source", &s);
        }
    }

    fn set_attendees(&mut self, _ctx: &mut StageContext, _attendees: Vec<PlayerRef>) {}

    fn is_over(&self, _ctx: &StageContext) -> bool {
        true
    }

    fn classification(&self, ctx: &StageContext) -> Vec<RankEntry> {
        let rank_id = AttrId::local("rank", self.owner());
        self.result
            .iter()
            .map(|r| RankEntry {
                player: *r,
                rank: ctx.players.get(*r).and_then(|p| p.get(&rank_id)).map(Value::as_uint).unwrap_or(0),
                status: 'Q',
            })
            .collect()
    }

    fn output(&self, _ctx: &StageContext) -> Vec<PlayerRef> {
        self.output.clone()
    }

    fn lock(&mut self, ctx: &mut StageContext) {
        self.base.locked = true;
        self.update_checksum(ctx);
        self.update_ranking(ctx);
        self.set_result(ctx);
    }

    /// `OnUnLocked` : les incidents (absents/retardataires signalés en
    /// poule) sont effacés.
    fn unlock(&mut self, ctx: &mut StageContext) {
        self.result.clear();
        self.output.clear();
        self.base.quota_exceedance = 0;
        self.base.locked = false;
        let incident = AttrId::global("incident");
        for r in &self.list {
            if let Some(p) = ctx.players.get_mut(*r) {
                p.remove(&incident);
            }
        }
    }

    fn reset(&mut self, ctx: &mut StageContext) {
        let _ = ctx;
    }

    fn remap_players(&mut self, map: &HashMap<PlayerRef, PlayerRef>) {
        self.remap_lists(map);
    }

    fn dump_to_html(&self, ctx: &StageContext, html: &mut String) {
        crate::contest::html::players_table(html, ctx.players, &self.list, &self.filter, self.owner());
        // Épreuve par équipes : l'équipe de chaque tireur.
        let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let teams: Vec<&crate::player::Player> = self.list.iter().filter_map(|r| ctx.players.get(*r)).filter(|p| p.class == PlayerClass::Team && !p.members.is_empty()).collect();
        if !teams.is_empty() {
            html.push_str(&format!("        <h3>{}</h3>
        <table class=\"List\">
", esc(&crate::i18n::tr("Fencers and teams"))));
            html.push_str(&format!(
                "          <tr class=\"ListHeader\">
            <th>{}</th>
            <th>{}</th>
            <th>{}</th>
            <th>{}</th>
          </tr>
",
                esc(&crate::i18n::tr("Team")),
                esc(&crate::i18n::tr("Name")),
                esc(&crate::i18n::tr("First name")),
                esc(&crate::i18n::tr("Club"))
            ));
            let mut row = 0;
            for team in teams {
                for m in &team.members {
                    let Some(p) = ctx.players.get(*m) else { continue };
                    html.push_str(if row % 2 == 1 { "          <tr class=\"OddRow\">
" } else { "          <tr class=\"EvenRow\">
" });
                    html.push_str(&format!(
                        "            <td>{}</td>
            <td>{}</td>
            <td>{}</td>
            <td>{}</td>
          </tr>
",
                        esc(&team.name()),
                        esc(&p.name()),
                        esc(&p.first_name()),
                        esc(p.str_attr("club").unwrap_or_default())
                    ));
                    row += 1;
                }
            }
            html.push_str("        </table>
");
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
