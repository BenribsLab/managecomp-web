+++
title = "Hosting ManageComp"
slug = "hosting"
weight = 14
description = "Install the server with Docker behind Apache."
+++

The whole server (API, online version, website and documentation) is deployed with a single Docker command.

## Requirements

- Docker and Docker Compose;
- a front web server for HTTPS (Apache, for example with ISPConfig and Let's Encrypt).

## Deployment

```sh
git clone <ManageComp Web repository>
cd managecomp-web
cp .env.example .env      # then set BP_ADMIN_KEY (at least 32 characters)
docker compose up -d --build
```

Containers only listen on the machine itself (`127.0.0.1`): only Apache is exposed to the Internet.

| Container | Local port | Domain |
|---|---|---|
| client (online version, API under `/api`) | 8090 | `app.`, `api.`, `arbitre.`, `live.` |
| site (presentation and documentation) | 8091 | main domain |

## Apache directives (ISPConfig)

For each site, with Let's Encrypt enabled, in the Apache directives:

```apache
ProxyPreserveHost On
ProxyPass /.well-known/acme-challenge !
ProxyPass / http://127.0.0.1:8090/
ProxyPassReverse / http://127.0.0.1:8090/
RequestHeader set X-Forwarded-Proto "https"
```

(use 8091 instead of 8090 for the main domain). Required modules: `proxy`, `proxy_http`, `headers`.

## Accounts and administration

Set `BP_ADMIN_EMAIL` in `.env` to your address, then sign up on the application with that address: the account is approved at once and opens the **Administration** screen.

Each club then requests access with **Create an account** (e-mail, password, club). The administrator approves the request by attaching it to a new or existing club, and can disable, delete or promote an account at any time.

Once signed in, everyone can create named API keys in **My account** for the desktop software or a third-party application: they give access to the same competitions as the account.

## Club keys (without an account)

The administration key (`BP_ADMIN_KEY`) creates each club's key:

```sh
curl -X POST https://api.managecomp.org/api/admin/clients \
  -H "x-api-key: <administration key>" \
  -H "content-type: application/json" \
  -d '{"name":"My club"}'
```

The club key is shown only once: write it down.

## Open sheet QR codes (third-party application)

For a club competition, a third-party application (an association management extension, for instance) can get one QR code per pool and per running table bout. **Any phone** that scans it opens ManageComp Referee on that sheet, in full (every bout), **without a PIN**; it has no access to any other sheet.

```sh
curl https://api.managecomp.org/api/contests/<competition>/open-sheets \
  -H "x-api-key: <club key>"
```

Response: `{ "sheets": [ { "token", "kind" (pool, match, lobby), "name", "competition", "piste", "time", "over", "fencers", "url", "svg" } ] }`. `url` is the address to put in the QR code, `svg` the QR code already drawn. Parameters: `?token=<token>` for one sheet, `?svg=false` for the addresses only.

These QR codes are not printed on the sheets: keep them for club competitions, since anyone who sees one can enter the scores of the sheet.

## Updates

```sh
git pull
docker compose up -d --build
```

Data (competitions, keys) lives in the `bellepoule-data` Docker volume and is kept.
