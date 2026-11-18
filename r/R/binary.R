# The AccessMod 6 binary: where it is, how to install it from the GitHub
# releases (a self-contained archive per platform: binary, PROJ database,
# web client), and how to call it.

.repo <- "unige-geohealth/accessmod_rs"

#' Release target of this machine
#'
#' The Rust target name of the release archive for the running platform:
#' `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`,
#' `aarch64-unknown-linux-gnu` or `x86_64-pc-windows-msvc`. Windows on arm
#' runs the x86_64 build (emulation).
#'
#' @param sysname,machine Operating system and processor (default: this
#'   machine, from [Sys.info()]).
#' @return A string.
#' @export
am_target <- function(sysname = Sys.info()[["sysname"]], machine = Sys.info()[["machine"]]) {
  arch <- switch(tolower(machine),
    "arm64" = , "aarch64" = "aarch64",
    "x86_64" = , "x86-64" = , "amd64" = , "x64" = "x86_64",
    stop("AccessMod 6 has no release for the processor ", machine, call. = FALSE)
  )
  switch(sysname,
    Darwin = paste0(arch, "-apple-darwin"),
    Linux = paste0(arch, "-unknown-linux-gnu"),
    Windows = {
      if (arch == "aarch64") {
        message("Windows on arm: using the x86_64 build (emulation).")
      }
      "x86_64-pc-windows-msvc"
    },
    stop("AccessMod 6 has no release for ", sysname, call. = FALSE)
  )
}

.install_root <- function() tools::R_user_dir("accessmod", "data")

.exe <- function() if (.Platform$OS.type == "windows") "accessmod.exe" else "accessmod"

#' Install the AccessMod 6 binary
#'
#' Downloads the release archive for this platform from GitHub, checks its
#' SHA-256 and unpacks it under [tools::R_user_dir()] (`accessmod`, `data`).
#' Nothing else is needed: GDAL and PROJ are inside the binary.
#'
#' @param version A release version such as `"6.0.0-alpha.1"`, or
#'   `"latest"`: the most recent release, pre-releases included.
#' @param force Download again even when this version is installed.
#' @return The path of the binary, invisibly.
#' @export
am_install <- function(version = "latest", force = FALSE) {
  release <- .release(version)
  version <- sub("^v", "", release$tag_name)
  target <- am_target()
  name <- paste0("accessmod-", version, "-", target)
  archive <- paste0(name, if (grepl("windows", target)) ".zip" else ".tar.gz")
  dir <- file.path(.install_root(), version)
  bin <- file.path(dir, name, .exe())
  if (file.exists(bin) && !force) {
    .set_current(bin)
    message("AccessMod ", version, " is installed: ", bin)
    return(invisible(bin))
  }
  assets <- vapply(release$assets, function(a) a$name, "")
  urls <- vapply(release$assets, function(a) a$browser_download_url, "")
  if (!archive %in% assets) {
    stop("release ", release$tag_name, " has no archive ", archive, call. = FALSE)
  }
  tmp <- tempfile("accessmod-")
  dir.create(tmp)
  on.exit(unlink(tmp, recursive = TRUE), add = TRUE)
  old <- options(timeout = max(1800, getOption("timeout")))
  on.exit(options(old), add = TRUE)
  file <- file.path(tmp, archive)
  message("Downloading ", archive)
  utils::download.file(urls[assets == archive], file, mode = "wb", quiet = TRUE)
  sums <- paste0(archive, ".sha256")
  if (sums %in% assets) {
    sum_file <- file.path(tmp, sums)
    utils::download.file(urls[assets == sums], sum_file, mode = "wb", quiet = TRUE)
    expected <- tolower(strsplit(trimws(readLines(sum_file, warn = FALSE)[1]), "[[:space:]]+")[[1]][1])
    actual <- digest::digest(file = file, algo = "sha256")
    if (!identical(expected, actual)) {
      stop("the download of ", archive, " is corrupt (SHA-256 differs): try again", call. = FALSE)
    }
  } else {
    warning("release ", release$tag_name, " has no checksum for ", archive, call. = FALSE)
  }
  staging <- file.path(tmp, "unpacked")
  if (grepl("\\.zip$", archive)) {
    utils::unzip(file, exdir = staging)
  } else {
    utils::untar(file, exdir = staging)
  }
  if (!file.exists(file.path(staging, name, .exe()))) {
    stop("the archive ", archive, " holds no ", .exe(), call. = FALSE)
  }
  dir.create(dir, recursive = TRUE, showWarnings = FALSE)
  unlink(file.path(dir, name), recursive = TRUE)
  if (!file.rename(file.path(staging, name), file.path(dir, name))) {
    # Another volume (temporary directory elsewhere): copy instead.
    file.copy(file.path(staging, name), dir, recursive = TRUE)
  }
  Sys.chmod(bin, "755")
  .set_current(bin)
  message("Installed AccessMod ", version, ": ", bin)
  invisible(bin)
}

