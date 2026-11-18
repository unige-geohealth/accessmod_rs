# The project session: one R6 object per project directory. It holds the
# project path and the engine, nothing else: every method reads the project,
# so two sessions (or R and the web interface) can share a project.

#' Open an AccessMod project
#'
#' The session object through which everything is done: `am$list_data()`,
#' `am$get()`, `am$job()`, `am$new_scenario()`, `am$run()`, … (type `am$`
#' and press Tab in RStudio).
#'
#' @param path Project directory (default: the working directory). With
#'   `import` or `demo`, the new project directory; it must not exist.
#' @param import A project file (`.am6p`, Project › Export in the web
#'   interface) or an AccessMod 5 data archive (`.zip`, Data manager ›
#'   Archive) imported as the project `path`.
#' @param demo Make the AccessMod 5 sample project at `path`.
#' @param binary The engine to use (default: [am_binary()]).
#' @return An `AccessMod` session.
#' @examples
#' \dontrun{
#' am <- am_init("~/am/demo", import = "demo.am6p")
#' am$list_data(type = "job")
#' tt <- am$get("job", class = "travel-time", tags = "test")
#' faster <- am$new_scenario(tt, data.frame(class = 1001, speed_kmh = 100))
#' run <- am$run(faster, tags = c("faster"))
#' run$get_output("rTravelTime")$read()
#' }
#' @export
am_init <- function(path = getwd(), import = NULL, demo = FALSE, binary = NULL) {
  AccessMod$new(path, import = import, demo = demo, binary = binary)
}

.engine <- function(binary) {
  bin <- tryCatch(am_binary(binary), accessmod_not_installed = function(e) NULL)
  if (is.null(bin) && interactive()) {
    answer <- utils::askYesNo(paste0("The AccessMod engine (", am_engine_version(), ", about 20 MB) is not installed. Install it now?"))
    if (isTRUE(answer)) bin <- am_install()
  }
  if (is.null(bin)) bin <- am_binary(binary)
  .check_engine(bin)
  bin
}

.new_dir <- function(path) {
  if (file.exists(path)) .am_stop(paste(path, "already exists: give a new directory"))
  parent <- dirname(path)
  dir.create(parent, recursive = TRUE, showWarnings = FALSE)
  file.path(normalizePath(parent), basename(path))
}

#' The AccessMod project session
#'
#' Made by [am_init()].
#'
#' @section Fields:
#' `path` (project directory), `name`, `info` (name, description, grid),
#' `binary` (the engine).
#' @section Methods:
#' \describe{
#'   \item{`list_data(type = NULL, class = NULL, tags = NULL)`}{The
#'     catalogue as in Data: `id`, `type` (raster, vector, table, job),
#'     `class` (a class code, or the analysis for jobs), `tags` (a list
#'     column), `file`, `run`. A job is the job of a finished analysis run;
#'     its `id` is the run's.}
#'   \item{`list_runs()`}{Runs: `id`, `analysis`, `status`, `tags`,
#'     `outcome`, `output_dir`.}
#'   \item{`get(type, class = NULL, tags = NULL, id = NULL)`}{The one
#'     Job or Dataset matching: every given tag among its tags, in any order.
#'     None or several: an error listing the candidates.}
#'   \item{`job(analysis, ...)`}{A new Job: `...` are the fields of the job
#'     document (`accessmod <analysis> schema`), Datasets for their paths;
#'     `schema_version` is set.}
#'   \item{`new_scenario(job, speeds)`}{A new Job with the speeds of the
#'     classes in `speeds` (a data frame: `class`, `speed_kmh`, optionally
#'     `mode`, `label`) changed; every class must be in the job's scenario.}
#'   \item{`import_data(file, class, tags, layer = NULL)`}{Import a file as
#'     a dataset of the class (e.g. `"vBarrier"`); returns the Dataset.}
#'   \item{`run(job, tags = NULL, overwrite = FALSE, memory = NULL, threads
#'     = NULL)`}{Run a job; outputs go to `runs/<id>` and become datasets
#'     tagged `tags` (default: the analysis and run). Tags already used by
#'     the output classes are refused unless `overwrite`. Returns the Run.}
#'   \item{`export(dir, results = TRUE)`}{Write a project file (`.am6p`) in
#'     `dir`; returns its path (engine 6.0.0-alpha.2 or later).}
#' }
#' @name AccessMod
NULL

