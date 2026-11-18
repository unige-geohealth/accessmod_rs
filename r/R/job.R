# Jobs: an analysis name and its job document (the JSON the command line
# and the web interface share, paths relative to the project). An `am_job`
# is a value: changing a copy leaves the original as it was, so scenarios can
# be derived in a loop.

#' Make a job
#'
#' @param analysis Analysis name: `"travel-time"`, `"merge-land-cover"`,
#'   `"referral"`, `"zonal-statistics"`, `"geographic-coverage"`,
#'   `"scaling-up"`, `"optimization"`, `"adjust-population-distribution"`.
#' @param document The job document as a list (see the job schemas,
#'   `accessmod <analysis> schema`); paths relative to the project. Use
#'   lists (not vectors) for JSON arrays of one element.
#' @param name Free name.
#' @return An `am_job`.
#' @export
am_job_new <- function(analysis, document, name = analysis) {
  stopifnot(is.character(analysis), length(analysis) == 1, is.list(document))
  structure(list(analysis = analysis, name = name, id = NA_integer_, document = document), class = "am_job")
}

#' Read a job file
#'
#' A job exported from the web interface (Jobs › Export job, or a run's
#' `job.json`).
#'
#' @param file The JSON file.
#' @param analysis Analysis name (see [am_job_new()]).
#' @return An `am_job`.
#' @export
am_job_read <- function(file, analysis) {
  am_job_new(analysis, jsonlite::fromJSON(file, simplifyVector = FALSE), name = basename(file))
}

#' Saved jobs of a project
#'
#' The jobs saved in the web interface (or in the project by
#' `accessmod serve`), e.g. the travel-time job of a project made there.
#'
#' @param project An `am_project`.
#' @return A data frame: `id`, `analysis`, `name`, `revision`.
#' @export
am_jobs <- function(project) {
  .check_project(project)
  rows <- .am_exec(c("project", "jobs", "--project", project$dir, "--json"))
  data.frame(
    id = vapply(rows, function(j) as.integer(j$id), 0L),
    analysis = vapply(rows, function(j) j$analysis, ""),
    name = vapply(rows, function(j) j$name, ""),
    revision = vapply(rows, function(j) as.integer(j$revision), 0L),
    stringsAsFactors = FALSE
  )
}

#' A saved job
#'
#' @param project An `am_project`.
#' @param job The job's identifier (a number) or name.
#' @return An `am_job`.
#' @export
am_job <- function(project, job) {
  .check_project(project)
  rows <- .am_exec(c("project", "jobs", "--project", project$dir, "--json"))
  key <- if (is.numeric(job)) "id" else "name"
  found <- Filter(function(j) identical(as.character(j[[key]]), as.character(job)), rows)
  if (length(found) == 0) stop("no saved job ", job, " in ", project$info$name, call. = FALSE)
  j <- found[[1]]
  structure(list(analysis = j$analysis, name = j$name, id = as.integer(j$id), document = j$document), class = "am_job")
}

#' @export
format.am_job <- function(x, ...) {
  d <- x$document
  lines <- sprintf("<AccessMod job> %s (%s)", x$name, x$analysis)
  if (!is.null(d$scenario)) lines <- c(lines, sprintf("  scenario: %d land-cover classes", length(d$scenario)))
  if (!is.null(d$land_cover)) lines <- c(lines, paste("  land cover:", d$land_cover))
  if (!is.null(d$stack)) lines <- c(lines, sprintf("  stack: %d items", length(d$stack)))
  paste(lines, collapse = "\n")
}

#' @export
print.am_job <- function(x, ...) {
  cat(format(x, ...), "\n", sep = "")
  invisible(x)
}

#' Scenario of a travel-time job
#'
#' The mobility of each land-cover class: a data frame with `class`
#' (integer), `label`, `mode` (`"walking"`, `"bicycling"`, `"motorized"`)
#' and `speed_kmh` (flat-ground speed; 0 makes the class a barrier). Classes
#' not listed are barriers.
#'
#' @param job An `am_job` with a scenario (travel time and the analyses built
#'   on it).
#' @param value The new scenario, a data frame with the same columns
#'   (`label` optional).
#' @return The scenario, or the job with the new scenario.
#' @examples
#' job <- am_job_new("travel-time", list(scenario = list(
#'   list(class = 1, mode = "walking", speed_kmh = 5),
#'   list(class = 1001, mode = "motorized", speed_kmh = 80)
#' )))
#' s <- am_scenario(job)
#' s$speed_kmh[s$class == 1001] <- 60
#' am_scenario(job) <- s
#' @export
am_scenario <- function(job) {
  stopifnot(inherits(job, "am_job"))
  entries <- job$document$scenario
  if (is.null(entries)) stop("job ", job$name, " has no scenario", call. = FALSE)
  str <- function(v) if (is.null(v)) NA_character_ else as.character(v)
  data.frame(
    class = vapply(entries, function(e) as.integer(e$class), 0L),
    label = vapply(entries, function(e) str(e$label), ""),
    mode = vapply(entries, function(e) as.character(e$mode), ""),
    speed_kmh = vapply(entries, function(e) as.numeric(e$speed_kmh), 0),
    stringsAsFactors = FALSE
  )
}

