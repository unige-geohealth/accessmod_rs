# Runs inside the pinned AccessMod 5 image (see export.sh): the geographic
# coverage (capacity) replay of replay/examples/loop_capacity_stat on the
# demo project: 10 facilities ordered by capacity (descending), anisotropic,
# towards facilities, 120 min, population captured removed at each step.
# Writes the capacity table (CSV), the residual population (GeoTIFF) and the
# catchments (FlatGeobuf) to /out/capacity/demo/oracle/.
source("global.R")
out <- "/out/capacity/demo/oracle"
dir.create(out, recursive = TRUE, showWarnings = FALSE)

conf <- amAnalysisReplayParseConf("/scripts/capacity-demo.json")
started <- Sys.time()
amAnalysisReplayExec(conf)
elapsed <- as.numeric(Sys.time() - started, units = "secs")

con <- DBI::dbConnect(RSQLite::SQLite(), "/data/dbgrass/demo/demo/sqlite.db")
write.csv(DBI::dbReadTable(con, "tCapacityStat__oracle"),
  file.path(out, "capacity.csv"), row.names = FALSE)
DBI::dbDisconnect(con)

amGrassNS(location = "demo", mapset = "demo", {
  execGRASS("g.region", raster = "rDem__dem@PERMANENT")
  execGRASS("r.out.gdal", flags = c("c", "m", "overwrite"),
    input = "rPopulationResidual__oracle", output = file.path(out, "population_residual.tif"),
    format = "GTiff", type = "Float64", createopt = "COMPRESS=DEFLATE")
  # The shapes directory is given with GRASS environment variables.
  shapes <- system(sprintf("echo %s", config$pathShapes), intern = TRUE)
  shp <- list.files(shapes, pattern = "^vCatchment__oracle\\.shp$", full.names = TRUE)
  system(sprintf("ogr2ogr -f FlatGeobuf -nomd -nlt PROMOTE_TO_MULTI %s %s",
    file.path(out, "catchments.fgb"), shp[1]))
})
writeLines(sprintf("%.3f", elapsed), file.path(out, "seconds.log"))
