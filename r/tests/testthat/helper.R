# Fake command-line records, as `dataset list --json` and `project runs
# --json` print them (parsed).
fake_datasets <- function() {
  list(
    list(id = 1, class = "rDem", tags = list("demo"), file = "datasets/1.tif", origin = list(type = "import")),
    list(id = 2, class = "rLandCover", tags = list("demo"), file = "datasets/2.tif", origin = list(type = "import")),
    list(id = 3, class = "vBarrier", tags = list("demo", "rivers"), file = "datasets/3.fgb", origin = list(type = "import")),
    list(id = 4, class = "vBarrier", tags = list("demo", "wetlands"), file = "datasets/4.fgb", origin = list(type = "import")),
    list(id = 12, class = "rLandCoverMerged", tags = list("test"), file = "runs/1/land_cover_merged.tif", origin = list(type = "run", run = 1))
  )
}

fake_runs <- function() {
  list(
    list(id = 1, analysis = "merge-land-cover", status = "done", kind = "analysis", tags = list("test"), job = list(stack = list())),
    list(id = 2, analysis = "travel-time", status = "done", kind = "analysis", tags = list("test"), job = scenario_doc()),
    list(id = 5, analysis = "travel-time", status = "failed", kind = "analysis", tags = list("test"), job = list()),
    list(id = 6, analysis = "travel-time", status = "done", kind = "analysis", tags = list("test 1", "x"), job = scenario_doc()),
    # An overwrite of run 2: its job is named with its number.
    list(id = 7, analysis = "travel-time", status = "done", kind = "analysis", tags = list("test"), job_tags = list("test", "7"), job = scenario_doc()),
    list(id = 11, analysis = "archive", status = "done", kind = "manage", tags = list(), job = list())
  )
}

scenario_doc <- function() {
  list(
    schema_version = 1L, dem = "datasets/1.tif", land_cover = "runs/1/land_cover_merged.tif",
    facilities = list(path = "datasets/7.fgb", id_field = "cat"),
    scenario = list(
      list(class = 1L, mode = "walking", speed_kmh = 5, label = "Built"),
      list(class = 1001L, mode = "motorized", speed_kmh = 80)
    )
  )
}

# An engine for the end-to-end tests: ACCESSMOD_BIN, or am_install().
has_engine <- function() !inherits(try(am_binary(), silent = TRUE), "try-error")

# Whether the engine has `project export` (6.0.0-alpha.2 and later).
has_export <- function() {
  identical(suppressWarnings(system2(am_binary(), c("project", "export", "--help"), stdout = FALSE, stderr = FALSE)), 0L)
}

# am_init without the warning of a development engine (another version than
# the one the package is made for).
init <- function(...) {
  withCallingHandlers(am_init(...), warning = function(w) {
    if (grepl("differs from the version", conditionMessage(w))) invokeRestart("muffleWarning")
  })
}
