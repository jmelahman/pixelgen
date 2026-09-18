# pixelgen

## Checking a change

Run the full hook suite before calling any change done:

```sh
prek run --all-files
```

## Running the page

```sh
prek run build-web --all-files
python3 -m http.server -d web 8000 --bind 127.0.0.1   # run in background
```

Rebuild after touching `crates/`; `web/` edits only need a reload.

## Inspecting with the Playwright MCP

1. `browser_navigate` to `http://127.0.0.1:8000/`; wait for `window.pixelgen`.
2. `browser_console_messages` at `error` - anything there is a bug.
3. Open an image: "Open image" + `browser_file_upload`, or
   `window.pixelgen.open(file)` via `browser_evaluate` (see
   `tools/web-smoke.mjs` for a synthetic photo). A scene is built automatically.
4. `browser_take_screenshot` to see the canvas; `browser_snapshot` for refs.