AccessMod <- R6::R6Class("AccessMod",
  cloneable = FALSE,
  public = list(
    initialize = function(path = getwd(), import = NULL, demo = FALSE, binary = NULL) {
      private$.bin <- .engine(binary)
      if (!is.null(import)) {
        if (isTRUE(demo)) .am_stop("give import or demo, not both")
        file <- normalizePath(import, mustWork = TRUE)
        target <- .new_dir(path)
        out <- private$exec(c("project", "import", file, target, "--json"))
        message(out$summary)
        path <- target
      } else if (isTRUE(demo)) {
        target <- .new_dir(path)
        private$exec(c("project", "demo", target, "--json"))
        path <- target
      }
      private$.path <- normalizePath(path, mustWork = TRUE)
      private$.info <- private$exec(c("project", "info", "--project", private$.path, "--json"))$project
    },
    list_data = function(type = NULL, class = NULL, tags = NULL) {
      .select(private$catalogue(), type, class, tags)
    },
    list_runs = function() {
      runs <- private$runs()
      text <- function(v) if (is.null(v)) NA_character_ else as.character(v)
      out <- data.frame(
        id = vapply(runs, function(r) as.integer(r$id), 0L),
        analysis = vapply(runs, function(r) r$analysis, ""),
        status = vapply(runs, function(r) text(r$status), ""),
        outcome = vapply(runs, function(r) text(r$outcome), ""),
        output_dir = vapply(runs, function(r) text(r$output_dir), ""),
        stringsAsFactors = FALSE
      )
      out$tags <- lapply(runs, function(r) as.character(unlist(r$tags)))
      out[, c("id", "analysis", "status", "tags", "outcome", "output_dir")]
    },
    get = function(type, class = NULL, tags = NULL, id = NULL) {
      runs <- if (identical(type, "job")) private$runs() else NULL
      catalogue <- if (is.null(runs)) private$catalogue() else .catalogue(list(), runs)
      row <- .select_one(catalogue, type, class, tags, id)
      if (type != "job") return(Dataset$new(row, private$.path))
      record <- Filter(function(r) as.integer(r$id) == row$id, runs)[[1]]
      Job$new(record$analysis, record$job, private$.path, run = row$id, tags = row$tags[[1]])
    },
    job = function(analysis, ...) {
      fields <- list(...)
      if (length(fields) > 0 && (is.null(names(fields)) || any(names(fields) == ""))) {
        .am_stop("am$job() takes named fields of the job document")
      }
      document <- c(list(schema_version = 1L), .resolve(fields, private$.path))
      Job$new(analysis, document, private$.path)
    },
    new_scenario = function(job, speeds) {
      private$own(job)
      Job$new(job$analysis, .with_speeds(job$document(), speeds, job$analysis), private$.path)
    },
    import_data = function(file, class, tags, layer = NULL) {
      args <- c("dataset", "import", normalizePath(file, mustWork = TRUE), "--class", class, "--tags", paste(tags, collapse = ","), "--project", private$.path, "--json")
      if (!is.null(layer)) args <- c(args, "--layer", layer)
      record <- private$exec(args)
      self$get(.dataset_type(class), id = record$id)
    },
    run = function(job, tags = NULL, overwrite = FALSE, memory = NULL, threads = NULL) {
      private$own(job)
      document <- job$document()
      # The project chooses the output directory (runs/<id>); the schema
      # wants one.
      if (is.null(document$output_dir)) document$output_dir <- "output"
      file <- tempfile(fileext = ".json")
      on.exit(unlink(file))
      writeLines(.to_json(document), file, useBytes = TRUE)
      args <- c("project", "run", job$analysis, "--job", file, "--project", private$.path, "--json")
      if (length(tags) > 0) args <- c(args, "--tags", paste(tags, collapse = ","))
      if (isTRUE(overwrite)) args <- c(args, "--overwrite")
      if (!is.null(memory)) args <- c(args, "--memory", as.character(memory))
      if (!is.null(threads)) args <- c(args, "--threads", as.character(threads))
      Run$new(private$exec(args), self)
    },
    export = function(dir, results = TRUE) {
      dir.create(dir, recursive = TRUE, showWarnings = FALSE)
      args <- c("project", "export", "--project", private$.path, "--dir", normalizePath(dir), "--json")
      if (!isTRUE(results)) args <- c(args, "--no-results")
      private$exec(args)$file
    },
    print = function(...) {
      grid <- private$.info$grid
      data <- private$catalogue()
      cat("<AccessMod project> ", private$.info$name, "\n  ", private$.path, "\n", sep = "")
      cat("  grid: ", grid$cols, " x ", grid$rows, " cells of ", grid$geo_transform[[2]], " m\n", sep = "")
      counts <- table(factor(data$type, levels = .TYPES))
      cat("  data: ", paste(counts, names(counts), collapse = ", "), " ($list_data())\n", sep = "")
      cat("  engine: ", am_version(private$.bin), "\n", sep = "")
      invisible(self)
    }
  ),
  active = list(
    path = function(value) .read_only(value, private$.path),
    name = function(value) .read_only(value, private$.info$name),
    info = function(value) .read_only(value, private$.info),
    binary = function(value) .read_only(value, private$.bin)
  ),
  private = list(
    .path = NULL, .info = NULL, .bin = NULL,
    exec = function(args) .exec_json(private$.bin, args),
    runs = function() private$exec(c("project", "runs", "--project", private$.path, "--json")),
    catalogue = function() {
      .catalogue(private$exec(c("dataset", "list", "--project", private$.path, "--json")), private$runs())
    },
    own = function(job) {
      if (!inherits(job, "AccessModJob")) .am_stop("expected a Job (am$get(\"job\", ...), am$job() or am$new_scenario())")
      if (!identical(job$project, private$.path)) .am_stop(paste("this job belongs to another project:", job$project))
    }
  )
)
