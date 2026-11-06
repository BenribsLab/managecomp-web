//! Classement de saison : cumul des points de plusieurs compétitions.
//!
//! Chaque compétition classée dont les points sont activés donne, pour
//! chaque tireur : prénom, nom, catégorie de la compétition, place, points.
//! Un même tireur est reconnu d'une compétition à l'autre, sans licence :
//!
//! 1. par le **prénom seul**, s'il n'y a qu'un nom avec ce prénom ;
//! 2. sinon par le **prénom et le nom**, quelle que soit la catégorie ;
//!    sans nom, par le seul nom connu de ce prénom dans la catégorie, sinon
//!    par le **prénom et la catégorie** ;
//! 3. séparation imposée : **prénom, nom et catégorie**.
//!
//! Un nom absent (compétitions de club où seul le prénom est saisi) ne
//! distingue personne. Corrections à la main ([`Overrides`]) : regrouper deux
//! tireurs, ou séparer tous ceux d'un prénom (règle 3 imposée).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::contest::Contest;

/// Résultat d'un tireur dans une compétition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Result {
    pub first_name: String,
    pub last_name: String,
    /// Catégorie de la compétition (code : « M11 », « S »…).
    pub category: String,
    pub place: u32,
    pub points: f64,
}

/// Une compétition de la saison.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeasonContest {
    /// Identifiant (chemin du fichier, identifiant sur le serveur…).
    pub id: String,
    pub name: String,
    pub date: String,
    pub category: String,
    /// Arme et genre (libellés : « Fleuret », « Hommes »…).
    #[serde(default)]
    pub weapon: String,
    #[serde(default)]
    pub gender: String,
    pub participants: usize,
    pub results: Vec<Result>,
}

impl SeasonContest {
    /// Résultats d'une compétition (vide si ses points ne sont pas activés
    /// ou si elle n'est pas classée).
    pub fn from_contest(id: &str, c: &Contest) -> Option<SeasonContest> {
        let points = c.final_points();
        if points.is_empty() {
            return None;
        }
        let category = c.category.xml_image();
        let results: Vec<Result> = points
            .iter()
            .filter_map(|(r, place, pts)| {
                let p = c.players.get(*r)?;
                Some(Result {
                    first_name: p.str_attr("first_name").unwrap_or_default().trim().to_string(),
                    last_name: p.str_attr("name").unwrap_or_default().trim().to_string(),
                    category: category.clone(),
                    place: *place,
                    points: *pts,
                })
            })
            .collect();
        let title = if c.label.trim().is_empty() { c.name.clone() } else { format!("{} - {}", c.name, c.label) };
        Some(SeasonContest {
            id: id.into(),
            name: title,
            date: c.date_image(),
            category,
            weapon: crate::i18n::tr(c.weapon.image),
            gender: c.gender_label(),
            participants: results.len(),
            results,
        })
    }
}

/// Corrections à la main.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overrides {
    /// Regroupements : identifiant → identifiant qui le remplace.
    #[serde(default)]
    pub merges: BTreeMap<String, String>,
    /// Prénoms (forme comparable) pour lesquels la règle 3 est imposée.
    #[serde(default)]
    pub splits: BTreeSet<String>,
}

impl Overrides {
    /// Regroupe `others` avec `into`.
    pub fn merge(&mut self, into: &str, others: &[String]) {
        for o in others {
            if o != into {
                self.merges.insert(o.clone(), into.to_string());
            }
        }
    }

    /// Sépare tous les tireurs d'un prénom (et défait leurs regroupements).
    pub fn split(&mut self, first_name: &str) {
        let key = norm(first_name);
        self.merges.retain(|from, to| !from.starts_with(&format!("{key}|")) && *from != key && !to.starts_with(&format!("{key}|")) && *to != key);
        self.splits.insert(key);
    }
}

