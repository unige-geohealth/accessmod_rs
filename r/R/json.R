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

# Run `bin` with `args` (each quoted for the shell) and return the JSON it
# prints, parsed. Every failure is an `accessmod_error`: the program cannot
# start; it exits non-zero (the issues of its JSON error, the failed checks
# of `doctor`, else the end of stderr); it prints no JSON.
.exec_json <- function(bin, args) {
  err <- tempfile()
  on.exit(unlink(err))
  out <- tryCatch(
    suppressWarnings(system2(bin, shQuote(args), stdout = TRUE, stderr = err)),
    error = function(e) .am_stop(paste0("cannot run the AccessMod engine (", bin, "): ", conditionMessage(e)), kind = "launch")
  )
  status <- attr(out, "status")
  text <- paste(out, collapse = "\n")
  parsed <- if (nzchar(text)) tryCatch(.from_json(text), error = function(e) NULL) else NULL
  if (!is.null(status) && status != 0) {
    issues <- .issues(parsed)
    if (length(issues) == 0) {
      log <- if (file.exists(err)) readLines(err, warn = FALSE) else character()
      issues <- grep("error|cancelled", log, value = TRUE, ignore.case = TRUE)
      if (length(issues) == 0) issues <- utils::tail(log, 5)
    }
    .am_stop(
      paste(c(sprintf("accessmod %s failed:", args[1]), paste0("  - ", issues)), collapse = "\n"),
      kind = if (is.character(parsed$kind)) parsed$kind else "engine", status = as.integer(status), issues = issues
    )
  }
  if (is.null(parsed)) {
    .am_stop(sprintf("accessmod %s printed no JSON: %s", args[1], substr(text, 1, 200)), kind = "engine")
  }
  parsed
}

# The issues of a JSON error, or the failed checks of `doctor --json`.
.issues <- function(parsed) {
  if (!is.list(parsed)) return(character())
  issues <- as.character(unlist(parsed$issues))
  if (length(issues) == 0 && is.list(parsed$checks)) {
    failed <- Filter(function(c) !isTRUE(c$ok), parsed$checks)
    issues <- vapply(failed, function(c) paste0(c$name, ": ", paste(c$detail, collapse = " ")), "")
  }
  issues
}

# A job document as JSON: lists stay arrays (even of one element), length-1
# atomic vectors are scalars (`I()` keeps one as an array), doubles keep
# their exact value (17 significant digits).
.to_json <- function(document) {
  jsonlite::toJSON(document, auto_unbox = TRUE, digits = I(17), null = "null", pretty = TRUE)
}

.from_json <- function(text) jsonlite::fromJSON(text, simplifyVector = FALSE)

# Whether `value` holds JSON values only: NULL, logical, number, string,
# lists of them (no function, environment, R6 object, factor).
.is_json_value <- function(value) {
  if (is.null(value)) return(TRUE)
  if (is.factor(value) || is.environment(value) || is.function(value)) return(FALSE)
  if (is.list(value)) return(all(vapply(value, .is_json_value, TRUE)))
  is.logical(value) || is.numeric(value) || is.character(value)
}

# Whole numbers as integers; anything else (1.9, a factor, NA) is refused.
.whole <- function(value, what) {
  if (is.factor(value) || !is.numeric(value) || anyNA(value) || any(!is.finite(value)) || any(value != round(value))) {
    .am_stop(paste(what, "must be whole numbers"))
  }
  as.integer(value)
}
