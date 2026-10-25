+++
title = "Lobby (série)"
weight = 6
description = "Course ou jeu : tout le monde joue en même temps, l'ordre d'arrivée donne des points."
+++

Le **Lobby (série)** sert aux épreuves sans matchs : une course, un jeu vidéo, une battle royale… Tous les participants d'un lobby jouent en même temps et arrivent dans un ordre. Chaque manche rapporte des points selon la place, et le total donne le classement.

Ajoutez la phase **Lobby (série)** à la [formule](@/docs/formule.md) : après l'appel, après des poules, avant un tableau, ou plusieurs lobbys à la suite (qualification, puis finale).

## Réglages de la phase

Cliquez sur la phase dans la formule pour la régler, même avant qu'elle commence.

| Réglage | À quoi il sert |
|---|---|
| Manches | nombre de courses de chaque lobby |
| Points par place | vide : N points au 1er, 1 au dernier ; ou une liste (25, 18, 15, 12, 10…) |
| Points d'un abandon | 0 par défaut |
| Qualifiés | **au total** (les N premiers du classement fusionné) ou **par lobby** (les N premiers de chaque lobby) |
| Séparer dans les lobbys par | club, ligue, pays : deux tireurs du même club vont dans des lobbys différents quand c'est possible |
| Chronomètre | durée d'une manche : l'organisation en déduit la durée d'un lobby |

## Composition des lobbys

Comme pour les poules, la composition se fait sur l'écran de la phase, d'après le nombre de participants :

- **Formule** : les formules possibles (« 1 lobby de 32 », « 4 lobbys de 8 », « 5 lobbys de 6 ou 7 »…) ;
- **Répartition** : **équilibrée (serpentin)** d'après le classement d'entrée, avec le décalage par club, ou **par force équivalente** (les meilleurs ensemble dans le lobby 1, les suivants dans le lobby 2…).

La composition reste modifiable tant qu'aucune arrivée n'est saisie.

## Saisie des arrivées

Un tableau par lobby, une colonne par manche : tapez la **place** de chaque tireur.

- **Ex aequo** : la même place pour plusieurs tireurs (deux 2ᵉ). Ils marquent les mêmes points et la place suivante est décalée (puis un 4ᵉ).
- **Abandon** : tapez `A`.
- Les points, les victoires et la place dans le lobby se mettent à jour à chaque saisie.

Le **classement** du lobby et de la phase : points, puis nombre de victoires, puis meilleure place. À égalité sur les trois, les tireurs sont ex aequo. Le bouton **Classement** affiche le classement de la phase (lobbys fusionnés) ; **Imprimer** imprime la feuille du lobby (une colonne vide par manche) ou le classement.

## Épreuve par équipes

Deux façons de faire, dans les réglages :

- **l'équipe court comme un seul participant** : elle a une place à chaque manche ;
- **chaque tireur de l'équipe court** : les tireurs sont répartis dans les lobbys, et l'équipe est classée d'après eux, par la **somme des points de ses tireurs** ou la **somme de ses N meilleurs tireurs**. Le classement individuel des tireurs s'affiche aussi.

## Divisions et suite de la compétition

Le lobby donne un classement, comme un tour de poules : les qualifiés passent à la phase suivante. Pour des divisions, placez un **point de scission** juste après (les 8 premiers en Division 1, le reste en Division 2…) : chaque division devient une compétition qui reprend la suite de la formule ([Formule](@/docs/formule.md)).

## Organisation, arbitres et page publique

- **Organisation** : chaque lobby se place sur une piste, avec un arbitre, comme une poule ([Organisation](@/docs/organisation.md)).
- **Téléphone de l'arbitre** : la feuille du lobby, manche par manche ; l'arbitre tape les places ([Arbitres sur téléphone](@/docs/arbitre.md)).
- **Page publique** (compete) : la formule (« 32 tireurs, 4 lobbys de 8, 3 manches, 2 qualifiés par lobby »), un onglet **Lobbys** avec les places de chaque manche, les points et le classement, les filtres par nom et par club et l'encadré « Mon tireur ».
