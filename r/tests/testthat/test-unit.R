test_that("each platform maps to its release target", {
  expect_equal(am_target("Darwin", "arm64"), "aarch64-apple-darwin")
  expect_equal(am_target("Darwin", "x86_64"), "x86_64-apple-darwin")
  expect_equal(am_target("Linux", "aarch64"), "aarch64-unknown-linux-gnu")
  expect_equal(am_target("Linux", "x86_64"), "x86_64-unknown-linux-gnu")
  expect_equal(am_target("Windows", "x86-64"), "x86_64-pc-windows-msvc")
  expect_message(expect_equal(am_target("Windows", "ARM64"), "x86_64-pc-windows-msvc"), "emulation")
  expect_error(am_target("SunOS", "x86_64"), "no release")
  expect_error(am_target("Linux", "riscv64"), "processor")
})

test_that("a scenario is edited on a copy and checked", {
  job <- am_job_new("travel-time", list(scenario = list(
    list(class = 1L, mode = "walking", speed_kmh = 5, label = "Built"),
    list(class = 1001L, mode = "motorized", speed_kmh = 80)
  )))
  s <- am_scenario(job)
  expect_equal(s$class, c(1L, 1001L))
  expect_equal(s$label, c("Built", NA))
  s$speed_kmh[s$class == 1001] <- 60
  faster <- job
  am_scenario(faster) <- s
  expect_equal(am_scenario(faster)$speed_kmh, c(5, 60))
  expect_equal(am_scenario(job)$speed_kmh, c(5, 80))
  expect_null(faster$document$scenario[[2]]$label)
  bad <- s
  bad$mode[1] <- "flying"
  expect_error(am_scenario(job) <- bad, "unknown mode")
  bad <- s
  bad$speed_kmh[1] <- -1
  expect_error(am_scenario(job) <- bad, ">= 0")
  expect_error(am_scenario(am_job_new("merge-land-cover", list())), "no scenario")
})

test_that("a missing binary says how to install it", {
  withr_env <- Sys.getenv("ACCESSMOD_BIN")
  old <- options(accessmod.binary = file.path(tempdir(), "nowhere"))
  on.exit(options(old))
  Sys.setenv(ACCESSMOD_BIN = "")
  on.exit(Sys.setenv(ACCESSMOD_BIN = withr_env), add = TRUE)
  current <- file.path(tools::R_user_dir("accessmod", "data"), "current")
  skip_if(file.exists(current), "a binary is installed")
  expect_error(am_binary(), "am_install")
})
