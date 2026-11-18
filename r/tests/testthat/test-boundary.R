test_that("JSON keeps arrays of one element and full precision", {
  json <- .to_json(list(stack = list(list(type = "barrier")), ids = list(3L), speed_kmh = 1 / 3, where = NULL))
  back <- .from_json(json)
  expect_equal(length(back$stack), 1)
  expect_true(is.list(back$ids) && length(back$ids) == 1)
  expect_match(gsub("[[:space:]]", "", json), "\"ids\":[3]", fixed = TRUE)
  expect_equal(back$speed_kmh, 1 / 3)
})

test_that("errors are classed conditions with the engine's issues", {
  err <- tryCatch(.am_stop("bad", kind = "validation", status = 2L, issues = c("a", "b")), error = identity)
  expect_s3_class(err, c("accessmod_validation", "accessmod_error"))
  expect_equal(err$issues, c("a", "b"))
  expect_equal(err$status, 2L)
})

test_that("each platform maps to its release target", {
  expect_equal(am_target("Darwin", "arm64"), "aarch64-apple-darwin")
  expect_equal(am_target("Darwin", "x86_64"), "x86_64-apple-darwin")
  expect_equal(am_target("Linux", "aarch64"), "aarch64-unknown-linux-gnu")
  expect_equal(am_target("Linux", "x86_64"), "x86_64-unknown-linux-gnu")
  expect_equal(am_target("Windows", "x86-64"), "x86_64-pc-windows-msvc")
  expect_message(expect_equal(am_target("Windows", "ARM64"), "x86_64-pc-windows-msvc"), "emulation")
  expect_error(am_target("SunOS", "x86_64"), "no release")
})

test_that("an archive without its checksum, or a corrupt one, is refused", {
  dir <- tempfile()
  dir.create(dir)
  file <- file.path(dir, "a.tar.gz")
  writeBin(charToRaw("x"), file)
  expect_error(.check_sha256(file), "no checksum")
  writeLines(paste(strrep("0", 64), " a.tar.gz"), paste0(file, ".sha256"))
  expect_error(.check_sha256(file), "corrupt")
  writeLines(paste(digest::digest(file = file, algo = "sha256"), " a.tar.gz"), paste0(file, ".sha256"))
  expect_silent(.check_sha256(file))
})
