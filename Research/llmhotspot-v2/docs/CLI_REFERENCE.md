# Command-Line Help for `llmhotspot`

This document contains the help content for the `llmhotspot` command-line program.

**Command Overview:**

* [`llmhotspot`↴](#llmhotspot)
* [`llmhotspot serve`↴](#llmhotspot-serve)
* [`llmhotspot status`↴](#llmhotspot-status)
* [`llmhotspot oauth`↴](#llmhotspot-oauth)
* [`llmhotspot oauth start`↴](#llmhotspot-oauth-start)
* [`llmhotspot oauth complete`↴](#llmhotspot-oauth-complete)
* [`llmhotspot oauth connect`↴](#llmhotspot-oauth-connect)
* [`llmhotspot oauth list`↴](#llmhotspot-oauth-list)
* [`llmhotspot oauth activate`↴](#llmhotspot-oauth-activate)
* [`llmhotspot oauth priority`↴](#llmhotspot-oauth-priority)
* [`llmhotspot oauth priority set`↴](#llmhotspot-oauth-priority-set)
* [`llmhotspot oauth disable`↴](#llmhotspot-oauth-disable)
* [`llmhotspot oauth enable`↴](#llmhotspot-oauth-enable)
* [`llmhotspot oauth disconnect`↴](#llmhotspot-oauth-disconnect)
* [`llmhotspot keys`↴](#llmhotspot-keys)
* [`llmhotspot keys create`↴](#llmhotspot-keys-create)
* [`llmhotspot keys list`↴](#llmhotspot-keys-list)
* [`llmhotspot keys revoke`↴](#llmhotspot-keys-revoke)
* [`llmhotspot keys expire`↴](#llmhotspot-keys-expire)
* [`llmhotspot keys copy`↴](#llmhotspot-keys-copy)
* [`llmhotspot endpoint`↴](#llmhotspot-endpoint)
* [`llmhotspot endpoint local`↴](#llmhotspot-endpoint-local)
* [`llmhotspot endpoint remote`↴](#llmhotspot-endpoint-remote)
* [`llmhotspot endpoint test`↴](#llmhotspot-endpoint-test)
* [`llmhotspot endpoint test local`↴](#llmhotspot-endpoint-test-local)
* [`llmhotspot endpoint test remote`↴](#llmhotspot-endpoint-test-remote)
* [`llmhotspot app`↴](#llmhotspot-app)
* [`llmhotspot app open-console`↴](#llmhotspot-app-open-console)
* [`llmhotspot app restart`↴](#llmhotspot-app-restart)
* [`llmhotspot app stop`↴](#llmhotspot-app-stop)
* [`llmhotspot session`↴](#llmhotspot-session)
* [`llmhotspot session start`↴](#llmhotspot-session-start)
* [`llmhotspot session stop`↴](#llmhotspot-session-stop)
* [`llmhotspot session status`↴](#llmhotspot-session-status)
* [`llmhotspot generate-cli-docs`↴](#llmhotspot-generate-cli-docs)

## `llmhotspot`

LLM Hotspot v2 control CLI

**Usage:** `llmhotspot [OPTIONS] <COMMAND>`

###### **Subcommands:**

* `serve` — Start daemon + API + UI (+ tray unless disabled)
* `status` — Show app and endpoint status
* `oauth` — OAuth account commands
* `keys` — API key lifecycle commands
* `endpoint` — Endpoint URL and test commands
* `app` — Application control commands
* `session` — Session controls
* `generate-cli-docs` — Generate CLI docs at docs/CLI_REFERENCE.md

###### **Options:**

* `--json` — Emit machine-readable JSON
* `--db-path <DB_PATH>` — SQLite database path (used by app-local commands; broker uses Postgres by default)

  Default value: `./llmhotspot.db`



## `llmhotspot serve`

Start daemon + API + UI (+ tray unless disabled)

**Usage:** `llmhotspot serve [OPTIONS]`

Examples:
  llmhotspot serve
  llmhotspot serve --listen 0.0.0.0:11434 --no-tray

###### **Options:**

* `--listen <LISTEN>`

  Default value: `127.0.0.1:11434`
* `--no-tray`



## `llmhotspot status`

Show app and endpoint status

**Usage:** `llmhotspot status`

Examples:
  llmhotspot status
  llmhotspot --json status



## `llmhotspot oauth`

OAuth account commands

**Usage:** `llmhotspot oauth <COMMAND>`

###### **Subcommands:**

* `start` — Start OAuth flow URL generation
* `complete` — Complete OAuth exchange using callback state + code
* `connect` — Connect/register an OAuth account
* `list` — List connected OAuth accounts
* `activate` — Activate account by id
* `priority` — Priority operations
* `disable` — Disable account
* `enable` — Enable account
* `disconnect` — Disconnect account



## `llmhotspot oauth start`

Start OAuth flow URL generation

**Usage:** `llmhotspot oauth start`

Examples:
  llmhotspot oauth start



## `llmhotspot oauth complete`

Complete OAuth exchange using callback state + code

**Usage:** `llmhotspot oauth complete [OPTIONS] --state <STATE> --code <CODE>`

Examples:
  llmhotspot oauth complete --state abc --code xyz

###### **Options:**

* `--state <STATE>`
* `--code <CODE>`
* `--redirect-uri <REDIRECT_URI>`



## `llmhotspot oauth connect`

Connect/register an OAuth account

**Usage:** `llmhotspot oauth connect --account <ACCOUNT> --email <EMAIL> --name <NAME>`

Examples:
  llmhotspot oauth connect --account acct_1 --email user@example.com --name Primary

###### **Options:**

* `--account <ACCOUNT>`
* `--email <EMAIL>`
* `--name <NAME>`



## `llmhotspot oauth list`

List connected OAuth accounts

**Usage:** `llmhotspot oauth list`

Examples:
  llmhotspot oauth list



## `llmhotspot oauth activate`

Activate account by id

**Usage:** `llmhotspot oauth activate --account <ACCOUNT>`

Examples:
  llmhotspot oauth activate --account acct_1

###### **Options:**

* `--account <ACCOUNT>`



## `llmhotspot oauth priority`

Priority operations

**Usage:** `llmhotspot oauth priority <COMMAND>`

###### **Subcommands:**

* `set` — Set account priority index



## `llmhotspot oauth priority set`

Set account priority index

**Usage:** `llmhotspot oauth priority set --account <ACCOUNT> --index <INDEX>`

Examples:
  llmhotspot oauth priority set --account acct_1 --index 0

###### **Options:**

* `--account <ACCOUNT>`
* `--index <INDEX>`



## `llmhotspot oauth disable`

Disable account

**Usage:** `llmhotspot oauth disable --account <ACCOUNT>`

Examples:
  llmhotspot oauth disable --account acct_1

###### **Options:**

* `--account <ACCOUNT>`



## `llmhotspot oauth enable`

Enable account

**Usage:** `llmhotspot oauth enable --account <ACCOUNT>`

Examples:
  llmhotspot oauth enable --account acct_1

###### **Options:**

* `--account <ACCOUNT>`



## `llmhotspot oauth disconnect`

Disconnect account

**Usage:** `llmhotspot oauth disconnect --account <ACCOUNT>`

Examples:
  llmhotspot oauth disconnect --account acct_1

###### **Options:**

* `--account <ACCOUNT>`



## `llmhotspot keys`

API key lifecycle commands

**Usage:** `llmhotspot keys <COMMAND>`

###### **Subcommands:**

* `create` — Create API key
* `list` — List API keys
* `revoke` — Revoke API key
* `expire` — Expire API key immediately
* `copy` — Copy/show raw API key



## `llmhotspot keys create`

Create API key

**Usage:** `llmhotspot keys create [OPTIONS] --label <LABEL>`

Examples:
  llmhotspot keys create --label ci-key
  llmhotspot keys create --label temp --expires 2026-02-13T20:00:00Z

###### **Options:**

* `--label <LABEL>`
* `--expires <EXPIRES>`



## `llmhotspot keys list`

List API keys

**Usage:** `llmhotspot keys list`

Examples:
  llmhotspot keys list



## `llmhotspot keys revoke`

Revoke API key

**Usage:** `llmhotspot keys revoke --key-id <KEY_ID>`

Examples:
  llmhotspot keys revoke --key-id <id>

###### **Options:**

* `--key-id <KEY_ID>`



## `llmhotspot keys expire`

Expire API key immediately

**Usage:** `llmhotspot keys expire --key-id <KEY_ID>`

Examples:
  llmhotspot keys expire --key-id <id>

###### **Options:**

* `--key-id <KEY_ID>`



## `llmhotspot keys copy`

Copy/show raw API key

**Usage:** `llmhotspot keys copy --key-id <KEY_ID>`

Examples:
  llmhotspot keys copy --key-id <id>

###### **Options:**

* `--key-id <KEY_ID>`



## `llmhotspot endpoint`

Endpoint URL and test commands

**Usage:** `llmhotspot endpoint <COMMAND>`

###### **Subcommands:**

* `local` — Show local endpoint URL
* `remote` — Show remote endpoint URL
* `test` — Run endpoint tests



## `llmhotspot endpoint local`

Show local endpoint URL

**Usage:** `llmhotspot endpoint local`



## `llmhotspot endpoint remote`

Show remote endpoint URL

**Usage:** `llmhotspot endpoint remote`



## `llmhotspot endpoint test`

Run endpoint tests

**Usage:** `llmhotspot endpoint test <COMMAND>`

###### **Subcommands:**

* `local` — Test local endpoint
* `remote` — Test remote endpoint



## `llmhotspot endpoint test local`

Test local endpoint

**Usage:** `llmhotspot endpoint test local`



## `llmhotspot endpoint test remote`

Test remote endpoint

**Usage:** `llmhotspot endpoint test remote`



## `llmhotspot app`

Application control commands

**Usage:** `llmhotspot app <COMMAND>`

###### **Subcommands:**

* `open-console` — Open console URL in browser
* `restart` — Mark app restart request
* `stop` — Stop running app



## `llmhotspot app open-console`

Open console URL in browser

**Usage:** `llmhotspot app open-console`



## `llmhotspot app restart`

Mark app restart request

**Usage:** `llmhotspot app restart`



## `llmhotspot app stop`

Stop running app

**Usage:** `llmhotspot app stop`



## `llmhotspot session`

Session controls

**Usage:** `llmhotspot session <COMMAND>`

###### **Subcommands:**

* `start` — Start hotspot session
* `stop` — Stop hotspot session
* `status` — Get hotspot session status



## `llmhotspot session start`

Start hotspot session

**Usage:** `llmhotspot session start`



## `llmhotspot session stop`

Stop hotspot session

**Usage:** `llmhotspot session stop`



## `llmhotspot session status`

Get hotspot session status

**Usage:** `llmhotspot session status`



## `llmhotspot generate-cli-docs`

Generate CLI docs at docs/CLI_REFERENCE.md

**Usage:** `llmhotspot generate-cli-docs`

## `llmhotspot broker`

Run tunnel broker API (Cloudflare provisioning service)

**Usage:** `llmhotspot broker [OPTIONS]`

###### **Options:**

* `--listen <LISTEN>`

  Default value: `0.0.0.0:3001`
* `--db-backend <DB_BACKEND>` — Broker database backend (`postgres` default, `sqlite` fallback)
* `--database-url <DATABASE_URL>` — Broker database URL (required for Postgres unless `LLMHOTSPOT_BROKER_DATABASE_URL` is set)



<hr/>

<small><i>
    This document was generated automatically by
    <a href="https://crates.io/crates/clap-markdown"><code>clap-markdown</code></a>.
</i></small>
