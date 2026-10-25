+++
title = "The formula"
slug = "formula"
weight = 4
description = "Build and reorder the phases of the competition."
+++

The formula is the sequence of phases: check-in, pool rounds, tables, split points, general ranking. It opens with the competition's **preferences**.

## Add and remove phases

- **Add** a phase after the selected one: pool round, table, split point, youth training.
- **Remove** a phase that has not been fenced yet.

## Change the order

{{ figure(src="proprietes.png", caption="The formula in the competition preferences: Move up, Move down, Add, Remove.") }}

The **Move up** and **Move down** buttons move the selected phase. A pool round moves with its composition. Only phases not yet fenced can move; check-in stays first and the general ranking last.

## Phase preferences

Select a phase to set its parameters: name, number of qualified fencers, maximum score, and:

- **individual event**: period length and number of periods (referees' timer);
- **team event**: match type, points per relay and maximum relay time ([Team events](@/docs/equipes.md)).

## Split point: divisions

A split point shares the fencers into **divisions**. In its preferences, name each division (as many as needed) and choose how to fill them:

- **by hand**: you choose each fencer's division at the split point;
- **by ranking**: the first fencers stay in the competition (a number of places or a percentage), each division takes the following ones, the last one takes the rest;
- **by criteria**: categories (e.g. M13, M15), birth years (one or more), "born in … or before", "born in … or after". A fencer goes to the first matching division, otherwise stays.

At the split point the distribution is pre-filled; it can still be changed fencer by fencer (**Division** column, or "Move the selection to"). When moving to the next phase, **each division becomes a new competition**, named after it; the other fencers carry on in the current competition.

## Division pools (promotion and relegation)

Add the **Division pools** phase: it shares the fencers into divisions (D1 at the top, D2…) and is followed by its pool round, fenced division by division (with the club offset). Settings: number of divisions, pools per division, promotions and relegations after each round, first distribution (by ranking or at random).

On the phase screen each fencer has a division (D1, D2… menu, or "Move the selection to"). For a new round, add another **Division pools** phase: the first X of each division go up, the last X go down (↑ ↓ arrows), and everything can still be changed by hand. In the general ranking, D1 comes first, then D2…

## Lobby (series)

For a race or a game (everybody plays at the same time, the order of arrival gives points), add the **Lobby (series)** stage: several lobbies in parallel, several races, adjustable points scale, qualified in total or per lobby ([Lobby (series)](@/docs/lobby.md)).

## Points and season ranking

In the competition preferences, the **Points** tab awards points from the general ranking place: by default, out of N participants the 1st gets N points and the last 1 point. You can add place bonuses (1st: +X), bonuses for finishing in the top 4, 8…, a weight for the competition, and choose the formula: "Classement simple" (N − R + 1 + B) × C, the official M15 one, the official one, or your own, saved under a name. Variables, as in the federation rules: N (participants), R (rank), C (coefficient, the weight) and B (bonus); functions LOG, LN, ARRONDI.SUP… Points are shown in the general ranking.

The **Season ranking** (left menu, or home screen button in the desktop software) adds up the points of the competitions of a folder (or of the club), by category. A fencer is recognised by first name alone if unique, otherwise by first name and category, otherwise by first name, last name and category; mistakes are fixed by hand (group two lines, separate a first name).
