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

test_that("numbers round-trip exactly: large ids and doubles near 1", {
  doc <- .from_json('{"ids":[1234567890123456],"speed_kmh":1.0000000000000002,"class":5,"x":0.1}')
  back <- .from_json(.to_json(doc))
  expect_identical(back$ids[[1]], 1234567890123456)
  expect_identical(back$speed_kmh, 1.0000000000000002)
  expect_identical(back$class, 5L)
  expect_identical(back$x, 0.1)
  expect_false(grepl("e+", .to_json(doc), fixed = TRUE))
})

test_that("failures keep the engine's issues, doctor checks included", {
  expect_equal(.issues(list(issues = list("a", "b"))), c("a", "b"))
  expect_equal(.issues(list(checks = list(list(name = "driver GTiff", ok = TRUE, detail = "ok"), list(name = "proj.db", ok = FALSE, detail = "missing")))), "proj.db: missing")
  expect_equal(.issues(NULL), character())
})

test_that("an engine that cannot run or prints no JSON is an accessmod_error", {
  skip_on_os("windows")
  fake <- tempfile()
  writeLines(c("#!/bin/sh", "echo not json"), fake)
  Sys.chmod(fake, "755")
  expect_error(.exec_json(fake, "doctor"), class = "accessmod_error")
  writeLines(c("#!/bin/sh", "echo 'boom error' >&2", "exit 3"), fake)
  err <- tryCatch(.exec_json(fake, "doctor"), error = identity)
  expect_s3_class(err, "accessmod_engine")
  expect_equal(err$status, 3L)
  expect_match(conditionMessage(err), "boom error")
  expect_error(.exec_json(file.path(tempdir(), "nowhere"), "doctor"), class = "accessmod_error")
})