/// Un circuit : les compétitions qui comptent pour un classement.
///
/// Un circuit simple retient les compétitions de ses catégories, armes et
/// genres (vide : toutes), plus celles ajoutées à la main (`include`), moins
/// celles retirées (`exclude`). Un classement général (`circuits` non vide)
/// réunit les compétitions de plusieurs circuits.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Circuit {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub weapons: Vec<String>,
    #[serde(default)]
    pub genders: Vec<String>,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Classement général : les circuits réunis.
    #[serde(default)]
    pub circuits: Vec<String>,
    /// Seulement les N meilleurs résultats de chaque tireur (aucun : tous).
    #[serde(default)]
    pub best: Option<u32>,
}

impl Circuit {
    pub fn is_general(&self) -> bool {
        !self.circuits.is_empty()
    }

    /// La compétition répond aux filtres (sans les ajouts / retraits).
    pub fn matches_filters(&self, c: &SeasonContest) -> bool {
        let ok = |list: &[String], value: &str| list.is_empty() || list.iter().any(|v| v == value);
        ok(&self.categories, &c.category) && ok(&self.weapons, &c.weapon) && ok(&self.genders, &c.gender)
    }

    /// La compétition compte pour ce circuit simple.
    pub fn contains(&self, c: &SeasonContest) -> bool {
        !self.exclude.contains(&c.id) && (self.include.contains(&c.id) || self.matches_filters(c))
    }
}

/// Fichier de la saison (`saison.json`) : corrections et circuits.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeasonConfig {
    #[serde(flatten)]
    pub overrides: Overrides,
    #[serde(default)]
    pub circuits: Vec<Circuit>,
}

impl SeasonConfig {
    pub fn circuit(&self, id: &str) -> Option<&Circuit> {
        self.circuits.iter().find(|c| c.id == id)
    }

    /// Ajoute ou remplace un circuit (identifiant donné s'il manque).
    pub fn upsert(&mut self, mut circuit: Circuit) -> String {
        if circuit.id.is_empty() {
            let mut n = self.circuits.len() + 1;
            while self.circuit(&format!("c{n}")).is_some() {
                n += 1;
            }
            circuit.id = format!("c{n}");
        }
        let own = circuit.id.clone();
        circuit.circuits.retain(|c| *c != own);
        match self.circuits.iter_mut().find(|c| c.id == own) {
            Some(c) => *c = circuit,
            None => self.circuits.push(circuit),
        }
        own
    }

    /// Retire un circuit (et des classements généraux qui le réunissaient).
    pub fn remove(&mut self, id: &str) {
        self.circuits.retain(|c| c.id != id);
        for c in &mut self.circuits {
            c.circuits.retain(|x| x != id);
        }
    }

    /// Compétitions d'un circuit (ordre de `all`), classement général compris.
    pub fn contests(&self, all: &[SeasonContest], id: &str) -> Vec<SeasonContest> {
        let mut ids = BTreeSet::new();
        self.collect(all, id, &mut ids, &mut BTreeSet::new());
        all.iter().filter(|c| ids.contains(&c.id)).cloned().collect()
    }

    fn collect(&self, all: &[SeasonContest], id: &str, ids: &mut BTreeSet<String>, seen: &mut BTreeSet<String>) {
        if !seen.insert(id.to_string()) {
            return;
        }
        let Some(circuit) = self.circuit(id) else { return };
        if circuit.is_general() {
            for sub in &circuit.circuits {
                self.collect(all, sub, ids, seen);
            }
        } else {
            ids.extend(all.iter().filter(|c| circuit.contains(c)).map(|c| c.id.clone()));
        }
    }

    /// Classement d'un circuit (`best` : ses N meilleurs résultats).
    pub fn ranking(&self, all: &[SeasonContest], id: &str) -> (Vec<SeasonContest>, Vec<SeasonRow>) {
        let contests = self.contests(all, id);
        let best = self.circuit(id).and_then(|c| c.best).map(|n| n as usize);
        let rows = ranking_best(&contests, &self.overrides, best);
        (contests, rows)
    }
}

