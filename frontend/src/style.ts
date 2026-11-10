import type {
  StyleSpecification,
  RasterDEMSourceSpecification,
} from "maplibre-gl";

// Native MapLibre relief, hillshade and terrain share the real DEM.
const DEM = "/api/lab/dem/{z}/{x}/{y}?geo=geography-v1-equal250-gauss5000-cut20000-hollows1";
const elevationSource: RasterDEMSourceSpecification = {
  type: "raster-dem",
  tiles: [DEM],
  encoding: "terrarium",
  tileSize: 256,
  minzoom: 0,
  maxzoom: 15,
  attribution: "Elevation: Mapzen / AWS Terrain Tiles",
};

export const buildStyle = (): StyleSpecification => ({
  version: 8,
  name: "hug-gamer-geography",
  projection: { type: "globe" },
  sources: {
    // Separate tile sampling for cartography and 3D terrain, as MapLibre recommends.
    relief: elevationSource,
    terrarium: elevationSource,
    water: {
      type: "vector",
      tiles:["/api/geography/water/{z}/{x}/{y}?geo=geography-v1-equal250-gauss5000-cut20000-hollows1"],minzoom:0,maxzoom:14,
      attribution:
        "© OpenMapTiles © OpenStreetMap contributors (natural water only)",
    },
  },
  layers: [
    {
      id: "space",
      type: "background",
      paint: { "background-color": "#0a1420" },
    },
    {
      id: "relief",
      type: "color-relief",
      source: "relief",
      paint: {
        "color-relief-color": [
          "interpolate",
          ["linear"],
          ["elevation"],
          -1,
          "#153345",
          0,
          "#60705a",
          500,
          "#827d65",
          2000,
          "#8d7965",
          4500,
          "#adaba1",
          7000,
          "#e0e4e3",
        ],
      },
    },
    {
      id: "hillshade",
      type: "hillshade",
      source: "relief",
      paint: {
        "hillshade-exaggeration": 0.5,
        "hillshade-shadow-color": "#18212a",
        "hillshade-highlight-color": "#eee7d2",
      },
    },
    {
      // Natural standing water only (lakes, ponds, rivers) — no reservoirs,
      // no man-made pools. Ocean/coastline already come from the DEM basemap.
      id: "natural-water",
      type: "fill",
      source: "water",
      "source-layer": "water",
      filter: [
        "in",
        ["get", "class"],
        ["literal", ["lake", "river", "pond", "water"]],
      ],
      paint: { "fill-color": "#2b6a9f", "fill-opacity": 0.92 },
    },
    {
      // Natural flowing water only (rivers, streams) — no canals/drains.
      id: "natural-waterway",
      type: "line",
      source: "water",
      "source-layer": "waterway",
      filter: ["in", ["get", "class"], ["literal", ["river", "stream"]]],
      paint: {
        "line-color": "#2b6a9f",
        "line-width": [
          "interpolate",
          ["linear"],
          ["zoom"],
          4,
          0.5,
          9,
          1,
          14,
          3,
        ],
        "line-opacity": 0.85,
      },
    },
  ],
  terrain: { source: "terrarium", exaggeration: 1 },
  // Globe projection supports atmosphere-blend but not the fog matrix, so keep
  // fog-* keys out (they log "calculateFogMatrix is not supported on globe
  // projection" and do nothing).
  sky: {
    "sky-color": "#0b1626",
    "horizon-color": "#2a4a6b",
    "atmosphere-blend": [
      "interpolate",
      ["linear"],
      ["zoom"],
      0,
      1,
      6,
      0.5,
      9,
      0,
    ],
  },
});
