# README media

The README uses a static overview first and an optional short view comparison
after installation. Both show the real development app with Atlas sample data.

## Captured assets

| Asset | Content | Dimensions | Size |
| --- | --- | --- | --- |
| `assets/readme/overview.png` | Tree view with nested fields and compact query options | 3840 × 1968 | 528 KiB |
| `assets/readme/document-views.gif` | Tree and JSON views, 3 seconds each, looping | 1000 × 512 | 140 KiB |

The GIF is assembled from two real screenshots. It compares the views; it is not
a recording of typing, edits, or query execution time. Both decoded frames and
the six-second duration were checked. The overview is sufficient without opening
the animated example in the README.

These replace the old dialog screenshot and 70-second, 21.8 MiB GIF. Raw captures
and extracted verification frames stay in ignored `target/readme-capture/`.

## Scene

- Application: the running lowercase `openmango` development instance, not the
  installed `OpenMango` app. Recheck its process and window IDs after a restart.
- Connection: `Atlas`; namespace: `sample_mflix.movies`.
- Filter: `{ "title": "The Matrix" }`, returning one document.
- Projection: `_id`, `title`, `year`, `rated`, `runtime`, `genres`, `released`,
  and `imdb`, each set to `1`.
- Tree: expand the document, `genres`, and `imdb`. Keep the compact options row
  visible; close editor popovers and menus.
- JSON: open the same movie. The editor reloads the complete document for editing,
  so the capture contains fields omitted by the Tree projection.
- Keep the same window size and position across both captures. Capture only the
  app window. Do not include connection URLs, account details, or unrelated data.

The maintainer operates the app and handles builds/restarts. Native macOS tools
capture the selected window; no Codex configuration changes are required.
No sample records were inserted, modified, or deleted for these captures.

The dataset's fields are documented in
[MongoDB's sample-data reference](https://www.mongodb.com/docs/manual/sample-data/sample-mflix/).

## Recreate the small loop

Place equal-size Tree and JSON PNG captures at
`target/readme-capture/demo-00.png` and `demo-01.png`, respectively. Then run:

```sh
ffmpeg -hide_banner -loglevel error -y \
  -framerate 1/3 -i target/readme-capture/demo-%02d.png \
  -filter_complex 'scale=1000:-2:flags=lanczos,split[frames][palette_input];[palette_input]palettegen=stats_mode=full[palette];[frames][palette]paletteuse=dither=none' \
  -loop 0 -final_delay 300 assets/readme/document-views.gif
```

Inspect both decoded frames, confirm the duration and file size, and check the
README image links before committing replacement media.