/// Valeurs distinctes (catégories, armes, genres) des compétitions.
pub fn distinct(contests: &[SeasonContest], f: impl Fn(&SeasonContest) -> &str) -> Vec<String> {
    let set: BTreeSet<String> = contests.iter().map(|c| f(c).to_string()).filter(|v| !v.is_empty()).collect();
    set.into_iter().collect()
}

/// Une ligne du classement de saison.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeasonRow {
    pub rank: u32,
    /// Identifiant du tireur (pour les corrections).
    pub key: String,
    pub first_name: String,
    pub last_name: String,
    pub category: String,
    pub total: f64,
    /// Points par compétition (même ordre que les compétitions).
    pub points: Vec<Option<f64>>,
    pub competitions: usize,
}

/// Forme comparable : minuscules, sans accents ni espaces superflus.
pub fn norm(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            '-' | '\'' | '’' => ' ',
            c => c,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Identifiant de chaque résultat, selon les règles de reconnaissance.
fn identify(contests: &[SeasonContest], overrides: &Overrides) -> Vec<Vec<String>> {
    // Résultats par prénom : (compétition, résultat).
    let mut by_first: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    for (ci, c) in contests.iter().enumerate() {
        for (ri, r) in c.results.iter().enumerate() {
            by_first.entry(norm(&r.first_name)).or_default().push((ci, ri));
        }
    }
    let mut keys: Vec<Vec<String>> = contests.iter().map(|c| vec![String::new(); c.results.len()]).collect();
    for (first, list) in by_first {
        let result = |&(ci, ri): &(usize, usize)| &contests[ci].results[ri];
        let full = |r: &Result| format!("{first}|{}|{}", norm(&r.category), norm(&r.last_name));
        if overrides.splits.contains(&first) {
            for at in &list {
                keys[at.0][at.1] = full(result(at));
            }
            continue;
        }
        // Personnes distinctes : les noms ; un nom absent ne distingue pas, et
        // la catégorie non plus (un tireur change de catégorie d'une
        // compétition à l'autre).
        let names: BTreeSet<String> = list.iter().map(|at| norm(&result(at).last_name)).filter(|n| !n.is_empty()).collect();
        if names.len() <= 1 {
            for at in &list {
                keys[at.0][at.1] = first.clone();
            }
            continue;
        }
        // Plusieurs noms : prénom et nom. Sans nom, le seul nom connu de ce
        // prénom dans la catégorie, sinon prénom et catégorie.
        for at in &list {
            let r = result(at);
            let last = norm(&r.last_name);
            keys[at.0][at.1] = if !last.is_empty() {
                format!("{first}||{last}")
            } else {
                let category = norm(&r.category);
                let same: BTreeSet<String> = list
                    .iter()
                    .map(|x| result(x))
                    .filter(|x| norm(&x.category) == category && !x.last_name.trim().is_empty())
                    .map(|x| norm(&x.last_name))
                    .collect();
                match same.len() {
                    1 => format!("{first}||{}", same.into_iter().next().unwrap_or_default()),
                    _ => format!("{first}|{category}"),
                }
            };
        }
    }
    // Regroupements à la main (en chaîne).
    for list in keys.iter_mut() {
        for key in list.iter_mut() {
            let mut guard = 0;
            while let Some(to) = overrides.merges.get(key.as_str()) {
                *key = to.clone();
                guard += 1;
                if guard > 32 {
                    break;
                }
            }
        }
    }
    keys
}

/// Tireurs d'une compétition à répartir (points nuls), pour les reconnaître
/// dans le classement club.
fn entries(id: &str, c: &Contest, fencers: &[crate::player::PlayerRef]) -> SeasonContest {
    let category = c.category.xml_image();
    let results = fencers
        .iter()
        .filter_map(|r| c.players.get(*r))
        .map(|p| Result {
            first_name: p.str_attr("first_name").unwrap_or_default().trim().to_string(),
            last_name: p.str_attr("name").unwrap_or_default().trim().to_string(),
            category: category.clone(),
            place: 0,
            points: 0.0,
        })
        .collect::<Vec<_>>();
    SeasonContest {
        id: id.into(),
        name: c.name.clone(),
        date: c.date_image(),
        category,
        weapon: crate::i18n::tr(c.weapon.image),
        gender: c.gender_label(),
        participants: results.len(),
        results,
    }
}

/// Rang de chaque tireur de `target` dans le classement club : le premier
/// circuit simple qui retient la compétition, sinon les compétitions de sa
/// catégorie (toutes s'il n'y en a pas). `None` : absent du classement.
pub fn seeding_ranks(all: &[SeasonContest], config: &SeasonConfig, target: &SeasonContest) -> Vec<Option<u32>> {
    let n = target.results.len();
    let others: Vec<SeasonContest> = all.iter().filter(|c| c.id != target.id).cloned().collect();
    let circuit = config.circuits.iter().find(|c| !c.is_general() && c.contains(target));
    let (mut contests, best) = match circuit {
        Some(c) => (config.contests(&others, &c.id), c.best.map(|n| n as usize)),
        None => {
            let same: Vec<SeasonContest> = others.iter().filter(|c| c.category == target.category).cloned().collect();
            (if same.is_empty() { others } else { same }, None)
        }
    };
    if contests.is_empty() {
        return vec![None; n];
    }
    contests.push(target.clone());
    let keys = identify(&contests, &config.overrides);
    let rows = ranking_best(&contests, &config.overrides, best);
    // Seulement les tireurs déjà classés (vus dans une autre compétition).
    let ranks: BTreeMap<&str, u32> = rows.iter().filter(|r| r.competitions > 1).map(|r| (r.key.as_str(), r.rank)).collect();
    keys.last().map(|k| k.iter().map(|key| ranks.get(key.as_str()).copied()).collect()).unwrap_or_else(|| vec![None; n])
}

/// Compétition de club : quand aucun tireur n'a de classement, le classement
/// club (saison) donne leur classement pour la répartition (serpentin, groupes
/// de niveau). Les tireurs absents du classement club viennent après.
/// Renvoie le nombre de tireurs classés.
pub fn apply_club_seeding(id: &str, c: &mut Contest, all: &[SeasonContest], config: &SeasonConfig) -> usize {
    use crate::player::{AttrId, PlayerClass};
    if c.is_team_event() {
        return 0;
    }
    let ranking = AttrId::global("ranking");
    let fencers: Vec<crate::player::PlayerRef> = c.players.refs().into_iter().filter(|r| c.players.get(*r).is_some_and(|p| p.is(PlayerClass::Fencer))).collect();
    if fencers.iter().any(|r| c.players.get(*r).is_some_and(|p| p.get(&ranking).is_some())) {
        return 0;
    }
    let target = entries(id, c, &fencers);
    let ranks = seeding_ranks(all, config, &target);
    let mut count = 0;
    for (r, rank) in fencers.iter().zip(ranks) {
        if let (Some(rank), Some(p)) = (rank, c.players.get_mut(*r)) {
            p.set_uint(&ranking, rank);
            count += 1;
        }
    }
    count
}

/// Classement de saison : total des points, du plus grand au plus petit.
/// `category` : seulement les compétitions de cette catégorie (code).
pub fn ranking(contests: &[SeasonContest], overrides: &Overrides) -> Vec<SeasonRow> {
    ranking_best(contests, overrides, None)
}

/// Classement où seuls les `best` meilleurs résultats de chaque tireur
/// comptent dans le total (tous si `None`).
pub fn ranking_best(contests: &[SeasonContest], overrides: &Overrides, best: Option<usize>) -> Vec<SeasonRow> {
    let keys = identify(contests, overrides);
    let mut rows: BTreeMap<String, SeasonRow> = BTreeMap::new();
    for (ci, c) in contests.iter().enumerate() {
        for (ri, r) in c.results.iter().enumerate() {
            let key = &keys[ci][ri];
            let row = rows.entry(key.clone()).or_insert_with(|| SeasonRow {
                rank: 0,
                key: key.clone(),
                first_name: r.first_name.clone(),
                last_name: String::new(),
                category: r.category.clone(),
                total: 0.0,
                points: vec![None; contests.len()],
                competitions: 0,
            });
            if row.last_name.is_empty() {
                row.last_name = r.last_name.clone();
            }
            row.points[ci] = Some(row.points[ci].unwrap_or(0.0) + r.points);
            row.total += r.points;
            row.competitions += 1;
        }
    }
    let mut rows: Vec<SeasonRow> = rows.into_values().collect();
    if let Some(n) = best {
        for row in rows.iter_mut() {
            let mut pts: Vec<f64> = row.points.iter().flatten().copied().collect();
            pts.sort_by(|a, b| b.total_cmp(a));
            row.total = pts.into_iter().take(n).sum();
        }
    }
    rows.sort_by(|a, b| b.total.total_cmp(&a.total).then_with(|| norm(&a.first_name).cmp(&norm(&b.first_name))));
    // Ex æquo : même rang.
    let mut previous: Option<(f64, u32)> = None;
    for (i, row) in rows.iter_mut().enumerate() {
        row.rank = match previous {
            Some((total, rank)) if (total - row.total).abs() < 1e-9 => rank,
            _ => i as u32 + 1,
        };
        previous = Some((row.total, row.rank));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(first: &str, last: &str, category: &str, points: f64) -> Result {
        Result { first_name: first.into(), last_name: last.into(), category: category.into(), place: 1, points }
    }

    fn contest(id: &str, results: Vec<Result>) -> SeasonContest {
        let category = results.first().map(|r| r.category.clone()).unwrap_or_default();
        SeasonContest { id: id.into(), name: id.into(), date: String::new(), category, weapon: "Fleuret".into(), gender: String::new(), participants: results.len(), results }
    }

    #[test]
    fn circuits_and_general_ranking() {
        let season = vec![
            contest("A", vec![r("Lina", "", "M11", 10.0)]),
            contest("B", vec![r("Lina", "", "M11", 4.0)]),
            contest("C", vec![r("Hugo", "", "M13", 8.0), r("Lina", "", "M13", 1.0)]),
        ];
        let mut cfg = SeasonConfig::default();
        let m11 = cfg.upsert(Circuit { name: "M11".into(), categories: vec!["M11".into()], best: Some(1), ..Default::default() });
        let m13 = cfg.upsert(Circuit { name: "M13".into(), categories: vec!["M13".into()], ..Default::default() });
        let all = cfg.upsert(Circuit { name: "Général".into(), circuits: vec![m11.clone(), m13.clone()], ..Default::default() });
        let (contests, rows) = cfg.ranking(&season, &m11);
        assert_eq!(contests.len(), 2);
        assert_eq!(rows[0].total, 10.0, "meilleur résultat seul");
        let (contests, rows) = cfg.ranking(&season, &all);
        assert_eq!(contests.len(), 3);
        assert_eq!(rows.iter().find(|r| r.first_name == "Lina").unwrap().total, 15.0);
        // Ajout / retrait à la main.
        let mut c = cfg.circuit(&m13).unwrap().clone();
        c.include.push("A".into());
        c.exclude.push("C".into());
        cfg.upsert(c);
        assert_eq!(cfg.contests(&season, &m13).iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["A"]);
        cfg.remove(&m13);
        assert_eq!(cfg.circuit(&all).unwrap().circuits, vec![m11]);
        // Ancien fichier (corrections seules) toujours lisible.
        let old: SeasonConfig = serde_json::from_str(r#"{"merges":{"a":"b"},"splits":[]}"#).unwrap();
        assert_eq!(old.overrides.merges.len(), 1);
    }

    #[test]
    fn first_name_alone_then_category_then_full_name() {
        let season = vec![
            contest("A", vec![r("Lina", "", "M11", 10.0), r("Hugo", "MARTIN", "M11", 8.0), r("Hugo", "", "M13", 5.0)]),
            contest("B", vec![r("Lina", "DUBOIS", "M11", 7.0), r("Hugo", "MARTIN", "M11", 6.0), r("Hugo", "ROUX", "M11", 4.0)]),
        ];
        let rows = ranking(&season, &Overrides::default());
        let get = |key: &str| rows.iter().find(|r| r.key == key).cloned();
        // Une seule Lina : prénom seul ; son nom vient du résultat qui le donne.
        let lina = get("lina").unwrap();
        assert_eq!((lina.total, lina.competitions, lina.last_name.as_str()), (17.0, 2, "DUBOIS"));
        // Deux Hugo (MARTIN, ROUX) : prénom + nom ; le Hugo M13 sans nom, seul de sa catégorie.
        assert_eq!(get("hugo||martin").unwrap().total, 14.0);
        assert_eq!(get("hugo||roux").unwrap().total, 4.0);
        assert_eq!(get("hugo|m13").unwrap().total, 5.0);
        assert_eq!(rows[0].key, "lina");
        assert_eq!(rows[0].rank, 1);
    }

    #[test]
    fn club_ranking_seeds_a_new_competition() {
        let season = vec![
            contest("A", vec![r("Lina", "", "M11", 10.0), r("Hugo", "", "M11", 6.0)]),
            contest("B", vec![r("Hugo", "", "M11", 9.0), r("Zoé", "", "M11", 1.0)]),
        ];
        let target = contest("N", vec![r("Zoé", "", "M11", 0.0), r("Hugo", "", "M11", 0.0), r("Lina", "", "M11", 0.0), r("Paul", "", "M11", 0.0)]);
        let ranks = seeding_ranks(&season, &SeasonConfig::default(), &target);
        assert_eq!(ranks, vec![Some(3), Some(1), Some(2), None]);
    }

    #[test]
    fn same_fencer_in_several_categories_is_one_row() {
        let season = vec![
            contest("A", vec![r("Emma", "MARTIN", "M11", 10.0), r("Emma", "PETIT", "M11", 3.0)]),
            contest("B", vec![r("Emma", "Martin", "M13", 6.0), r("Emma", "", "M13", 2.0)]),
            contest("C", vec![r("Emma", "PETIT", "", 1.0)]),
        ];
        let rows = ranking(&season, &Overrides::default());
        assert_eq!(rows.len(), 2, "{rows:?}");
        // MARTIN : M11 + M13 + l'Emma sans nom en M13 (seule Emma nommée de M13).
        assert_eq!(rows.iter().find(|r| r.key == "emma||martin").unwrap().total, 18.0);
        assert_eq!(rows.iter().find(|r| r.key == "emma||petit").unwrap().total, 4.0);
    }

    #[test]
    fn manual_merge_and_split() {
        let season = vec![
            contest("A", vec![r("Léa", "", "M9", 10.0), r("Lea", "PETIT", "M11", 3.0)]),
            contest("B", vec![r("Léa", "MOREAU", "M9", 9.0)]),
        ];
        let rows = ranking(&season, &Overrides::default());
        // Deux personnes (MOREAU, PETIT) : la Léa sans nom en M9 rejoint MOREAU.
        assert_eq!(rows.iter().find(|r| r.key == "lea||moreau").unwrap().total, 19.0);
        let mut o = Overrides::default();
        o.split("Léa");
        let rows = ranking(&season, &o);
        assert_eq!(rows.len(), 3, "séparées : chacune son identité complète");
        o.merge("lea|m9|moreau", &["lea|m9|".to_string()]);
        let rows = ranking(&season, &o);
        assert_eq!(rows.iter().find(|r| r.key == "lea|m9|moreau").unwrap().total, 19.0);
    }
}
