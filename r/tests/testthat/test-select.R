test_that("the catalogue lists datasets and the jobs of finished analysis runs", {
  cat <- .catalogue(fake_datasets(), fake_runs())
  expect_equal(nrow(cat), 9)
  jobs <- .select(cat, type = "job")
  expect_equal(jobs$id, c(1L, 2L, 6L, 7L))
  expect_equal(jobs$class, c("merge-land-cover", "travel-time", "travel-time", "travel-time"))
  # An overwrite keeps both jobs apart: the later one carries its number.
  expect_equal(.select(cat, type = "job", tags = c("test", "7"))$id, 7L)
  expect_equal(.select(cat, type = "raster", class = "rLandCoverMerged")$run, 1L)
  expect_equal(nrow(.select(cat, type = "table")), 0)
})

test_that("tags match exactly, all of them, in any order", {
  cat <- .catalogue(fake_datasets(), fake_runs())
  expect_equal(.select(cat, type = "vector", tags = c("rivers", "demo"))$id, 3L)
  expect_equal(nrow(.select(cat, type = "vector", tags = "river")), 0)
  expect_equal(.select(cat, type = "job", tags = "test 1")$id, 6L)
  expect_equal(nrow(.select(cat, type = "job", tags = "test")), 3)
})

test_that("one match or an error naming the candidates", {
  cat <- .catalogue(fake_datasets(), fake_runs())
  expect_equal(.select_one(cat, "job", class = "merge-land-cover", tags = "test")$id, 1L)
  # Run 7 overwrote run 2 (same tags): its number tells them apart.
  expect_error(.select_one(cat, "job", class = "travel-time", tags = "test"), class = "accessmod_ambiguous")
  expect_equal(.select_one(cat, "job", class = "travel-time", tags = c("test", "7"))$id, 7L)
  err <- tryCatch(.select_one(cat, "vector", class = "vBarrier"), error = identity)
  expect_s3_class(err, "accessmod_ambiguous")
  expect_match(conditionMessage(err), "vector 3 \\(vBarrier: demo rivers\\)")
  expect_match(conditionMessage(err), "vector 4")
  expect_error(.select_one(cat, "raster", class = "rTravelTime"), class = "accessmod_not_found")
  expect_error(.select(cat, type = "rasters"), "unknown type")
})

test_that("a fractional id is refused, not truncated", {
  cat <- .catalogue(fake_datasets(), fake_runs())
  expect_error(.select(cat, id = 1.9), "whole numbers")
  expect_error(.select(cat, id = 3e9), "integer range")
  expect_error(.select(cat, id = c(1, 2)), "one id")
  expect_equal(.select(cat, type = "raster", id = 12)$class, "rLandCoverMerged")
})
