# accessmod (R)

Run AccessMod 6 analyses from R: import a project made in the web interface,
change scenarios, regenerate the merged land cover and compute travel times,
in a loop. The package is a thin layer over the AccessMod 6 command line:
the engine is the released binary (GDAL and PROJ inside), installed by the
package itself. No compiled code, no Rtools; Windows, macOS and Linux, x86_64
and arm64.

**Status: experimental (6.0.0-alpha.1).** Projects, saved jobs, scenarios,
merge land cover and travel time have R functions; every other analysis runs
through `am_run()` with its job document. What is planned:
[`docs/r-package.md`](../docs/r-package.md).

## Install

```r
install.packages("remotes")
remotes::install_github("unige-geohealth/accessmod_rs", subdir = "r")
accessmod::am_install()   # the AccessMod 6 binary for this machine (~20 MB)
accessmod::am_doctor()    # check it
```

`am_install()` downloads the latest release from GitHub and checks its
SHA-256; `am_install("6.0.0-alpha.1")` pins a version. To use another binary
(a source build, a country server's), set `ACCESSMOD_BIN` or
`options(accessmod.binary = "/path/to/accessmod")`.

## A project from the web interface

Build the project in the web interface (`accessmod serve`, or the desktop
application): import the data, merge the land cover, set up a travel-time
analysis and save it. Then Project › Export gives an `.am6p` file:

```r
library(accessmod)
p <- am_project_import("my-project.am6p", "~/am/my-project")
am_jobs(p)                       # saved jobs: id, analysis, name
tt <- am_job(p, "Travel time")   # by name or id
am_scenario(tt)                  # class, label, mode, speed_kmh
```

AccessMod 5 data archives (Data manager › Archive, `.zip`) import the same
way.

## Scenarios in a loop

An `am_job` is a value: a modified copy leaves the original unchanged.

```r
merge <- am_job(p, "Merge land cover")
merged <- am_merge_land_cover(p, merge, tags = c("base"))

results <- lapply(c(20, 40, 60), function(speed) {
  job <- tt
  s <- am_scenario(job)
  s$speed_kmh[s$mode == "motorized"] <- speed
  am_scenario(job) <- s
  run <- am_travel_time(p, job, land_cover = merged, tags = c("roads", speed))
  data.frame(speed = speed, mean_s = am_outputs(run, "rTravelTime")$mean)
})
do.call(rbind, results)
```

To change the land cover itself (new roads, a flooded area as a barrier),
import the layers in the web interface or with `accessmod dataset import`,
edit the merge job's `stack`, and merge again: each run's merged land cover
feeds `am_travel_time(..., land_cover = run)`.

Outputs are project datasets (`am_outputs(run)`: paths, mean, min, max);
`am_raster(run, "rTravelTime")` reads one with `terra`. Travel times are in
seconds.

## Without the web interface

`am_project_demo(dir)` makes the AccessMod 5 sample project; `am_job_new()`
writes any job document (schemas: `accessmod <analysis> schema`); paths are
relative to the project. The package's test `tests/testthat/test-demo.R` is
a complete example: merge, then two travel-time scenarios.

## Functions

| Function | Does |
| --- | --- |
| `am_install()`, `am_binary()`, `am_version()`, `am_doctor()` | binary |
| `am_project_import()`, `am_project()`, `am_project_demo()` | projects |
| `am_datasets()`, `am_runs()`, `am_jobs()`, `am_job()` | content |
| `am_job_new()`, `am_job_read()`, `am_scenario()`, `am_scenario<-` | jobs |
| `am_run()`, `am_merge_land_cover()`, `am_travel_time()` | runs |
| `am_outputs()`, `am_raster()` | results |
