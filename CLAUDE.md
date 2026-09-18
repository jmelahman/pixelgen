# pixelgen

## Checking a change

Run the full hook suite before calling any change done:

```sh
prek run --all-files
```

All of it has to pass, including `build the wasm bundle` and
`smoke-test the page`: those two are the only checks that the page in `web/`
actually builds and starts, and they are what the deploy relies on.

prek provides most of what the hooks need: a Rust toolchain with the
`wasm32-unknown-unknown` target, `wasm-bindgen-cli`, Node 22+, and, when no
Chrome is installed, a `chrome-headless-shell` download cached under
`~/.cache/pixelgen/browsers`. The machine still has to have:

- **A C linker** (`cc`). Without it every Rust hook fails with
  "linker `cc` not found".
- **`rustfmt` and `clippy`** for the cargo in use. prek's own toolchain is the
  minimal profile, so without them the cargo hooks fail with
  "no such command: `fmt`" / "`clippy`" (or set `PREK_RUST_PROFILE=default`).
- **The shared libraries Chrome links against.** A desktop has them; a bare
  container may not (`ldd` the downloaded binary to see which are missing).

`CHROME=/path/to/chrome` picks the browser explicitly; otherwise the smoke test
uses a `chromium` / `google-chrome` on `PATH` if there is one.
