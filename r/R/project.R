# Projects: a directory with a grid, datasets, saved jobs and runs. The R
# object is the path and the project's metadata; everything else is read
# from the project when asked.

.project <- function(dir) {
  dir <- normalizePath(dir, mustWork = TRUE)
  info <- .am_exec(c("project", "info", "--project", dir, "--json"))
  structure(list(dir = dir, info = info$project), class = "am_project")
}

#' Open an AccessMod 6 project
#'
#' @param dir Project directory, e.g. one made by [am_project_import()] or by
#'   the web interface (`~/AccessMod/<project>`).
#' @return An `am_project`: the directory and the project's name, grid and
#'   description.
#' @export
am_project <- function(dir) .project(dir)

#' Import a project
#'
#' An AccessMod 6 project file (`.am6p`, Project › Export in the web
#' interface) or an AccessMod 5 data archive (Data manager › Archive, `.zip`)
#' becomes a new project directory. Nothing is left on failure.
#'
#' @param file The project file or archive.
#' @param dir The new project directory; it must not exist.
#' @param name Project name (default: the one in the file, or the archive's
#'   file name).
#' @return The `am_project`.
#' @export
am_project_import <- function(file, dir, name = "") {
  file <- normalizePath(file, mustWork = TRUE)
  if (file.exists(dir)) stop(dir, " already exists", call. = FALSE)
  parent <- dirname(dir)
  dir.create(parent, recursive = TRUE, showWarnings = FALSE)
  target <- file.path(normalizePath(parent), basename(dir))
  args <- c("project", "import", file, target, "--json")
  if (nzchar(name)) args <- c(args, "--name", name)
  out <- .am_exec(args)
  message(out$summary)
  .project(target)
}

#' Create the demo project
#'
#' The AccessMod 5 sample project (Switzerland, small), datasets tagged
#' `demo`: a quick way to try the package.
#'
#' @param dir The new project directory; it must not exist.
#' @return The `am_project`.
#' @export
am_project_demo <- function(dir) {
  if (file.exists(dir)) stop(dir, " already exists", call. = FALSE)
  parent <- dirname(dir)
  dir.create(parent, recursive = TRUE, showWarnings = FALSE)
  target <- file.path(normalizePath(parent), basename(dir))
  .am_exec(c("project", "demo", target, "--json"))
  .project(target)
}

#' @export
format.am_project <- function(x, ...) {
  grid <- x$info$grid
  sprintf(
    "<AccessMod project> %s\n  %s\n  grid: %s x %s cells of %s m",
    x$info$name, x$dir, grid$cols, grid$rows, grid$geo_transform[[2]]
  )
}

#' @export
print.am_project <- function(x, ...) {
  cat(format(x, ...), "\n", sep = "")
  invisible(x)
}

.check_project <- function(project) {
  if (!inherits(project, "am_project")) stop("`project` must be an am_project (see am_project())", call. = FALSE)
}

#' Datasets of a project
#'
#' @param project An `am_project`.
#' @param class Only this AccessMod class, e.g. `"rLandCoverMerged"`,
#'   `"vFacility"`, `"rTravelTime"`.
#' @return A data frame: `id`, `class`, `tags` (space separated), `file`
#'   (relative to the project), `path` (absolute), and for rasters `mean`,
#'   `min`, `max` and `valid_cells`.
#' @export
am_datasets <- function(project, class = NULL) {
  .check_project(project)
  args <- c("dataset", "list", "--project", project$dir, "--json")
  if (!is.null(class)) args <- c(args, "--class", class)
  rows <- .am_exec(args)
  num <- function(v) if (is.null(v)) NA_real_ else as.numeric(v)
  out <- data.frame(
    id = vapply(rows, function(d) as.integer(d$id), 0L),
    class = vapply(rows, function(d) d$class, ""),
    tags = vapply(rows, function(d) paste(unlist(d$tags), collapse = " "), ""),
    file = vapply(rows, function(d) d$file, ""),
    mean = vapply(rows, function(d) num(d$summary$stats$mean), 0),
    min = vapply(rows, function(d) num(d$summary$stats$min), 0),
    max = vapply(rows, function(d) num(d$summary$stats$max), 0),
    valid_cells = vapply(rows, function(d) num(d$summary$stats$valid_cells), 0),
    stringsAsFactors = FALSE
  )
  out$path <- file.path(project$dir, out$file)
  out
}

#' Runs of a project
#'
#' @param project An `am_project`.
#' @return A data frame: `id`, `analysis`, `status`, `tags`, `outcome`,
#'   `output_dir`.
#' @export
am_runs <- function(project) {
  .check_project(project)
  rows <- .am_exec(c("project", "runs", "--project", project$dir, "--json"))
  str <- function(v) if (is.null(v)) NA_character_ else as.character(v)
  data.frame(
    id = vapply(rows, function(r) as.integer(r$id), 0L),
    analysis = vapply(rows, function(r) r$analysis, ""),
    status = vapply(rows, function(r) str(r$status), ""),
    tags = vapply(rows, function(r) paste(unlist(r$tags), collapse = " "), ""),
    outcome = vapply(rows, function(r) str(r$outcome), ""),
    output_dir = vapply(rows, function(r) r$output_dir, ""),
    stringsAsFactors = FALSE
  )
}
