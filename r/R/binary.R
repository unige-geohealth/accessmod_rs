# The AccessMod engine: one binary per platform (GDAL and PROJ inside),
# installed from the GitHub release archives at the version this package is
# made for, checked (SHA-256, then `doctor`) before it is used.

.repo <- "unige-geohealth/accessmod_rs"

#' Engine version this package is made for
#' @return A string such as `"6.0.0-alpha.2"`.
#' @export
am_engine_version <- function() {
  utils::packageDescription("accessmod", fields = "Config/accessmod/engine")
}

#' Release target of this machine
#'
#' The Rust target name of the release archive for a platform:
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
    .am_stop(paste("AccessMod has no release for the processor", machine))
  )
  switch(sysname,
    Darwin = paste0(arch, "-apple-darwin"),
    Linux = paste0(arch, "-unknown-linux-gnu"),
    Windows = {
      if (arch == "aarch64") message("Windows on arm: using the x86_64 build (emulation).")
      "x86_64-pc-windows-msvc"
    },
    .am_stop(paste("AccessMod has no release for", sysname))
  )
}

.install_root <- function() tools::R_user_dir("accessmod", "data")

.exe <- function() if (.Platform$OS.type == "windows") "accessmod.exe" else "accessmod"

#' Install the AccessMod engine
#'
#' Downloads the release archive of this platform from GitHub (about 20 MB),
#' checks its SHA-256, unpacks it, runs `accessmod doctor` on it, and only
#' then makes it the engine used by [am_init()]. It goes under
#' `tools::R_user_dir("accessmod", "data")`; [am_uninstall()] removes it.
#'
#' Offline: download `accessmod-<version>-<target>.tar.gz` (`.zip` on
#' Windows) and its `.sha256` from the release page, then give the archive
#' as `file`; the `.sha256` must sit next to it.
#'
#' @param version Engine version; default: the one this package is made for
#'   ([am_engine_version()]).
#' @param force Install again when this version is already installed.
#' @param file A release archive already downloaded (offline install).
#' @return The path of the engine, invisibly.
#' @export
am_install <- function(version = am_engine_version(), force = FALSE, file = NULL) {
  version <- sub("^v", "", version)
  target <- am_target()
  name <- paste0("accessmod-", version, "-", target)
  archive <- paste0(name, if (grepl("windows", target)) ".zip" else ".tar.gz")
  dir <- file.path(.install_root(), version)
  bin <- file.path(dir, name, .exe())
  if (file.exists(bin) && !force) {
    .doctor(bin, version)
    .set_current(bin)
    message("AccessMod ", version, " is installed: ", bin)
    return(invisible(bin))
  }
  # Staged next to its destination (one volume: the final move is a rename),
  # checked whole, then swapped in; a previous install is kept until then.
  dir.create(dir, recursive = TRUE, showWarnings = FALSE)
  tmp <- tempfile("staging-", tmpdir = dir)
  dir.create(tmp)
  on.exit(unlink(tmp, recursive = TRUE), add = TRUE)
  if (is.null(file)) {
    release <- .release(version)
    assets <- vapply(release$assets, function(a) a$name, "")
    urls <- vapply(release$assets, function(a) a$browser_download_url, "")
    for (needed in c(archive, paste0(archive, ".sha256"))) {
      if (!needed %in% assets) .am_stop(paste0("release v", version, " has no ", needed))
    }
    old <- options(timeout = max(1800, getOption("timeout")))
    on.exit(options(old), add = TRUE)
    message("Downloading ", archive)
    file <- file.path(tmp, archive)
    utils::download.file(urls[assets == archive], file, mode = "wb", quiet = TRUE)
    utils::download.file(urls[assets == paste0(archive, ".sha256")], paste0(file, ".sha256"), mode = "wb", quiet = TRUE)
  } else if (basename(file) != archive) {
    .am_stop(paste0("expected ", archive, " for this machine and version, got ", basename(file)))
  }
  .check_sha256(file)
  staging <- file.path(tmp, "unpacked")
  unpacked <- if (grepl("\\.zip$", archive)) utils::unzip(file, exdir = staging) else utils::untar(file, exdir = staging)
  staged <- file.path(staging, name, .exe())
  if (is.null(unpacked) || (is.numeric(unpacked) && any(unpacked != 0)) || !file.exists(staged)) .am_stop(paste("cannot unpack", archive))
  Sys.chmod(staged, "755")
  .doctor(staged, version)
  # The working engine is moved aside (a name of this install only), the
  # new one moved in, and the old one removed only once `current` points
  # to the new one.
  final <- file.path(dir, name)
  previous <- file.path(tmp, "previous")
  if (file.exists(final) && !file.rename(final, previous)) .am_stop(paste("cannot replace", final))
  if (!file.rename(file.path(staging, name), final)) {
    if (file.exists(previous) && !file.rename(previous, final)) {
      .am_stop(paste0("cannot move the engine into ", final, ", nor restore the previous one (kept in ", previous, ")"))
    }
    .am_stop(paste("cannot move the engine into", final))
  }
  .set_current(bin)
  message("Installed AccessMod ", version, ": ", bin)
  invisible(bin)
}

