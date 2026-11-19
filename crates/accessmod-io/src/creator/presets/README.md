# Project Creator presets

Data files the Project Creator applies when it builds a project
(`docs/project-creator.md`, contract 6). They are scientific assumptions:
versioned, reviewed like code, recorded in each project's manifest with
their version, and shown to the user before a project is created.

| File | What it sets |
| --- | --- |
| `worldcover.json` | ESA WorldCover classes: land cover labels and the default walking scenario (km/h) |
| `osm_roads.json` | OSM `highway` values to road classes (merged as class + 1000), labels, speeds in the walking, bicycle and motorized scenarios; exclusions |
| `osm_barriers.json` | OSM `waterway` values kept as barriers |
| `osm_facilities.json` | OSM health facilities kept as a starting point |

All files are **provisional** until reviewed by the AccessMod team: speeds,
the wetland and mangrove rules and the road classes in particular. A change
raises `version`; a project keeps the version it was built with.
