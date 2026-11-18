# The demo project end to end through a session: merge land cover, travel
# time, scenarios derived from a run's job, then the same from an exported
# project file, the jobs found by tags (as in Data › Job). Needs the engine
# (ACCESSMOD_BIN, or am_install()).

test_that("scenarios on the demo, then on its project file", {
  skip_if_not(has_engine(), "no AccessMod engine (ACCESSMOD_BIN or am_install())")
  root <- tempfile("am-")
  on.exit(unlink(root, recursive = TRUE), add = TRUE)
  am <- init(file.path(root, "demo"), demo = TRUE)
  expect_output(print(am), "AccessMod project")
  expect_equal(nrow(am$list_data(type = "job")), 0)

  merge <- am$job("merge-land-cover",
    dem = am$get("raster", class = "rDem"),
    stack = list(
      list(type = "road", path = am$get("vector", class = "vRoad"), class_field = "class_n", label_field = "label"),
      list(type = "barrier", path = am$get("vector", class = "vBarrier", tags = "rivers")),
      list(type = "land_cover", path = am$get("raster", class = "rLandCover"))
    )
  )
  merged_run <- am$run(merge, tags = "base")
  expect_equal(merged_run$status, "done")
  merged <- merged_run$get_output("rLandCoverMerged")
  expect_equal(merged$run, merged_run$id)

  tt <- am$job("travel-time",
    dem = am$get("raster", class = "rDem"), land_cover = merged,
    facilities = list(path = am$get("vector", class = "vFacility"), id_field = "cat"),
    scenario = lapply(c(1, 2, 3, 1001, 1002, 1003), function(k) {
      list(class = k, mode = if (k > 1000) "motorized" else "walking", speed_kmh = if (k > 1000) 60 else 4)
    }),
    method = "isotropic", direction = "from_facilities", neighbourhood = 16L
  )
  base <- am$run(tt, tags = "test")
  expect_equal(base$status, "done")

  # The job of a run, by tags, as in Data › Job; slower roads take longer.
  baseline <- am$get("job", class = "travel-time", tags = "test")
  expect_equal(baseline$run, base$id)
  slow <- am$new_scenario(baseline, data.frame(class = 1001:1003, speed_kmh = 10))
  slow_run <- am$run(slow, tags = c("slow"))
  # Outcome: "<n> cells reached, max <t> s".
  max_time <- function(run) as.numeric(sub(".*max ([0-9.]+) s.*", "\\1", run$outcome))
  expect_gt(max_time(slow_run), max_time(base))
  expect_equal(nrow(slow_run$list_outputs("rTravelTime")), 1)
  expect_equal(baseline$scenario()$speed_kmh[4], 60)
  expect_error(am$run(baseline, tags = "test"), class = "accessmod_error")
  expect_equal(am$run(baseline, tags = "test", overwrite = TRUE)$status, "done")

  # The project file, without results: jobs are there, outputs are not.
  file <- am$export(file.path(root, "exports"), results = FALSE)
  copy <- init(file.path(root, "copy"), import = file)
  job <- copy$get("job", class = "travel-time", tags = "slow")
  expect_equal(job$scenario()$speed_kmh[4], 10)
  copied_run <- copy$list_runs()
  expect_true(all(c("merge-land-cover", "travel-time") %in% copied_run$analysis))
  expect_error(copy$run(job, tags = "again"), class = "accessmod_error")
  # Its runs are there, their outputs are not: said as such.
  slow_copy <- copy$get_run(job$run)
  expect_equal(nrow(slow_copy$list_outputs()), 0)
  expect_error(slow_copy$get_output("rTravelTime"), "without results")

  # With results: the copy runs its jobs.
  full <- init(file.path(root, "full"), import = am$export(file.path(root, "exports"), results = TRUE))
  again <- full$run(full$new_scenario(full$get("job", class = "travel-time", tags = "slow"), data.frame(class = 1001, speed_kmh = 30)), tags = "again")
  expect_equal(again$status, "done")
  expect_equal(again$get_output("rTravelTime")$class, "rTravelTime")
  expect_error(slow$with(land_cover = full$get("raster", class = "rLandCoverMerged", tags = "base")), "another project")
})
