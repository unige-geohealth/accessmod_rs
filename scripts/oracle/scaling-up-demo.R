# Runs inside the pinned AccessMod 5 image (see export.sh): the scaling-up
# replay of replay/examples/demo_scaling_up on the demo project (its
# population rPopulation__cor_pop is not in the image: rPopulation__demo_patients
# is used, as in the coverage oracle). Up to 10 new facilities, 120 min,
# anisotropic, existing facilities kept, exclusion 5 km around facilities,
# suitability from population, distance to facilities and travel time from
# roads. Writes to /out/scaling-up/demo/oracle/: the capacity table (CSV),
# the facilities with the new ones (FlatGeobuf), the residual population
# (GeoTIFF), the catchments of the new facilities (FlatGeobuf) and the thin
# rasterisation of the roads (GeoTIFF).
source("global.R")
out <- "/out/scaling-up/demo/oracle"
dir.create(out, recursive = TRUE, showWarnings = FALSE)

conf <- amAnalysisReplayParseConf("/scripts/scaling-up-demo.json")
started <- Sys.time()
amAnalysisReplayExec(conf)
elapsed <- as.numeric(Sys.time() - started, units = "secs")

con <- DBI::dbConnect(RSQLite::SQLite(), "/data/dbgrass/demo/demo/sqlite.db")
write.csv(DBI::dbReadTable(con, "tCapacityStatNew__oracle"),
  file.path(out, "capacity.csv"), row.names = FALSE)
DBI::dbDisconnect(con)

amGrassNS(location = "demo", mapset = "demo", {
  execGRASS("g.region", raster = "rDem__dem@PERMANENT")
  execGRASS("r.out.gdal", flags = c("c", "m", "overwrite"),
    input = "rPopulationResidual__oracle", output = file.path(out, "population_residual.tif"),
    format = "GTiff", type = "Float64", createopt = "COMPRESS=DEFLATE")
  # Thin rasterisation of the roads (v.to.rast without -d), the start cells
  # of the travel-time suitability factor.
  execGRASS("v.to.rast", flags = "overwrite", input = "vRoad__demo", output = "oracle_roads_thin",
    use = "val", value = 1)
  execGRASS("r.out.gdal", flags = c("c", "m", "overwrite"), input = "oracle_roads_thin",
    output = file.path(out, "roads_thin.tif"), format = "GTiff", type = "Byte",
    createopt = "COMPRESS=DEFLATE")
  execGRASS("v.out.ogr", flags = c("overwrite"), input = "vFacilityNew__oracle",
    output = file.path(out, "facilities.fgb"), format = "FlatGeobuf")
  shapes <- system(sprintf("echo %s", config$pathShapes), intern = TRUE)
  shp <- list.files(shapes, pattern = "^vCatchmentNew__oracle\\.shp$", full.names = TRUE)
  system(sprintf("ogr2ogr -f FlatGeobuf -nomd -nlt PROMOTE_TO_MULTI %s %s",
    file.path(out, "catchments.fgb"), shp[1]))
})
writeLines(sprintf("%.3f", elapsed), file.path(out, "seconds.log"))
