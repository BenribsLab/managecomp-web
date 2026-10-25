+++
title = "Lobby (series)"
slug = "lobby"
weight = 6
description = "Race or game: everybody plays at the same time, the order of arrival gives points."
+++

The **Lobby (series)** stage is for events without bouts: a race, a video game, a battle royale… Everybody in a lobby plays at the same time and arrives in an order. Each race gives points by place, and the total makes the ranking.

Add the **Lobby (series)** stage to the [formula](@/docs/formule.md): after the roll call, after pools, before a table, or several lobbies in a row (qualification, then final).

## Stage settings

Click the stage in the formula to set it up, even before it starts.

| Setting | Purpose |
|---|---|
| Lobbies in parallel | 1 lobby with everybody, or several (32 fencers → 4 lobbies of 8) |
| Races | number of races of each lobby |
| Points per place | empty: N points for the 1st, 1 for the last; or a list (25, 18, 15, 12, 10…) |
| Points for an abandon | 0 by default |
| Qualified | **in total** (the first N of the merged ranking) or **per lobby** (the first N of each lobby) |
| Separate in the lobbies by | club, league, country: two fencers of the same club go into different lobbies when possible |
| Timer | duration of a race: the organisation deduces the duration of a lobby |

Lobbies are made when the stage starts, in a serpentine from the entry ranking, like pools. As long as no arrival is entered, changing the number of lobbies or the separation redoes the distribution (online version: **Recomposer les lobbys** button).

## Entering arrivals

One table per lobby, one column per race: type the **place** of each fencer.

- **Ties**: the same place for several fencers (two 2nd). They score the same points and the next place is shifted (then a 4th).
- **Abandon**: type `A`.
- Points, wins and the place in the lobby update with each entry.

The **ranking** of the lobby and of the stage: points, then number of wins, then best place. Equal on all three, the fencers are tied. The **Ranking** button shows the ranking of the stage (merged lobbies); **Print** prints the lobby sheet (one empty column per race) or the ranking.

## Team events

Two ways, in the settings:

- **the team races as one competitor**: it gets one place per race;
- **each fencer of the team races**: the fencers are spread over the lobbies, and the team is ranked from them, by the **sum of the points of its fencers** or the **sum of its N best fencers**. The individual ranking is shown too.

## Divisions and the rest of the competition

The lobby gives a ranking, like a pool round: the qualified go to the next stage. For divisions, put a **splitting point** right after it (the first 8 in Division 1, the rest in Division 2…): each division becomes a competition that continues the formula ([Formula](@/docs/formule.md)).

## Organisation, referees and public page

- **Organisation**: each lobby is placed on a piste, with a referee, like a pool ([Organisation](@/docs/organisation.md)).
- **Referee's phone**: the lobby sheet, race by race; the referee types the places ([Referees on their phone](@/docs/arbitre.md)).
- **Public page** (compete): the formula ("32 fencers, 4 lobbies of 8, 3 races, 2 qualified per lobby"), a **Lobbies** tab with the places of each race, the points and the ranking, filters by name and club, and the "My fencer" box.
