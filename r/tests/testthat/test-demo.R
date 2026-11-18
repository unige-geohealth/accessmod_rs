# The demo project end to end: merge land cover, then travel time, for two
# scenarios. Needs the binary (ACCESSMOD_BIN, or am_install()).
has_binary <- function() !inherits(try(accessmod::am_binary(), silent = TRUE), "try-error")

test_that("scenarios on the demo: merge, travel time, slower roads take longer", {
  skip_if_not(has_binary(), "no AccessMod binary (ACCESSMOD_BIN or am_install())")
  dir <- file.path(tempfile("am-"), "demo")
  p <- am_project_demo(dir)
  on.exit(unlink(dirname(dir), recursive = TRUE), add = TRUE)
  expect_s3_class(p, "am_project")
  expect_output(print(p), "AccessMod project")
  ds <- am_datasets(p)
  path <- function(class, tag = "demo") ds$file[ds$class == class & grepl(tag, ds$tags)][1]

  merge <- am_job_new("merge-land-cover", list(
    schema_version = 1L,
    dem = path("rDem"),
    stack = list(
      list(type = "road", path = path("vRoad"), class_field = "class_n", label_field = "label"),
      list(type = "barrier", path = path("vBarrier", "rivers")),
      list(type = "land_cover", path = path("rLandCover"))
    )
  ))
  merged <- am_merge_land_cover(p, merge, tags = c("r", "merged"))
  expect_equal(merged$status, "done")
  expect_equal(nrow(am_outputs(merged, "rLandCoverMerged")), 1)

  travel <- am_job_new("travel-time", list(
    schema_version = 1L,
    dem = path("rDem"),
    land_cover = "",
    facilities = list(path = path("vFacility"), id_field = "cat"),
    scenario = list(),
    method = "isotropic",
    direction = "from_facilities",
    neighbourhood = 16L
  ))
  am_scenario(travel) <- data.frame(
    class = c(1, 2, 3, 1001, 1002, 1003),
    mode = c("walking", "walking", "walking", "motorized", "motorized", "motorized"),
    speed_kmh = c(5, 4, 2, 80, 50, 30)
  )
  fast <- am_travel_time(p, travel, land_cover = merged, tags = c("r", "fast"))
  slow_job <- travel
  s <- am_scenario(slow_job)
  s$speed_kmh[s$class > 1000] <- 10
  am_scenario(slow_job) <- s
  slow <- am_travel_time(p, slow_job, land_cover = merged, tags = c("r", "slow"))
  mean_of <- function(run) am_outputs(run, "rTravelTime")$mean
  expect_gt(mean_of(slow), mean_of(fast))
  expect_equal(nrow(am_runs(p)), 3)
  expect_equal(nrow(am_jobs(p)), 0)
  # Same tags again: refused, unless overwrite.
  expect_error(am_travel_time(p, travel, land_cover = merged, tags = c("r", "fast")), "failed")
  again <- am_travel_time(p, travel, land_cover = merged, tags = c("r", "fast"), overwrite = TRUE)
  expect_equal(again$status, "done")
})
