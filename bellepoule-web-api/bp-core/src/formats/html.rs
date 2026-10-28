//! Export HTML (`Contest::DumpToHTML`, `Schedule::DumpToHTML`,
//! `PlayersList::DumpToHTML`).
//!
//! Le document comporte un menu d'onglets (un par phase) ; chaque phase
//! fournit son propre fragment HTML (listes ici, poules et tableaux dans
//! leurs modules respectifs). Comme en C++, le texte n'est pas échappé.

use crate::attribute::{self, Look, Scope};
use crate::formats::discrete::attribute_user_image;
use crate::formats::header::ContestHeader;
use crate::player::{AttrId, Owner, PlayerRef, PlayerStore};

/// En-tête `<style>` + `<script>` (contenu de `supervisor/css.h`).
pub const HEAD: &str = include_str!("html_head.txt");

/// Une phase dans le document.
#[derive(Debug, Clone)]
pub struct HtmlRound {
    /// Identifiant de la phase (`round_<id>`).
    pub id: u32,
    /// Position dans la suite de phases (y compris les phases sans onglet).
    pub index: usize,
    /// Nom de classe traduit (« Poules », « Tableau »…).
    pub class_name: String,
    /// Nom de la phase.
    pub name: String,
    /// Fragment HTML de la phase.
    pub html: String,
}

/// `Contest::DumpToHTML`. Seules les phases jusqu'à `current_stage` incluse
/// ont un contenu ; la première est affichée par défaut.
pub fn dump_contest(header: &ContestHeader, rounds: &[HtmlRound], current_stage: usize) -> String {
    let name = &header.name;
    let date = header.date_image();
    let weapon = header.weapon_image();
    let gender = header.gender_label();
    let category = header.category_image();
    let mut out = String::new();
    out.push_str(&format!(
        "<html>\n  <head>\n    <Title>{name} {date} - {weapon} - {gender} - {category}</Title>\n    <meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"/>\n{HEAD}  </head>\n\n"
    ));
    out.push_str(&format!(
        "  <body onLoad=\"javascript:OnLoad ();\">\n    <div id='main_div'>\n      <div class=\"Title\">\n        <center>\n          <h1>{name}</h1>\n          <h1>{date}</h1>\n          <h1>{weapon} - {gender} - {category}</h1>\n        </center>\n      <div>\n\n"
    ));

    // Schedule::DumpToHTML
    out.push_str("      <div id=\"menu\">\n        <ul id=\"menu_bar\">\n");
    for r in rounds {
        out.push_str(&format!(
            "          <li><a href=\"\" onclick=\"javascript:OnTabClicked ('round_{}'); return false;\">{} {}</a></li>\n",
            r.id, r.class_name, r.name
        ));
    }
    out.push_str("        </ul>\n      </div>\n\n");
    for r in rounds {
        out.push_str(&format!("      <div class=\"Round\" id=\"round_{}\"", r.id));
        if r.index == 0 {
            out.push_str(" style=\"display: inline\">\n");
        } else {
            out.push_str(">\n");
        }
        out.push_str(&format!("        <h1>{}</h1>\n", format!("{} {}", r.class_name, r.name).trim()));
        if r.index <= current_stage {
            out.push_str(&r.html);
        }
        out.push_str("      </div>\n");
    }

    out.push_str(
        "      <div class=\"Footer\">\n        <h6>Managed by ManageComp, derived from BellePoule <a href=\"https://managecomp.org\">https://managecomp.org</a><h6/>\n      <div>\n    </div>\n  </body>\n",
    );
    out.push_str("</html>\n");
    out
}

/// `Classification::IsTableBorder` : places 2, 4, 8, 16… (puissances de 2).
pub fn is_table_border(place: u32) -> bool {
    place > 1 && place.is_power_of_two()
}

/// `PlayersList::DumpToHTML` : tableau des joueurs `refs` (ordre
/// d'affichage) avec les colonnes `columns` (noms de code des attributs).
/// `data_owner` porte les attributs locaux ; `table_border` indique les
/// lignes précédées d'un séparateur (indice de ligne à partir de 0).
pub fn dump_players_list(store: &PlayerStore, refs: &[PlayerRef], columns: &[&str], data_owner: Owner, table_border: &dyn Fn(u32) -> bool) -> String {
    let mut out = String::from("        <table class=\"List\">\n");
    out.push_str("          <tr class=\"ListHeader\">\n");
    let descs: Vec<_> = columns.iter().filter_map(|c| attribute::registry().by_code(c)).collect();
    for d in &descs {
        out.push_str(&format!("            <th>{}</th>\n", crate::i18n::tr(d.user_name)));
    }
    out.push_str("          </tr>\n");

    for (i, r) in refs.iter().enumerate() {
        let Some(p) = store.get(*r) else { continue };
        if table_border(i as u32) {
            out.push_str("          <tr class=\"Separator\">\n");
        }
        out.push_str(if i % 2 == 1 { "          <tr class=\"OddRow\">\n" } else { "          <tr class=\"EvenRow\">\n" });
        for d in &descs {
            let id = if d.scope == Scope::Global { AttrId::global(d.code_name) } else { AttrId::local(d.code_name, data_owner) };
            match p.get(&id) {
                Some(v) => out.push_str(&format!("            <td>{}</td>\n", attribute_user_image(d, v, Look::LongText))),
                None => out.push_str("            <td></td>\n"),
            }
        }
        out.push_str("          </tr>\n");
    }
    out.push_str("        </table>\n");
    out
}
