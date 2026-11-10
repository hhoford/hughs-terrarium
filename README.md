# hugh's terrarium

A browser-based dragonfly and queen simulation with a globe map and a Rust server.

## Run locally

Requirements: Node.js 22 or newer, Rust/Cargo, and a C compiler plus OpenSSL development files for the Rust dependencies.

```sh
./run-local.sh
```

The script builds the frontend and backend, then serves the app at `http://127.0.0.1:8787`. On first use, the server fetches elevation and water tiles from the public sources listed below and saves a new local world under `.runtime/`. Runtime data stays out of source control. Keep this directory to continue the same local world; remove it to start a new one.

For development, run the backend with `HUG_GEOGRAPHY_ROOT=.runtime/geography HUG_GEOGRAPHY_MODE=build HUG_WORLD_ROOT=.runtime/world cargo run --manifest-path backend/Cargo.toml` and, separately, run `npm --prefix frontend run dev`. The frontend dev server proxies API requests to `127.0.0.1:8787`.

## Data sources

- Elevation tiles: AWS Terrain Tiles in Terrarium encoding. The app attributes these to Mapzen / AWS Terrain Tiles; see the [AWS dataset record](https://registry.opendata.aws/terrain-tiles/) and [Tilezen attribution terms](https://github.com/tilezen/joerd/blob/master/docs/attribution.md).
- Natural water features: OpenFreeMap vector tiles, derived from OpenStreetMap. The app includes OpenMapTiles and OpenStreetMap attribution; see [OpenFreeMap attribution and license notes](https://openfreemap.org/) and the [OpenStreetMap ODbL notice](https://www.openstreetmap.org/copyright).
- Bundled polar elevation values: `backend/data/polar-heights.i16.gz` with metadata in `backend/data/polar-heights.json`.

This source snapshot has no project-level license file; no license is asserted here. Review the licenses of dependencies and data sources before redistribution.
