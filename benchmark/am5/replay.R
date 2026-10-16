# Runs inside the AccessMod 5 image (see am5_replay in benchmark/lib.sh):
# loads global.R, then runs the replay configuration given as argument
# $REPEAT times (lib.sh uses 1: repeats in one session hit AccessMod 5 caches)
# on the image's demo project (AccessMod 5 globals, such as
# `config`, must not be overwritten). Writes the seconds spent
# loading global.R to /out/load.txt and those of each replay (analysis and
# writing its outputs to the project) to /out/seconds.txt.
started <- Sys.time()
source("global.R")
writeLines(sprintf("%.2f", as.numeric(Sys.time() - started, units = "secs")), "/out/load.txt")

replay_config <- commandArgs(trailingOnly = TRUE)[1]
runs <- as.integer(Sys.getenv("REPEAT", "3"))
seconds <- vapply(seq_len(runs), function(k) {
  conf <- amAnalysisReplayParseConf(replay_config)
  started <- Sys.time()
  amAnalysisReplayExec(conf)
  as.numeric(Sys.time() - started, units = "secs")
}, numeric(1))
writeLines(sprintf("%.2f", seconds), "/out/seconds.txt")