.release <- function(version) {
  url <- if (identical(version, "latest")) {
    sprintf("https://api.github.com/repos/%s/releases?per_page=20", .repo)
  } else {
    sprintf("https://api.github.com/repos/%s/releases/tags/v%s", .repo, sub("^v", "", version))
  }
  file <- tempfile(fileext = ".json")
  on.exit(unlink(file))
  headers <- c(Accept = "application/vnd.github+json")
  token <- Sys.getenv("GITHUB_PAT", Sys.getenv("GITHUB_TOKEN"))
  if (nzchar(token)) headers <- c(headers, Authorization = paste("Bearer", token))
  ok <- tryCatch(
    {
      utils::download.file(url, file, quiet = TRUE, headers = headers)
      TRUE
    },
    error = function(e) FALSE,
    warning = function(w) FALSE
  )
  if (!ok) {
    stop("cannot read the AccessMod releases on GitHub (", url, "): offline, or no such version?", call. = FALSE)
  }
  found <- jsonlite::fromJSON(file, simplifyVector = FALSE)
  if (identical(version, "latest")) {
    found <- Filter(function(r) !isTRUE(r$draft), found)
    if (length(found) == 0) stop("no AccessMod release on GitHub yet", call. = FALSE)
    found <- found[[1]]
  }
  found
}

.set_current <- function(bin) {
  dir.create(.install_root(), recursive = TRUE, showWarnings = FALSE)
  writeLines(normalizePath(bin), file.path(.install_root(), "current"))
}

#' Path of the AccessMod 6 binary
#'
#' In order: the option `accessmod.binary`, the environment variable
#' `ACCESSMOD_BIN`, then the copy installed by [am_install()].
#'
#' @return The path of the binary.
#' @export
am_binary <- function() {
  candidates <- c(getOption("accessmod.binary", ""), Sys.getenv("ACCESSMOD_BIN"))
  current <- file.path(.install_root(), "current")
  if (file.exists(current)) candidates <- c(candidates, readLines(current, warn = FALSE)[1])
  for (bin in candidates) {
    if (nzchar(bin) && file.exists(bin)) return(normalizePath(bin))
  }
  stop("AccessMod 6 is not installed: run accessmod::am_install()", call. = FALSE)
}

#' Version of the AccessMod 6 binary
#' @return A string such as `"6.0.0-alpha.1"`.
#' @export
am_version <- function() {
  out <- suppressWarnings(system2(am_binary(), "--version", stdout = TRUE, stderr = TRUE))
  sub("^accessmod ", "", out[1])
}

#' Check the AccessMod 6 installation
#'
#' Runs `accessmod doctor`: GDAL drivers, PROJ database, scratch directory,
#' memory and disk.
#'
#' @return A data frame of checks (`name`, `ok`, `detail`), invisibly; the
#'   failed checks are reported.
#' @export
am_doctor <- function() {
  out <- .am_exec(c("doctor", "--json"))
  checks <- do.call(rbind, lapply(out$checks, function(c) {
    data.frame(name = c$name, ok = isTRUE(c$ok), detail = paste(c$detail, collapse = " "))
  }))
  failed <- checks[!checks$ok, , drop = FALSE]
  if (nrow(failed) > 0) {
    warning(paste0(failed$name, ": ", failed$detail, collapse = "\n"), call. = FALSE)
  } else {
    message("AccessMod ", am_version(), ": ", nrow(checks), " checks passed")
  }
  invisible(checks)
}

# Run the binary with `args` (each one quoted for the shell) and parse its
# JSON output. A failure stops with the command's own message: the issues of
# its JSON error, else what it wrote on stderr.
.am_exec <- function(args, json = TRUE) {
  bin <- am_binary()
  err <- tempfile()
  on.exit(unlink(err))
  out <- suppressWarnings(system2(bin, shQuote(args), stdout = TRUE, stderr = err))
  status <- attr(out, "status")
  text <- paste(out, collapse = "\n")
  if (!is.null(status) && status != 0) {
    parsed <- tryCatch(jsonlite::fromJSON(text, simplifyVector = FALSE), error = function(e) NULL)
    issues <- unlist(parsed$issues)
    if (length(issues) == 0) {
      log <- readLines(err, warn = FALSE)
      issues <- grep("error|cancelled", log, value = TRUE, ignore.case = TRUE)
      if (length(issues) == 0) issues <- utils::tail(log, 5)
    }
    stop(paste(c(paste0("accessmod ", args[1], " failed (exit ", status, "):"), paste0("  - ", issues)), collapse = "\n"),
      call. = FALSE
    )
  }
  if (!json) return(invisible(text))
  if (!nzchar(text)) return(NULL)
  jsonlite::fromJSON(text, simplifyVector = FALSE)
}
