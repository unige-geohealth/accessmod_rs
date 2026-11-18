job <- function() Job$new("travel-time", scenario_doc(), "/p", run = 2, tags = "test")
ds <- function(project = "/p") {
  row <- .select(.catalogue(fake_datasets(), list()), id = 12)
  Dataset$new(row, project)
}

test_that("a job is immutable, and variants never change the baseline, aliases included", {
  tt <- job()
  alias <- tt
  before <- .to_json(tt$document())
  a <- tt$with(limit_s = 3600)
  b <- Job$new(tt$analysis, .with_speeds(tt$document(), data.frame(class = 1001, speed_kmh = 20), tt$analysis), tt$project)
  expect_identical(.to_json(tt$document()), before)
  expect_identical(.to_json(alias$document()), before)
  expect_equal(a$document()$limit_s, 3600)
  expect_equal(b$scenario()$speed_kmh, c(5, 20))
  expect_error(tt$analysis <- "x", "read only")
  expect_error(tt$clone(), "attempt to apply non-function")
  expect_true(is.na(a$run))
})

test_that("datasets give their paths, foreign ones are refused", {
  tt <- job()
  merged <- ds()
  expect_equal(tt$with(land_cover = merged)$document()$land_cover, "runs/1/land_cover_merged.tif")
  facilities <- tt$with(facilities = merged)$document()$facilities
  expect_equal(facilities, list(path = "runs/1/land_cover_merged.tif", id_field = "cat"))
  stack <- tt$with(stack = list(list(type = "land_cover", path = merged)))$document()$stack
  expect_equal(stack[[1]]$path, "runs/1/land_cover_merged.tif")
  expect_null(tt$with(limit_s = NULL)$document()$limit_s)
  expect_error(tt$with(land_cover = ds("/other")), "another project")
  expect_error(tt$with(1), "named fields")
})

test_that("new speeds: listed classes only, each once, finite and >= 0", {
  d <- scenario_doc()
  out <- .with_speeds(d, data.frame(class = 1, speed_kmh = 4, mode = "bicycling"), "travel-time")
  expect_equal(out$scenario[[1]]$speed_kmh, 4)
  expect_equal(out$scenario[[1]]$mode, "bicycling")
  expect_equal(out$scenario[[1]]$label, "Built")
  expect_equal(out$scenario[[2]], d$scenario[[2]])
  expect_error(.with_speeds(d, data.frame(class = 7, speed_kmh = 4), "travel-time"), "not in the scenario: 7")
  expect_error(.with_speeds(d, data.frame(class = c(1, 1), speed_kmh = 4), "travel-time"), "once")
  expect_error(.with_speeds(d, data.frame(class = 1, speed_kmh = -1), "travel-time"), ">= 0")
  expect_error(.with_speeds(d, data.frame(class = 1, speed_kmh = Inf), "travel-time"), "finite")
  expect_error(.with_speeds(d, data.frame(class = 1, speed_kmh = 4, mode = "flying"), "travel-time"), "unknown mode")
  expect_error(.with_speeds(d, data.frame(speed = 4), "travel-time"), "columns class and speed_kmh")
  expect_error(.with_speeds(list(), data.frame(class = 1, speed_kmh = 1), "merge-land-cover"), "no scenario")
})

test_that("fractional classes and text or factor speeds are refused, not truncated", {
  d <- scenario_doc()
  expect_error(.with_speeds(d, data.frame(class = 1.9, speed_kmh = 4), "travel-time"), "whole numbers")
  expect_error(.with_speeds(d, data.frame(class = factor(1), speed_kmh = 4), "travel-time"), "whole numbers")
  expect_error(.with_speeds(d, data.frame(class = 1, speed_kmh = factor("80")), "travel-time"), "not text or factors")
  expect_error(.with_speeds(d, data.frame(class = 1, speed_kmh = "80"), "travel-time"), "not text or factors")
})

test_that("a made object cannot be re-initialised through an alias", {
  tt <- job()
  alias <- tt
  expect_error(alias$initialize("referral", list(), "/elsewhere"), "made once")
  expect_equal(tt$analysis, "travel-time")
  expect_equal(tt$project, "/p")
  expect_error(Job$new("travel-time", list(f = function() 1), "/p"), "JSON values only")
  expect_error(Job$new("travel-time", list(f = factor("a")), "/p"), "JSON values only")
  expect_error(Job$new(NULL, list(), "/p"), "a name")
})
