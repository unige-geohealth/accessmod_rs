# Project Creator sources for the end-to-end tests

The server's source cache (`<data>/cache/sources/`) filled beforehand, at
the keys of `accessmod_fetch::catalogue`: the Project Creator finds every
source cached and contacts no provider. Real data cut around Juba, South
Sudan (as `crates/accessmod-io/tests/data/creator/`): Copernicus DEM GLO-90,
ESA WorldCover 2021 (CC BY 4.0), WorldPop R2025A (CC BY 4.0), OpenStreetMap
through the HOT exports (ODbL). The geoBoundaries files are made up: two
squares named "Juba west" and "Juba east".