#' Remove installed AccessMod engines
#' @param version One version, or `NULL` for every engine installed by
#'   [am_install()].
#' @return `NULL`, invisibly.
#' @export
am_uninstall <- function(version = NULL) {
  root <- .install_root()
  target <- if (is.null(version)) root else file.path(root, sub("^v", "", version))
  unlink(target, recursive = TRUE)
  current <- file.path(root, "current")
  if (file.exists(current) && !file.exists(readLines(current, warn = FALSE)[1])) unlink(current)
  invisible(NULL)
}

.check_sha256 <- function(file) {
  sums <- paste0(file, ".sha256")
  if (!file.exists(sums)) .am_stop(paste("no checksum next to", basename(file), "(expected", basename(sums), ")"))
  expected <- tolower(strsplit(trimws(readLines(sums, warn = FALSE)[1]), "[[:space:]]+")[[1]][1])
  if (!identical(expected, digest::digest(file = file, algo = "sha256"))) {
    .am_stop(paste(basename(file), "is corrupt (its SHA-256 differs): download it again"))
  }
}

# `doctor` passes and the engine is the version expected.
.doctor <- function(bin, version = NULL) {
  out <- .exec_json(bin, c("doctor", "--json"))
  issues <- .issues(out)
  if (length(issues) > 0) .am_stop(paste("the engine fails its checks:", paste(issues, collapse = "; ")), kind = "doctor", issues = issues)
  if (!is.null(version) && !identical(am_version(bin), version)) {
    .am_stop(paste0("the engine reports version ", am_version(bin), ", expected ", version))
  }
  invisible(out)
}

.release <- function(version) {
  url <- sprintf("https://api.github.com/repos/%s/releases/tags/v%s", .repo, version)
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
  if (!ok) .am_stop(paste0("cannot read release v", version, " on GitHub: offline, or no such version? (", url, ")"))
  jsonlite::fromJSON(file, simplifyVector = FALSE)
}

.set_current <- function(bin) {
  dir.create(.install_root(), recursive = TRUE, showWarnings = FALSE)
  writeLines(normalizePath(bin), file.path(.install_root(), "current"))
}

#' Path of the AccessMod engine
#'
#' In order: `binary`, the option `accessmod.binary`, the environment
#' variable `ACCESSMOD_BIN`, then the engine installed by [am_install()]. The
#' desktop application's engine can be used through `binary` or
#' `ACCESSMOD_BIN` (see the package README).
#'
#' @param binary An explicit path.
#' @return The path, or an error saying how to install the engine.
#' @export
am_binary <- function(binary = NULL) {
  candidates <- c(binary, getOption("accessmod.binary", ""), Sys.getenv("ACCESSMOD_BIN"))
  current <- file.path(.install_root(), "current")
  if (file.exists(current)) candidates <- c(candidates, readLines(current, warn = FALSE)[1])
  for (bin in candidates) {
    if (!is.null(bin) && nzchar(bin) && file.exists(bin)) return(normalizePath(bin))
  }
  .am_stop("the AccessMod engine is not installed: run accessmod::am_install()", kind = "not_installed")
}

#' Version of an AccessMod engine
#' @param binary The engine (default: [am_binary()]).
#' @return A string such as `"6.0.0-alpha.2"`.
#' @export
am_version <- function(binary = am_binary()) {
  out <- suppressWarnings(system2(binary, "--version", stdout = TRUE, stderr = TRUE))
  sub("^accessmod ", "", out[1])
}

# Engines of another major version are refused; another version of the same
# major only warns (development builds, a newer release).
.check_engine <- function(binary) {
  have <- am_version(binary)
  want <- am_engine_version()
  if (!identical(sub("\\..*", "", have), sub("\\..*", "", want))) {
    .am_stop(paste0("engine ", have, " is not compatible with this package (made for ", want, "): run accessmod::am_install()"))
  }
  if (!identical(have, want)) warning("AccessMod engine ", have, " differs from the version this package is made for (", want, ")", call. = FALSE)
  invisible(have)
}