#' @rdname am_scenario
#' @export
`am_scenario<-` <- function(job, value) {
  stopifnot(inherits(job, "am_job"), is.data.frame(value))
  missing_cols <- setdiff(c("class", "mode", "speed_kmh"), names(value))
  if (length(missing_cols) > 0) stop("the scenario needs the columns ", paste(missing_cols, collapse = ", "), call. = FALSE)
  if (anyDuplicated(value$class)) stop("a class appears twice in the scenario", call. = FALSE)
  bad <- !value$mode %in% c("walking", "bicycling", "motorized")
  if (any(bad)) stop("unknown mode: ", paste(unique(value$mode[bad]), collapse = ", "), call. = FALSE)
  if (any(is.na(value$speed_kmh) | value$speed_kmh < 0)) stop("speeds must be numbers >= 0", call. = FALSE)
  job$document$scenario <- lapply(seq_len(nrow(value)), function(i) {
    e <- list(class = as.integer(value$class[i]), mode = value$mode[i], speed_kmh = as.numeric(value$speed_kmh[i]))
    if ("label" %in% names(value) && !is.na(value$label[i]) && nzchar(value$label[i])) e$label <- value$label[i]
    e
  })
  job
}

#' Run a job on a project
#'
#' The run's outputs go to `runs/<id>` in the project and become datasets of
#' the project, tagged with `tags`.
#'
#' @param project An `am_project`.
#' @param job An `am_job`.
#' @param tags Tags of the datasets written (default: the analysis name and
#'   the run). Datasets of the same class and tags are refused unless
#'   `overwrite`.
#' @param overwrite Replace the datasets of the output classes already
#'   tagged so, once the run succeeds.
#' @param memory,threads Memory budget (e.g. `"2G"`) and worker threads;
#'   default: the command line's (half the available memory, every core up to 16).
#' @return An `am_run`: the run record (`id`, `status`, `outcome`,
#'   `output_dir`, `datasets`, `job`), with the project.
#' @export
am_run <- function(project, job, tags = NULL, overwrite = FALSE, memory = NULL, threads = NULL) {
  .check_project(project)
  stopifnot(inherits(job, "am_job"))
  document <- job$document
  # The project sets the output directory (runs/<id>); the schema wants one.
  if (is.null(document$output_dir)) document$output_dir <- "output"
  file <- tempfile(fileext = ".json")
  on.exit(unlink(file))
  jsonlite::write_json(document, file, auto_unbox = TRUE, digits = NA, null = "null", pretty = TRUE)
  args <- c("project", "run", job$analysis, "--job", file, "--project", project$dir, "--json")
  if (length(tags) > 0) args <- c(args, "--tags", paste(tags, collapse = ","))
  if (isTRUE(overwrite)) args <- c(args, "--overwrite")
  if (!is.null(memory)) args <- c(args, "--memory", as.character(memory))
  if (!is.null(threads)) args <- c(args, "--threads", as.character(threads))
  record <- .am_exec(args)
  record$project <- project
  structure(record, class = "am_run")
}

#' @export
format.am_run <- function(x, ...) {
  sprintf("<AccessMod run %s> %s, %s\n  %s\n  outputs: %s", x$id, x$analysis, x$status, x$outcome %||% "", x$output_dir)
}

#' @export
print.am_run <- function(x, ...) {
  cat(format(x, ...), "\n", sep = "")
  invisible(x)
}

`%||%` <- function(a, b) if (is.null(a)) b else a

#' Output datasets of a run
#'
#' @param run An `am_run`.
#' @param class Only this class, e.g. `"rTravelTime"`.
#' @return A data frame as [am_datasets()], restricted to the run's outputs.
#' @export
am_outputs <- function(run, class = NULL) {
  stopifnot(inherits(run, "am_run"))
  all <- am_datasets(run$project, class)
  all[all$id %in% unlist(run$datasets), , drop = FALSE]
}

#' Read an output raster
#'
#' Needs the `terra` package.
#'
#' @param run An `am_run`.
#' @param class The raster's class, e.g. `"rTravelTime"` (seconds),
#'   `"rLandCoverMerged"`.
#' @return A `terra::SpatRaster`.
#' @export
am_raster <- function(run, class) {
  if (!requireNamespace("terra", quietly = TRUE)) stop("reading rasters needs the terra package", call. = FALSE)
  out <- am_outputs(run, class)
  if (nrow(out) == 0) stop("run ", run$id, " wrote no ", class, call. = FALSE)
  terra::rast(out$path[1])
}

#' Merge land cover, then travel time
#'
#' `am_merge_land_cover()` runs a merge-land-cover job. `am_travel_time()`
#' runs a travel-time job, on the merged land cover of `land_cover` when it
#' is given: the merged land cover of each scenario is generated, then used.
#'
#' @param project An `am_project`.
#' @param job An `am_job` of the analysis.
#' @param land_cover For `am_travel_time()`: a merge run (`am_run`) whose
#'   merged land cover replaces the job's, or the path of a merged land cover
#'   relative to the project.
#' @param ... Passed to [am_run()]: `tags`, `overwrite`, `memory`, `threads`.
#' @return The `am_run`.
#' @export
am_merge_land_cover <- function(project, job, ...) {
  if (job$analysis != "merge-land-cover") stop("job ", job$name, " is not a merge-land-cover job", call. = FALSE)
  am_run(project, job, ...)
}

#' @rdname am_merge_land_cover
#' @export
am_travel_time <- function(project, job, land_cover = NULL, ...) {
  if (job$analysis != "travel-time") stop("job ", job$name, " is not a travel-time job", call. = FALSE)
  if (inherits(land_cover, "am_run")) {
    merged <- am_outputs(land_cover, "rLandCoverMerged")
    if (nrow(merged) == 0) stop("run ", land_cover$id, " wrote no merged land cover", call. = FALSE)
    land_cover <- merged$file[1]
  }
  if (!is.null(land_cover)) job$document$land_cover <- land_cover
  am_run(project, job, ...)
}
