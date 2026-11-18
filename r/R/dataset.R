# Datasets and runs, as the project records them. Both are read only.

#' An AccessMod dataset
#'
#' A raster, vector or table of the project, from `am$get()`,
#' `am$import_data()` or `run$get_output()`. Give it to `am$job()` or
#' `job$with()` to use it in a job.
#'
#' @section Fields:
#' `id`, `type` (raster, vector, table), `class` (e.g. `"rTravelTime"`),
#' `tags`, `file` (relative to the project), `run` (the run that wrote it, or
#' `NA`), `project`.
#' @section Methods:
#' \describe{
#'   \item{`path()`}{The absolute path of its file.}
#'   \item{`read()`}{Read it: rasters and vectors with `terra` (installed
#'     separately), tables as a data frame.}
#' }
#' @name Dataset
NULL

Dataset <- R6::R6Class("AccessModDataset",
  cloneable = FALSE,
  public = list(
    initialize = function(row, project) {
      private$.row <- row
      private$.project <- project
    },
    path = function() file.path(private$.project, private$.row$file),
    read = function() {
      type <- private$.row$type
      if (type == "table") return(utils::read.csv(self$path(), stringsAsFactors = FALSE, check.names = FALSE))
      if (!requireNamespace("terra", quietly = TRUE)) .am_stop("reading rasters and vectors needs the terra package: install.packages(\"terra\")")
      if (type == "raster") terra::rast(self$path()) else terra::vect(self$path())
    },
    print = function(...) {
      r <- private$.row
      cat("<AccessMod dataset ", r$id, "> ", r$class, " [", paste(r$tags[[1]], collapse = " "), "] ", r$file, "\n", sep = "")
      invisible(self)
    }
  ),
  active = list(
    id = function(value) .read_only(value, private$.row$id),
    type = function(value) .read_only(value, private$.row$type),
    class = function(value) .read_only(value, private$.row$class),
    tags = function(value) .read_only(value, private$.row$tags[[1]]),
    file = function(value) .read_only(value, private$.row$file),
    run = function(value) .read_only(value, private$.row$run),
    project = function(value) .read_only(value, private$.project)
  ),
  private = list(.row = NULL, .project = NULL)
)

#' An AccessMod run
#'
#' The record of a run, from `am$run()`.
#'
#' @section Fields:
#' `id`, `analysis`, `status` (`"done"`), `outcome` (one line), `tags`,
#' `output_dir`, `datasets` (ids of the datasets it wrote), `record` (the
#' whole record, a list).
#' @section Methods:
#' \describe{
#'   \item{`list_outputs(class = NULL)`}{Its output datasets, as
#'     `am$list_data()`.}
#'   \item{`get_output(class)`}{The one output of that class, a Dataset.}
#' }
#' @name Run
NULL

Run <- R6::R6Class("AccessModRun",
  cloneable = FALSE,
  public = list(
    initialize = function(record, session) {
      private$.record <- record
      private$.session <- session
    },
    list_outputs = function(class = NULL) {
      data <- private$.session$list_data(class = class)
      data[data$type != "job" & data$id %in% self$datasets, , drop = FALSE]
    },
    get_output = function(class) {
      out <- self$list_outputs(class)
      if (nrow(out) == 1) return(Dataset$new(out, private$.session$path))
      if (nrow(out) > 1) .am_stop(paste0("run ", self$id, " wrote several ", class, ": ", paste(out$id, collapse = ", ")), kind = "ambiguous")
      missing <- setdiff(self$datasets, private$.session$list_data()$id)
      .am_stop(paste0(
        "run ", self$id, " has no output ", class, " in this project",
        if (length(missing) > 0) " (its outputs are missing: a project file exported without results?)" else ""
      ), kind = "not_found")
    },
    print = function(...) {
      cat("<AccessMod run ", self$id, "> ", self$analysis, ", ", self$status, " [", paste(self$tags, collapse = " "), "]\n", sep = "")
      if (!is.null(private$.record$outcome)) cat("  ", private$.record$outcome, "\n", sep = "")
      cat("  outputs: ", length(self$datasets), " datasets ($list_outputs())\n", sep = "")
      invisible(self)
    }
  ),
  active = list(
    id = function(value) .read_only(value, as.integer(private$.record$id)),
    analysis = function(value) .read_only(value, private$.record$analysis),
    status = function(value) .read_only(value, private$.record$status),
    outcome = function(value) .read_only(value, if (is.null(private$.record$outcome)) NA_character_ else private$.record$outcome),
    tags = function(value) .read_only(value, as.character(unlist(private$.record$tags))),
    output_dir = function(value) .read_only(value, private$.record$output_dir),
    datasets = function(value) .read_only(value, as.integer(unlist(private$.record$datasets))),
    record = function(value) .read_only(value, private$.record)
  ),
  private = list(.record = NULL, .session = NULL)
)
