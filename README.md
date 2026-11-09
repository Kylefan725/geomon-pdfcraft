# GeoMon PDF

GeoMonNext's embedded PDF viewer and native processor, based on
[PDFCraft](https://github.com/storytold/pdfcraft) revision
`2ef2e2b82286acbf4c1ff72fcb7a82ef52fe7a54`.

This branch carries the existing GeoMonNext integration as ordinary Rust source.
It preserves the hosted viewer's permissions, save acknowledgement, review tools,
page selection, alignment coordinates, and Traditional Chinese text support.

- `apps/geomon-pdf` owns inspection, rendering, extraction, and markup verification.
- `apps/geomon-pdf-viewer` owns the browser bridge and hosted viewer tests.
- `crates/ui-egui/src/hosted` owns the embedded viewer UI and policy.
- `Cargo.lock` pins the dependencies used by both applications.

GeoMonNext owns project authorization, drawing revisions, storage, map alignment,
and deployment. Its `scripts/construction/pdf/build.sh` downloads a verified fork
revision, supplies the pinned font input, and packages the native and browser
applications. The browser protocol remains version 2.

Build with Rust 1.95.0. Set `CRAFT_FONTS_DIR` to the font input prepared by the
GeoMonNext build, set `CRAFT_I18N_DIR` to the catalog checkout pinned in
`craft-i18n.rev`, and set `PDFCRAFT_SYSTEM_FONTS=0` to use the same fonts in tests.

```sh
cargo +1.95.0 build --locked --release -p geomon-pdf
cargo +1.95.0 build --locked --release --target wasm32-unknown-unknown -p geomon-pdf-viewer
cargo +1.95.0 test --locked --release -p geomon-pdf -p geomon-pdf-viewer -p pdfcraft-annot -p pdfcraft-fonts
```

Upstream changes are reviewed separately from GeoMonNext integration changes.
The initial migration preserves the source assembled by GeoMonNext revision
`e10d5ab1cb7c2d71b8d131ed668f867a04411d0f`.

PDFCraft and these changes are licensed under [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE). Keep the required [notices](NOTICE) and
[asset attribution](ATTRIBUTION.md). ArtCraft's trademark logos and the upstream
screenshot containing its mark are excluded from this fork.
