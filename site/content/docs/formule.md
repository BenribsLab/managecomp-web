+++
title = "La formule"
weight = 4
description = "Composer et réorganiser les phases de la compétition."
+++

La formule est la suite des phases : appel, tours de poules, tableaux, points de scission, classement général. Elle s'ouvre avec les **préférences** de la compétition.

## Ajouter et retirer des phases

- **Ajouter** une phase après la phase sélectionnée : tour de poules, tableau, point de scission, entraînement des jeunes.
- **Retirer** une phase qui n'a pas encore été jouée.

## Changer l'ordre

{{ figure(src="proprietes.png", caption="La formule dans les préférences de la compétition : Monter, Descendre, Ajouter, Retirer.") }}

Les boutons **Monter** et **Descendre** déplacent la phase sélectionnée. Un tour de poules se déplace avec sa composition. Seules les phases qui n'ont pas encore été jouées bougent ; l'appel reste en premier et le classement général en dernier.

## Préférences d'une phase

Sélectionnez une phase pour régler ses paramètres : nom, nombre de qualifiés, score maximum, et :

- **épreuve individuelle** : durée d'une manche et nombre de manches (chronomètre des arbitres) ;
- **épreuve par équipes** : type de rencontre, points par relais et temps maximum d'un relais ([Rencontres par équipes](@/docs/equipes.md)).

## Point de scission : les divisions

Un point de scission partage les tireurs en **divisions**. Dans ses préférences, donnez un nom à chaque division (autant que nécessaire) et choisissez comment les remplir :

- **à la main** : vous choisissez la division de chaque tireur au moment de la scission ;
- **au classement** : les premiers restent dans la compétition (un nombre de places ou un pourcentage), chaque division prend les suivants, la dernière prend le reste ;
- **par critères** : catégories (ex. M13, M15), années de naissance (une ou plusieurs), « nés en … ou avant », « nés en … ou après ». Un tireur va dans la première division qui lui convient, sinon il reste.

En arrivant au point de scission, la répartition est préremplie ; elle reste modifiable tireur par tireur (colonne **Division**, ou « Placer la sélection dans »). Au passage à la phase suivante, **chaque division devient une nouvelle compétition**, à son nom ; les autres tireurs continuent dans la compétition en cours.

## Poules de division (montées et descentes)

Ajoutez la phase **Poules de division** : elle répartit les tireurs en divisions (D1 en haut, D2…) et elle est suivie de son tour de poules, joué division par division (avec le décalage par club). Réglages : nombre de divisions, poules par division, montées et descentes après chaque tour, répartition de départ (au classement ou au hasard).

Sur l'écran de la phase, chaque tireur a sa division (menu D1, D2…, ou « Placer la sélection dans »). Pour un nouveau tour, ajoutez une autre phase **Poules de division** : les X premiers de chaque division y montent, les X derniers descendent (flèches ↑ ↓), et tout reste modifiable à la main. Au classement général, la D1 vient d'abord, puis la D2…

## Lobby (série)

Pour une course ou un jeu (tout le monde joue en même temps, l'ordre d'arrivée donne des points), ajoutez la phase **Lobby (série)** : plusieurs lobbys en parallèle, plusieurs manches, barème de points réglable, qualifiés au total ou par lobby ([Lobby (série)](@/docs/lobby.md)).

## Points et classement de saison

Dans les préférences de la compétition, l'onglet **Points** attribue des points selon la place au classement général : par défaut, sur N participants, le 1er gagne N points et le dernier 1 point. On peut ajouter des bonus par place (le 1er : +X), des bonus pour finir dans les 4, dans les 8…, un poids de la compétition, et choisir la formule : « Classement simple » (N − R + 1 + B) × C, « Classement officiel M15 », « Classement officiel », ou la vôtre, à enregistrer sous un nom. Variables, comme dans le règlement de la fédération : N (participants), R (rang), C (coefficient, le poids) et B (bonus) ; fonctions LOG, LN, ARRONDI.SUP… Les points apparaissent dans le classement général.

Le **Classement de saison** (menu de gauche, ou bouton de l'accueil sur le logiciel de bureau) additionne les points des compétitions d'un dossier (ou du club), par catégorie. Un tireur est reconnu par son prénom seul s'il est unique, sinon par son prénom et la catégorie, sinon par son prénom, son nom et la catégorie ; les erreurs se corrigent à la main (regrouper deux lignes, séparer un prénom).
