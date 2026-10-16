# Runs inside the pinned AccessMod 5 image (see export.sh): the optimization
# replays of tests/optimization/config.json on the demo project: the 10
# catchments of vCatchment__demo (cat_join) on rPopulation__demo_patients,
# 10 facilities, with one facility at least per zone of vZone__demo
# (admin_name, cat; facilities vFacility__demo, cat) and without that
# constraint; then 15 km discs around the facilities (see below). Writes to
# /out/optimization/demo/oracle/ the result tables (CSV) and the catchments
# (FlatGeobuf).
source("global.R")
out <- "/out/optimization/demo/oracle"
dir.create(out, recursive = TRUE, showWarnings = FALSE)

conf <- amAnalysisReplayParseConf("/scripts/optimization-demo.json")
cases <- list(admin = TRUE, no_admin = FALSE)
for (name in names(cases)) {
  conf$args$adminCheck <- cases[[name]]
  started <- Sys.time()
  amAnalysisReplayExec(conf)
  elapsed <- as.numeric(Sys.time() - started, units = "secs")
  writeLines(sprintf("%.3f", elapsed), file.path(out, sprintf("seconds_%s.log", name)))
  con <- DBI::dbConnect(RSQLite::SQLite(), "/data/dbgrass/demo/demo/sqlite.db")
  write.csv(DBI::dbReadTable(con, "tOptimization__oracle"),
    file.path(out, sprintf("optimization_%s.csv", name)), row.names = FALSE)
  DBI::dbDisconnect(con)
}

amGrassNS(location = "demo", mapset = "demo", {
  shapes <- system(sprintf("echo %s", config$pathShapes), intern = TRUE)
  shp <- list.files(shapes, pattern = "^vCatchment__demo\\.shp$", full.names = TRUE)
  system(sprintf("ogr2ogr -f FlatGeobuf -nomd -nlt PROMOTE_TO_MULTI %s %s",
    file.path(out, "catchments.fgb"), shp[1]))
})

# Catchments that do not follow the cells: 15 km discs around the demo
# facilities (overlapping), so that exact coverage fractions and polygon
# differences matter. Written as vCatchment__oracle_discs, optimised
# without zone constraint, exported as discs.fgb and optimization_discs.csv.
amGrassNS(location = "demo", mapset = "demo", {
  shapes <- normalizePath(system(sprintf("echo %s", config$pathShapes), intern = TRUE))
  facilities <- sf::st_as_sf(read_VECT("vFacility__demo"))
  discs <- sf::st_buffer(facilities[, "cat"], 15000)
  discs$cat_join <- discs$cat
  shp <- file.path(shapes, "vCatchment__oracle_discs.shp")
  sf::st_write(discs[, "cat_join"], shp, delete_dsn = TRUE, quiet = TRUE)
  system(sprintf("ogr2ogr -f FlatGeobuf -nomd %s %s", file.path(out, "discs.fgb"), shp))
  started <- Sys.time()
  amAnalysisOptimization(
    inputCatchment = "vCatchment__oracle_discs",
    inputPopulation = "rPopulation__demo_patients",
    outputOptimization = "tOptimization__oracle_discs",
    idFieldCatchment = "cat_join",
    nTot = 10,
    adminCheck = FALSE,
    pBarTitle = "Optimization analysis"
  )
  elapsed <- as.numeric(Sys.time() - started, units = "secs")
  writeLines(sprintf("%.3f", elapsed), file.path(out, "seconds_discs.log"))
  con <- DBI::dbConnect(RSQLite::SQLite(), "/data/dbgrass/demo/demo/sqlite.db")
  write.csv(DBI::dbReadTable(con, "tOptimization__oracle_discs"),
    file.path(out, "optimization_discs.csv"), row.names = FALSE)
  DBI::dbDisconnect(con)
})
