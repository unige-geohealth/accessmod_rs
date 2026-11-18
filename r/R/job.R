# Jobs: an analysis and its job document (the JSON the command line and the
# web interface share; paths relative to the project). A Job is immutable:
# `$with()` and `am$new_scenario()` return new jobs, so a baseline stays as
# it was however many variants a loop derives from it, aliases included.

.MODES <- c("walking", "bicycling", "motorized")

#' An AccessMod job (immutable)
#'
#' Made by `am$get("job", ...)` (the job of a run, as in Data › Job),
#' `am$job()` or `am$new_scenario()`; run by `am$run()`. Fields are read
#' only; `$with()` returns a changed copy.
#'
#' @section Fields:
#' \describe{
#'   \item{`analysis`}{Analysis name, e.g. `"travel-time"`.}
#'   \item{`run`}{The run it comes from, or `NA`.}
#'   \item{`tags`}{The tags of that run.}
#'   \item{`project`}{The project directory it belongs to.}
#' }
#' @section Methods:
#' \describe{
#'   \item{`document()`}{The job document, a list (a copy).}
#'   \item{`scenario()`}{The scenario: a data frame `class`, `label`, `mode`,
#'     `speed_kmh`.}
#'   \item{`with(...)`}{A new job with fields of the document set (`NULL`
#'     removes one). A Dataset gives its path: for a field such as
#'     `facilities` (an object with a `path`), its `path`. Datasets inside
#'     lists (e.g. a merge `stack`) give their paths too.}
#' }
#' @name Job
NULL

Job <- R6::R6Class("AccessModJob",
  cloneable = FALSE,
  public = list(
    initialize = function(analysis, document, project, run = NA_integer_, tags = character()) {
      private$.analysis <- analysis
      private$.document <- document
      private$.project <- project
      private$.run <- as.integer(run)
      private$.tags <- as.character(tags)
    },
    document = function() private$.document,
    scenario = function() .scenario_frame(private$.document, private$.analysis),
    with = function(...) {
      changes <- list(...)
      if (length(changes) == 0 || is.null(names(changes)) || any(names(changes) == "")) {
        .am_stop("$with() takes named fields of the job document, e.g. $with(land_cover = merged)")
      }
      document <- private$.document
      for (field in names(changes)) {
        value <- changes[[field]]
        current <- document[[field]]
        if (inherits(value, "AccessModDataset") && is.list(current) && !is.null(current$path)) {
          current$path <- .dataset_path(value, private$.project)
          document[[field]] <- current
        } else if (is.null(value)) {
          document[[field]] <- NULL
        } else {
          document[[field]] <- .resolve(value, private$.project)
        }
      }
      Job$new(private$.analysis, document, private$.project)
    },
    print = function(...) {
      d <- private$.document
      cat("<AccessMod job> ", private$.analysis, sep = "")
      if (!is.na(private$.run)) cat(" of run ", private$.run, " [", paste(private$.tags, collapse = " "), "]", sep = "")
      cat("\n")
      for (field in intersect(c("dem", "land_cover", "population", "facilities", "zones"), names(d))) {
        value <- d[[field]]
        cat("  ", field, ": ", if (is.list(value)) value$path else value, "\n", sep = "")
      }
      if (!is.null(d$stack)) cat("  stack: ", length(d$stack), " items\n", sep = "")
      if (!is.null(d$scenario)) cat("  scenario: ", length(d$scenario), " classes ($scenario())\n", sep = "")
      invisible(self)
    }
  ),
  active = list(
    analysis = function(value) .read_only(value, private$.analysis),
    run = function(value) .read_only(value, private$.run),
    tags = function(value) .read_only(value, private$.tags),
    project = function(value) .read_only(value, private$.project)
  ),
  private = list(.analysis = NULL, .document = NULL, .project = NULL, .run = NULL, .tags = NULL)
)

.read_only <- function(value, current) {
  if (!missing(value)) .am_stop("AccessMod objects are read only: use $with() or am$new_scenario() for a changed job")
  current
}

# Datasets anywhere in `value` become their paths (relative to the project).
.resolve <- function(value, project) {
  if (inherits(value, "AccessModDataset")) return(.dataset_path(value, project))
  if (inherits(value, "R6")) .am_stop("only Datasets can go into a job document")
  if (is.list(value)) {
    out <- lapply(value, .resolve, project = project)
    names(out) <- names(value)
    return(out)
  }
  value
}

.dataset_path <- function(dataset, project) {
  if (!identical(dataset$project, project)) {
    .am_stop(paste0("dataset ", dataset$id, " belongs to another project (", dataset$project, ")"))
  }
  dataset$file
}

.scenario_frame <- function(document, analysis) {
  entries <- document$scenario
  if (is.null(entries)) .am_stop(paste("this", analysis, "job has no scenario"))
  text <- function(v) if (is.null(v)) NA_character_ else as.character(v)
  data.frame(
    class = vapply(entries, function(e) as.integer(e$class), 0L),
    label = vapply(entries, function(e) text(e$label), ""),
    mode = vapply(entries, function(e) as.character(e$mode), ""),
    speed_kmh = vapply(entries, function(e) as.numeric(e$speed_kmh), 0),
    stringsAsFactors = FALSE
  )
}

# The scenario of `document` with the classes of `speeds` changed: columns
# `class` and `speed_kmh`, optionally `mode` and `label`. Every class must be
# in the scenario once; speeds finite and >= 0.
.with_speeds <- function(document, speeds, analysis) {
  if (!is.data.frame(speeds) || !all(c("class", "speed_kmh") %in% names(speeds))) {
    .am_stop("speeds must be a data frame with the columns class and speed_kmh (optionally mode, label)")
  }
  current <- .scenario_frame(document, analysis)
  classes <- as.integer(speeds$class)
  if (anyNA(classes) || anyDuplicated(classes)) .am_stop("each class must be given once, as a whole number")
  unknown <- setdiff(classes, current$class)
  if (length(unknown) > 0) .am_stop(paste("classes not in the scenario:", paste(unknown, collapse = ", ")))
  speed <- as.numeric(speeds$speed_kmh)
  if (any(!is.finite(speed) | speed < 0)) .am_stop("speeds must be finite numbers >= 0 (0 makes a barrier)")
  if ("mode" %in% names(speeds)) {
    bad <- setdiff(speeds$mode, .MODES)
    if (length(bad) > 0) .am_stop(paste0("unknown mode: ", paste(bad, collapse = ", "), "; one of ", paste(.MODES, collapse = ", ")))
  }
  entries <- document$scenario
  for (i in seq_along(classes)) {
    k <- which(current$class == classes[i])
    entries[[k]]$speed_kmh <- speed[i]
    if ("mode" %in% names(speeds)) entries[[k]]$mode <- as.character(speeds$mode[i])
    if ("label" %in% names(speeds) && !is.na(speeds$label[i])) entries[[k]]$label <- as.character(speeds$label[i])
  }
  document$scenario <- entries
  document
}
