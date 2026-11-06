//! Configuration du chronomètre et journal d'arbitrage d'un match.
use serde::{Deserialize, Serialize};
use crate::xml::XmlWriter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoutTiming {
    /// Durée d'une manche ; pour une rencontre par équipes, temps maximum d'un relais.
    pub period_seconds: u32,
    pub periods: u32,
    /// Rencontres par équipes : type de relais, taille des équipes, points par relais.
    #[serde(default)]
    pub relay: crate::relay::RelayFormat,
}
impl Default for BoutTiming {
    fn default() -> Self { Self { period_seconds: 180, periods: 1, relay: Default::default() } }
}
impl BoutTiming {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=3600).contains(&self.period_seconds) || !(1..=20).contains(&self.periods) {
            return Err("durée : 1 à 3600 secondes ; nombre de manches : 1 à 20".into());
        }
        Ok(())
    }
    pub fn load(&mut self, node: roxmltree::Node) {
        let mut relay = self.relay;
        relay.load(node);
        let value = Self {
            period_seconds: crate::xml::attr_u32(node, "DureeManche", self.period_seconds),
            periods: crate::xml::attr_u32(node, "NbManches", self.periods),
            relay,
        };
        if value.validate().is_ok() { *self = value; }
    }
    pub fn save(&self, w: &mut XmlWriter) {
        w.attr_fmt("DureeManche", self.period_seconds);
        w.attr_fmt("NbManches", self.periods);
        self.relay.save(w);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardColor { Yellow, Red, Black }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardEvent {
    pub id: String,
    /// 0 = tireur de gauche, 1 = tireur de droite, dans l'ordre de la feuille.
    pub side: u8,
    pub color: CardColor,
    pub period: u32,
    pub remaining_ms: u64,
    pub issued_at_ms: u64,
    #[serde(default)]
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Officiating {
    pub revision: u64,
    pub period: u32,
    pub remaining_ms: u64,
    /// Brouillon du téléphone, distinct du résultat validé du match.
    #[serde(default)]
    pub scores: [u32; 2],
    /// Indice du tireur vert ; 0 = premier nom de la feuille.
    #[serde(default)]
    pub green_side: u8,
    #[serde(default)]
    pub cards: Vec<CardEvent>,
}
impl Officiating {
    /// Ramène le chrono, la manche et les scores dans les bornes de la phase
    /// (réglages modifiés pendant le match, téléphone en retard…) : l'état
    /// d'un arbitre n'est pas refusé pour si peu.
    pub fn clamp(&mut self, timing: BoutTiming, max_score: u32) {
        self.period = self.period.clamp(1, timing.periods.max(1));
        self.remaining_ms = self.remaining_ms.min(timing.period_seconds as u64 * 1000);
        if max_score > 0 {
            for s in &mut self.scores {
                *s = (*s).min(max_score);
            }
        }
        self.green_side = self.green_side.min(1);
        self.revision = self.revision.max(1);
        for card in &mut self.cards {
            card.period = card.period.clamp(1, 20);
            card.remaining_ms = card.remaining_ms.min(3_600_000);
            card.side = card.side.min(1);
        }
    }

    pub fn validate(&self, timing: BoutTiming, max_score: u32) -> Result<(), String> {
        if self.green_side > 1 || self.revision == 0 || self.period == 0 || self.period > timing.periods
            || self.remaining_ms > timing.period_seconds as u64 * 1000
            || self.scores.iter().any(|s| *s > max_score) || self.cards.len() > 256 {
            return Err("état du chronomètre ou des cartons invalide".into());
        }
        let mut ids = std::collections::HashSet::new();
        for card in &self.cards {
            if card.id.is_empty() || card.id.len() > 80 || !ids.insert(&card.id)
                || card.side > 1 || card.period == 0 || card.period > 20
                || card.remaining_ms > 3_600_000 {
                return Err("carton invalide".into());
            }
        }
        Ok(())
    }
    pub fn summary(&self) -> String {
        self.cards.iter().map(|c| {
            let color = match c.color { CardColor::Yellow => "jaune", CardColor::Red => "rouge", CardColor::Black => "noir" };
            let seconds = c.remaining_ms.div_ceil(1000);
            format!("T{} : {} · M{} {:02}:{:02}{}", c.side + 1, color, c.period, seconds / 60, seconds % 60, if c.cancelled { " (annulé)" } else { "" })
        }).collect::<Vec<_>>().join("\n")
    }
}
