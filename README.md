# microcms-tui

English | [日本語](README.ja.md)

A Ratatui terminal UI for microCMS. Browse APIs and content, edit JSON, manage publication status, members, and media without leaving the terminal.

## Install

Download a binary for macOS, Linux, or Windows from [GitHub Releases](https://github.com/istone-you/microcms-tui/releases).

To run or install from source:

```sh
cargo run -- --service-id your-service --api-key your-api-key
cargo install --path .
```

## Configuration

```sh
microcms-tui --service-id your-service --api-key your-api-key
microcms-tui --service-id your-service --api-key your-api-key --endpoint blogs --save-config
```

- CLI: `--service-id`, `--api-key`, `--endpoint`, `--save-config`
- Environment: `MICROCMS_SERVICE_ID`, `MICROCMS_API_KEY`
- Precedence: CLI > environment > config file
- `--endpoint` only sets the initial selection; endpoints remain switchable in the TUI

Alternate API domains can be configured with:

- `MICROCMS_CONTENT_API_URL`
- `MICROCMS_MANAGEMENT_API_URL`

Set only the domain. The service ID and fixed `/api/v1` path are added automatically.

```sh
MICROCMS_CONTENT_API_URL=https://microcms-staging.net
```

Never commit API keys or other credentials.

## Keybindings

Press `?` for context-sensitive help. `Ctrl-C` always quits.

### Global / navigation

| Key | Action |
| --- | --- |
| `?` | Open help |
| `j`/`k`, `Up`/`Down` | Move or scroll |
| `Enter` | Select API or open JSON preview |
| `b`/`Esc` | Back or close |
| `r` | Reload |
| `n`/`p`, `PageDown`/`PageUp` | Next or previous page |
| `M` | Open members |
| `a` | Open media |

### Content

| Key | Action |
| --- | --- |
| `Space` | Mark content for bulk actions |
| `c` / `C` | POST create / create draft |
| `u` / `U` | PUT create with ID / create draft with ID |
| `e` / `E` | PATCH edit / edit with `status=draft` |
| `d` | Delete marked or current content |
| `P` / `D` | Publish / set draft |
| `s` | View or change publication reservation |
| `v` | View published and draft versions |
| `m` | Change creator |

Published writes, publication changes, and deletion require confirmation. JSON create and edit use `$EDITOR`, falling back to `vi`.

### Content query

| Key | Query |
| --- | --- |
| `/` | Keyword search `q` |
| `f` / `o` | `filters` / `orders` |
| `l` | Select schema-backed `fields` |
| `z` | Select `depth` |
| `i` | Comma-separated `ids` |
| `K` | `draftKey` |
| `F` | `richEditorFormat` |
| `x` | Clear all queries |

### Media

| Key | Action |
| --- | --- |
| `u` / `d` | Upload / delete |
| `/` / `t` / `A` | Filter by file name / tags / alt |
| `I` | Toggle image-only results |
| `l` | Set page limit |
| `x` | Clear filters |

Rows can be selected with the mouse, and the wheel scrolls lists and JSON previews.

## API permissions

Grant only the permissions needed by the features you use:

- Content API: GET / POST / PUT / PATCH / DELETE
- Management API: API and service information, content metadata, publication status, and scheduling
- Members: list/detail retrieval and creator changes
- Media: retrieval, upload, and deletion

## Notes

- List APIs support browsing, queries, CRUD, bulk actions, and publication changes
- Object APIs are GET/JSON-preview only
- Create templates and field ordering use the Management API schema
- API error JSON is shown in a modal
- Status dots: published green, draft cyan, scheduled magenta, closed red, unknown gray
