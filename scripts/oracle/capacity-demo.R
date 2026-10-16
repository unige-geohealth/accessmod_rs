# Runs inside the pinned AccessMod 5 image (see export.sh): geographic
# coverage (capacity) replays on the demo project, anisotropic, towards
# facilities, 120 min, population captured removed at each step.
# - demo (replay/examples/loop_capacity_stat): 10 facilities ordered by
#   capacity (descending). Writes the capacity table (CSV), the residual
#   population (GeoTIFF) and the catchments (FlatGeobuf) to
#   /out/capacity/demo/oracle/.
# - demo-extended: the same, ordered by the population within 120 min
#   (descending), with the population on barriers, the extended columns and
#   the zonal coverage of vZone__demo (cat, admin_name). Writes the capacity
#   and zonal tables (CSV), the residual population and the population on
#   barriers (GeoTIFF) to /out/capacity/demo-extended/oracle/.
source("global.R")

replay <- function(config, tag, out) {
  dir.create(out, recursive = TRUE, showWarnings = FALSE)
  conf <- amAnalysisReplayParseConf(config)
  started <- Sys.time()
  amAnalysisReplayExec(conf)
  elapsed <- as.numeric(Sys.time() - started, units = "secs")
  writeLines(sprintf("%.3f", elapsed), file.path(out, "seconds.log"))

  con <- DBI::dbConnect(RSQLite::SQLite(), "/data/dbgrass/demo/demo/sqlite.db")
  write.csv(DBI::dbReadTable(con, sprintf("tCapacityStat__%s", tag)),
    file.path(out, "capacity.csv"), row.names = FALSE)
  zonal <- sprintf("tZonalStat__%s", tag)
  if (DBI::dbExistsTable(con, zonal)) {
    write.csv(DBI::dbReadTable(con, zonal), file.path(out, "zonal.csv"), row.names = FALSE)
  }
  DBI::dbDisconnect(con)

  amGrassNS(location = "demo", mapset = "demo", {
    execGRASS("g.region", raster = "rDem__dem@PERMANENT")
    rasters <- c(
      population_residual = sprintf("rPopulationResidual__%s", tag),
      population_on_barriers = sprintf("rPopulationOnBarrier__%s", tag)
    )
    for (name in names(rasters)) {
      if (amRastExists(rasters[[name]])) {
        execGRASS("r.out.gdal", flags = c("c", "m", "overwrite"),
          input = rasters[[name]], output = file.path(out, paste0(name, ".tif")),
          format = "GTiff", type = "Float64", createopt = "COMPRESS=DEFLATE")
      }
    }
  })
  elapsed
}

replay("/scripts/capacity-demo.json", "oracle", "/out/capacity/demo/oracle")
amGrassNS(location = "demo", mapset = "demo", {
  # The shapes directory is given with GRASS environment variables.
  shapes <- system(sprintf("echo %s", config$pathShapes), intern = TRUE)
  shp <- list.files(shapes, pattern = "^vCatchment__oracle\\.shp$", full.names = TRUE)
  system(sprintf("ogr2ogr -f FlatGeobuf -nomd -nlt PROMOTE_TO_MULTI %s %s",
    file.path("/out/capacity/demo/oracle", "catchments.fgb"), shp[1]))
})
replay("/scripts/capacity-demo-extended.json", "extended", "/out/capacity/demo-extended/oracle")
