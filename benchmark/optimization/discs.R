# Runs inside the AccessMod 5 image (am5_session in benchmark/lib.sh) on its
# demo project: discs of radius <radius m> centred on a lattice of
# <spacing m> over the population region (rPopulation__demo_patients), as
# overlapping catchments, then amAnalysisOptimization selecting <n>
# facilities without zone constraint. Writes the discs (/out/discs.fgb),
# the result table (/out/optimization.csv), the seconds spent loading
# global.R (/out/load.txt) and those of the analysis (/out/seconds.txt).
started <- Sys.time()
source("global.R")
writeLines(sprintf("%.2f", as.numeric(Sys.time() - started, units = "secs")), "/out/load.txt")

args <- as.numeric(commandArgs(trailingOnly = TRUE))
spacing <- args[1]
radius <- args[2]
selected <- args[3]
amGrassNS(location = "demo", mapset = "demo", {
  execGRASS("g.region", raster = "rPopulation__demo_patients")
  population <- read_RAST("rPopulation__demo_patients")
  e <- as.vector(terra::ext(population))
  centres <- expand.grid(
    x = seq(e[1] + spacing / 2, e[2], by = spacing),
    y = seq(e[3] + spacing / 2, e[4], by = spacing)
  )
  points <- sf::st_as_sf(centres, coords = c("x", "y"), crs = sf::st_crs(terra::crs(population)))
  discs <- sf::st_buffer(points, radius)
  discs$cat_join <- seq_len(nrow(discs))
  shapes <- normalizePath(system(sprintf("echo %s", config$pathShapes), intern = TRUE))
  shp <- file.path(shapes, "vCatchment__bench_discs.shp")
  sf::st_write(discs[, "cat_join"], shp, quiet = TRUE)
  system(sprintf("ogr2ogr -f FlatGeobuf -nomd /out/discs.fgb %s", shp))
  started <- Sys.time()
  result <- amAnalysisOptimization(
    inputCatchment = "vCatchment__bench_discs",
    inputPopulation = "rPopulation__demo_patients",
    outputOptimization = "tOptimization__bench_discs",
    idFieldCatchment = "cat_join",
    nTot = selected,
    adminCheck = FALSE,
    pBarTitle = "Optimization analysis"
  )
  seconds <- as.numeric(Sys.time() - started, units = "secs")
  writeLines(sprintf("%.2f", seconds), "/out/seconds.txt")
  write.csv(result, "/out/optimization.csv", row.names = FALSE)
})
