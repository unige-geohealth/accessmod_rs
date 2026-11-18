# The boundary with the command line: run it, parse its JSON, turn its
# failures into classed R errors. Plain functions, tested as such.

# An `accessmod_error` condition: `kind` (the CLI's error kind, or a kind of
# this package), `status` (exit status) and `issues`.
.am_stop <- function(message, kind = "invalid", status = NA_integer_, issues = character()) {
  stop(structure(
    class = c(paste0("accessmod_", kind), "accessmod_error", "error", "condition"),
    list(message = message, call = NULL, kind = kind, status = status, issues = issues)
  ))
}

# Run `bin` with `args` (each quoted for the shell) and parse the JSON it
# prints. A failure raises an `accessmod_error` with the issues of its JSON
# error, else the last lines of stderr.
.exec_json <- function(bin, args) {
  err <- tempfile()
  on.exit(unlink(err))
  out <- suppressWarnings(system2(bin, shQuote(args), stdout = TRUE, stderr = err))
  status <- attr(out, "status")
  text <- paste(out, collapse = "\n")
  if (!is.null(status) && status != 0) {
    parsed <- tryCatch(jsonlite::fromJSON(text, simplifyVector = FALSE), error = function(e) NULL)
    issues <- as.character(unlist(parsed$issues))
    if (length(issues) == 0) {
      log <- readLines(err, warn = FALSE)
      issues <- grep("error|cancelled", log, value = TRUE, ignore.case = TRUE)
      if (length(issues) == 0) issues <- utils::tail(log, 5)
    }
    .am_stop(
      paste(c(sprintf("accessmod %s failed:", args[1]), paste0("  - ", issues)), collapse = "\n"),
      kind = if (is.null(parsed$kind)) "engine" else parsed$kind, status = as.integer(status), issues = issues
    )
  }
  if (!nzchar(text)) return(NULL)
  jsonlite::fromJSON(text, simplifyVector = FALSE)
}

# A job document as JSON: lists stay arrays (even of one element), length-1
# atomic vectors are scalars, numbers keep full precision.
.to_json <- function(document) {
  jsonlite::toJSON(document, auto_unbox = TRUE, digits = NA, null = "null", pretty = TRUE)
}

.from_json <- function(text) jsonlite::fromJSON(text, simplifyVector = FALSE)
