# microcms-tui

`microcms-tui` is a Ratatui terminal UI for managing microCMS content. It discovers available APIs through the Management API, lets you select an endpoint inside the TUI, and provides paginated browsing plus JSON-editor-based create, update, and delete operations.

## Install and run

Only the service ID and API key are required at startup:

```sh
cargo run -- --service-id your-service --api-key your-api-key
```

Install the binary locally:

```sh
cargo install --path .
microcms-tui --service-id your-service --api-key your-api-key
```

An endpoint can optionally be preselected in the picker:

```sh
microcms-tui --service-id your-service --api-key your-api-key --endpoint blogs
```

Use `--save-config` to save the effective values to the platform-standard config directory:

```sh
microcms-tui --service-id your-service --api-key your-api-key --endpoint blogs --save-config
```

The TOML config contains optional `service_id`, `api_key`, and `default_endpoint` fields. Credentials can also be supplied with `MICROCMS_SERVICE_ID` and `MICROCMS_API_KEY`. Values are resolved in this order: CLI flags, environment variables, then the config file. `--endpoint` and `default_endpoint` only preselect an API; endpoint selection and switching happen inside the TUI.

Do not commit a config file containing a real API key.

The API key needs Management API access for endpoint discovery and `GET`, `POST`, `PATCH`, and `DELETE` permissions for the target Content API endpoints.

Publishing and returning content to draft with `P`/`D` requires the Management API **Change content publication status** permission. Status dots require the Management API **Retrieve Content (List/Detail)** permission, and schema-based create templates require **Retrieve API Information** permission.

Create and update operations open a temporary JSON file using `$EDITOR`, falling back to `vi`. Create JSON is generated from the selected endpoint's Management API schema and shows every parsed field with a microCMS-compatible empty value; create is disabled when that schema cannot be loaded or contains no parseable user fields. The update buffer uses the same schema template, overlaid with the selected content, so fields absent from the API response remain editable. Create and update preserve empty user-field values in POST/PATCH payloads. System metadata fields such as content IDs and timestamps are hidden from the edit buffer and omitted from POST/PATCH payloads automatically. Closing the editor without changing the JSON cancels the create/update operation.

Content labels use the first non-empty value in Management API `apiFields` order; no field name such as `title` or `name` receives special priority. JSON previews show system metadata first, followed by user fields in `apiFields` order and then any fields absent from the schema. Create and update editor buffers keep user fields in the same schema order. Content API keys come from `fieldId`, not the schema's display `name`.

## Keybindings

- `Ctrl-C`: quit from any screen or mode
- `?`: open or close the centered in-app keybinding help (except while typing in an input modal)

Endpoint picker:

- `Esc`: quit
- `j`/`Down` and `k`/`Up`: move selection
- `Enter`: browse the selected endpoint
- `r`: reload available APIs

Content browser:

- `b` or `Esc`: return to the endpoint picker
- `j`/`Down` and `k`/`Up`: move selection
- `Space`: select or deselect the current content for bulk actions
- `Enter`: open the selected content JSON preview fullscreen
- `r`: reload the current page
- `n` or `PageDown`: load the next page
- `p` or `PageUp`: load the previous page
- `c`: create published/default content with POST
- `C`: create draft content with POST `status=draft`
- `u`: create published/default content with a specified ID using PUT
- `U`: create draft content with a specified ID using PUT `status=draft`
- `e`: edit the selected content with the default PATCH
- `E`: edit draft content with PATCH `status=draft`
- `d`: request deletion of all marked contents, or the current content when none are marked
- `/`: edit keyword search (`q`) inline
- `f`: edit a `filters` expression inline
- `o`: edit an `orders` expression inline
- `x`: clear `q`, `filters`, and `orders`
- `P`: publish all marked contents, or the current content when none are marked
- `D`: return all marked contents to draft, or the current content when none are marked

Fullscreen JSON preview:

- `Enter` or `Esc`: close fullscreen preview
- `j`/`Down` and `k`/`Up`: scroll the JSON preview
- `g`/`G`: jump to the top/bottom
- `n`/`PageDown` and `p`/`PageUp`: move to the next/previous content on the current page
- `e`/`E`: edit the displayed content with default/draft PATCH
- `d`: request deletion of the displayed content
- `P`/`D`: publish or return the displayed content to draft

Selection, create, query, endpoint navigation, and page-fetch actions are disabled while the fullscreen preview is open.

Uppercase `C`, `U`, and `E` are thin wrappers over the Content API `status=draft` query. `D` is different: it changes publication status through the Management API, while `E` sends a Content API PATCH with `status=draft`.

Default/published writes (`c`, `u`, and `e`) and Management API publication changes (`P` and `D`) show a centered confirmation before the request is sent. Confirm with `y` or cancel with `n`/`Esc`. Draft writes (`C`, `U`, and `E`) continue immediately after the JSON editor without this confirmation. Delete keeps its existing confirmation flow.

Search, filter, and order strings are passed directly to the microCMS Content API as the `q`, `filters`, and `orders` query parameters. For example, orders can be `publishedAt`, `-publishedAt`, or `publishedAt,-updatedAt`. When `orders` is omitted, microCMS may relevance-sort keyword search results.

Query input is shown in a centered prompt inside the TUI. Press `Enter` to apply the input or `Esc` to cancel it; applying empty or whitespace-only input clears that value.

Marked content rows have a yellow left bar. Content row dots use Management API status metadata: green is published, cyan is draft, adjacent green and cyan dots mean published with a newer draft, red is closed, and gray is unknown. Gray means metadata was unavailable or could not be matched to the Content API result. Publication status changes reload the current page; an item may be hidden afterward if API key permissions or the active query/filter exclude it.

## Object API support

Object-format endpoints are supported as GET-only content. The single object returned by `GET /api/v1/{endpoint}` can be reloaded and viewed in the normal or fullscreen JSON preview. Pagination, bulk selection, create, edit, delete, and publication-status actions are disabled for Object APIs.

microCMS documents Content API POST, PUT, and DELETE as list-format-only operations, so this TUI does not send them to Object APIs. PATCH requires a content ID and is not documented for Object APIs, and Management API publish/draft changes are also left unsupported rather than inferred.

## MVP scope

This milestone supports endpoint discovery, querying, list retrieval, publication status changes, POST/PUT creation, update, and delete operations through a JSON editor. It does not provide schema-aware forms or rich-text editing.
