# accessmod (R)

Run AccessMod 6 from R: open a project made in the web interface, take the
job of a past run, derive scenarios from it (speeds, land cover, …) and run
them, in a loop. One session object per project, `am`; type `am$` and Tab in
RStudio to see what it does.

**Status: experimental.** The API may still change before 6.0.0
([design](../docs/r-package.md)). Windows, macOS and Linux, x86_64 and arm64.
Dependencies: `R6`, `jsonlite`, `digest` (none of them has dependencies);
`terra` to read rasters and vectors.

## Install

```r
install.packages("remotes")
remotes::install_github("unige-geohealth/accessmod_rs", subdir = "r")
accessmod::am_install()
```

The package is plain R. The computations run in the AccessMod engine, a
program of about 20 MB with GDAL and PROJ inside, the same as the web
interface's. `am_install()` downloads it from the
[releases](https://github.com/unige-geohealth/accessmod_rs/releases), at the
version this package is made for (`am_engine_version()`), checks its
SHA-256, runs its self-check (`doctor`) and only then uses it. It lives in
`tools::R_user_dir("accessmod", "data")`; `am_uninstall()` removes it.
`am_init()` offers to install it when it is missing.

- **Offline**: download `accessmod-<version>-<target>.tar.gz` (`.zip` on
  Windows) and its `.sha256` from the release page, put them side by side,
  then `am_install(file = "path/to/accessmod-…")`. `am_target()` gives the
  target of the machine.
- **Another engine** (a source build, the desktop application's):
  `am_init(binary = "path/to/accessmod")`, or `ACCESSMOD_BIN`. The desktop
  application's engine also needs `PROJ_DATA` set to the `share/proj`
  folder among its resources.

## Open a project

```r
library(accessmod)
am <- am_init("~/AccessMod/demo")                      # a project of the web interface
am <- am_init("~/am/my-project", import = "my-project.am6p")   # Project › Export
am <- am_init("~/am/old", import = "am5_export.zip")   # AccessMod 5: Data manager › Archive
am <- am_init("~/am/demo", demo = TRUE)                # the AccessMod 5 sample project
am
am$list_data()                    # id, type, class, tags, file, run: as Data
am$list_data(type = "job")        # the jobs of past runs, as Data › Job
am$list_runs()
```

## Select

`am$get(type, class, tags, id)` returns the one Job or Dataset matching:
`type` is `"raster"`, `"vector"`, `"table"` or `"job"`; `class` a class code
(`"rTravelTime"`, `"vBarrier"`, …) or, for jobs, the analysis
(`"travel-time"`, `"merge-land-cover"`, …); every tag given must be among
its tags, in any order. None or several: an error lists the candidates.

```r
tt     <- am$get("job", class = "travel-time", tags = "test")
merge  <- am$get("job", class = "merge-land-cover", tags = "test")
rivers <- am$get("vector", class = "vBarrier", tags = c("demo", "rivers"))
tt$scenario()                     # class, label, mode, speed_kmh
```

## Scenarios

Jobs are immutable: `am$new_scenario()` and `$with()` return new jobs, so a
loop can derive any number of variants from the same baseline.

```r
merged <- am$run(merge, tags = "base")$get_output("rLandCoverMerged")

runs <- lapply(c(20, 40, 60), function(speed) {
  job <- am$new_scenario(tt, data.frame(class = c(1001, 1002, 1003), speed_kmh = speed))
  am$run(job$with(land_cover = merged), tags = c("roads", speed))
})
runs[[1]]
runs[[1]]$get_output("rTravelTime")$read()   # terra SpatRaster, seconds
```

To change the land cover itself (a flooded area as a barrier, a new road):

```r
flood <- am$import_data("flood.gpkg", class = "vBarrier", tags = c("flood", "2024"))
stack <- c(merge$document()$stack, list(list(type = "barrier", path = flood)))
flooded <- am$run(merge$with(stack = stack), tags = "flood")$get_output("rLandCoverMerged")
am$run(tt$with(land_cover = flooded), tags = "flood")
```

Each run's outputs are datasets of the project, tagged as given: give each
scenario its own tags (or `overwrite = TRUE` to replace).

## New jobs

`am$job(analysis, ...)` takes the fields of the job document (schemas:
`accessmod <analysis> schema`, or a job exported from the web interface);
Datasets give their paths. The package's test
[`tests/testthat/test-demo.R`](tests/testthat/test-demo.R) builds a merge and
a travel-time job on the demo from scratch.

## Errors

Failures are R conditions of class `accessmod_error` (and
`accessmod_<kind>`: `validation`, `not_found`, `ambiguous`, …) with the
engine's `issues`:

```r
tryCatch(am$run(job), accessmod_error = function(e) e$issues)
```
