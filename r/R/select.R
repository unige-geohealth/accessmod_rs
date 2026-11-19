# The project catalogue as the web interface's Data screen shows it:
# datasets (raster, vector, table) and jobs, a job being the job document of
# a finished analysis, its class the analysis, its tags those the engine
# names it with (`job_tags`: its tags, and its number after an overwrite).
# Selection by type, class, tags and id, with the same rules everywhere.

.TYPES <- c("raster", "vector", "table", "job")

.dataset_type <- function(class) {
  switch(substr(class, 1, 1),
    r = "raster",
    v = "vector",
    t = "table",
    "other"
  )
}

# `datasets`: parsed `dataset list --json`; `runs`: parsed `project runs
# --json`. One row per dataset and per job; `tags` is a list column.
.catalogue <- function(datasets, runs) {
  jobs <- Filter(function(r) identical(r$status, "done") && !identical(r$kind, "manage"), runs)
  rows <- c(
    lapply(datasets, function(d) {
      list(
        id = as.integer(d$id), type = .dataset_type(d$class), class = d$class,
        tags = as.character(unlist(d$tags)), file = d$file,
        run = if (identical(d$origin$type, "run")) as.integer(d$origin$run) else NA_integer_
      )
    }),
    lapply(jobs, function(r) {
      list(
        id = as.integer(r$id), type = "job", class = r$analysis,
        tags = as.character(unlist(if (length(r$job_tags) > 0) r$job_tags else r$tags)),
        file = NA_character_, run = as.integer(r$id)
      )
    })
  )
  out <- data.frame(
    id = vapply(rows, function(r) r$id, 0L),
    type = vapply(rows, function(r) r$type, ""),
    class = vapply(rows, function(r) r$class, ""),
    file = vapply(rows, function(r) r$file, ""),
    run = vapply(rows, function(r) r$run, 0L),
    stringsAsFactors = FALSE
  )
  out$tags <- lapply(rows, function(r) r$tags)
  out[, c("id", "type", "class", "tags", "file", "run")]
}

# Rows of `catalogue` matching every criterion given: `type` and `class`
# exactly, every tag of `tags` among the row's tags (any order), `id`.
.select <- function(catalogue, type = NULL, class = NULL, tags = NULL, id = NULL) {
  if (!is.null(type) && !type %in% .TYPES) {
    .am_stop(paste0("unknown type \"", type, "\": one of ", paste(.TYPES, collapse = ", ")))
  }
  keep <- rep(TRUE, nrow(catalogue))
  if (!is.null(type)) keep <- keep & catalogue$type == type
  if (!is.null(class)) keep <- keep & catalogue$class == class
  if (!is.null(id)) {
    id <- .whole(id, "id")
    if (length(id) != 1) .am_stop("give one id")
    keep <- keep & catalogue$id == id
  }
  if (length(tags) > 0) {
    tags <- as.character(tags)
    keep <- keep & vapply(catalogue$tags, function(t) all(tags %in% t), TRUE)
  }
  out <- catalogue[keep, , drop = FALSE]
  rownames(out) <- NULL
  out
}

.describe <- function(rows) {
  paste0(rows$type, " ", rows$id, " (", rows$class, ": ", vapply(rows$tags, paste, "", collapse = " "), ")")
}

# The one row matching, or an error naming the criteria (none) or the
# candidates (several). Among several, the one whose tags are exactly those
# given wins: after an overwrite, "walking" names the job tagged "walking",
# not "walking 7".
.select_one <- function(catalogue, type, class = NULL, tags = NULL, id = NULL) {
  found <- .select(catalogue, type, class, tags, id)
  if (nrow(found) > 1 && length(tags) > 0) {
    exact <- vapply(found$tags, function(t) setequal(t, as.character(tags)), TRUE)
    if (sum(exact) == 1) found <- found[exact, , drop = FALSE]
  }
  criteria <- paste(c(
    paste("type", type),
    if (!is.null(class)) paste("class", class),
    if (length(tags) > 0) paste("tags", paste(tags, collapse = " ")),
    if (!is.null(id)) paste("id", id)
  ), collapse = ", ")
  if (nrow(found) == 0) .am_stop(paste0("nothing matches ", criteria), kind = "not_found")
  if (nrow(found) > 1) {
    .am_stop(paste0(nrow(found), " match ", criteria, "; give class, more tags or id:\n", paste0("  - ", .describe(found), collapse = "\n")),
      kind = "ambiguous"
    )
  }
  found
}
